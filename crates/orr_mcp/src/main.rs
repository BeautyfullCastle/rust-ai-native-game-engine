//! `orr_mcp`: MCP server (stdio) for Orrery. See the crate docs for how to
//! register it with Claude Code and other MCP clients.
//!
//! ```text
//! orr_mcp [--erp URL] [--token TOKEN] [--tools GROUPS] [--print-agents-md] [--verbose]
//! ```

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use orr_mcp::{serve, Bridge, McpServer, ToolGroups, GROUPS};

const USAGE: &str = "usage: orr_mcp [--erp URL] [--token TOKEN] [--tools GROUPS] [--print-agents-md] [--verbose]\n\
\n\
  --erp URL           the ERP endpoint of the host (default: $ORR_ERP_URL or ws://127.0.0.1:7777)\n\
  --token TOKEN       the ERP token (default: $ORR_ERP_TOKEN; none = a host in --dev-no-auth mode)\n\
  --tools GROUPS      comma separated tool groups to offer (default all): scene, propose, verify, sim, history\n\
  --timeout SECONDS   how long one ERP call may take (default 120)\n\
  --print-agents-md   print the AGENTS.md generated from the host and exit\n\
  --verbose           log each request to stderr\n\
\n\
Speaks MCP (2025-06-18) as newline-delimited JSON-RPC on stdin/stdout; logs go to stderr.\n\
Register with Claude Code:  claude mcp add orrery -- orr_mcp --erp ws://127.0.0.1:7777 --token ...";

struct Args {
    erp: String,
    token: Option<String>,
    tools: ToolGroups,
    timeout: u64,
    print_agents: bool,
    verbose: bool,
}

fn parse_args() -> Result<Args, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        erp: env("ORR_ERP_URL").unwrap_or_else(|| "ws://127.0.0.1:7777".to_string()),
        token: env("ORR_ERP_TOKEN"),
        tools: ToolGroups::ALL,
        timeout: 120,
        print_agents: false,
        verbose: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--erp" => a.erp = value("--erp")?,
            "--token" => a.token = Some(value("--token")?),
            "--tools" => a.tools = ToolGroups::parse(&value("--tools")?)?,
            "--timeout" => a.timeout = value("--timeout")?.parse().map_err(|e| format!("--timeout: {e}"))?,
            "--print-agents-md" => a.print_agents = true,
            "--verbose" => a.verbose = true,
            "-h" | "--help" => return Err(String::new()),
            "--version" => {
                println!("orr_mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    if !a.erp.starts_with("ws://") && !a.erp.starts_with("wss://") {
        return Err(format!("--erp must be a WebSocket URL like ws://127.0.0.1:7777, not '{}'", a.erp));
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let bridge = Bridge::to_url(&args.erp, args.token.clone(), Duration::from_secs(args.timeout));
    if args.print_agents {
        let mut bridge = bridge;
        return match orr_mcp::generate_agents_md(&mut bridge, args.tools) {
            Ok(text) => {
                let mut out = std::io::stdout().lock();
                if out.write_all(text.as_bytes()).and_then(|()| out.flush()).is_err() {
                    return ExitCode::FAILURE;
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {}", e.0);
                ExitCode::FAILURE
            }
        };
    }
    let mut server = McpServer::new(bridge, args.tools);
    server.verbose = args.verbose;
    eprintln!(
        "orr_mcp {}: MCP on stdio, engine at {}, tool groups: {} (of {})",
        env!("CARGO_PKG_VERSION"),
        args.erp,
        args.tools.names().join(","),
        GROUPS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(",")
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match serve(&mut server, stdin.lock(), stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("orr_mcp: {e}");
            ExitCode::FAILURE
        }
    }
}
