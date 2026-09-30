//! The host of the editor also serves agents (`--erp`): an agent and the
//! person share one document, one undo history and one play session, because
//! they are two clients of the same host thread.
#![allow(clippy::disallowed_types)]

mod common;

use common::*;
use orr_editor::editor::{default_scene_path, Editor, Mode};
use orr_editor::HostSpec;
use orr_reflect::decimal;
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

/// An editor whose host thread also listens on a loopback port (dev mode), and that address.
fn editor_with_erp() -> (Editor, String) {
    let spec = HostSpec::Local { scene: default_scene_path(), listen: Some(ServerConfig::new(Auth::DevNoAuth)), debug_hooks: false };
    let mut ed = Editor::start(&spec).expect("start");
    ed.sync();
    let (url, _) = ed.erp_status().expect("the host listens");
    (ed, url)
}

fn agent(url: &str) -> ErpClient {
    ErpClient::connect(url, None).expect("connect the agent")
}

fn body_guid(ed: &Editor, name: &str) -> String {
    guid_named(ed, name).to_string()
}

#[test]
fn agent_edit_shows_in_the_editor_and_shares_the_undo_history() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_yaml = scene_text(&mut editor);
    let mut a = agent(&url);

    a.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();

    // The window sees the agent's edit, and its origin.
    editor.sync();
    let t = target_named(&editor, "body_05");
    assert_eq!(field(&mut editor, &t, BODY, "pos.x"), orr_reflect::Value::Fixed(decimal::parse_fp("1.5").unwrap()));
    assert_eq!(editor.history().entries.last().unwrap().origin, "agent:dev");
    assert!(editor.is_dirty());
    // ... and so does the frame the viewport draws.
    assert_eq!(editor.checksum(), doc_checksum(&mut editor));

    // The person's Undo takes back the agent's edit: one shared history.
    assert!(editor.undo());
    assert_eq!(scene_text(&mut editor), before_yaml);

    // And the agent sees the person's undo.
    let h = a.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"][0]["undone"], true);
    assert_eq!(h["can_redo"], true);
}

#[test]
fn agent_plays_and_rewinds_the_editor_session() {
    let (mut editor, url) = editor_with_erp();
    let scene_sum = doc_checksum(&mut editor);
    let mut a = agent(&url);

    // The agent starts play, runs 120 ticks, seeks to 30 and runs to 120 again.
    a.call("sim.start", json!({})).unwrap();
    let first = a.call("sim.step", json!({"n": 120})).unwrap();
    a.call("sim.seek", json!({"tick": 30})).unwrap();
    let second = a.call("sim.step", json!({"n": 90})).unwrap();
    assert_eq!(first["head_tick"], 120);
    assert_eq!(second["head_tick"], 120);
    assert_eq!(first["checksum"], second["checksum"], "rewind and replay must be bit-identical");

    // The window is in play mode on the same session, at the same tick.
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    assert_eq!(editor.timeline().unwrap().tick, 120);
    assert_eq!(first["checksum"], format!("{:#018x}", editor.checksum()));

    // The person presses Stop; the agent sees edit mode and the untouched scene.
    editor.stop();
    let s = a.call("sim.state", J::Null).unwrap();
    assert_eq!(s["mode"], "edit");
    assert_eq!(s["checksum"], format!("{scene_sum:#018x}"));
}

#[test]
fn agent_stop_returns_the_window_to_edit_mode() {
    let (mut editor, url) = editor_with_erp();
    editor.step(10);
    assert_eq!(editor.mode(), Mode::Play);
    agent(&url).call("sim.stop", json!({})).unwrap();
    editor.sync();
    assert_eq!(editor.mode(), Mode::Edit);
    assert!(editor.timeline().is_none());
    assert!(editor.erp_status().is_some());
}

/// The M5 flow against the editor itself: an agent proposes over ERP, the
/// proposal waits in the window's Agent list, the agent verifies it with the
/// game's metrics and the bot, and accepts it; the person can undo it.
#[test]
fn agent_proposes_verifies_and_accepts_in_the_editor() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_yaml = scene_text(&mut editor);
    let mut a = agent(&url);

    let id = a.call("proposal.begin", json!({"label": "raise body_05"})).unwrap()["id"].as_str().unwrap().to_string();
    a.call(
        "proposal.apply",
        json!({"id": id, "ops": [
            {"op": "rename", "entity": guid, "name": "hero"},
            {"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [0, 12.5]},
        ]}),
    )
    .unwrap();

    // Waiting in the window, the document itself untouched.
    editor.sync();
    assert_eq!(editor.proposals().len(), 1);
    assert_eq!(editor.proposals()[0].origin, "agent:dev");
    assert_eq!(scene_text(&mut editor), before_yaml);
    let detail = editor.proposal_detail(&id).expect("the diff reached the window");
    assert_eq!(detail.summary.fields_changed.len(), 1);

    let report = a.call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 120}, "checks": ["lost_bodies.max == 0"]})).unwrap();
    a.call("proposal.accept", json!({"id": id})).unwrap();
    assert_eq!(report["passed"], true, "{report}");
    assert!(report.to_string().contains("lost_bodies"), "the editor's host must report the game's metrics");

    editor.sync();
    assert!(editor.proposals().is_empty());
    assert_eq!(editor.history().entries.last().unwrap().origin, "agent:dev");
    assert_ne!(scene_text(&mut editor), before_yaml);
    editor.undo();
    assert_eq!(scene_text(&mut editor), before_yaml);
}

/// A person's Stop hands the recording to the host, so an agent can verify
/// against the play the person just made.
#[test]
fn person_play_is_the_agents_last_play() {
    let (mut editor, url) = editor_with_erp();
    editor.step(90);
    editor.stop();
    let r = agent(&url).call("verify.self", json!({"inputs": {"kind": "last_play"}})).unwrap();
    assert_eq!(r["ticks"], 90, "{r}");
}

/// What an agent did lands in the window's activity feed: one line each, the
/// old value of an edit, the touched entities, and reads flagged. The
/// person's own requests are not in it.
#[test]
fn agent_activity_reaches_the_feed_with_old_values() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let t = target_named(&editor, "body_05");
    let before_x = field(&mut editor, &t, BODY, "pos.x");
    let mut a = agent(&url);

    a.call("world.query", json!({"limit": 1})).unwrap();
    a.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    editor.sync();

    let feed = editor.feed();
    let patch = feed.entries().iter().find(|e| e.method == "world.patch").expect("the edit is in the feed");
    assert_eq!(patch.client, "dev");
    assert_eq!(patch.summary, format!("world.patch {guid} {BODY}.pos.x = 1.5"));
    let change = patch.change.as_ref().unwrap();
    assert_eq!(change.old.as_ref().map(orr_editor::model::format_json), Some(orr_reflect::decimal::fp_to_decimal(match before_x {
        orr_reflect::Value::Fixed(f) => f,
        _ => panic!("fixed"),
    })));
    assert_eq!(change.new.as_ref().map(orr_editor::model::format_json).as_deref(), Some("1.5"));
    assert_eq!(patch.entities, vec![guid.clone()]);
    assert!(feed.last_action().unwrap().starts_with("agent dev: world.patch"));

    // Reads are in the log but hidden by the default filter and not counted as unseen.
    assert!(feed.entries().iter().any(|e| e.method == "world.query" && e.read));
    assert!(feed.visible().all(|e| !e.read));
    assert!(editor.feed().unseen() >= 2, "connect and the edit");
    // The editor's own requests (the panels' refreshes) are not recorded at all.
    assert!(feed.entries().iter().all(|e| e.client != "user"), "{:?}", feed.entries().iter().map(|e| (&e.client, &e.method)).collect::<Vec<_>>());

    // The entity the agent touched is handed to the window once, for the viewport pulse.
    assert_eq!(editor.feed_mut().take_touched(), vec![guid]);
    assert!(editor.feed_mut().take_touched().is_empty());
    editor.feed_mut().mark_seen();
    assert_eq!(editor.feed().unseen(), 0);
}

/// The whole agent flow without a person: propose, verify, accept. The feed
/// keeps the diff of the proposal that is gone and the verification report.
#[test]
fn agent_flow_leaves_the_diff_and_the_report_in_the_feed() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_yaml = scene_text(&mut editor);
    let mut a = agent(&url);

    let id = a.call("proposal.begin", json!({"label": "raise"})).unwrap()["id"].as_str().unwrap().to_string();
    a.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [0, 12.5]}]})).unwrap();
    a.call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0", "dynamic_bodies.delta == 0"]})).unwrap();
    a.call("proposal.accept", json!({"id": id})).unwrap();
    editor.sync();

    let entries = editor.feed().entries();
    let accept = entries.iter().find(|e| e.method == "proposal.accept").unwrap();
    assert_eq!(accept.proposal.as_deref(), Some("p1"));
    let diff = accept.diff.as_ref().expect("the diff was captured before accept");
    assert!(diff.text.contains("pos: [0, 12.5]"), "{}", diff.text);
    assert_eq!(diff.summary.fields_changed.len(), 1);
    assert!(editor.proposals().is_empty(), "the proposal is gone, the feed still has its diff");
    assert!(editor.feed().closed_diff("p1").is_some());

    let verify = entries.iter().find(|e| e.method == "proposal.verify").unwrap();
    let detail = verify.verify.as_ref().unwrap();
    assert_eq!(detail["ticks"], 60);
    assert_eq!(detail["checks"]["results"].as_array().unwrap().len(), 2);
    assert!(verify.summary.starts_with("proposal.verify p1: "), "{}", verify.summary);
    assert_eq!(editor.history().entries.last().unwrap().origin, "agent:dev");

    // The person takes it back with Undo.
    editor.undo();
    assert_eq!(scene_text(&mut editor), before_yaml);
}

/// An editor started without `--erp` has no listener: only the window talks to its host.
#[test]
fn without_erp_the_host_has_no_socket() {
    let ed = demo_editor();
    assert!(ed.erp_status().is_none());
}
