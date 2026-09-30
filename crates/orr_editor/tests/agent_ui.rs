//! egui_kittest tests of the Agent tab: real widgets, a real agent (an
//! `ErpClient` on its own thread against the editor's embedded ERP server),
//! real clicks. The tab is a read-only activity feed: nothing here approves
//! anything. No GPU (the viewport shows its notice; the tab does not need it).

mod common;

use std::sync::mpsc::{channel, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_edit::{format_value, Op, Origin, Target};
use orr_editor::agent_ui::{LBL_PREVIEW, TXT_NO_AGENT, TXT_PASSED};
use orr_editor::app::BottomTab;
use orr_editor::EditorApp;
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

type Reply = Sender<Result<J, String>>;

/// An agent: an ERP client on its own thread that runs the calls it is given.
struct Agent {
    tx: Option<Sender<(String, J, Reply)>>,
    thread: Option<JoinHandle<()>>,
}

impl Agent {
    fn connect(url: &str) -> Agent {
        let url = url.to_string();
        let (tx, rx) = channel::<(String, J, Reply)>();
        let thread = thread::spawn(move || {
            let mut c = ErpClient::connect(&url, None).expect("connect");
            for (method, params, reply) in rx {
                let _ = reply.send(c.call(&method, params).map_err(|e| e.to_string()));
            }
        });
        Agent { tx: Some(tx), thread: Some(thread) }
    }

    /// Runs one call, stepping the window's frames (which poll ERP) until it answers.
    fn call(&self, h: &mut Harness<'_, EditorApp>, method: &str, params: J) -> Result<J, String> {
        let (rtx, rrx) = channel();
        self.tx.as_ref().unwrap().send((method.to_string(), params, rtx)).unwrap();
        for _ in 0..20_000 {
            h.step();
            if let Ok(r) = rrx.try_recv() {
                h.run_steps(2);
                return r;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the agent's {method} never answered");
    }

    fn ok(&self, h: &mut Harness<'_, EditorApp>, method: &str, params: J) -> J {
        self.call(h, method, params).unwrap_or_else(|e| panic!("{method}: {e}"))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A window on the demo scene with ERP (dev mode) running, and an agent connected to it.
fn with_agent() -> (Harness<'static, EditorApp>, Agent) {
    let mut editor = demo_editor();
    let url = editor.start_erp(ServerConfig::new(Auth::DevNoAuth)).unwrap();
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(move |_cc| EditorApp::new(editor, None));
    h.run_steps(3);
    let agent = Agent::connect(&url);
    // The first call makes sure the connection is up.
    agent.ok(&mut h, "rpc.discover", J::Null);
    (h, agent)
}

fn harness() -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None))
}

fn body_guid(h: &Harness<'_, EditorApp>, name: &str) -> String {
    guid_named(&h.state().editor, name).to_string()
}

fn open_agent_tab(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("Agent").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Agent);
}

/// True if some label contains `text` (several can).
fn shows(h: &Harness<'_, EditorApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

/// Clicks every widget whose label contains `text`: the row's header, and (harmlessly)
/// a status bar label that repeats the text.
fn expand(h: &mut Harness<'_, EditorApp>, text: &str) {
    let n = h.query_all_by_label_contains(text).count();
    assert!(n > 0, "no row with '{text}'");
    for i in 0..n {
        let clicked = h.query_all_by_label_contains(text).nth(i).map(|node| node.click()).is_some();
        if clicked {
            h.run_steps(3);
        }
    }
}

#[test]
fn an_agent_edit_shows_in_the_feed_and_expands_to_old_and_new() {
    let (mut h, agent) = with_agent();
    let guid = body_guid(&h, "body_05");
    let before = h.state().editor.view().field(&Target::Guid(orr_reflect::Guid::parse(&guid).unwrap()), BODY, "pos.x").unwrap();
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.5"}));
    open_agent_tab(&mut h);

    let summary = format!("world.patch {guid} {BODY}.pos.x = 1.5");
    assert!(shows(&h, &summary), "the feed row");
    assert!(shows(&h, "dev"), "the agent's name is on the row");
    assert!(!shows(&h, &format!("{} \u{2192}", format_value(&before))), "collapsed: no old -> new yet");
    expand(&mut h, &summary);
    assert!(shows(&h, &format_value(&before)), "old value");
    assert!(shows(&h, "1.5"), "new value");
    assert!(shows(&h, "\u{2192}"));
    // The status bar names the last action.
    let last = h.state().editor.feed().last_action().unwrap().to_string();
    assert!(last.starts_with("agent dev: world.patch"), "{last}");
}

#[test]
fn proposal_verify_accept_by_the_agent_show_the_diff_and_the_report_and_undo_works() {
    let (mut h, agent) = with_agent();
    let original = h.state().editor.doc().to_yaml();
    let guid = body_guid(&h, "body_05");
    let id = agent.ok(&mut h, "proposal.begin", json!({"label": "rename+move"}))["id"].as_str().unwrap().to_string();
    agent.ok(
        &mut h,
        "proposal.apply",
        json!({"id": id, "ops": [
            {"op": "rename", "entity": guid, "name": "hero"},
            {"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [2, 30]},
        ]}),
    );
    let report = agent.ok(&mut h, "proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0"]}));
    assert_eq!(report["passed"], true);
    agent.ok(&mut h, "proposal.accept", json!({"id": id}));
    open_agent_tab(&mut h);

    // The document changed and the history has the agent's entry.
    assert!(h.state().editor.doc().list_proposals().is_empty());
    let last = h.state().editor.doc().history().into_iter().last().unwrap();
    assert_eq!((last.label.as_str(), last.origin.clone()), ("rename+move", Origin::Agent("dev".into())));

    // The feed rows.
    assert!(shows(&h, &format!("proposal.begin {id} \"rename+move\"")));
    assert!(shows(&h, &format!("proposal.verify {id}: 1/1 checks passed (60 ticks)")));
    assert!(shows(&h, &format!("proposal.accept {id}")));

    // The accepted proposal's diff was kept, though the proposal is gone.
    expand(&mut h, &format!("proposal.accept {id}"));
    assert!(shows(&h, "--- base"), "the unified diff");
    assert!(shows(&h, "~ rename e_"), "the structural summary");
    assert!(shows(&h, "~ 1 rename, ~ 1 field"));

    // The verification report: verdict, the check, the metrics.
    expand(&mut h, &format!("proposal.verify {id}"));
    assert!(h.query_by_label(TXT_PASSED).is_some(), "the verdict");
    assert!(shows(&h, "lost_bodies"));

    // No approval controls exist any more.
    for label in ["Accept", "Reject", "Verify", "checks (one per line)"] {
        assert!(h.query_by_label(label).is_none(), "'{label}' must not exist");
    }

    // The History tab has the entry, and Ctrl+Z takes the agent's work back.
    h.get_by_label("History").click();
    h.run_steps(2);
    assert!(shows(&h, "[agent:dev]"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().editor.doc().to_yaml(), original);
}

#[test]
fn an_open_proposal_lists_with_a_view_only_preview_toggle() {
    let (mut h, agent) = with_agent();
    let guid = body_guid(&h, "body_05");
    let id = agent.ok(&mut h, "proposal.begin", json!({"label": "lift"}))["id"].as_str().unwrap().to_string();
    agent.ok(&mut h, "proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [2, 30]}]}));
    open_agent_tab(&mut h);
    assert!(shows(&h, "Open proposals (1)"));
    assert!(shows(&h, "agent:dev"));
    assert!(shows(&h, "1 op"));
    let yaml = h.state().editor.doc().to_yaml();

    assert!(h.state().editor.previewing().is_none());
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    assert!(h.state().editor.previewing().is_some());
    assert_eq!(h.state().editor.doc().to_yaml(), yaml, "a preview changes nothing");
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    assert!(h.state().editor.previewing().is_none());
    for label in ["Accept", "Reject", "Verify"] {
        assert!(h.query_by_label(label).is_none(), "'{label}' must not exist");
    }

    // When the agent takes the proposal back, the section empties and the preview goes.
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    agent.ok(&mut h, "proposal.reject", json!({"id": id}));
    h.run_steps(3);
    assert!(h.state().editor.previewing().is_none());
    assert!(shows(&h, "Open proposals (0)"));
}

#[test]
fn reads_are_hidden_until_the_toggle_is_on() {
    let (mut h, agent) = with_agent();
    agent.ok(&mut h, "world.query", json!({"limit": 3}));
    open_agent_tab(&mut h);
    let read_entries = h.state().editor.feed().entries().iter().filter(|e| e.read).count();
    assert!(read_entries >= 2, "rpc.discover and world.query were recorded");
    assert!(!shows(&h, "world.query ("), "hidden by default");
    assert!(shows(&h, "reads ("), "the toggle is there");
    h.query_all_by_label_contains("reads (").next().unwrap().click();
    h.run_steps(3);
    assert!(shows(&h, "world.query ("));
    assert!(h.state().editor.feed().filter.reads);
    h.query_all_by_label_contains("reads (").next().unwrap().click();
    h.run_steps(3);
    assert!(!shows(&h, "world.query ("));
}

#[test]
fn the_tab_badge_counts_unseen_entries_and_clears_when_shown() {
    let (mut h, agent) = with_agent();
    let guid = body_guid(&h, "body_05");
    // Timeline is the tab on screen: agent work piles up unseen (a read does not count).
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Timeline);
    let base = h.state().editor.feed().unseen();
    agent.ok(&mut h, "world.query", json!({"limit": 1}));
    assert_eq!(h.state().editor.feed().unseen(), base, "reads do not count");
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "3"}));
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "4"}));
    assert_eq!(h.state().editor.feed().unseen(), base + 2);
    // The badge is drawn next to the tab.
    assert!(shows(&h, &format!(" {} ", base + 2)));

    open_agent_tab(&mut h);
    assert_eq!(h.state().editor.feed().unseen(), 0, "showing the tab clears the badge");
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "5"}));
    assert_eq!(h.state().editor.feed().unseen(), 0, "and it stays clear while the tab is on screen");
}

#[test]
fn an_agent_edit_pulses_the_touched_entity_in_edit_and_play_mode() {
    let (mut h, agent) = with_agent();
    let guid = body_guid(&h, "body_05");
    let target = Target::Guid(orr_reflect::Guid::parse(&guid).unwrap());
    assert!(h.state().ui.pulses.is_empty());
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.5"}));
    assert!(h.state().ui.pulses.iter().any(|p| p.target == target), "edit mode: {:?}", h.state().ui.pulses);
    // Pulses run out (the harness steps egui time forward).
    h.run_steps(20);
    assert!(h.state().ui.pulses.is_empty(), "the pulse lasts about 1.5 s: {:?}", h.state().ui.pulses);

    // In play mode too.
    agent.ok(&mut h, "sim.start", json!({}));
    assert!(h.state().editor.is_playing_mode());
    agent.ok(&mut h, "world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "2.5"}));
    assert!(h.state().ui.pulses.iter().any(|p| p.target == target), "play mode: {:?}", h.state().ui.pulses);
}

#[test]
fn the_header_names_connected_agents_or_says_how_to_connect() {
    // No ERP at all: the hint has the exact command lines.
    let mut h = harness();
    h.run();
    open_agent_tab(&mut h);
    assert!(shows(&h, TXT_NO_AGENT));
    assert!(shows(&h, "orr_editor --erp 127.0.0.1:7777 --erp-dev"));
    assert!(shows(&h, "claude mcp add orrery -- orr_mcp --erp ws://127.0.0.1:7777"));
    assert!(shows(&h, "Nothing yet"));

    // ERP on, an agent connected: its name and capabilities.
    let (mut h, agent) = with_agent();
    open_agent_tab(&mut h);
    assert!(!shows(&h, TXT_NO_AGENT));
    assert!(shows(&h, "read,scene_edit,sim_control,approve"), "dev mode includes approve");
    let agents = h.state().editor.agents();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "dev");
    drop(agent);
    // Gone: the hint returns, with the real address.
    for _ in 0..200 {
        h.step();
        if h.state().editor.agents().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(2);
    assert!(shows(&h, TXT_NO_AGENT));
    assert!(shows(&h, "claude mcp add orrery -- orr_mcp --erp ws://127.0.0.1:"));
}

#[test]
fn a_failed_request_is_shown_with_its_error() {
    let (mut h, agent) = with_agent();
    let bad = agent.call(&mut h, "world.patch", json!({"entity": "e_deadbeef", "component": BODY, "path": "pos.x", "value": 1}));
    assert!(bad.is_err());
    open_agent_tab(&mut h);
    assert!(shows(&h, "world.patch e_deadbeef"));
    assert!(shows(&h, "\u{2718}"));
    expand(&mut h, "world.patch e_deadbeef");
    assert!(shows(&h, "error: "));
}

/// A proposal staged through the document (the script way) is listed and previewable without any agent.
#[test]
fn staged_proposals_can_be_previewed_and_nothing_can_approve_them() {
    let mut h = harness();
    h.run();
    let guid = guid_named(&h.state().editor, "body_05");
    let ed = &mut h.state_mut().editor;
    let id = ed.doc_mut().propose("rename+move", Origin::Agent("claude".into())).unwrap();
    ed.doc_mut().proposal_apply(id, Op::SetField { guid, component: BODY.into(), path: "pos".into(), value: vec2(2, 30) }).unwrap();
    open_agent_tab(&mut h);
    assert!(shows(&h, "rename+move"));
    assert!(shows(&h, "agent:claude"));
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    assert!(h.state().editor.previewing().is_some());
    assert!(h.query_by_label("Accept").is_none() && h.query_by_label("Reject").is_none() && h.query_by_label("Verify").is_none());
}
