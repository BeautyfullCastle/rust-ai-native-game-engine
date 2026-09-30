//! Random and malformed input never panics, and the server keeps answering.

mod common;

use std::time::Duration;

use common::*;
use orr_edit::PlayController;
use orr_remote::methods::METHODS;
use orr_remote::{call_local, Caps, ErpClient, ErpTarget, HostLimits};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Map, Value as J};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const PATHS: &[&str] = &[
    "", "pos", "pos.x", "pos.y", "pos.", ".x", "[0]", "[", "]", "a..b", "pos.x.y", "pos[0]", "shape", "shape.radius", "shape.verts[99]",
    "shape.half_extents[1]", "vel", "kind", "flags", "flags[0]", "mask", "layer", "friction", "x[1][2]", "  ", "é", "pos\u{0}",
];
const COMPONENTS: &[&str] = &["orr_physics::Body", "orr_physics::Collider", "PaddleTag", "Scene", "orr_physics::PhysicsState", "Nope", "", "orr_physics::Body ", "é"];
const ENTITIES: &[&str] = &["e_00000001", "e_0000000a", "e_00000031", "e_deadbeef", "0v0", "1v0", "5v0", "4294967295v0", "12v99", "x", "", "e_", "e_ZZZZZZZZ", "11111111111v1"];

fn random_string(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => String::new(),
        1 => (0..rng.below(40)).map(|_| char::from(32 + (rng.next() % 95) as u8)).collect(),
        2 => (0..rng.below(20)).map(|_| char::from_u32((rng.next() % 0x2fff) as u32 + 0x80).unwrap_or('?')).collect(),
        3 => "a".repeat(rng.below(3000)),
        4 => "\"\\\n\t\u{0}\u{7f}".to_string(),
        _ => rng.pick(PATHS).to_string(),
    }
}

fn random_number(rng: &mut Rng) -> J {
    match rng.below(12) {
        0 => json!(0),
        1 => json!(-1),
        2 => json!(u64::MAX),
        3 => json!(i64::MIN),
        4 => serde_json::from_str("1e400").unwrap_or(J::Null),
        5 => serde_json::from_str("-1e-400").unwrap_or(J::Null),
        6 => serde_json::from_str("123456789012345678901234567890").unwrap_or(J::Null),
        7 => serde_json::from_str("0.000000000000000000000000000001").unwrap_or(J::Null),
        8 => serde_json::from_str("140737488355328").unwrap_or(J::Null),
        9 => serde_json::from_str("-140737488355328.5").unwrap_or(J::Null),
        10 => json!((rng.next() as i64) >> rng.below(60)),
        _ => json!(rng.below(100000)),
    }
}

fn random_json(rng: &mut Rng, depth: usize) -> J {
    match rng.below(if depth >= 4 { 5 } else { 8 }) {
        0 => J::Null,
        1 => J::Bool(rng.below(2) == 0),
        2 | 3 => random_number(rng),
        4 => J::String(random_string(rng)),
        5 => J::Array((0..rng.below(5)).map(|_| random_json(rng, depth + 1)).collect()),
        _ => {
            let mut m = Map::new();
            for _ in 0..rng.below(5) {
                let key = match rng.below(4) {
                    0 => "kind".to_string(),
                    1 => "x".to_string(),
                    _ => random_string(rng),
                };
                m.insert(key, random_json(rng, depth + 1));
            }
            J::Object(m)
        }
    }
}

/// Params for a method: mostly plausible keys with random values.
fn random_params(rng: &mut Rng, method: &str) -> J {
    if rng.below(8) == 0 {
        return random_json(rng, 0);
    }
    let mut m = Map::new();
    let docs = METHODS.iter().find(|d| d.name == method);
    if let Some(d) = docs {
        for p in d.params {
            if rng.below(6) == 0 {
                continue; // sometimes leave a parameter out
            }
            let v = match p.name {
                "entity" => J::String(rng.pick(ENTITIES).to_string()),
                "component" if rng.below(3) > 0 => J::String(rng.pick(COMPONENTS).to_string()),
                "path" if rng.below(3) > 0 => J::String(rng.pick(PATHS).to_string()),
                "n" => json!(rng.below(30)),
                "tick" => json!(rng.below(60)),
                "permille" => random_number(rng),
                "topics" => J::Array((0..rng.below(4)).map(|_| json!(*rng.pick(&["tick", "history", "events", "notes", "frames", "x"]))).collect()),
                _ => random_json(rng, 1),
            };
            m.insert(p.name.to_string(), v);
        }
    }
    if rng.below(4) == 0 {
        m.insert(random_string(rng), random_json(rng, 1));
    }
    J::Object(m)
}

fn random_method(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => random_string(rng),
        1 => "world.".to_string(),
        _ => rng.pick(METHODS).name.to_string(),
    }
}

fn valid_request(rng: &mut Rng, id: u64) -> String {
    let method = random_method(rng);
    let params = random_params(rng, &method);
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn random_message(rng: &mut Rng, id: u64) -> String {
    match rng.below(10) {
        0 => (0..rng.below(300)).map(|_| char::from(32 + (rng.next() % 95) as u8)).collect(),
        1 => {
            let s = valid_request(rng, id);
            let cut = rng.below(s.len().max(1));
            s.chars().take(cut).collect()
        }
        2 => {
            // A valid envelope with wrong field types.
            let bad = random_json(rng, 2);
            match rng.below(4) {
                0 => json!({"jsonrpc": "2.0", "id": id, "method": bad}).to_string(),
                1 => json!({"jsonrpc": bad, "id": id, "method": "sim.state"}).to_string(),
                2 => json!({"jsonrpc": "2.0", "id": bad, "method": "sim.state"}).to_string(),
                _ => json!({"jsonrpc": "2.0", "id": id, "method": "sim.state", "params": bad}).to_string(),
            }
        }
        3 => random_json(rng, 0).to_string(),
        4 => format!("{}{}", "[".repeat(rng.below(2000)), "]".repeat(rng.below(50))),
        5 => format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"world.patch\",\"params\":{{\"value\":{}}}}}", "9".repeat(rng.below(400))),
        _ => valid_request(rng, id),
    }
}

/// Sends the messages, then a fence request, and reads every reply up to the fence.
/// Returns the replies (parsed).
fn send_batch(c: &mut ErpClient, msgs: &[String]) -> Vec<J> {
    for m in msgs {
        c.send_text(m).unwrap();
    }
    c.send_text(r#"{"jsonrpc":"2.0","id":"fence","method":"sim.state"}"#).unwrap();
    let mut replies = Vec::new();
    loop {
        let text = c.recv_text(Duration::from_secs(20)).unwrap().expect("no fence reply: the server stopped answering");
        let j: J = serde_json::from_str(&text).unwrap_or_else(|e| panic!("reply is not JSON ({e}): {text}"));
        if j["id"] == "fence" {
            return replies;
        }
        replies.push(j);
    }
}

fn check_reply(j: &J) {
    assert_eq!(j["jsonrpc"], "2.0", "{j}");
    if let Some(e) = j.get("error") {
        let code = e["code"].as_i64().unwrap_or_else(|| panic!("error without a code: {j}"));
        assert!(e["message"].is_string(), "{j}");
        assert_ne!(code, orr_remote::INTERNAL_ERROR, "a request made the host panic: {j}");
    } else if j.get("method").is_none() {
        assert!(j.get("result").is_some(), "{j}");
    }
}

#[test]
fn two_thousand_random_and_malformed_messages_never_panic_the_server() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut replies = 0;
    let mut errors = 0;
    for batch in 0..20u64 {
        let msgs: Vec<String> = (0..100).map(|i| random_message(&mut rng, batch * 100 + i)).collect();
        for r in send_batch(&mut c, &msgs) {
            check_reply(&r);
            replies += 1;
            errors += usize::from(r.get("error").is_some());
        }
    }
    eprintln!("fuzz: {replies} replies, {errors} errors, {} successes", replies - errors);
    assert!(replies > 1000, "{replies} replies");
    assert!(errors > 300, "the fuzz should hit many error paths, hit {errors}");
    assert!(host.is_running(), "the host thread panicked");
    // Still answering, and not confused: a normal session works.
    let mut fresh = host.client("tok-all");
    assert!(fresh.call("world.query", json!({"limit": 3})).unwrap()["total"].as_u64().unwrap() > 0);
    // (The fuzz may have started a play session or edited the scene: stop and load a clean scene.)
    let _ = fresh.call("sim.stop", json!({}));
    let _ = fresh.call("tx.rollback", json!({}));
    fresh.call("scene.load", json!({"text": demo_text()})).unwrap();
    assert_eq!(checksum(&fresh.call("sim.checksum", json!({})).unwrap()), demo_doc().checksum());
    // The unauthenticated path takes garbage too.
    let mut raw = ErpClient::connect(&host.url, None).unwrap();
    for _ in 0..30 {
        let m = random_message(&mut rng, 1);
        if raw.send_text(&m).is_err() {
            break;
        }
    }
    let mut ok = ErpClient::connect(&host.url, Some("tok-read")).unwrap();
    ok.call("sim.state", J::Null).unwrap();
}

/// The same without a socket: the dispatcher directly, in edit mode and in play mode.
#[test]
fn the_dispatcher_survives_random_calls_in_edit_and_play_mode() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let limits = HostLimits { max_step_per_call: 20, ..HostLimits::default() };
    for round in 0..4 {
        let mut doc = demo_doc();
        let mut play: Option<PlayController<PhysGame>> = None;
        if round % 2 == 1 {
            play = Some(PlayController::start_play(&doc, doc.play_config(2, 60)).unwrap());
        }
        for _ in 0..3000 {
            let method = random_method(&mut rng);
            let method = if method.starts_with("watch.") { "sim.state".to_string() } else { method };
            let params = random_params(&mut rng, &method);
            let caps = if rng.below(5) == 0 { Caps::parse("read").unwrap() } else { Caps::ALL };
            let mut target = ErpTarget { doc: &mut doc, play: &mut play };
            match call_local(&mut target, &limits, "fuzz", caps, &method, &params) {
                Ok(_) => {}
                Err(e) => {
                    assert_ne!(e.code, orr_remote::INTERNAL_ERROR, "{method} {params}: {e}");
                    assert!(e.data.as_ref().is_some_and(|d| d["kind"].is_string()), "{method}: {e:?}");
                }
            }
        }
        // The state is still sane: the scene serializes and reloads.
        let text = doc.to_yaml();
        let mut again = demo_doc();
        again.load_yaml(&text).expect("the scene after the fuzz is valid");
    }
}
