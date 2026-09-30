//! Crash isolation: the simulation lives on the host's thread (or in another
//! process), so when it panics or goes away the editor stays up, says so,
//! and offers to start it again.
#![allow(clippy::disallowed_types)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::app::{LBL_RECONNECT, LBL_RESTART};
use orr_editor::editor::{default_scene_path, Editor, Mode, Owner};
use orr_editor::EditorApp;
use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, ServerConfig};

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn shows(h: &Harness<'_, EditorApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

#[test]
fn a_panic_in_the_host_thread_does_not_take_the_editor_down() {
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(crashable_editor(), None));
    settle(&mut h);
    // Some unsaved work, and play running, when the host dies.
    {
        let ed = &mut h.state_mut().editor;
        ed.select_named("body_05");
        ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(7));
        ed.step(5);
    }
    settle(&mut h);
    assert!(h.state().editor.down().is_none());
    let frame_before = h.state().editor.checksum();

    h.state_mut().editor.debug_crash_host();
    // The host thread dies a moment after it answered: the next frames notice.
    let end = Instant::now() + Duration::from_secs(10);
    while h.state().editor.down().is_none() {
        assert!(Instant::now() < end, "the editor never noticed");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);

    // The editor is alive and says what happened.
    let down = h.state().editor.down().expect("the editor noticed").clone();
    assert!(down.reason.starts_with("simulation host stopped: "), "{}", down.reason);
    assert!(down.reason.contains("debug.panic"), "the panic's own message is shown: {}", down.reason);
    assert!(down.local);
    assert!(shows(&h, "simulation host stopped: "), "the banner is on screen");
    assert!(shows(&h, LBL_RESTART), "with the way back");
    assert!(shows(&h, "edits that were not saved are lost"), "and what it costs");
    assert!(h.state().editor.status().is_some_and(|m| m.error && m.text.starts_with("simulation host stopped")));
    // The last frame stays on screen; actions on a dead host are refused politely, not panics.
    assert_eq!(h.state().editor.checksum(), frame_before);
    let ed = &mut h.state_mut().editor;
    assert!(!ed.undo());
    assert!(!ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(1)));
    ed.step(1);
    ed.pump();
    h.run_steps(3);

    // Restart: the scene is opened again in a new host; the unsaved edit is gone.
    h.get_by_label(LBL_RESTART).click();
    settle(&mut h);
    assert!(h.state().editor.down().is_none());
    assert!(!shows(&h, LBL_RESTART));
    let ed = &mut h.state_mut().editor;
    assert_eq!(ed.mode(), Mode::Edit);
    assert!(!ed.is_dirty(), "unsaved edits were lost, as the banner said");
    assert!(ed.history().entries.is_empty());
    assert!(ed.status().is_some_and(|m| !m.error && m.text.contains("restarted")), "{:?}", ed.status());
    // It works again: edit, play.
    ed.select_named("body_05");
    assert!(ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(2)));
    ed.step(10);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 10);
    assert_eq!(ed.checksum(), doc_or_live(ed));
}

fn doc_or_live(ed: &mut Editor) -> u64 {
    let r = ed.host_call("sim.state", serde_json::json!({})).unwrap();
    orr_remote::wire::parse_checksum(&r["checksum"]).unwrap()
}

#[test]
fn the_unsaved_edits_survive_if_they_were_saved_before_the_crash() {
    let path = temp_path("crash_saved.scene.yaml");
    std::fs::copy(default_scene_path(), &path).unwrap();
    let spec = orr_editor::HostSpec::Local { scene: path.clone(), listen: None, debug_hooks: true };
    let mut ed = Editor::start(&spec).unwrap();
    ed.sync();
    ed.select_named("body_05");
    ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(9));
    assert!(ed.save());
    ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(11));
    ed.debug_crash_host();
    let end = Instant::now() + Duration::from_secs(10);
    while ed.down().is_none() {
        assert!(Instant::now() < end, "the editor never noticed");
        ed.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ed.restart());
    ed.sync();
    let t = target_named(&ed, "body_05");
    assert_eq!(field(&mut ed, &t, BODY, "pos.x"), fixed(9), "the saved edit is there, the later one is not");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_remote_host_that_goes_away_shows_disconnected_and_reconnect_works() {
    // A host on a port we can start again.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let start = |port: u16| {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.bind = format!("127.0.0.1:{port}").parse().unwrap();
        let text = std::fs::read_to_string(default_scene_path()).unwrap();
        spawn_phys_host(text, Some(default_scene_path()), cfg).expect("host")
    };
    let host = start(port);
    let url = format!("ws://127.0.0.1:{port}");
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe({
        let url = url.clone();
        move |_cc| EditorApp::new(Editor::attach(&url, None).expect("attach"), None)
    });
    settle(&mut h);
    {
        let ed = &mut h.state_mut().editor;
        ed.select_named("body_05");
        ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(5));
        assert!(ed.is_dirty());
    }
    assert!(h.state().editor.down().is_none());

    // The host goes away (another process ending looks the same from here).
    drop(host);
    let end = Instant::now() + Duration::from_secs(10);
    while h.state().editor.down().is_none() {
        assert!(Instant::now() < end, "the editor never noticed");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(3);
    let down = h.state().editor.down().unwrap().clone();
    assert!(down.reason.starts_with("disconnected"), "{}", down.reason);
    assert!(!down.local);
    assert!(shows(&h, "disconnected"));
    assert!(shows(&h, LBL_RECONNECT));
    assert!(!shows(&h, LBL_RESTART));

    // The host is back (a fresh one on the same address): Reconnect attaches to it and shows its scene.
    let _host = start(port);
    h.get_by_label(LBL_RECONNECT).click();
    settle(&mut h);
    assert!(h.state().editor.down().is_none(), "{:?}", h.state().editor.down());
    let ed = &mut h.state_mut().editor;
    assert!(!ed.is_dirty(), "that host has its own document");
    assert!(ed.status().is_some_and(|m| m.text == "reconnected"), "{:?}", ed.status());
    assert_eq!(ed.checksum(), doc_or_live(ed));
    // And a failed reconnect leaves the editor down, with the reason.
    drop(_host);
    let mut ed = Editor::attach(&url, None).err();
    assert!(ed.take().is_some(), "nobody listens any more");
}
