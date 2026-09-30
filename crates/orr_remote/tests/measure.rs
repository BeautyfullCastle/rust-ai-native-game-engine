//! Bandwidth and latency of the frame stream over localhost.
//!
//! Numbers are printed (run with `--release -- --nocapture`); the asserts only
//! check that the stream works.
#![allow(clippy::disallowed_types)] // a tool crate: timing uses the wall clock

mod common;

use std::time::{Duration, Instant};

use common::*;
use orr_bridge::{Bridge, SimControl};
use orr_edit::EditorDoc;
use orr_reflect::Scene;
use orr_remote::{Auth, RemoteBridge, RemoteConfig, ServerConfig};
use orr_sample::physics_game::{register_reflect, PhysConfig, PhysGame, SceneMode};
use orr_session::{ControlOp, Speed};
use orr_sim::Simulation;
use serde_json::{json, Value as J};

fn big_doc(bodies: u32) -> EditorDoc {
    let sim = Simulation::<PhysGame>::new(PhysConfig::new(bodies, SceneMode::Rain), 60, 1);
    let mut types = orr_reflect::TypeRegistry::new();
    register_reflect(&mut types);
    let scene = Scene::unbake(&types, sim.frame(), None).expect("unbake");
    EditorDoc::from_scene(scene, types, Simulation::<PhysGame>::build_registry(), SEED).expect("doc")
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

/// What the host thread pays per published frame: `Frame::to_bytes` + lz4.
fn build_cost(doc: &EditorDoc) -> (u64, usize) {
    let frame = doc.frame();
    let iters = 100;
    let t = Instant::now();
    let mut len = 0;
    for _ in 0..iters {
        len = orr_remote::wire::encode_frame_message(&json!({"tick": 1}), &frame.to_bytes()).len();
    }
    (t.elapsed().as_micros() as u64 / iters, len)
}

fn measure(label: &str, make_doc: impl Fn() -> EditorDoc + Send + Clone + 'static, ticks_before: u32) {
    let mut cfg = ServerConfig::new(Auth::Tokens(vec![token("m", "tok", "all")]));
    cfg.limits.max_step_per_call = 5000;
    let (build_us, _) = build_cost(&make_doc());
    let host = TestHost::start_with(cfg, make_doc.clone());
    let mut c = host.client("tok");
    let entities = c.call("sim.state", J::Null).unwrap()["entities"].as_u64().unwrap();
    c.call("sim.start", json!({})).unwrap();
    if ticks_before > 0 {
        c.call("sim.step", json!({"n": ticks_before})).unwrap();
    }
    let head0 = c.call("sim.state", J::Null).unwrap()["head_tick"].as_u64().unwrap();

    // Request round trip (no sim work).
    let mut rtt: Vec<u64> = (0..300)
        .map(|_| {
            let t = Instant::now();
            c.call("sim.state", J::Null).unwrap();
            t.elapsed().as_micros() as u64
        })
        .collect();
    rtt.sort_unstable();

    let mut cfg = RemoteConfig::new(&host.url);
    cfg.token = Some("tok".into());
    cfg.max_fps = 1000;
    let mut b = RemoteBridge::<PhysGame>::connect(cfg).unwrap();
    let start = Instant::now();
    while b.snapshot().is_none() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    let first_snapshot_ms = start.elapsed().as_millis();

    // Step one tick at a time and wait for the snapshot: step request -> snapshot on the view side.
    let mut e2e: Vec<u64> = Vec::new();
    let mut want = head0;
    for _ in 0..200 {
        want += 1;
        let t = Instant::now();
        b.control(ControlOp::Step(1)).unwrap();
        while b.snapshot().is_none_or(|s| s.tick() != want) {
            assert!(t.elapsed() < Duration::from_secs(10), "no snapshot of tick {want}");
            std::hint::spin_loop();
        }
        e2e.push(t.elapsed().as_micros() as u64);
    }
    e2e.sort_unstable();
    let m = b.metrics();

    // Real-time play at 1x (60 ticks/s), 3 seconds: what a viewer receives.
    let before = b.metrics();
    b.control(ControlOp::SetSpeed(Speed(1000))).unwrap();
    b.control(ControlOp::Play).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    b.control(ControlOp::Pause).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let after = b.metrics();
    let frames = after.frames - before.frames;
    let bytes = after.frame_bytes - before.frame_bytes;
    assert!(frames > 100, "a 60 Hz stream should deliver well over 100 frames in 3 s, got {frames}");
    let mean_latency = (after.latency_sum_us - before.latency_sum_us) / frames.max(1);

    println!("== {label}: {entities} entities");
    println!(
        "   frame message: {} bytes on the wire (lz4), {} bytes raw ({}% of raw); decode {} us",
        m.last_frame_bytes,
        m.last_frame_raw_bytes,
        m.last_frame_bytes * 100 / m.last_frame_raw_bytes.max(1),
        m.last_decode_us
    );
    println!("   ERP request round trip (sim.state): median {} us, p95 {} us, max {} us", percentile(&rtt, 50), percentile(&rtt, 95), rtt[rtt.len() - 1]);
    println!(
        "   step(1) request -> snapshot on the view side: median {} us, p95 {} us, max {} us",
        percentile(&e2e, 50),
        percentile(&e2e, 95),
        e2e[e2e.len() - 1]
    );
    println!(
        "   real-time 60 Hz stream: {frames} frames in 3 s = {} KB/s ({} bytes/tick average), server-build -> published on the view side: mean {} us, max {} us",
        bytes / 3 / 1000,
        bytes / frames.max(1),
        mean_latency,
        after.max_latency_us
    );
    println!("   first snapshot after connect: {first_snapshot_ms} ms");
    println!("   host-thread cost per published frame (to_bytes + lz4): {build_us} us");
}

#[test]
fn measure_demo_scene() {
    measure("demo scene", demo_doc, 60);
}

#[test]
fn measure_1000_body_scene() {
    measure("1000-body physics scene", || big_doc(1000), 120);
}
