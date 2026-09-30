//! The ERP server embedded in the editor: an agent and the person share one
//! document, one undo history and one play session.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use orr_edit::{Origin, Target};
use orr_editor::editor::{default_scene_path, Editor, Mode};
use orr_reflect::{decimal, Value};
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

/// Runs `client` on its own thread against `url`, polling the editor's ERP
/// server on this thread (as the window does every frame) until it is done.
fn with_agent<R: Send + 'static>(editor: &mut Editor, url: &str, client: impl FnOnce(&mut ErpClient) -> R + Send + 'static) -> R {
    let url = url.to_string();
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut c = ErpClient::connect(&url, None).expect("connect");
        let r = client(&mut c);
        tx.send(()).ok();
        r
    });
    // About 30 s at 1 ms per round (no wall clock: clippy forbids `Instant`).
    let mut rounds = 0u32;
    while rx.try_recv().is_err() {
        rounds += 1;
        assert!(rounds < 30_000, "the agent did not finish");
        editor.poll_erp();
        thread::sleep(Duration::from_millis(1));
    }
    handle.join().expect("agent thread")
}

fn editor_with_erp() -> (Editor, String) {
    let mut editor = Editor::open(&default_scene_path()).expect("demo scene");
    let url = editor.start_erp(ServerConfig::new(Auth::DevNoAuth)).expect("start ERP");
    (editor, url)
}

fn body_guid(editor: &Editor, name: &str) -> String {
    let info = editor.view().entities().into_iter().find(|e| e.name.as_deref() == Some(name)).expect("entity by name");
    info.guid.expect("scene entity has a GUID").as_str().to_string()
}

fn pos_x(editor: &Editor, guid: &str) -> Value {
    let g = orr_reflect::Guid::parse(guid).unwrap();
    editor.view().field(&Target::Guid(g), BODY, "pos.x").unwrap()
}

#[test]
fn agent_edit_shows_in_the_editor_and_shares_the_undo_history() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_yaml = editor.doc().to_yaml();
    let before_x = pos_x(&editor, &guid);

    let g = guid.clone();
    with_agent(&mut editor, &url, move |c| {
        c.call("world.patch", json!({"entity": g, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    });

    // The window sees the agent's edit at once, and its origin.
    assert_eq!(pos_x(&editor, &guid), Value::Fixed(decimal::parse_fp("1.5").unwrap()));
    let last = editor.doc().history().into_iter().last().unwrap();
    assert_eq!(last.origin, Origin::Agent("dev".into()));
    assert!(editor.doc().is_dirty());

    // The person's Undo takes back the agent's edit: one shared history.
    editor.undo();
    assert_eq!(pos_x(&editor, &guid), before_x);
    assert_eq!(editor.doc().to_yaml(), before_yaml);

    // And the agent sees the person's undo.
    let h = with_agent(&mut editor, &url, |c| c.call("history.list", J::Null).unwrap());
    assert_eq!(h["entries"][0]["undone"], true);
    assert_eq!(h["can_redo"], true);
}

#[test]
fn agent_plays_and_rewinds_the_editor_session() {
    let (mut editor, url) = editor_with_erp();
    let scene_sum = editor.doc().checksum();

    // The agent starts play, runs 120 ticks, seeks to 30 and runs to 120 again.
    let (first, second) = with_agent(&mut editor, &url, |c| {
        c.call("sim.start", json!({})).unwrap();
        let a = c.call("sim.step", json!({"n": 120})).unwrap();
        c.call("sim.seek", json!({"tick": 30})).unwrap();
        let b = c.call("sim.step", json!({"n": 90})).unwrap();
        (a, b)
    });
    assert_eq!(first["head_tick"], 120);
    assert_eq!(second["head_tick"], 120);
    assert_eq!(first["checksum"], second["checksum"], "rewind and replay must be bit-identical");

    // The window is in play mode on the same session, at the same tick.
    assert_eq!(editor.mode(), Mode::Play);
    assert_eq!(editor.timeline().unwrap().tick, 120);
    assert_eq!(first["checksum"], format!("{:#018x}", editor.checksum()));

    // The person presses Stop; the agent sees edit mode and the untouched scene.
    editor.stop();
    let s = with_agent(&mut editor, &url, |c| c.call("sim.state", J::Null).unwrap());
    assert_eq!(s["mode"], "edit");
    assert_eq!(s["checksum"], format!("{scene_sum:#018x}"));
}

#[test]
fn agent_stop_returns_the_window_to_edit_mode() {
    let (mut editor, url) = editor_with_erp();
    editor.step(10);
    assert_eq!(editor.mode(), Mode::Play);
    with_agent(&mut editor, &url, |c| {
        c.call("sim.stop", json!({})).unwrap();
    });
    assert_eq!(editor.mode(), Mode::Edit);
    assert!(editor.erp_status().is_some());
}

/// The M5 flow against the editor itself: an agent proposes over ERP, the
/// proposal waits in the window's Agent list, the agent verifies it with the
/// game's metrics and the bot, and accepts it; the person can undo it.
#[test]
fn agent_proposes_verifies_and_accepts_in_the_editor() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_yaml = editor.doc().to_yaml();

    let g = guid.clone();
    let id = with_agent(&mut editor, &url, move |c| {
        let id = c.call("proposal.begin", json!({"label": "raise body_05"})).unwrap()["id"].as_str().unwrap().to_string();
        c.call(
            "proposal.apply",
            json!({"id": id, "ops": [
                {"op": "rename", "entity": g, "name": "hero"},
                {"op": "patch", "entity": g, "component": BODY, "path": "pos", "value": [0, 12.5]},
            ]}),
        )
        .unwrap();
        id
    });

    // Waiting in the window, the document itself untouched.
    let open = editor.doc().list_proposals();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].origin, Origin::Agent("dev".into()));
    assert_eq!(editor.doc().to_yaml(), before_yaml);

    let report = with_agent(&mut editor, &url, move |c| {
        let r = c
            .call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 120}, "checks": ["lost_bodies.max == 0"]}))
            .unwrap();
        c.call("proposal.accept", json!({"id": id})).unwrap();
        r
    });
    assert_eq!(report["passed"], true, "{report}");
    assert!(report.to_string().contains("lost_bodies"), "the editor's ERP must report the game's metrics");

    assert!(editor.doc().list_proposals().is_empty());
    let last = editor.doc().history().into_iter().last().unwrap();
    assert_eq!(last.origin, Origin::Agent("dev".into()));
    assert_ne!(editor.doc().to_yaml(), before_yaml);
    editor.undo();
    assert_eq!(editor.doc().to_yaml(), before_yaml);
}

/// A person's Stop hands the recording to ERP, so an agent can verify
/// against the play the person just made.
#[test]
fn person_play_is_the_agents_last_play() {
    let (mut editor, url) = editor_with_erp();
    editor.step(90);
    editor.stop();
    let r = with_agent(&mut editor, &url, |c| c.call("verify.self", json!({"inputs": {"kind": "last_play"}})).unwrap());
    assert_eq!(r["ticks"], 90, "{r}");
}

/// What an agent did lands in the window's activity feed: one line each, the
/// old value of an edit, the touched entities, and reads flagged.
#[test]
fn agent_activity_reaches_the_feed_with_old_values() {
    let (mut editor, url) = editor_with_erp();
    let guid = body_guid(&editor, "body_05");
    let before_x = pos_x(&editor, &guid);

    let g = guid.clone();
    with_agent(&mut editor, &url, move |c| {
        c.call("world.query", json!({"limit": 1})).unwrap();
        c.call("world.patch", json!({"entity": g, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    });
    editor.poll_erp();

    let feed = editor.feed();
    let patch = feed.entries().iter().find(|e| e.method == "world.patch").expect("the edit is in the feed");
    assert_eq!(patch.client, "dev");
    assert_eq!(patch.summary, format!("world.patch {guid} {BODY}.pos.x = 1.5"));
    let change = patch.change.as_ref().unwrap();
    assert_eq!(change.old.as_ref(), Some(&before_x));
    assert_eq!(change.new, Some(Value::Fixed(decimal::parse_fp("1.5").unwrap())));
    assert_eq!(patch.entities, vec![guid.clone()]);
    assert!(feed.last_action().unwrap().starts_with("agent dev: world.patch"));

    // Reads are in the log but hidden by the default filter and not counted as unseen.
    assert!(feed.entries().iter().any(|e| e.method == "world.query" && e.read));
    assert!(feed.visible().all(|e| !e.read));
    assert!(editor.feed().unseen() >= 2, "connect and the edit");

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
    let before_yaml = editor.doc().to_yaml();

    with_agent(&mut editor, &url, move |c| {
        let id = c.call("proposal.begin", json!({"label": "raise"})).unwrap()["id"].as_str().unwrap().to_string();
        c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [0, 12.5]}]})).unwrap();
        c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0", "dynamic_bodies.delta == 0"]})).unwrap();
        c.call("proposal.accept", json!({"id": id})).unwrap();
    });
    editor.poll_erp();

    let entries = editor.feed().entries();
    let accept = entries.iter().find(|e| e.method == "proposal.accept").unwrap();
    assert_eq!(accept.proposal.as_deref(), Some("p1"));
    let diff = accept.diff.as_ref().expect("the diff was captured before accept");
    assert!(diff.text.contains("pos: [0, 12.5]"), "{}", diff.text);
    assert_eq!(diff.summary.fields_changed.len(), 1);
    assert!(editor.doc().list_proposals().is_empty(), "the proposal is gone, the feed still has its diff");
    assert!(editor.feed().closed_diff("p1").is_some());

    let verify = entries.iter().find(|e| e.method == "proposal.verify").unwrap();
    let detail = verify.verify.as_ref().unwrap();
    assert_eq!(detail.report.ticks, 60);
    assert_eq!(detail.outcome.as_ref().unwrap().results.len(), 2);
    assert!(verify.summary.starts_with("proposal.verify p1: "), "{}", verify.summary);
    assert_eq!(editor.doc().history().into_iter().last().unwrap().origin, Origin::Agent("dev".into()));

    // The person takes it back with Undo.
    editor.undo();
    assert_eq!(editor.doc().to_yaml(), before_yaml);
}
