//! Headless tests of interpolation and rollback smoothing with synthetic
//! sim states (no bridge, no GPU).
#![allow(clippy::float_arithmetic)]

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_view::{
    lerp_angle, Extracted, Extractor, InterpMode, RenderItem, Shape, Style, Transform2, Vec2, ViewConfig, ViewLifecycle,
    ViewWorld,
};

struct NoExtract;
impl Extractor for NoExtract {
    fn extract(&self, _: FrameView<'_>, _: &mut Vec<Extracted>) {}
}

const STYLE: Style = Style { shape: Shape::Circle, size: 1.0, half_y: 0.0, color: [1.0; 4] };
const RATE: f32 = 60.0;

fn ent(index: u32, version: u32) -> Entity {
    Entity { index, version }
}

fn item(e: Entity, x: f32, y: f32, mode: InterpMode) -> Extracted {
    Extracted { entity: e, transform: Transform2::new(Vec2::new(x, y), 0.0), mode, style: STYLE }
}

fn world(cfg: ViewConfig) -> ViewWorld<NoExtract> {
    ViewWorld::new(NoExtract, cfg)
}

fn items(w: &ViewWorld<NoExtract>) -> Vec<RenderItem> {
    let mut out = Vec::new();
    w.render_items(&mut out);
    out
}

fn pos_of(w: &ViewWorld<NoExtract>, e: Entity) -> Option<Vec2> {
    items(w).iter().find(|i| i.entity == e).map(|i| i.transform.pos)
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn interpolates_between_the_last_two_ticks_by_alpha() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(5, &[item(e, 0.0, 0.0, InterpMode::Prediction)], &[item(e, 10.0, 4.0, InterpMode::Prediction)], false);
    assert_eq!(pos_of(&w, e), Some(Vec2::new(0.0, 0.0)), "alpha 0 shows the previous tick");
    w.advance(0.25 / RATE);
    let p = pos_of(&w, e).unwrap();
    assert!(near(p.x, 2.5, 1e-4) && near(p.y, 1.0, 1e-4), "{p:?}");
    w.advance(0.25 / RATE);
    let p = pos_of(&w, e).unwrap();
    assert!(near(p.x, 5.0, 1e-4) && near(p.y, 2.0, 1e-4), "{p:?}");
    w.advance(5.0 / RATE);
    assert_eq!(pos_of(&w, e), Some(Vec2::new(10.0, 4.0)), "alpha is clamped at 1");
}

#[test]
fn none_mode_shows_the_newest_tick() {
    let e = ent(1, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(2, &[item(e, 0.0, 0.0, InterpMode::None)], &[item(e, 10.0, 0.0, InterpMode::None)], false);
    assert_eq!(pos_of(&w, e), Some(Vec2::new(10.0, 0.0)));
}

#[test]
fn angles_take_the_short_way() {
    let deg = std::f32::consts::PI / 180.0;
    let mid = lerp_angle(359.0 * deg, 1.0 * deg, 0.5);
    // 0 or 360 degrees, not 180.
    assert!(near(mid.sin(), 0.0, 1e-4) && mid.cos() > 0.99, "{mid}");
    assert!(near(lerp_angle(-3.0, 3.0, 0.5).cos(), -1.0, 0.01), "the short way here goes through pi");
}

#[test]
fn spawn_shows_at_its_first_position_and_despawn_removes() {
    let e = ent(7, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(1, &[], &[], false);
    w.push_predicted(2, &[], &[item(e, 100.0, 100.0, InterpMode::Prediction)], false);
    for i in 0..=10 {
        w.advance(0.1 / RATE);
        assert_eq!(pos_of(&w, e), Some(Vec2::new(100.0, 100.0)), "streak from nowhere at step {i}");
    }
    assert_eq!(w.take_lifecycle(), vec![ViewLifecycle::Spawned(e)]);
    w.push_predicted(3, &[item(e, 100.0, 100.0, InterpMode::Prediction)], &[], false);
    assert_eq!(pos_of(&w, e), None);
    assert_eq!(w.take_lifecycle(), vec![ViewLifecycle::Despawned(e)]);
}

#[test]
fn a_reused_entity_index_is_a_new_entity() {
    let old = ent(3, 1);
    let new = ent(3, 2);
    let mut w = world(ViewConfig::default());
    w.push_predicted(1, &[], &[item(old, 0.0, 0.0, InterpMode::Prediction)], false);
    w.advance(1.0 / RATE);
    w.push_predicted(2, &[item(old, 0.0, 0.0, InterpMode::Prediction)], &[item(old, 0.0, 0.0, InterpMode::Prediction)], false);
    w.advance(1.0 / RATE);
    // Same index, next version, close by: matching by index alone would
    // interpolate from (0,0) and start a 50-unit correction.
    w.push_predicted(3, &[item(old, 0.0, 0.0, InterpMode::Prediction)], &[item(new, 50.0, 0.0, InterpMode::Prediction)], false);
    assert_eq!(w.correction_offset(new), Some(0.0));
    assert_eq!(pos_of(&w, old), None);
    for _ in 0..20 {
        assert_eq!(pos_of(&w, new), Some(Vec2::new(50.0, 0.0)));
        w.advance(0.1 / RATE);
    }
    let events = w.take_lifecycle();
    assert!(events.contains(&ViewLifecycle::Despawned(old)) && events.contains(&ViewLifecycle::Spawned(new)));
}

/// Drives a 60 Hz sim into a 144 Hz renderer with jittered publish times.
/// The entity moves `speed` units per tick; from tick `rollback_tick` on, the
/// sim's positions (past ones too) are `shift` units further, as after a
/// rollback that changed an input. Returns the rendered x per frame.
fn run_rollback(cfg: ViewConfig, shift: f32, seconds: f32) -> (Vec<f32>, f32) {
    let e = ent(0, 0);
    let speed = 6.0_f32;
    let rollback_tick = 60u64;
    let dt = 1.0 / 144.0;
    // Snapshots from the rollback tick on hold the corrected history (both
    // frames of the pair); earlier snapshots hold the old guess.
    let head_state = |head: u64, shifted: bool| {
        let shifted = shifted && head >= rollback_tick;
        let f = |t: u64| speed * t as f32 + if shifted && t + 3 >= rollback_tick { shift } else { 0.0 };
        (item(e, f(head.saturating_sub(1)), 0.0, InterpMode::Prediction), item(e, f(head), 0.0, InterpMode::Prediction))
    };

    let mut w = world(cfg);
    let mut next_head = 1u64;
    let mut xs = Vec::new();
    let frames = (seconds / dt) as usize;
    for i in 0..frames {
        let now = i as f32 * dt;
        // Publish every tick whose (jittered) time has come.
        loop {
            let jitter = ((next_head * 7919) % 5) as f32 * 0.001 - 0.002;
            if next_head as f32 / RATE + jitter <= now {
                let (prev, cur) = head_state(next_head, true);
                w.push_predicted(next_head, &[prev], &[cur], false);
                next_head += 1;
            } else {
                break;
            }
        }
        w.advance(dt);
        // NaN before the entity first exists keeps index and frame number equal.
        xs.push(pos_of(&w, e).map_or(f32::NAN, |p| p.x));
    }
    let final_offset = w.correction_offset(e).unwrap_or(0.0);
    (xs, final_offset)
}

fn max_step(xs: &[f32]) -> f32 {
    xs.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn rollback_correction_is_smoothed_without_a_pop() {
    let shift = 30.0;
    // Baseline: the same run without a rollback. Publish-time jitter alone makes
    // some frames move up to twice the normal 2.5 units (the clock holds at
    // alpha 1 while the next tick is late).
    let (baseline_xs, _) = run_rollback(ViewConfig::default(), 0.0, 2.0);
    let baseline = max_step(&baseline_xs);
    let (xs, final_offset) = run_rollback(ViewConfig::default(), shift, 2.0);
    let worst = max_step(&xs);
    // The correction adds at most its first decay step, 30 * (1 - exp(-dt / tau)), about 1.7.
    assert!(worst <= baseline + 2.5, "pop of {worst} units with the rollback, {baseline} without");
    assert!(worst < shift / 3.0, "the pop is not much smaller than the error of {shift}");
    assert!(final_offset < 0.05, "offset did not settle: {final_offset}");
    // It converged onto the corrected track: x follows the sim, about one tick
    // behind the newest tick (half a tick to one and a half with clock jitter).
    let last_frame_time = (xs.len() - 1) as f32 / 144.0;
    let shown_tick = (*xs.last().unwrap() - shift) / 6.0;
    let lag_ticks = last_frame_time * RATE - shown_tick;
    assert!((0.3..=1.6).contains(&lag_ticks), "shows tick {shown_tick} at time {last_frame_time}, {lag_ticks} ticks behind");
}

#[test]
fn without_smoothing_the_same_rollback_pops_by_the_full_error() {
    // Control for the test above: with the decay off the correction is one jump.
    let cfg = ViewConfig { correction_tau: 0.0, ..ViewConfig::default() };
    let (xs, _) = run_rollback(cfg, 30.0, 2.0);
    assert!(max_step(&xs) >= 25.0, "expected a jump of about 30, got {}", max_step(&xs));
}

#[test]
fn correction_offset_converges_monotonically() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(1, &[item(e, 0.0, 0.0, InterpMode::Prediction)], &[item(e, 0.0, 0.0, InterpMode::Prediction)], false);
    w.advance(1.0 / RATE);
    // The sim now says the entity was 20 units to the right all along.
    w.push_predicted(2, &[item(e, 20.0, 0.0, InterpMode::Prediction)], &[item(e, 20.0, 0.0, InterpMode::Prediction)], false);
    let mut last = w.correction_offset(e).unwrap();
    assert!(near(last, 20.0, 0.5), "the offset holds the error at first: {last}");
    let before = pos_of(&w, e).unwrap().x;
    assert!(near(before, 0.0, 0.5), "no jump on the correction frame: {before}");
    for _ in 0..(3 * 144) {
        w.advance(1.0 / 144.0);
        let now = w.correction_offset(e).unwrap();
        assert!(now <= last, "offset grew: {last} -> {now}");
        last = now;
    }
    assert_eq!(last, 0.0);
    assert_eq!(pos_of(&w, e), Some(Vec2::new(20.0, 0.0)));
}

#[test]
fn a_teleport_is_not_smoothed() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(1, &[item(e, 0.0, 0.0, InterpMode::Prediction)], &[item(e, 0.0, 0.0, InterpMode::Prediction)], false);
    w.advance(1.0 / RATE);
    w.push_predicted(2, &[item(e, 1000.0, 0.0, InterpMode::Prediction)], &[item(e, 1000.0, 0.0, InterpMode::Prediction)], false);
    assert_eq!(w.correction_offset(e), Some(0.0));
    assert!(near(pos_of(&w, e).unwrap().x, 1000.0, 1e-3));
}

#[test]
fn snapshot_mode_plays_confirmed_frames_late_and_never_shows_unconfirmed() {
    let e = ent(0, 0);
    let cfg = ViewConfig::default();
    let mut w = world(cfg);
    let dt = 1.0 / 144.0;
    let speed = 6.0;
    let mut next_verified = 10u64;
    let mut shown_tick_lag = Vec::new();
    let mut xs = Vec::new();
    for i in 0..(4 * 144) {
        let now = i as f32 * dt;
        // Confirmed frames arrive at 60 Hz, starting at tick 10.
        while (next_verified - 9) as f32 / RATE <= now {
            let it = item(e, speed * next_verified as f32, 0.0, InterpMode::Snapshot);
            w.push_verified(next_verified, &[it]);
            // The predicted head runs 5 ticks ahead; Snapshot items must be ignored there.
            let h = next_verified + 5;
            let ahead = item(e, speed * h as f32, 0.0, InterpMode::Snapshot);
            w.push_predicted(h, &[ahead], &[ahead], false);
            next_verified += 1;
        }
        w.advance(dt);
        let newest = next_verified - 1;
        if let Some(p) = pos_of(&w, e) {
            let shown_tick = p.x / speed;
            assert!(shown_tick <= newest as f32 + 1e-3, "shows the future: tick {shown_tick} > verified {newest}");
            shown_tick_lag.push(newest as f32 - shown_tick);
            xs.push(p.x);
        }
    }
    assert!(w.take_lifecycle().is_empty(), "Snapshot entities must not appear as predicted ones");
    let settled = &shown_tick_lag[shown_tick_lag.len() / 2..];
    for lag in settled {
        assert!((lag - cfg.snapshot_delay_ticks).abs() < 1.25, "lag {lag} ticks, wanted about {}", cfg.snapshot_delay_ticks);
    }
    // Smooth: playback moves about one tick per tick (2.5 units per frame), no jumps.
    let worst = max_step(&xs[xs.len() / 2..]);
    assert!(worst < 4.5, "playback jumped by {worst} units");
}
