#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_reflect::TypeRegistry;
use orr_remote::{Auth, Caps, ErpClient, ErpServer, GameHooks, Host, ServerConfig, TokenEntry};
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics};
use orr_sim::{PlayerSlot, Simulation};
use serde_json::{json, Value as J};

pub const DEMO_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml");
pub const SEED: u64 = 7;

/// What `orr_remote_host` plugs in: PhysGame's metrics and scripted player.
pub fn phys_hooks() -> GameHooks {
    GameHooks::new("PhysGame").with_metrics(PhysMetrics).with_bot(|seed, tick, slot| bot_input(seed, tick, PlayerSlot(slot)))
}

pub fn types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    register_reflect(&mut t);
    t
}

pub fn demo_text() -> String {
    std::fs::read_to_string(DEMO_PATH).expect("scenes/physics_demo.scene.yaml")
}

pub fn demo_doc() -> EditorDoc {
    EditorDoc::from_yaml(&demo_text(), types(), Simulation::<PhysGame>::build_registry(), SEED).expect("load demo scene")
}

pub fn token(client: &str, secret: &str, caps: &str) -> TokenEntry {
    TokenEntry { client: client.into(), token: secret.into(), caps: Caps::parse(caps).unwrap() }
}

/// A host loop on its own thread (the way an embedding editor or the headless binary runs), with a real socket.
pub struct TestHost {
    pub url: String,
    pub addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestHost {
    pub fn start(cfg: ServerConfig) -> TestHost {
        Self::start_with(cfg, demo_doc)
    }

    pub fn start_with(mut cfg: ServerConfig, make_doc: impl FnOnce() -> EditorDoc + Send + 'static) -> TestHost {
        if cfg.limits.game.metrics.is_none() {
            cfg.limits.game = phys_hooks();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let doc = make_doc();
            let server = ErpServer::start(cfg).expect("start server");
            tx.send((server.url(), server.local_addr())).unwrap();
            let mut host = Host::<PhysGame>::new(doc, server);
            host.run(&s, Duration::from_micros(500));
        });
        let (url, addr) = rx.recv_timeout(Duration::from_secs(20)).expect("server did not start");
        TestHost { url, addr, stop, thread: Some(thread) }
    }

    /// Tokens: `agent` (all), `reader` (read), `editor` (read + scene_edit), `driver` (read + sim_control).
    pub fn standard() -> TestHost {
        Self::start(ServerConfig::new(Auth::Tokens(vec![
            token("claude", "tok-all", "all"),
            token("reader", "tok-read", "read"),
            token("editor", "tok-edit", "read,scene_edit"), // can propose and verify, cannot approve
            token("driver", "tok-sim", "read,sim_control"),
            token("other", "tok-other", "all"),
        ])))
    }

    pub fn dev() -> TestHost {
        Self::start(ServerConfig::new(Auth::DevNoAuth))
    }

    pub fn client(&self, secret: &str) -> ErpClient {
        ErpClient::connect(&self.url, Some(secret)).expect("connect")
    }

    /// Did the host thread survive (no panic escaped)?
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for TestHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The GUID of the entity with this display name.
pub fn guid_of(c: &mut ErpClient, name: &str) -> String {
    let r = c.call("world.query", json!({"name": name})).unwrap();
    let list = r["entities"].as_array().unwrap();
    let hit = list.iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("no entity named {name}"));
    hit["guid"].as_str().unwrap().to_string()
}

pub fn u(j: &J, key: &str) -> u64 {
    j[key].as_u64().unwrap_or_else(|| panic!("{key} in {j}"))
}

pub fn checksum(j: &J) -> u64 {
    orr_remote::wire::parse_checksum(&j["checksum"]).unwrap_or_else(|| panic!("checksum in {j}"))
}
