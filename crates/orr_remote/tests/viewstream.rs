//! The `viewstream` topic of ERP: a view in another process (or machine, or
//! language) gets the schema and the view frames as bytes, over a WebSocket
//! (binary messages) or plain TCP (hex in JSON), and they are the same bytes.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use common::{demo_text, TestHost};
use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, ErpClient, LocalHost, ServerConfig};
use orr_viewstream::{ViewFrame, FLAG_DISCONTINUITY, FLAG_PAUSED, HEADER_LEN, RECORD_LEN};
use serde_json::{json, Value as J};

const WAIT: Duration = Duration::from_secs(20);

fn phys_host() -> LocalHost {
    spawn_phys_host(demo_text(), None, ServerConfig::new(Auth::DevNoAuth)).expect("host")
}

fn subscribed(url: &str) -> ErpClient {
    let mut c = ErpClient::connect(url, None).unwrap();
    let r = c.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1000})).unwrap();
    assert_eq!(r["topics"], json!(["viewstream"]));
    c
}

/// The next frame of at least `tick`.
fn frame_from(c: &mut ErpClient, tick: u64) -> (Vec<u8>, ViewFrame) {
    loop {
        let bytes = c.wait_frame(WAIT).unwrap().unwrap_or_else(|| panic!("no view frame of tick {tick}"));
        let f = ViewFrame::decode(&bytes).unwrap();
        if f.tick >= tick {
            return (bytes, f);
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

#[test]
fn schema_comes_first_then_frames_that_match_the_sim() {
    let host = phys_host();
    let mut c = subscribed(host.url().unwrap());
    let schema = c.wait_notification("watch.viewstream.schema", WAIT).unwrap().expect("the schema is sent right after the subscription");
    let s = &schema["params"];
    assert_eq!(s["format"], "orrery.viewstream");
    assert_eq!(s["game"], "PhysGame");
    assert_eq!(s["player_count"], 2);
    assert_eq!(s["tick_rate"], 60);
    assert_eq!(s["input"]["size"], 24);
    assert_eq!(s["frame"]["record_len"], RECORD_LEN);
    assert_eq!(s["kinds"][3]["name"], "paddle");

    // No session yet: nothing to show. Start one: the first frame is tick 0, a jump, paused.
    c.call("sim.start", json!({})).unwrap();
    let (_, f0) = frame_from(&mut c, 0);
    assert_eq!(f0.tick, 0);
    assert!(f0.has(FLAG_DISCONTINUITY) && f0.has(FLAG_PAUSED));
    let total = c.call("world.query", json!({"limit": 1})).unwrap()["total"].as_u64().unwrap();
    assert_eq!(f0.entities.len() as u64, total, "one record per entity");

    // A step is smooth: prev is the tick before, not a copy of cur.
    c.call("sim.step", json!({"n": 30})).unwrap();
    let (_, f) = frame_from(&mut c, 30);
    assert_eq!(f.tick, 30);
    assert!(!f.has(FLAG_DISCONTINUITY), "stepping is not a jump");
    assert!(f.entities.iter().any(|e| e.prev != e.cur), "moving bodies differ between prev and cur");

    // The numbers are the sim's: a dynamic body's record against `world.get` of the same entity.
    let rec = f.entities.iter().find(|e| e.kind == 1 && e.cur[1] > 0.0).expect("a dynamic body");
    let handle = format!("{}v{}", rec.id & 0xffff_ffff, rec.id >> 32);
    let coord = |c: &mut ErpClient, path: &str| -> f64 {
        let v = c.call("world.get", json!({"entity": handle, "component": "orr_physics::Body", "path": path})).unwrap();
        v["value"].to_string().trim_matches('"').parse::<f64>().unwrap()
    };
    assert!((coord(&mut c, "pos.x") - f64::from(rec.cur[0])).abs() < 1e-3);
    assert!((coord(&mut c, "pos.y") - f64::from(rec.cur[1])).abs() < 1e-3);

    // A seek is a jump: flagged, and prev equals cur.
    c.call("sim.seek", json!({"tick": 10})).unwrap();
    let (_, j) = frame_from(&mut c, 10);
    assert_eq!(j.tick, 10);
    assert!(j.has(FLAG_DISCONTINUITY) && j.has(FLAG_PAUSED));
    assert!(j.entities.iter().all(|e| e.prev == e.cur));
}

#[test]
fn tcp_carries_the_same_bytes_as_the_websocket() {
    let host = phys_host();
    let url = host.url().unwrap().to_string();
    let addr = url.strip_prefix("ws://").unwrap().to_string();
    let mut ws = subscribed(&url);

    let mut tcp = TcpStream::connect(&addr).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let mut lines = BufReader::new(tcp.try_clone().unwrap()).lines();
    writeln!(tcp, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": "watch.subscribe", "params": {"topics": ["viewstream"], "max_fps": 1000}})).unwrap();
    let mut got_schema = false;
    let mut answered = false;
    while !(got_schema && answered) {
        let m: J = serde_json::from_str(&lines.next().expect("the server closed the connection").unwrap()).unwrap();
        if m["id"] == 1 {
            assert_eq!(m["result"]["topics"], json!(["viewstream"]));
            answered = true;
        } else if m["method"] == "watch.viewstream.schema" {
            assert_eq!(m["params"]["game"], "PhysGame");
            got_schema = true;
        }
    }

    // Drive from the WebSocket client, read on both.
    ws.call("sim.start", json!({})).unwrap();
    ws.call("sim.step", json!({"n": 12})).unwrap();
    let (ws_bytes, f) = frame_from(&mut ws, 12);
    assert_eq!(f.tick, 12);
    let tcp_bytes = loop {
        let m: J = serde_json::from_str(&lines.next().expect("the server closed the connection").unwrap()).unwrap();
        if m["method"] == "watch.viewstream" {
            assert_eq!(m["params"]["encoding"], "hex");
            let text = m["params"]["data"].as_str().unwrap();
            let bytes: Vec<u8> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect();
            if ViewFrame::decode(&bytes).unwrap().tick == 12 {
                break bytes;
            }
        }
    };
    assert_eq!(tcp_bytes, ws_bytes, "TCP (hex) and WebSocket (binary) deliver the same message");
}

#[test]
fn python_client_decodes_the_same_frame() {
    // (On Windows `python3` may be a Store stub that fails: skip then too.)
    if !Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("SKIPPED: python3 is not installed");
        return;
    }
    let host = phys_host();
    let url = host.url().unwrap().to_string();
    let mut ws = subscribed(&url);
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/viewstream_client.py");
    let out = Command::new("python3").args([script, "--addr", url.strip_prefix("ws://").unwrap(), "--step", "30", "--show", "2"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the Python client failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let last: J = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    let (bytes, f) = frame_from(&mut ws, 30);
    assert_eq!(f.tick, 30);
    assert_eq!(last["tick"], 30);
    assert_eq!(last["entities"], f.entities.len());
    let expected = format!("0x{:016x}", fnv(&bytes[HEADER_LEN..HEADER_LEN + f.entities.len() * RECORD_LEN]));
    assert_eq!(last["fnv"], expected.as_str(), "Python and Rust decode the same records");
    assert!(stdout.contains("game PhysGame, 2 players"), "{stdout}");
}

#[test]
fn a_host_without_a_view_stream_says_so() {
    let host = TestHost::standard();
    let mut c = ErpClient::connect_with_url_token(&host.url, "tok-all").unwrap();
    let e = c.call_err("watch.subscribe", json!({"topics": ["viewstream"]}));
    assert!(e.message.contains("no view stream"), "{}", e.message);
    // And a host with one refuses a proposal as the source.
    let phys = phys_host();
    let mut c = ErpClient::connect(phys.url().unwrap(), None).unwrap();
    let e = c.call_err("watch.subscribe", json!({"topics": ["viewstream"], "source": "proposal:p1"}));
    assert!(e.message.contains("not a proposal"), "{}", e.message);
}
