//! Test helpers: an in-process ERP host with the physics demo, and the reference result of the
//! scenario through the Rust bridge (the same as `orr_ffi/tests/c_client.rs` computes for the C client).
#![allow(dead_code, clippy::disallowed_types)]

use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, LocalHost, ServerConfig};
use orr_sample::physics_game::{PhysGame, PhysInput, TICK_RATE};
use orr_session::PlaySession;
use orr_tui::headless::fnv;
use orr_viewstream::{HEADER_LEN, RECORD_LEN};

pub const DEMO_SCENE: &str = include_str!("../../../../scenes/physics_demo.scene.yaml");

/// A host on a free loopback port, no authentication.
pub fn host() -> LocalHost {
    spawn_phys_host(DEMO_SCENE.to_string(), None, ServerConfig::new(Auth::DevNoAuth)).expect("host")
}

fn scenario_input(p: i32, t: i32) -> PhysInput {
    PhysInput::new((t / 20 + p) % 3 - 1, (t / 30 + 2 * p) % 3 - 1, (t / 25 + p) % 3 - 1, p == 0 && t % 40 < 5)
}

/// `RESULT ...` of the same scenario through the Rust bridge and the view stream source.
pub fn rust_result(ticks: i32) -> String {
    let doc = orr_remote::sample::phys_doc(DEMO_SCENE).unwrap();
    let mut cfg = doc.play_config(2, TICK_RATE);
    cfg.start_paused = false;
    let session = PlaySession::<PhysGame>::from_frame(cfg, doc.frame()).unwrap();
    let host = PlayHost::new(session, PlayerSlot(0)).with_bot(|_slot, tick| scenario_input(1, tick as i32));
    let mut bridge = InProc::new(host, BridgeConfig::default());
    let mut source = orr_sample::physics_stream::phys_stream_source(0, 2);
    let (mut hash, mut frames, mut count) = (0xcbf2_9ce4_8422_2325u64, 0, 0);
    for t in 1..=ticks {
        bridge.set_input(PlayerSlot(0), scenario_input(0, t)).unwrap();
        bridge.step(1);
        let frame = source.pump(&mut bridge).frame.expect("one frame per tick");
        let bytes = frame.encode();
        count = frame.entities.len();
        hash = fnv(hash, &frame.tick.to_le_bytes());
        hash = fnv(hash, &bytes[HEADER_LEN..HEADER_LEN + count * RECORD_LEN]);
        frames += 1;
    }
    format!("RESULT entities={count} frames={frames} fnv=0x{hash:016x}")
}
