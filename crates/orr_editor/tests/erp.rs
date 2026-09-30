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
