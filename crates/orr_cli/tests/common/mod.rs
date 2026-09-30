#![allow(dead_code)]
// Test harness: wall-clock waits and threads are fine here.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_reflect::TypeRegistry;
use orr_remote::{default_build_id, Auth, Caps, ErpClient, ErpServer, GameHooks, Host, ServerConfig, TokenEntry};
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics};
use orr_sim::{PlayerSlot, Simulation};
use serde_json::{json, Value as J};

pub const DEMO_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml");
pub const AGENTS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/AGENTS.md");

pub fn demo_text() -> String {
    std::fs::read_to_string(DEMO_PATH).expect("scenes/physics_demo.scene.yaml")
}

/// An ERP host on its own thread (what `orr_remote_host` runs), PhysGame, the demo scene, a real socket.
pub struct TestHost {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestHost {
    /// Tokens: `claude-tok` (all), `limited-tok` (read + scene_edit: cannot accept).
    pub fn start() -> TestHost {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut types = TypeRegistry::new();
            register_reflect(&mut types);
            let doc = EditorDoc::from_yaml(&demo_text(), types, Simulation::<PhysGame>::build_registry(), 7).expect("demo scene");
            let entry = |client: &str, token: &str, caps: &str| TokenEntry { client: client.into(), token: token.into(), caps: Caps::parse(caps).unwrap() };
            let mut cfg = ServerConfig::new(Auth::Tokens(vec![entry("claude", "claude-tok", "all"), entry("limited", "limited-tok", "read,scene_edit")]));
            cfg.limits.build_id = default_build_id("PhysGame");
            cfg.limits.game = GameHooks::new("PhysGame").with_metrics(PhysMetrics).with_bot(|seed, tick, slot| bot_input(seed, tick, PlayerSlot(slot)));
            let server = ErpServer::start(cfg).expect("start the ERP server");
            tx.send(server.url()).unwrap();
            let mut host = Host::<PhysGame>::new(doc, server);
            host.run(&s, Duration::from_micros(500));
        });
        let url = rx.recv_timeout(Duration::from_secs(20)).expect("the host did not start");
        TestHost { url, stop, thread: Some(thread) }
    }

    /// A direct ERP client (to look at the host behind the adapter's back).
    pub fn erp(&self, token: &str) -> ErpClient {
        ErpClient::connect(&self.url, Some(token)).expect("connect")
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

/// The GUID of the entity named `name`, through a direct ERP client.
pub fn guid_of(c: &mut ErpClient, name: &str) -> String {
    let r = c.call("world.query", json!({"name": name})).unwrap();
    r["entities"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("no entity {name}"))["guid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The result of one `orr` run.
pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Run {
    /// stdout parsed as JSON.
    pub fn json(&self) -> J {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.out))
    }

    /// Both streams, for "never printed" checks.
    pub fn all(&self) -> String {
        format!("{}\n{}", self.out, self.err)
    }
}

/// The `orr` command of a test: environment cleaned, pointed at `url` with `token`.
pub fn command(url: &str, token: Option<&str>, args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_orr"));
    c.args(args).env_remove("ORR_ERP").env_remove("ORR_ERP_URL").env_remove("ORR_ERP_TOKEN").env("ORR_ERP", url);
    if let Some(t) = token {
        c.env("ORR_ERP_TOKEN", t);
    }
    c
}

pub fn finish(o: std::process::Output) -> Run {
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into_owned(), err: String::from_utf8_lossy(&o.stderr).into_owned() }
}

impl TestHost {
    /// Runs `orr` against this host with the token of the `claude` client (all capabilities).
    pub fn orr(&self, args: &[&str]) -> Run {
        finish(command(&self.url, Some("claude-tok"), args).output().expect("spawn orr"))
    }

    /// Runs `orr` with another token (none = no token).
    pub fn orr_as(&self, token: Option<&str>, args: &[&str]) -> Run {
        finish(command(&self.url, token, args).output().expect("spawn orr"))
    }

    /// Runs `orr` with `input` on its stdin.
    pub fn orr_stdin(&self, input: &str, args: &[&str]) -> Run {
        use std::io::Write;
        let mut child = command(&self.url, Some("claude-tok"), args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn orr");
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        finish(child.wait_with_output().unwrap())
    }

    /// The scene as YAML, through a direct ERP client.
    pub fn yaml(&self) -> String {
        self.erp("claude-tok").call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string()
    }

    /// The history entries, through a direct ERP client.
    pub fn history(&self) -> Vec<J> {
        self.erp("claude-tok").call("history.list", json!({})).unwrap()["entries"].as_array().unwrap().clone()
    }
}
