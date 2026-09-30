//! Runs the real `orr_remote_host` binary and drives it over a socket.

#![cfg(feature = "sample-host")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use orr_remote::ErpClient;
use serde_json::{json, Value as J};

struct Host {
    child: Child,
    url: String,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(args: &[&str]) -> Host {
    let mut child = Command::new(env!("CARGO_BIN_EXE_orr_remote_host"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn orr_remote_host");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut url = None;
    for line in lines.by_ref() {
        let line = line.unwrap();
        if let Some(rest) = line.split("ERP listening on ").nth(1) {
            url = Some(rest.trim().to_string());
            break;
        }
    }
    // Keep draining stdout so the child never blocks on a full pipe.
    std::thread::spawn(move || for _ in lines {});
    Host { child, url: url.expect("the host prints its URL") }
}

#[test]
fn the_host_binary_serves_the_default_scene_and_runs_play() {
    let host = start(&["--bind", "127.0.0.1:0", "--token", "agent:s3cret:all", "--token", "viewer:v1ew:read"]);
    assert!(host.url.starts_with("ws://127.0.0.1:"), "{}", host.url);
    let mut c = ErpClient::connect(&host.url, Some("s3cret")).unwrap();
    let q = c.call("world.query", json!({"limit": 2})).unwrap();
    assert_eq!(q["total"], 49, "the default scene is scenes/physics_demo.scene.yaml");
    c.call("sim.start", json!({"run": true})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let s = c.call("sim.pause", J::Null).unwrap();
    assert!(s["head_tick"].as_u64().unwrap() >= 10, "play runs in real time: {s}");
    let mut v = ErpClient::connect(&host.url, Some("v1ew")).unwrap();
    assert_eq!(v.call_err("sim.step", json!({"n": 1})).code, orr_remote::PERMISSION_DENIED);
    assert!(ErpClient::connect(&host.url, Some("wrong")).is_err());
}

#[test]
fn the_host_binary_refuses_to_start_without_tokens_and_takes_dev_mode() {
    let out = Command::new(env!("CARGO_BIN_EXE_orr_remote_host")).args(["--bind", "127.0.0.1:0"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--token"), "{}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new(env!("CARGO_BIN_EXE_orr_remote_host")).args(["--dev-no-auth", "--bind", "0.0.0.0:0"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("loopback"), "{}", String::from_utf8_lossy(&out.stderr));

    let host = start(&["--dev-no-auth", "--bind", "127.0.0.1:0", "--players", "3"]);
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    let s = c.call("sim.start", json!({})).unwrap();
    assert_eq!(s["player_count"], 3);
}
