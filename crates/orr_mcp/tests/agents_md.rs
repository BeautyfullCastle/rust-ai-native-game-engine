//! `docs/AGENTS.md` is the guide generated for the demo scene and stays
//! pinned: it must regenerate identically. To update it on purpose (the
//! engine version, the types or the tools changed):
//! `ORR_UPDATE_AGENTS=1 cargo test -p orr_mcp --test agents_md`.

mod common;

use std::time::Duration;

use common::*;
use orr_mcp::{Bridge, ToolGroups};

fn generate(host: &TestHost) -> String {
    let mut bridge = Bridge::to_url(&host.url, Some("claude-tok".into()), Duration::from_secs(30));
    orr_mcp::generate_agents_md(&mut bridge, ToolGroups::ALL).expect("generate")
}

#[test]
fn the_committed_guide_regenerates_identically() {
    let host = TestHost::start();
    let text = generate(&host);
    if std::env::var_os("ORR_UPDATE_AGENTS").is_some() {
        std::fs::write(AGENTS_PATH, &text).expect("write docs/AGENTS.md");
    }
    let committed = std::fs::read_to_string(AGENTS_PATH).expect("docs/AGENTS.md (generate it with ORR_UPDATE_AGENTS=1)");
    assert_eq!(
        text, committed,
        "docs/AGENTS.md is stale: the engine version, build id, types, metrics or tools changed. Regenerate it with \
         `ORR_UPDATE_AGENTS=1 cargo test -p orr_mcp --test agents_md`"
    );
    // Generating twice, and from another connection, gives the same text.
    assert_eq!(generate(&TestHost::start()), text);
}

#[test]
fn the_guide_is_pinned_to_the_engine_and_describes_the_workflow() {
    let host = TestHost::start();
    let text = generate(&host);
    assert!(text.contains(&format!("**orrery {}**", env!("CARGO_PKG_VERSION"))));
    assert!(text.contains(&format!("build id `0x{:016x}`", orr_remote::default_build_id("PhysGame"))));
    assert!(text.contains("schema fingerprint `0x"));
    for want in [
        "propose_changes",
        "verify_proposal",
        "accept_proposal",
        "## Value format",
        "e_7f3a91c2",
        "lost_bodies",
        "no_divergence_before",
        "--sample-every 1",
        "events created and removed entirely within one tick",
        "every_tick_boundary_observed",
        "orr_physics::Body",
        "`approve`",
    ] {
        assert!(text.contains(want), "the guide mentions {want}");
    }
    // The guide is CLI-first: the commands, the exit codes, and MCP as the alternative at the end.
    for want in ["orr apply", "orr set", "orr activity -f", "ORR_ERP_TOKEN", "## Exit codes and errors", "## MCP alternative", "orr_remote_host --dev-no-auth"] {
        assert!(text.contains(want), "the guide mentions {want}");
    }
    assert!(text.find("## The workflow").unwrap() < text.find("## MCP alternative").unwrap());
    // Pure text: no address of the host it was made from, no timestamp, no token.
    assert!(!text.contains(&host.url) && !text.contains("claude-tok"));
    assert!(text.ends_with('\n') && !text.contains("\r"));
}

#[test]
fn print_agents_md_writes_the_same_text_to_stdout() {
    let host = TestHost::start();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_orr_mcp"))
        .args(["--erp", &host.url, "--token", "claude-tok", "--print-agents-md"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), generate(&host));
    // An unreachable host is an error on stderr, nothing on stdout.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_orr_mcp")).args(["--erp", "ws://127.0.0.1:1", "--print-agents-md"]).output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot talk to the engine"));
}

#[test]
fn tool_definitions_are_consistent() {
    use orr_mcp::tools;
    let all = tools::defs(ToolGroups::ALL);
    let mut names: Vec<&str> = all.iter().map(|t| t.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len(), "names are unique");
    for t in &all {
        assert!(orr_mcp::GROUPS.iter().any(|(g, _)| *g == t.group), "{} has a known group", t.name);
        let s = t.input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        for r in s["required"].as_array().unwrap() {
            assert!(s["properties"].get(r.as_str().unwrap()).is_some(), "{}: required {r} is a property", t.name);
        }
        assert!(["read", "scene_edit", "sim_control", "approve"].contains(&t.needs));
    }
    assert!(ToolGroups::parse("scene, propose").unwrap().has("propose"));
    assert!(!ToolGroups::parse("scene").unwrap().has("sim"));
    assert_eq!(ToolGroups::parse("all").unwrap(), ToolGroups::ALL);
    assert!(ToolGroups::parse("").is_err() && ToolGroups::parse("nope").is_err());
}
