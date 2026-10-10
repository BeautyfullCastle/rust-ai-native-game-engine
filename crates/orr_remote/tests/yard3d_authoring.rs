//! Authored Yard3D scenes use the same local ERP document and play host as
//! generated scenes, including save authority and failed-load atomicity.
#![cfg(feature = "sample-host")]
#![allow(clippy::disallowed_types)] // Test-side deadlines, never simulation state.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use orr_remote::yard3d::{
    spawn_yard3d_scene_host, spawn_yard3d_yaml_host, yard3d_doc, yard3d_doc_from_yaml,
    YARD_BUILD_ID, YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE,
};
use orr_remote::{Auth, Caps, ErpClient, LocalHost, ServerConfig};
use orr_sample::yard3d_game::{Scene as YardScene, Yard3D, YardConfig};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics3d::Body";
const AUTHORING_SCENE: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
const MALFORMED: &str = "schema: orr.scene/1\nentities: [\n";
const WRONG_GAME: &str =
    "schema: orr.scene/1\nentities:\n  e_00000001:\n    orr_physics::Body: {}\n";

fn scene_text() -> String {
    let config = YardConfig {
        rain_per_second: 0,
        max_entities: 128,
        ..YardConfig::new(4)
    };
    format!(
        "# Authored Yard3D fixture: keep source comments until edited.\n{}",
        yard3d_doc(config).unwrap().to_yaml()
    )
}

fn config() -> ServerConfig {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    cfg
}

fn client(host: &LocalHost) -> ErpClient {
    ErpClient::with_transport(Box::new(
        host.connector().connect("user", Caps::ALL).unwrap(),
    ))
}

fn patch_z(client: &mut ErpClient) {
    let queried = client
        .call("world.query", json!({"components": [BODY]}))
        .unwrap();
    let guid = queried["entities"][0]["guid"].as_str().unwrap();
    client
        .call(
            "world.patch",
            json!({"entity": guid, "component": BODY, "path": "pos.z", "value": "1.25"}),
        )
        .unwrap();
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr_yard3d_authoring_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn scene(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn shipped_authoring_fixture_is_small_bounded_and_playable() {
    let doc = yard3d_doc_from_yaml(AUTHORING_SCENE).unwrap();
    assert_eq!(doc.frame().alive_count(), 3);
    assert_eq!(
        doc.scene()
            .entities
            .iter()
            .map(|(guid, entity)| (guid.as_str(), entity.name.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            ("e_00000001", Some("ground")),
            ("e_00000002", Some("box_left")),
            ("e_00000003", Some("box_right")),
        ]
    );
    let scene = doc.frame().singleton::<YardScene>();
    assert_eq!(scene.rain_interval, 0);
    assert_eq!(scene.rain_batch, 0);
    assert_eq!(scene.max_entities, 128);
    let mut sim =
        Simulation::<Yard3D>::from_frame(doc.frame(), YARD_TICK_RATE, YARD_BUILD_ID).unwrap();
    for tick in 1..=60 {
        sim.step(&TickInputs::new(tick, YARD_PLAYERS));
    }
    assert_eq!(sim.tick(), 60);
    assert_eq!(sim.frame().alive_count(), 3);
    assert_ne!(sim.checksum(), doc.checksum());
    assert_eq!(doc.to_yaml(), AUTHORING_SCENE);
}

#[test]
fn yaml_document_preserves_source_and_bakes_deterministic_playable_bytes() {
    let text = scene_text();
    let a = yard3d_doc_from_yaml(&text).unwrap();
    let b = yard3d_doc_from_yaml(&text).unwrap();
    assert_eq!(a.to_yaml(), text);
    assert!(!a.is_dirty());
    assert_eq!(a.seed(), YARD_SEED);
    assert!(a.frame().alive_count() > 4);
    assert_eq!(a.frame().to_bytes(), b.frame().to_bytes());
    assert_eq!(a.checksum(), b.checksum());

    let mut a = Simulation::<Yard3D>::from_frame(a.frame(), YARD_TICK_RATE, YARD_BUILD_ID)
        .expect("Yard3D bake hooks initialize hidden physics storage");
    let mut b = Simulation::<Yard3D>::from_frame(b.frame(), YARD_TICK_RATE, YARD_BUILD_ID).unwrap();
    for tick in 1..=6 {
        let input = TickInputs::new(tick, YARD_PLAYERS);
        a.step(&input);
        b.step(&input);
        assert_eq!(a.frame().to_bytes(), b.frame().to_bytes());
    }
    assert_eq!(a.tick(), 6);
}

#[test]
fn malformed_or_wrong_game_scenes_never_publish_a_host() {
    let dir = TempDir::new();
    for (name, text) in [("bad.scene.yaml", MALFORMED), ("2d.scene.yaml", WRONG_GAME)] {
        assert!(yard3d_doc_from_yaml(text).is_err());
        assert!(spawn_yard3d_yaml_host(text.to_string(), config()).is_err());
        let path = dir.scene(name);
        std::fs::write(&path, text).unwrap();
        let error = spawn_yard3d_scene_host(path.clone(), config())
            .err()
            .expect("a bad scene must not start a host");
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }
    let missing = dir.scene("missing.scene.yaml");
    let error = spawn_yard3d_scene_host(missing.clone(), config())
        .err()
        .expect("missing scene is an error, not a generated fallback");
    assert!(error.contains(&missing.display().to_string()), "{error}");
    assert!(!missing.exists());
}

#[test]
fn path_host_saves_the_loaded_file_without_granting_client_path_authority() {
    let dir = TempDir::new();
    let path = dir.scene("authored.scene.yaml");
    let other = dir.scene("unrelated.scene.yaml");
    let text = scene_text();
    std::fs::write(&path, &text).unwrap();
    std::fs::write(&other, "leave this file alone").unwrap();
    let mut cfg = config();
    cfg.limits.scene_path = Some(other.clone());
    let host = spawn_yard3d_scene_host(path.clone(), cfg).unwrap();
    assert!(host.url().is_none());
    let mut client = client(&host);
    assert_eq!(client.call("scene.save", J::Null).unwrap()["text"], text);
    assert_eq!(
        client.call("sim.state", J::Null).unwrap()["scene_path"],
        path.display().to_string()
    );
    patch_z(&mut client);
    let saved = client
        .call("scene.save", json!({"write": true, "path": other}))
        .unwrap();
    assert_eq!(saved["written"], path.display().to_string());
    assert_eq!(saved["dirty"], false);
    let disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(disk, saved["text"].as_str().unwrap());
    assert_ne!(disk, text);
    assert_eq!(
        std::fs::read_to_string(other).unwrap(),
        "leave this file alone"
    );
    let reloaded = yard3d_doc_from_yaml(&disk).unwrap();
    assert_eq!(
        reloaded.checksum(),
        orr_remote::wire::parse_checksum(&saved["checksum"]).unwrap()
    );
}

#[test]
fn yaml_host_retains_configured_path_and_failed_load_preserves_document_history_and_path() {
    let dir = TempDir::new();
    let path = dir.scene("intended.scene.yaml");
    let replacement = dir.scene("rejected.scene.yaml");
    let text = scene_text();
    let mut cfg = config();
    cfg.limits.scene_path = Some(path.clone());
    cfg.limits.allow_scene_paths = true;
    let host = spawn_yard3d_yaml_host(text.clone(), cfg).unwrap();
    let mut client = client(&host);
    patch_z(&mut client);
    let before = client.call("scene.save", J::Null).unwrap();
    let history = client.call("history.list", J::Null).unwrap();
    assert_eq!(before["dirty"], true);
    for invalid in [MALFORMED, WRONG_GAME] {
        assert!(client
            .call("scene.load", json!({"text": invalid, "path": replacement}))
            .is_err());
        assert_eq!(client.call("scene.save", J::Null).unwrap(), before);
        assert_eq!(client.call("history.list", J::Null).unwrap(), history);
        assert_eq!(
            client.call("sim.state", J::Null).unwrap()["scene_path"],
            path.display().to_string()
        );
    }
    let saved = client.call("scene.save", json!({"write": true})).unwrap();
    assert_eq!(saved["written"], path.display().to_string());
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        saved["text"].as_str().unwrap()
    );
    assert!(!replacement.exists());

    let unnamed = spawn_yard3d_yaml_host(text, config()).unwrap();
    let mut unnamed = self::client(&unnamed);
    assert_eq!(
        unnamed
            .call_err("scene.save", json!({"write": true}))
            .kind(),
        Some("no_scene_path")
    );
}

#[test]
fn loaded_host_steps_seeks_plays_and_stops_on_the_authored_document() {
    let text = scene_text();
    let doc = yard3d_doc_from_yaml(&text).unwrap();
    let mut reference =
        Simulation::<Yard3D>::from_frame(doc.frame(), YARD_TICK_RATE, YARD_BUILD_ID).unwrap();
    let mut sums = vec![reference.checksum()];
    for tick in 1..=6 {
        reference.step(&TickInputs::new(tick, YARD_PLAYERS));
        sums.push(reference.checksum());
    }
    let host = spawn_yard3d_yaml_host(text.clone(), config()).unwrap();
    let mut client = client(&host);
    let discovered = client.call("rpc.discover", J::Null).unwrap();
    assert_eq!(discovered["engine"]["game"], "Yard3D");
    let started = client.call("sim.start", J::Null).unwrap();
    assert_eq!(started["mode"], "play");
    assert_eq!(started["playing"], false);
    assert_eq!(started["tick_rate"], YARD_TICK_RATE);
    assert_eq!(started["player_count"], YARD_PLAYERS);
    let stepped = client.call("sim.step", json!({"n": 6})).unwrap();
    assert_eq!(stepped["head_tick"], 6);
    assert_eq!(
        orr_remote::wire::parse_checksum(&stepped["checksum"]).unwrap(),
        sums[6]
    );
    let seeked = client.call("sim.seek", json!({"tick": 3})).unwrap();
    assert_eq!(seeked["head_tick"], 3);
    assert_eq!(
        orr_remote::wire::parse_checksum(&seeked["checksum"]).unwrap(),
        sums[3]
    );

    client.call("sim.play", J::Null).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = client.call("sim.state", J::Null).unwrap();
        if state["head_tick"].as_u64().unwrap() > 6 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "loaded scene play did not advance"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let paused = client.call("sim.pause", J::Null).unwrap();
    assert_eq!(paused["playing"], false);
    let seeked = client.call("sim.seek", json!({"tick": 6})).unwrap();
    assert_eq!(
        orr_remote::wire::parse_checksum(&seeked["checksum"]).unwrap(),
        sums[6]
    );
    client.call("sim.stop", J::Null).unwrap();
    let stopped = client.call("sim.state", J::Null).unwrap();
    assert_eq!(stopped["mode"], "edit");
    assert_eq!(stopped["head_tick"], 0);
    assert_eq!(
        orr_remote::wire::parse_checksum(&stopped["checksum"]).unwrap(),
        sums[0]
    );
    assert_eq!(client.call("scene.save", J::Null).unwrap()["text"], text);
}
