//! `orr`: the Orrery command line.
//!
//! A thin ERP client for shell agents (Claude Code, Codex, scripts, CI) and
//! people. It holds no scene and runs no simulation: every command is one or
//! a few calls to a running host (the editor started with `--erp`, or the
//! headless `orr_remote_host`), through the same call logic, texts and op
//! format the MCP adapter `orr_mcp` uses. `orr help` documents the commands.
//!
//! Exit codes: 0 ok, 1 the command failed (an ERP error), 2 usage error,
//! 3 connection or authentication error, 4 checks failed (`verify`, `apply`).

mod args;
mod cmds;
mod help;
mod ops;

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use orr_mcp::tools::Out;
use orr_mcp::{Bridge, Style};
use serde_json::{json, Map, Value as J};

/// Why a command did not succeed; decides the exit code.
#[derive(Debug)]
pub enum CliErr {
    /// Exit 2: the command line is wrong.
    Usage(String),
    /// Exit 1: the host answered with an error (or the request cannot be done).
    Erp(String),
    /// Exit 3: the host could not be reached, or refused the token.
    Conn(String),
    /// Exit 4: checks failed. The report was already printed.
    Checks,
}

impl CliErr {
    fn code(&self) -> u8 {
        match self {
            CliErr::Erp(_) => 1,
            CliErr::Usage(_) => 2,
            CliErr::Conn(_) => 3,
            CliErr::Checks => 4,
        }
    }
}

/// The connection and output settings of one run.
pub struct Ctx {
    pub bridge: Bridge,
    pub json: bool,
    pub url: String,
    pub token: Option<String>,
}

impl Ctx {
    /// One ERP call.
    pub fn call(&mut self, method: &str, params: J) -> Result<J, CliErr> {
        self.bridge.call(method, params).map_err(|f| self.fail(f.0))
    }

    /// One MCP-tool-shaped operation (shared with `orr_mcp`), with `args` as its arguments.
    pub fn tool(&mut self, name: &str, args: J) -> Result<Out, CliErr> {
        let empty = Map::new();
        let map = args.as_object().unwrap_or(&empty);
        orr_mcp::tools::run(&mut self.bridge, name, map).map_err(|f| self.fail(f.0))
    }

    /// A failed call: a connection error (exit 3) if the host could not be reached, else an ERP error (exit 1).
    pub fn fail(&self, text: String) -> CliErr {
        match self.bridge.connection_error() {
            Some(why) => self.conn_err(why),
            None => CliErr::Erp(text),
        }
    }

    /// The connection error (exit 3), with how to start a host.
    pub fn conn_err(&self, why: &str) -> CliErr {
        CliErr::Conn(format!(
            "cannot reach an Orrery host at {}: {why}\n\
             Start one:  orr_editor --erp 127.0.0.1:7777 --erp-dev     (headless: orr_remote_host --dev-no-auth)\n\
             Another address: --erp <url> or $ORR_ERP. A host that requires a token: --token or $ORR_ERP_TOKEN.",
            args::redact_url(&self.url)
        ))
    }

    /// Prints a result: the JSON with `--json`, else the text.
    pub fn emit(&self, text: &str, json: &J) {
        if self.json {
            out(&serde_json::to_string_pretty(json).unwrap_or_default());
        } else {
            out(text.trim_end_matches('\n'));
        }
    }

    /// `text` with the token and any `token=` query removed.
    fn redact(&self, text: &str) -> String {
        let mut t = text.to_string();
        if let Some(tok) = self.token.as_deref().filter(|t| !t.is_empty()) {
            t = t.replace(tok, "***");
        }
        let mut result = String::new();
        let mut rest = t.as_str();
        while let Some(i) = rest.find("token=") {
            result.push_str(&rest[..i]);
            result.push_str("token=***");
            let after = &rest[i + 6..];
            rest = &after[after.find(|c: char| c == '&' || c.is_whitespace()).unwrap_or(after.len())..];
        }
        result.push_str(rest);
        t = result;
        t
    }
}

/// Writes a line to stdout; a closed pipe (`orr ... | head`) ends the program quietly.
pub fn out(text: &str) {
    let mut o = std::io::stdout().lock();
    if writeln!(o, "{text}").and_then(|()| o.flush()).is_err() {
        std::process::exit(0);
    }
}

/// Writes text to stdout as it is (no added newline).
pub fn out_raw(text: &str) {
    let mut o = std::io::stdout().lock();
    if o.write_all(text.as_bytes()).and_then(|()| o.flush()).is_err() {
        std::process::exit(0);
    }
}

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let (g, rest) = match args::split_global(all) {
        Ok(v) => v,
        Err(e) => return finish_early(&e, false),
    };
    if g.version {
        out(&format!("orr {}", env!("CARGO_PKG_VERSION")));
        return ExitCode::SUCCESS;
    }
    let Some(cmd) = rest.first().cloned() else {
        if g.help {
            out(help::overview());
            return ExitCode::SUCCESS;
        }
        eprintln!("error: no command given\n\n{}", help::overview());
        return ExitCode::from(2);
    };
    if cmd == "help" {
        return match rest.get(1) {
            None => {
                out(help::overview());
                ExitCode::SUCCESS
            }
            Some(c) => match help::command(c) {
                Some(t) => {
                    out(t);
                    ExitCode::SUCCESS
                }
                None => finish_early(&CliErr::Usage(format!("no command '{c}'. Run `orr help`.")), g.json),
            },
        };
    }
    if g.help {
        return match help::command(&cmd) {
            Some(t) => {
                out(t);
                ExitCode::SUCCESS
            }
            None => finish_early(&CliErr::Usage(format!("unknown command '{cmd}'. Run `orr help`.")), g.json),
        };
    }
    if help::command(&cmd).is_none() {
        return finish_early(&CliErr::Usage(format!("unknown command '{cmd}'. Run `orr help`.")), g.json);
    }
    let mut bridge = Bridge::to_url(&g.erp, g.token.clone(), Duration::from_secs(g.timeout));
    bridge.style = Style::Cli;
    let mut ctx = Ctx { bridge, json: g.json, url: g.erp.clone(), token: g.token.clone() };
    match cmds::dispatch(&mut ctx, &cmd, &rest[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliErr::Checks) => ExitCode::from(4),
        Err(e) => {
            let msg = match &e {
                CliErr::Usage(m) | CliErr::Erp(m) | CliErr::Conn(m) => ctx.redact(m),
                CliErr::Checks => String::new(),
            };
            report(&msg, e.code(), ctx.json, matches!(e, CliErr::Usage(_)).then_some(cmd.as_str()));
            ExitCode::from(e.code())
        }
    }
}

fn finish_early(e: &CliErr, json: bool) -> ExitCode {
    let msg = match e {
        CliErr::Usage(m) | CliErr::Erp(m) | CliErr::Conn(m) => m.clone(),
        CliErr::Checks => String::new(),
    };
    report(&msg, e.code(), json, None);
    ExitCode::from(e.code())
}

fn report(msg: &str, code: u8, json: bool, usage_of: Option<&str>) {
    if json {
        eprintln!("{}", json!({"error": msg, "exit_code": code}));
        return;
    }
    eprintln!("error: {msg}");
    if let Some(u) = usage_of.and_then(help::usage_line) {
        eprintln!("usage: {u}\n(`orr help {}` for details and examples)", usage_of.unwrap_or_default());
    }
}
