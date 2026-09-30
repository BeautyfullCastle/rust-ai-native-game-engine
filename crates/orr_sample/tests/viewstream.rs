//! The physics scene as a view stream: the frames a foreign view would get
//! show what the Rust viewport draws, carry the rollback flag, derive the
//! spawn/despawn lifecycle from id sets, and have the sizes the docs say.
//! No window, no GPU.
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeSet;

use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_ecs::Entity;
use orr_edit::EditorDoc;
use orr_physics::Body;
use orr_reflect::TypeRegistry;
use orr_sample::arena_view::Loopback;
use orr_sample::physics_game::{register_reflect, PhysConfig, PhysGame, PhysInput, SceneMode, TICK_RATE};
use orr_sample::physics_host::{physics_pair, SimMetrics};
use orr_sample::physics_stream::{phys_schema, phys_stream_source, KIND_DYNAMIC, KIND_PADDLE};
use orr_sample::physics_view::PhysExtractor;
use orr_session::{PlayConfig, PlaySession};
use orr_sim::Simulation;
use orr_view::{InterpMode, RenderItem, ViewConfig, ViewWorld};
use orr_viewstream::*;

const DEMO_SCENE: &str = include_str!("../../../scenes/physics_demo.scene.yaml");

fn demo_session() -> PlaySession<PhysGame> {
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    let doc = EditorDoc::from_yaml(DEMO_SCENE, types, Simulation::<PhysGame>::build_registry(), 7).unwrap();
    let mut cfg: PlayConfig = doc.play_config(2, TICK_RATE);
    // Stepped by the bridge's clock calls: a paused session would ignore them.
    cfg.start_paused = false;
    PlaySession::<PhysGame>::from_frame(cfg, doc.frame()).unwrap()
}

fn demo_bridge() -> InProc<PhysGame, PlayHost<PhysGame>> {
    let host = PlayHost::new(demo_session(), PlayerSlot(0)).with_bot(|_slot, tick| PhysInput::new(((tick / 20) % 3) as i32 - 1, 0, 0, tick % 40 < 5));
    InProc::new(host, BridgeConfig::default())
}

fn player_input(tick: u64) -> PhysInput {
    PhysInput::new(((tick / 25) % 3) as i32 - 1, ((tick / 35) % 3) as i32 - 1, 0, false)
}

#[test]
fn frame_shows_what_the_viewport_draws() {
    let mut bridge = demo_bridge();
    let mut source = phys_stream_source(1, 2);
    for t in 1..=40u64 {
        bridge.set_input(PlayerSlot(0), player_input(t)).unwrap();
        bridge.step(1);
    }
    let snap = bridge.snapshot().unwrap();
    let pumped = source.pump(&mut bridge);
    let frame = pumped.frame.expect("the bridge published a snapshot");
    assert_eq!(frame.tick, 40);
    assert_eq!(frame.verified_tick, 40);
    assert!(!frame.has(FLAG_ROLLED_BACK) && !frame.has(FLAG_DISCONTINUITY));
    assert_eq!(frame.entities.len() as u32, snap.predicted().alive_count(), "every entity of the demo scene is streamed");

    // The Rust viewport (ViewWorld + PhysExtractor): at alpha 0 it shows the
    // previous tick, at alpha 1 the current one. The stream's records must match both.
    let extractor = || PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 };
    let mut at_prev = ViewWorld::new(extractor(), ViewConfig::default());
    at_prev.update(0.0, Some(&snap));
    let mut at_cur = ViewWorld::new(extractor(), ViewConfig::default());
    at_cur.update(1.0 / TICK_RATE as f32, Some(&snap));
    let (mut prev_items, mut cur_items) = (Vec::new(), Vec::new());
    at_prev.render_items(&mut prev_items);
    at_cur.render_items(&mut cur_items);
    assert_eq!(prev_items.len(), frame.entities.len());
    let by_id = |items: &[RenderItem], id: u64| items.iter().find(|i| entity_id(i.entity) == id).copied().unwrap();
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    for rec in &frame.entities {
        let (p, c) = (by_id(&prev_items, rec.id), by_id(&cur_items, rec.id));
        assert!(close(rec.prev[0], p.transform.pos.x) && close(rec.prev[1], p.transform.pos.y), "prev position of {:x}", rec.id);
        assert!(close(rec.cur[0], c.transform.pos.x) && close(rec.cur[1], c.transform.pos.y), "cur position of {:x}", rec.id);
        assert!(close(rec.cur[2], c.transform.rot), "cur rotation of {:x}", rec.id);
        assert!(close(rec.size, c.style.size) && close(rec.half_y, c.style.half_y), "size of {:x}", rec.id);
        for (q, f) in rec.rgba.iter().zip(c.style.color) {
            assert!((f32::from(*q) / 255.0 - f).abs() <= 0.5 / 255.0 + 1e-6, "color of {:x}", rec.id);
        }
    }
    // And the sim itself: a dynamic body's record is its Body, converted once.
    let view = snap.predicted();
    let mut dynamic = 0;
    for rec in frame.entities.iter().filter(|r| r.kind == KIND_DYNAMIC) {
        let e = Entity { index: rec.id as u32, version: (rec.id >> 32) as u32 };
        let body = view.get::<Body>(e).unwrap();
        assert_eq!(rec.cur[0], body.pos.x.to_f32());
        assert_eq!(rec.cur[1], body.pos.y.to_f32());
        dynamic += 1;
    }
    assert!(dynamic > 0);
    let schema = phys_schema(1, 2, TICK_RATE);
    let props = frame.props_by_entity(|k| schema.props_words(k)).expect("the property section adds up");
    let paddles: Vec<u32> = frame
        .entities
        .iter()
        .zip(&props)
        .filter(|(r, _)| r.kind == KIND_PADDLE)
        .map(|(_, p)| u32::from_le_bytes(p[..4].try_into().unwrap()))
        .collect();
    assert_eq!(paddles.iter().copied().collect::<BTreeSet<_>>(), BTreeSet::from([0, 1]), "one paddle per player slot");
    // The same frame, through bytes.
    assert_eq!(ViewFrame::decode(&frame.encode()).unwrap(), frame);
}

#[test]
fn input_layout_comes_from_the_reflection_of_the_input_type() {
    let schema = phys_schema(0xabc, 2, TICK_RATE).to_json();
    assert_eq!(schema["input"]["size"], 24);
    let fields: Vec<(String, u64, u64, String)> = schema["input"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), f["offset"].as_u64().unwrap(), f["size"].as_u64().unwrap(), f["type"].as_str().unwrap().to_string()))
        .collect();
    let expect = |n: &str, o, s, t: &str| (n.to_string(), o, s, t.to_string());
    assert_eq!(fields, vec![expect("axis_x", 0, 8, "fixed"), expect("axis_y", 8, 8, "fixed"), expect("spin", 16, 4, "i32"), expect("buttons", 20, 4, "flags")]);
    assert_eq!(schema["build_id"], "0x0000000000000abc");
    assert_eq!(schema["player_count"], 2);
    assert_eq!(schema["kinds"].as_array().unwrap().len(), 4);
    // The layout is the real one.
    assert_eq!(std::mem::size_of::<PhysInput>(), 24);
    assert_eq!(std::mem::offset_of!(PhysInput, buttons), 20);
}

#[test]
fn a_rollback_sets_the_flag_and_the_range() {
    let scene = PhysConfig::new(40, SceneMode::Rain);
    let pair = physics_pair(scene, Loopback { latency_ticks: 6, jitter_ticks: 2 }, SimMetrics::new());
    let mut bridge = InProc::new(pair, BridgeConfig::default());
    let mut source = phys_stream_source(1, 2);
    let (mut frames, mut rolled_back) = (Vec::new(), Vec::new());
    for t in 1..=240u64 {
        bridge.set_input(PlayerSlot(0), player_input(t)).unwrap();
        bridge.step(1);
        let pumped = source.pump(&mut bridge);
        let Some(f) = pumped.frame else { continue };
        if f.has(FLAG_ROLLED_BACK) {
            rolled_back.push(f.clone());
        }
        frames.push(f);
    }
    assert_eq!(frames.len(), 240, "one frame per published snapshot");
    let stats = bridge.snapshot().unwrap().stats();
    assert!(stats.rollbacks > 0, "the latency link must cause rollbacks");
    assert!(!rolled_back.is_empty(), "the stream says so");
    assert!(rolled_back.len() as u64 <= stats.rollbacks);
    for f in &rolled_back {
        let (from, to) = f.rollback.expect("a rollback frame carries its range");
        assert!(from <= to && to <= f.tick, "range {from}..{to} inside the head {}", f.tick);
    }
    assert!(frames.iter().any(|f| f.verified_tick < f.tick), "with prediction the verified tick trails the head");
    assert!(frames.iter().filter(|f| !f.has(FLAG_ROLLED_BACK)).all(|f| f.rollback.is_none()));
    assert!(frames.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
}

#[test]
fn spawn_and_despawn_are_derived_from_id_sets() {
    let mut scene = PhysConfig::new(40, SceneMode::Rain);
    scene.spawn_rate = 120;
    let session = PlaySession::<PhysGame>::new(PlayConfig::new(2, 5, TICK_RATE), scene);
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let mut source = phys_stream_source(1, 2);
    let mut known: BTreeSet<u64> = BTreeSet::new();
    let mut spawned_total = 0;
    for _ in 0..60 {
        bridge.step(1);
        let f = source.pump(&mut bridge).frame.unwrap();
        let now: BTreeSet<u64> = f.entities.iter().map(|e| e.id).collect();
        assert_eq!(now.len(), f.entities.len(), "ids are unique within a frame");
        let (spawned, despawned) = (now.difference(&known).count(), known.difference(&now).count());
        assert_eq!(despawned, 0, "nothing despawns in this scene");
        spawned_total += spawned;
        known = now;
    }
    assert!(spawned_total > 40, "the initial bodies and the spawned ones all show up as new ids (saw {spawned_total})");
}

/// Measures the size of the stream (the numbers in `docs/view-stream.md`).
#[test]
fn bytes_per_tick() {
    let mut bridge = demo_bridge();
    let mut source = phys_stream_source(1, 2);
    bridge.step(3);
    let demo = source.pump(&mut bridge).frame.unwrap();
    let demo_bytes = demo.encode().len();
    let props_bytes = demo.props.len();
    assert_eq!(demo_bytes, HEADER_LEN + demo.entities.len() * RECORD_LEN + props_bytes);

    let session = PlaySession::<PhysGame>::new(PlayConfig::new(2, 5, TICK_RATE), PhysConfig::new(1000, SceneMode::Pile));
    let mut big = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    big.step(3);
    let f = phys_stream_source(1, 2).pump(&mut big).frame.unwrap();
    let big_bytes = f.encode().len();
    println!(
        "view stream size: demo scene {} entities = {demo_bytes} bytes/tick ({} KB/s at {TICK_RATE} Hz); 1000 bodies {} entities = {big_bytes} bytes/tick ({} KB/s)",
        demo.entities.len(),
        demo_bytes * TICK_RATE as usize / 1000,
        f.entities.len(),
        big_bytes * TICK_RATE as usize / 1000,
    );
    assert!(demo_bytes < 4096, "the demo scene fits in a few KB per tick");
    assert!(big_bytes < 64 * 1024);
}
