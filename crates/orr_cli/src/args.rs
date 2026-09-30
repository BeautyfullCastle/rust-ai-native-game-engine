//! Command line parsing: global options, and per-command flags.

use crate::CliErr;

/// Options that apply to every command.
pub struct Global {
    pub erp: String,
    pub token: Option<String>,
    pub json: bool,
    pub timeout: u64,
    pub help: bool,
    pub version: bool,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Commands whose operands may contain a `--name` of their own (`spawn --name crate`).
const OWN_NAME: &[&str] = &["spawn", "propose", "apply"];

/// Splits the process arguments into the global options and the rest
/// (the command word first). Global options may appear anywhere, except that
/// `--name` (informational: the host's auth decides who you are) is left in
/// place for the commands that take a `--name` themselves.
pub fn split_global(args: Vec<String>) -> Result<(Global, Vec<String>), CliErr> {
    let mut g = Global {
        erp: env("ORR_ERP").or_else(|| env("ORR_ERP_URL")).unwrap_or_else(|| "ws://127.0.0.1:7777".to_string()),
        token: env("ORR_ERP_TOKEN"),
        json: false,
        timeout: 120,
        help: false,
        version: false,
    };
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let is_name = flag == "--name" && rest.first().is_none_or(|c| !OWN_NAME.contains(&c.as_str()));
        match flag.as_str() {
            "--erp" | "--token" | "--timeout" => {
                let v = match inline {
                    Some(v) => v,
                    None => it.next().ok_or_else(|| CliErr::Usage(format!("{flag} needs a value")))?,
                };
                match flag.as_str() {
                    "--erp" => g.erp = v,
                    "--token" => g.token = Some(v),
                    _ => g.timeout = v.parse().map_err(|_| CliErr::Usage("--timeout needs a number of seconds".into()))?,
                }
            }
            "--name" if is_name => {
                if inline.is_none() {
                    it.next().ok_or_else(|| CliErr::Usage("--name needs a value".into()))?;
                }
            }
            "--json" => g.json = true,
            "-h" | "--help" => g.help = true,
            "--version" | "-V" => g.version = true,
            _ => rest.push(a),
        }
    }
    if !g.erp.contains("://") {
        g.erp = format!("ws://{}", g.erp);
    }
    if !g.erp.starts_with("ws://") && !g.erp.starts_with("wss://") {
        return Err(CliErr::Usage(format!("--erp must be a WebSocket URL like ws://127.0.0.1:7777, not '{}'", redact_url(&g.erp))));
    }
    Ok((g, rest))
}

/// A URL without its query string (it may carry a token).
pub fn redact_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?..."),
        None => url.to_string(),
    }
}

/// The flags of one command line, parsed.
#[derive(Default)]
pub struct Parsed {
    /// Operands, in order.
    pub pos: Vec<String>,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
}

fn looks_negative_number(s: &str) -> bool {
    s.strip_prefix('-').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
}

/// Parses `args` with `spec`: `(name, takes_value)`. `--flag=value` works for
/// value flags. With `lenient`, unknown `--flags` stay operands (the op
/// mini-syntax has its own, like `spawn --name crate`).
pub fn parse(args: &[String], spec: &[(&str, bool)], lenient: bool) -> Result<Parsed, CliErr> {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "-" || !a.starts_with('-') || looks_negative_number(a) {
            p.pos.push(a.clone());
            continue;
        }
        let (name, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        match spec.iter().find(|(n, _)| *n == name) {
            Some((n, true)) => {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        let v = args.get(i).ok_or_else(|| CliErr::Usage(format!("{n} needs a value")))?.clone();
                        i += 1;
                        v
                    }
                };
                p.opts.push(((*n).to_string(), v));
            }
            Some((n, false)) => {
                if inline.is_some() {
                    return Err(CliErr::Usage(format!("{n} takes no value")));
                }
                p.flags.push((*n).to_string());
            }
            None if lenient => p.pos.push(a.clone()),
            None => return Err(CliErr::Usage(format!("unknown option '{name}'"))),
        }
    }
    Ok(p)
}

impl Parsed {
    /// True if the flag was given.
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    /// The last value of an option.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Every value of a repeatable option, in order.
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.opts.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }

    /// An option as an unsigned number.
    pub fn num(&self, name: &str) -> Result<Option<u64>, CliErr> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| CliErr::Usage(format!("{name} needs a non-negative integer, got '{v}'"))),
        }
    }
}
