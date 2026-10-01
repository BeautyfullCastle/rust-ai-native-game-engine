//! The sim side of the Yard3D sample: a two-peer loopback session with a
//! bot on the second peer (like the 2D physics sample), timed per call.

use std::sync::Arc;

use orr_bridge::BridgeConfig;
use orr_fp::FrameRng;
use orr_session::SessionConfig;
use orr_sim::PlayerSlot;

use crate::arena_view::{Loopback, LOCAL_SLOT};
use crate::physics_host::{SimMetrics, TimedPair};
use crate::yard3d_game::{NoCommand, Yard3D, YardConfig, YardInput, SHOOT, SPAWN_BALL, TICK_RATE, YARD_HALF};

const NET_SEED: u64 = 777;
const SESSION_SEED: u64 = 42;

/// Session settings of the local peer (slot 0) and the bot peer (slot 1).
pub fn session_configs() -> (SessionConfig, SessionConfig) {
    (
        SessionConfig::new(2, PlayerSlot(LOCAL_SLOT), SESSION_SEED, TICK_RATE),
        SessionConfig::new(2, PlayerSlot(1), SESSION_SEED, TICK_RATE),
    )
}

/// A ray from `origin` (units) toward `target` (units) as an input, integers only.
fn aim(buttons: u32, origin: [i32; 3], target: [i32; 3]) -> YardInput {
    let d = [target[0] - origin[0], target[1] - origin[1], target[2] - origin[2]];
    let len = ((d[0] as i64).pow(2) + (d[1] as i64).pow(2) + (d[2] as i64).pow(2)).isqrt().max(1);
    YardInput {
        buttons,
        _pad: 0,
        origin: origin.map(|c| c * 100),
        dir: d.map(|c| (i64::from(c) * 1000 / len) as i32),
    }
}

/// A bot that shoots balls into the yard from a random spot on a ring, and
/// sometimes drops a ball. It picks a new spot every 30 to 90 ticks, often
/// enough that the local peer keeps mispredicting it.
pub fn yard_bot(seed: u64) -> impl FnMut(u64) -> (YardInput, Vec<NoCommand>) {
    let mut rng = FrameRng::new(seed);
    let mut current = YardInput::default();
    let mut left = 0u32;
    let mut mode = 0u32;
    move |tick| {
        if left == 0 {
            let angle = rng.range_i32(0, 360) as f64 * std::f64::consts::PI / 180.0;
            let radius = f64::from(YARD_HALF) + 6.0;
            let origin = [(angle.cos() * radius) as i32, 10, (angle.sin() * radius) as i32];
            let target = [rng.range_i32(-10, 11), 0, rng.range_i32(-10, 11)];
            mode = rng.next_u32() % 4;
            let buttons = if mode == 0 { SPAWN_BALL } else { SHOOT };
            current = aim(buttons, origin, target);
            left = 30 + rng.next_u32() % 60;
        }
        left -= 1;
        // Hold the button in bursts.
        let held = if mode == 0 { tick % 30 < 12 } else { tick % 60 < 20 };
        (if held { current } else { YardInput { buttons: 0, ..current } }, Vec::new())
    }
}

/// The local player's input in headless runs: shoots at the middle of the yard in bursts, from a slowly turning spot.
pub fn scripted_local(tick: u64) -> YardInput {
    if tick % 120 >= 20 {
        return YardInput::default();
    }
    let angle = (tick / 120) as f64 * 0.7;
    let origin = [(angle.cos() * 20.0) as i32, 9, (angle.sin() * 20.0) as i32];
    aim(SHOOT, origin, [0, 1, 0])
}

/// The Yard3D scene as a two-peer loopback session with simulated latency.
pub fn yard_pair(scene: YardConfig, net: Loopback, metrics: Arc<SimMetrics>) -> TimedPair<Yard3D> {
    let (cfg_a, cfg_b) = session_configs();
    TimedPair::new(move || scene, cfg_a, cfg_b, net, NET_SEED, yard_bot(1234), metrics)
}

/// Bridge settings of the sample: timing goes to `metrics`.
pub fn yard_bridge_config(metrics: Arc<SimMetrics>) -> BridgeConfig<Yard3D> {
    BridgeConfig::default().with_step_observer(move |t| metrics.record_step(t))
}
