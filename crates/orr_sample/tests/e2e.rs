//! Headless end-to-end run: arena through the bridge, loopback session with
//! latency (so rollbacks happen), view world with smoothing. No window, no GPU.
#![allow(clippy::float_arithmetic)]

use std::time::Duration;

use orr_bridge::{Bridge, InProc, Pacing, Threaded, ThreadedConfig};
use orr_sample::arena_view::{arena_bridge_config, loopback_pair, ArenaExtractor, Keys, Loopback, LOCAL_SLOT};
use orr_view::{InterpMode, RenderItem, ViewConfig, ViewWorld};
use orr_bridge::PlayerSlot;

const FRAMES_PER_TICK: u32 = 3;

/// Renders `ticks` ticks at 3 frames per tick; returns the largest per-frame
/// step of any player circle (by entity), and how many rollbacks happened.
fn run(cfg: ViewConfig, ticks: u32) -> (f32, u64) {
    let mut bridge = InProc::new(loopback_pair(Loopback { latency_ticks: 6, jitter_ticks: 2 }), arena_bridge_config());
    let mut view = ViewWorld::new(ArenaExtractor { remote_mode: InterpMode::Prediction }, cfg);
    let dt = Duration::from_secs_f64(1.0 / 60.0 / f64::from(FRAMES_PER_TICK));
    // The local player walks in a square so both peers keep changing direction.
    let mut last: Vec<(orr_ecs::Entity, f32, f32)> = Vec::new();
    let mut worst = 0.0_f32;
    let mut items: Vec<RenderItem> = Vec::new();
    for tick in 0..ticks {
        let keys = Keys { right: (tick / 40) % 2 == 0, up: (tick / 40) % 2 == 1, ..Keys::default() };
        bridge.set_input(PlayerSlot(LOCAL_SLOT), keys.to_input()).unwrap();
        for f in 0..FRAMES_PER_TICK {
            // The sim advances on the first frame of each tick, like a real clock would.
            bridge.update(if f == 0 { Duration::from_nanos(16_666_667) } else { Duration::ZERO });
            let snap = bridge.snapshot();
            view.update(dt.as_secs_f32(), snap.as_ref());
            items.clear();
            view.render_items(&mut items);
            // Players only (size of the player radius): bullets spawn and vanish.
            let players: Vec<_> = items.iter().filter(|i| i.style.size > 10.0).collect();
            for p in &players {
                if let Some(prev) = last.iter().find(|l| l.0 == p.entity) {
                    let step = (p.transform.pos.x - prev.1).hypot(p.transform.pos.y - prev.2);
                    if tick > 30 {
                        worst = worst.max(step);
                    }
                }
            }
            last = players.iter().map(|p| (p.entity, p.transform.pos.x, p.transform.pos.y)).collect();
        }
    }
    (worst, bridge.snapshot().unwrap().stats().rollbacks)
}

#[test]
fn rollbacks_do_not_pop_the_view() {
    let (smooth, rollbacks) = run(ViewConfig::default(), 900);
    assert!(rollbacks > 20, "expected many rollbacks with latency 6, got {rollbacks}");
    let (raw, _) = run(ViewConfig { correction_tau: 0.0, ..ViewConfig::default() }, 900);
    // Normal motion is 6 units per tick, 2 per frame; the frame after a tick may move a bit more.
    eprintln!("largest per-frame step: smoothed {smooth:.1}, unsmoothed {raw:.1}");
    assert!(smooth < 14.0, "smoothed view popped by {smooth}");
    assert!(raw > smooth * 1.5, "control run should pop more: raw {raw}, smoothed {smooth}");
}

#[test]
fn threaded_bridge_drives_the_same_view() {
    let mut bridge = Threaded::spawn(
        || loopback_pair(Loopback::default()),
        arena_bridge_config(),
        ThreadedConfig { pacing: Pacing::Manual, max_catchup: 8 },
    )
    .unwrap();
    let mut view = ViewWorld::new(ArenaExtractor { remote_mode: InterpMode::Snapshot }, ViewConfig::default());
    let mut items = Vec::new();
    for _ in 0..240 {
        bridge.update(Duration::from_nanos(16_666_667));
        view.update(1.0 / 60.0, bridge.snapshot().as_ref());
    }
    view.render_items(&mut items);
    let players = items.iter().filter(|i| i.style.size > 10.0).count();
    assert_eq!(players, 2, "both players are drawn (local predicted, remote from confirmed frames)");
    assert!(view.playback_tick().is_some());
}
