//! `orr_editor --connect`: the editor attaches to a host that runs apart
//! from it (here a host thread with a WebSocket listener, which is what
//! `orr_remote_host` is) and shows and edits THAT host's scene and play
//! session. Frames arrive over the WebSocket (lz4) instead of in-process.
#![allow(clippy::disallowed_types)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use orr_editor::editor::{default_scene_path, Editor, Mode};
use orr_editor::Target;
use orr_reflect::{decimal, Value};
use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, ErpClient, LocalHost, ServerConfig, TokenEntry};
use serde_json::{json, Value as J};

/// A host of its own (scene copied to a temp file), listening on a free port.
fn host_with(auth: Auth, name: &str) -> (LocalHost, String, std::path::PathBuf) {
    let scene = temp_path(name);
    std::fs::copy(default_scene_path(), &scene).unwrap();
    let mut cfg = ServerConfig::new(auth);
    cfg.limits.max_step_per_call = 20_000;
    let host = spawn_phys_host(std::fs::read_to_string(&scene).unwrap(), Some(scene.clone()), cfg).expect("start the host");
    let url = host.url().expect("listening").to_string();
    (host, url, scene)
}

fn dev_host(name: &str) -> (LocalHost, String, std::path::PathBuf) {
    host_with(Auth::DevNoAuth, name)
}

fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_editor_attaches_to_a_running_host_and_shows_its_scene() {
    let (_host, url, scene) = dev_host("attach_show.scene.yaml");
    // Something about the host's scene that the editor could not know: a body moved before it connects.
    let mut agent = ErpClient::connect(&url, None).unwrap();
    let guid = agent.call("world.query", json!({"name": "body_05"})).unwrap()["entities"][0]["guid"].as_str().unwrap().to_string();
    agent.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "-4.5"})).unwrap();

    let mut ed = Editor::attach(&url, None).expect("attach");
    ed.sync();
    assert!(!ed.spec().is_local());
    assert_eq!(ed.down(), None);
    // The host's scene: file, entities, unsaved edit, history.
    assert_eq!(ed.path().as_deref(), Some(scene.as_path()));
    assert!(ed.title().contains("attach_show.scene.yaml") && ed.title().contains(&url), "{}", ed.title());
    assert!(ed.is_dirty(), "the host's document has an edit the editor did not make");
    let t = target_named(&ed, "body_05");
    assert_eq!(field(&mut ed, &t, BODY, "pos.x"), Value::Fixed(decimal::parse_fp("-4.5").unwrap()));
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(ed.history().entries[0].origin, "agent:dev");
    // The frames came over the WebSocket: the viewport has the host's frame, byte for byte.
    assert_eq!(ed.checksum(), doc_checksum(&mut ed));
    assert!(!ed.bodies().is_empty());
    let (shown, clients) = ed.erp_status().unwrap();
    assert_eq!(shown, url);
    let _ = clients;
}

#[test]
fn an_attached_editor_edits_the_hosts_document_and_an_agent_sees_it() {
    let (_host, url, _scene) = dev_host("attach_edit.scene.yaml");
    let mut ed = Editor::attach(&url, None).unwrap();
    ed.sync();
    let mut agent = ErpClient::connect(&url, None).unwrap();

    // The person edits through the window: one drag is one history entry, by "user".
    ed.select_named("body_05");
    ed.begin_edit("move body");
    for x in 1..=10 {
        ed.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(x));
    }
    ed.end_edit();
    ed.sync();
    let h = agent.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"].as_array().unwrap().len(), 1);
    assert_eq!(h["entries"][0]["origin"], "user");
    assert_eq!(h["entries"][0]["label"], "move body");
    let guid = guid_named(&ed, "body_05").to_string();
    let v = agent.call("world.get", json!({"entity": guid, "component": BODY, "path": "pos.x"})).unwrap();
    assert_eq!(v["value"].to_string(), "10");

    // The agent edits; the window sees it (frame and inspector), and the feed names the agent.
    agent.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "12.5"})).unwrap();
    ed.sync();
    let t = Target::Guid(guid_named(&ed, "body_05"));
    assert_eq!(field(&mut ed, &t, BODY, "pos.x"), Value::Fixed(decimal::parse_fp("12.5").unwrap()));
    assert_eq!(ed.checksum(), doc_checksum(&mut ed));
    assert_eq!(ed.history().entries.len(), 2);
    assert_eq!(ed.history().entries[1].origin, "agent:dev");
    let patch = ed.feed().entries().iter().find(|e| e.method == "world.patch").expect("the agent's edit is in the feed");
    assert_eq!(patch.client, "dev");
    assert!(ed.feed().entries().iter().all(|e| e.client != "user"), "the editor's own requests are not the feed's business");
    assert_eq!(ed.inspect().map(|i| i.target.clone()), Some(Target::Guid(guid_named(&ed, "body_05"))));
    let inspected = &ed.inspect().unwrap().components;
    let body = inspected.iter().find(|(n, _)| n == BODY).unwrap();
    let Value::Struct(fields) = &body.1 else { panic!("struct") };
    let Some((_, Value::Vec2(p))) = fields.iter().find(|(n, _)| n == "pos") else { panic!("pos") };
    assert_eq!(p.x, decimal::parse_fp("12.5").unwrap(), "the inspector follows the host");

    // The person's Undo takes back the agent's edit on the host: one shared history.
    assert!(ed.undo());
    let h = agent.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"][1]["undone"], true);
}

#[test]
fn an_attached_editor_plays_rewinds_and_saves_on_the_host() {
    let (_host, url, scene) = dev_host("attach_play.scene.yaml");
    let mut ed = Editor::attach(&url, None).unwrap();
    ed.sync();
    ed.step(60);
    ed.sync();
    assert_eq!(ed.mode(), Mode::Play);
    let tl = ed.timeline().unwrap();
    assert_eq!((tl.tick, tl.last_tick), (60, 60));
    assert_eq!(ed.checksum(), tl.checksum);
    let first = checksums(&mut ed, 0, 60);
    ed.seek(10);
    ed.sync();
    assert_eq!(ed.checksum(), first[10]);
    // A second client sees the same session.
    let mut agent = ErpClient::connect(&url, None).unwrap();
    assert_eq!(agent.call("sim.state", J::Null).unwrap()["head_tick"], 10);
    ed.stop();
    ed.sync();
    assert_eq!(ed.mode(), Mode::Edit);
    // Save writes the HOST's scene file.
    ed.select_named("body_05");
    ed.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(6));
    assert!(ed.save());
    ed.sync();
    assert!(!ed.is_dirty());
    let text = std::fs::read_to_string(&scene).unwrap();
    assert!(text.contains("pos: [6,") || text.contains("x: 6"), "the host wrote the edit to its file:\n{}", &text[..text.len().min(400)]);
}

#[test]
fn a_token_host_names_the_editor_by_its_token_and_keeps_out_the_wrong_one() {
    let tokens = vec![
        TokenEntry { client: "user".into(), token: "person-secret".into(), caps: orr_remote::Caps::ALL },
        TokenEntry { client: "claude".into(), token: "agent-secret".into(), caps: orr_remote::Caps::ALL },
    ];
    let (_host, url, _scene) = host_with(Auth::Tokens(tokens), "attach_tokens.scene.yaml");
    assert!(Editor::attach(&url, Some("wrong")).is_err(), "a refused token is an error, not a hang");
    assert!(Editor::attach(&url, None).is_err(), "a token host needs a token");
    let mut ed = Editor::attach(&url, Some("person-secret")).expect("attach with the person's token");
    ed.sync();
    ed.select_named("body_05");
    ed.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(3));
    let mut agent = ErpClient::connect(&url, Some("agent-secret")).unwrap();
    let h = agent.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"][0]["origin"], "user", "the token named `user` is a person's view");
    let g = guid_named(&ed, "body_05").to_string();
    agent.call("world.patch", json!({"entity": g, "component": BODY, "path": "pos.x", "value": 4})).unwrap();
    ed.sync();
    assert_eq!(ed.history().entries[1].origin, "agent:claude");
    assert_eq!(ed.feed().entries().iter().filter(|e| e.method == "world.patch").count(), 1);
    wait_until("the agent to show in the client list", || {
        ed.sync();
        ed.agents().iter().any(|a| a.name == "claude")
    });
}

#[test]
fn attaching_to_nothing_is_an_error_with_a_reason() {
    // A host without the editor's types (an empty registry game) would not know PhysGame types; here the
    // check that matters is the error path of an address with nobody behind it.
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let e = Editor::attach(&format!("ws://{free}"), None).err().expect("nobody listens there");
    assert!(e.contains("cannot connect"), "{e}");
}
