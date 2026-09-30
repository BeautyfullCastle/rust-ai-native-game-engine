#![allow(dead_code)]
// Test harness: wall-clock waits and threads are fine here.
#![allow(clippy::disallowed_types)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
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

/// `orr_mcp` as a child process, driven over its stdio like an MCP client does.
pub struct McpChild {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    /// Every line the server wrote to stdout, in order.
    pub stdout_lines: Vec<String>,
    stderr: Arc<std::sync::Mutex<String>>,
    next_id: u64,
}

impl McpChild {
    pub fn start(args: &[&str]) -> McpChild {
        let mut child = Command::new(env!("CARGO_BIN_EXE_orr_mcp"))
            .args(args)
            .env_remove("ORR_ERP_TOKEN")
            .env_remove("ORR_ERP_URL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn orr_mcp");
        let stdin = child.stdin.take();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines() {
                match l {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let err = child.stderr.take().unwrap();
        let sink = stderr.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let mut g = sink.lock().unwrap();
                g.push_str(&l);
                g.push('\n');
            }
        });
        McpChild { child, stdin, lines, stdout_lines: Vec::new(), stderr, next_id: 1 }
    }

    /// Starts it against `host` with the token of `client_token`.
    pub fn against(host: &TestHost, token: &str, extra: &[&str]) -> McpChild {
        let mut args = vec!["--erp", host.url.as_str(), "--token", token];
        args.extend_from_slice(extra);
        McpChild::start(&args)
    }

    pub fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin closed");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    pub fn send(&mut self, msg: &J) {
        self.send_raw(&msg.to_string());
    }

    /// The next line the server writes (parsed), or `None` after `wait`.
    pub fn recv_within(&mut self, wait: Duration) -> Option<J> {
        let line = self.lines.recv_timeout(wait).ok()?;
        self.stdout_lines.push(line.clone());
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}")))
    }

    pub fn recv(&mut self) -> J {
        self.recv_within(Duration::from_secs(60)).expect("no reply from orr_mcp within 60 s")
    }

    /// A request; returns the whole response message (`result` or `error`).
    pub fn request_raw(&mut self, method: &str, params: J) -> J {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let r = self.recv();
        assert_eq!(r["id"], id, "reply to another request: {r}");
        assert_eq!(r["jsonrpc"], "2.0");
        r
    }

    /// A request that must succeed; returns `result`.
    pub fn request(&mut self, method: &str, params: J) -> J {
        let r = self.request_raw(method, params);
        assert!(r.get("error").is_none(), "{method} failed: {r}");
        r["result"].clone()
    }

    pub fn initialize(&mut self) -> J {
        let r = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        r
    }

    /// `tools/call`; returns the result object (with `isError`).
    pub fn call_tool(&mut self, name: &str, args: J) -> J {
        self.request("tools/call", json!({"name": name, "arguments": args}))
    }

    /// The text of a tool result.
    pub fn text(result: &J) -> String {
        result["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {result}")).to_string()
    }

    /// Everything the server wrote to stderr so far.
    pub fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Closes stdin and waits for the server to exit; returns its exit code.
    pub fn finish(mut self) -> Option<i32> {
        drop(self.stdin.take());
        let end = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                // Drain what is left on stdout.
                while let Some(_l) = self.recv_within(Duration::from_millis(200)) {}
                return st.code();
            }
            if std::time::Instant::now() > end {
                let _ = self.child.kill();
                panic!("orr_mcp did not exit after stdin closed");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Every stdout line is one JSON-RPC 2.0 message (a response or a notification).
    pub fn assert_stdout_is_only_json_rpc(&self) {
        assert!(!self.stdout_lines.is_empty());
        for l in &self.stdout_lines {
            let j: J = serde_json::from_str(l).unwrap_or_else(|e| panic!("stdout has a non-JSON line ({e}): {l}"));
            assert_eq!(j["jsonrpc"], "2.0", "{l}");
            assert!(j.get("result").is_some() || j.get("error").is_some() || j.get("method").is_some(), "{l}");
        }
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
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
