//! Headless tests of 3D interpolation (slerp) and rollback smoothing with
//! synthetic sim states (no bridge, no GPU).
#![allow(clippy::float_arithmetic)]

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{fp, FPQuat, FPVec3};
use orr_view::{
    fp_to_quat, fp_to_vec3, Extracted3, Extractor3, InterpMode, Quat, RenderItem3, Shape3, Style3, Transform3, Vec3,
    ViewConfig, ViewLifecycle, ViewWorld3,
};

struct NoExtract;
impl Extractor3 for NoExtract {
    fn extract(&self, _: FrameView<'_>, _: &mut Vec<Extracted3>) {}
}

const RATE: f32 = 60.0;

fn style() -> Style3 {
    Style3::new(Shape3::Sphere { radius: 1.0 }, [1.0; 3])
}

fn ent(index: u32, version: u32) -> Entity {
    Entity { index, version }
}

fn about_y(a: f32) -> Quat {
    Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), a)
}

fn item(e: Entity, pos: Vec3, rot: Quat, mode: InterpMode) -> Extracted3 {
    Extracted3 { entity: e, transform: Transform3::new(pos, rot), mode, style: style() }
}

fn world(cfg: ViewConfig) -> ViewWorld3<NoExtract> {
    ViewWorld3::new(NoExtract, cfg)
}

fn shown(w: &ViewWorld3<NoExtract>, e: Entity) -> Option<Transform3> {
    let mut out: Vec<RenderItem3> = Vec::new();
    w.render_items(&mut out);
    out.iter().find(|i| i.entity == e).map(|i| i.transform)
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

#[test]
fn position_lerps_and_rotation_slerps_by_alpha() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(
        5,
        &[item(e, v(0.0, 0.0, 0.0), about_y(0.0), InterpMode::Prediction)],
        &[item(e, v(10.0, 4.0, -2.0), about_y(1.2), InterpMode::Prediction)],
        false,
    );
    let t = shown(&w, e).unwrap();
    assert_eq!(t.pos, v(0.0, 0.0, 0.0));
    assert!(t.rot.angle_to(about_y(0.0)) < 1e-5, "alpha 0 shows the previous tick");
    w.advance(0.5 / RATE);
    let t = shown(&w, e).unwrap();
    assert!((t.pos - v(5.0, 2.0, -1.0)).length() < 1e-4, "{:?}", t.pos);
    assert!(t.rot.angle_to(about_y(0.6)) < 1e-4, "halfway is 0.6 rad, got {:?}", t.rot);
    // Rotation interpolates along the shortest arc even when the quaternion sign flipped between ticks.
    let flipped = Quat::new(-about_y(0.3).x, -about_y(0.3).y, -about_y(0.3).z, -about_y(0.3).w);
    let mut w = world(ViewConfig::default());
    w.push_predicted(
        2,
        &[item(e, Vec3::ZERO, about_y(0.1), InterpMode::Prediction)],
        &[item(e, Vec3::ZERO, flipped, InterpMode::Prediction)],
        false,
    );
    w.advance(0.5 / RATE);
    assert!(shown(&w, e).unwrap().rot.angle_to(about_y(0.2)) < 1e-4);
    w.advance(5.0 / RATE);
    assert!(shown(&w, e).unwrap().rot.angle_to(about_y(0.3)) < 1e-5, "alpha is clamped at 1");
}

#[test]
fn none_mode_shows_the_newest_pose_and_the_quaternion_stays_unit() {
    let e = ent(1, 0);
    let mut w = world(ViewConfig::default());
    w.push_predicted(
        2,
        &[item(e, Vec3::ZERO, about_y(0.0), InterpMode::None)],
        &[item(e, v(10.0, 0.0, 0.0), about_y(2.0), InterpMode::None)],
        false,
    );
    let t = shown(&w, e).unwrap();
    assert_eq!(t.pos, v(10.0, 0.0, 0.0));
    assert!(t.rot.angle_to(about_y(2.0)) < 2e-3);
    assert!((t.rot.dot(t.rot) - 1.0).abs() < 1e-5);
}

/// Drives a 60 Hz sim into a 144 Hz renderer. The entity moves `speed` units per tick and turns
/// `spin` radians per tick about y. From `rollback_tick` on, the sim's poses (past ones too) are
/// `shift` units and `twist` radians further, as after a rollback that changed an input. Returns
/// the shown (x, yaw) per frame.
fn run_rollback(cfg: ViewConfig, shift: f32, twist: f32, seconds: f32) -> Vec<(f32, f32)> {
    let e = ent(0, 0);
    let (speed, spin) = (0.2, 0.05);
    let pose = |tick: i64, shifted: bool| {
        let (dx, dr) = if shifted { (shift, twist) } else { (0.0, 0.0) };
        (v(tick as f32 * speed + dx, 0.0, 0.0), about_y(tick as f32 * spin + dr))
    };
    let rollback_tick = 120i64;
    let mut w = world(cfg);
    let dt = 1.0 / 144.0;
    let mut out = Vec::new();
    let (mut sim_t, mut tick, mut last) = (0.0f32, 0i64, 0i64);
    let mut t = 0.0f32;
    while t < seconds {
        t += dt;
        // The sim ticks at 60 Hz of its own clock.
        sim_t += dt;
        while sim_t >= 1.0 / RATE {
            sim_t -= 1.0 / RATE;
            tick += 1;
        }
        if tick != last {
            last = tick;
            let shifted = tick >= rollback_tick;
            let (p, r) = pose(tick, shifted);
            // The previous tick as the sim knows it now: shifted too once the rollback happened.
            let (pp, pr) = pose(tick - 1, shifted);
            let rolled = tick == rollback_tick;
            w.push_predicted(
                tick as u64,
                &[item(e, pp, pr, InterpMode::Prediction)],
                &[item(e, p, r, InterpMode::Prediction)],
                rolled,
            );
        }
        w.advance(dt);
        if let Some(s) = shown(&w, e) {
            // Yaw from the rotated x axis.
            let fwd = s.rot.rotate(v(1.0, 0.0, 0.0));
            out.push((s.pos.x, (-fwd.z).atan2(fwd.x)));
        }
    }
    out
}

fn max_step(xs: &[f32]) -> f32 {
    xs.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

/// Largest change between neighbors of an angle series, across the +-pi wrap.
fn max_angle_step(xs: &[f32]) -> f32 {
    let wrap = |d: f32| (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    xs.windows(2).map(|w| wrap(w[1] - w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn rollback_correction_is_smoothed_in_position_and_rotation() {
    let (shift, twist) = (3.0, 0.8);
    let smooth = run_rollback(ViewConfig::default(), shift, twist, 3.0);
    let xs: Vec<f32> = smooth.iter().map(|p| p.0).collect();
    let yaws: Vec<f32> = smooth.iter().map(|p| p.1).collect();
    // Without the shift the largest per-frame step is speed * 60 / 144 = 0.083 units and
    // spin * 60 / 144 = 0.021 rad. Smoothing stays within a few times that, with no pop.
    assert!(max_step(&xs) < 0.5, "position popped by {}", max_step(&xs));
    assert!(max_angle_step(&yaws) < 0.15, "rotation popped by {}", max_angle_step(&yaws));
    // The shift is still seen eventually: the end state is the new truth.
    let (x_end, yaw_end) = *smooth.last().unwrap();
    let tick_end = (3.0 * RATE).floor();
    assert!((x_end - (tick_end * 0.2 + shift)).abs() < 0.4, "x {x_end}");
    let want_yaw = tick_end * 0.05 + twist;
    let d = (yaw_end - want_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    assert!(d.abs() < 0.08, "yaw {yaw_end} vs {want_yaw}");
}

#[test]
fn without_smoothing_the_same_rollback_pops_by_the_full_error() {
    let cfg = ViewConfig { correction_tau: 0.0, ..ViewConfig::default() };
    let r = run_rollback(cfg, 3.0, 0.8, 3.0);
    let xs: Vec<f32> = r.iter().map(|p| p.0).collect();
    let yaws: Vec<f32> = r.iter().map(|p| p.1).collect();
    assert!(max_step(&xs) > 2.5, "{}", max_step(&xs));
    assert!(max_angle_step(&yaws) > 0.6, "{}", max_angle_step(&yaws));
}

#[test]
fn correction_offset_and_angle_decay_to_zero() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    let still = |p: Vec3, r: Quat| item(e, p, r, InterpMode::Prediction);
    w.push_predicted(1, &[still(Vec3::ZERO, about_y(0.0))], &[still(Vec3::ZERO, about_y(0.0))], false);
    w.advance(1.0 / RATE);
    // The sim moves the entity by 2 and turns it by 1 rad in the past of the same head tick.
    w.push_predicted(1, &[still(v(2.0, 0.0, 0.0), about_y(1.0))], &[still(v(2.0, 0.0, 0.0), about_y(1.0))], true);
    let (d0, a0) = (w.correction_offset(e).unwrap(), w.correction_angle(e).unwrap());
    assert!((d0 - 2.0).abs() < 1e-4 && (a0 - 1.0).abs() < 1e-3, "{d0} {a0}");
    // Shown pose right after the correction is still the old one.
    let t = shown(&w, e).unwrap();
    assert!(t.pos.length() < 1e-4 && t.rot.angle_to(about_y(0.0)) < 1e-3);
    let (mut last_d, mut last_a) = (d0, a0);
    for _ in 0..200 {
        w.advance(1.0 / 120.0);
        let (d, a) = (w.correction_offset(e).unwrap(), w.correction_angle(e).unwrap());
        assert!(d <= last_d + 1e-6 && a <= last_a + 1e-6, "monotonic decay");
        (last_d, last_a) = (d, a);
    }
    assert_eq!((last_d, last_a), (0.0, 0.0), "settled");
    let t = shown(&w, e).unwrap();
    assert!((t.pos - v(2.0, 0.0, 0.0)).length() < 1e-4 && t.rot.angle_to(about_y(1.0)) < 1e-3);
}

#[test]
fn a_teleport_is_not_smoothed() {
    let e = ent(0, 0);
    let mut w = world(ViewConfig::default());
    let at = |x: f32| item(e, v(x, 0.0, 0.0), about_y(0.0), InterpMode::Prediction);
    w.push_predicted(1, &[at(0.0)], &[at(0.0)], false);
    w.advance(1.0 / RATE);
    w.push_predicted(1, &[at(5000.0)], &[at(5000.0)], true);
    assert_eq!(w.correction_offset(e), Some(0.0));
    assert_eq!(shown(&w, e).unwrap().pos, v(5000.0, 0.0, 0.0));
}

#[test]
fn spawn_despawn_and_a_reused_index_are_separate_entities() {
    let a = ent(3, 1);
    let b = ent(3, 2);
    let mut w = world(ViewConfig::default());
    w.push_predicted(1, &[], &[item(a, Vec3::ZERO, Quat::IDENTITY, InterpMode::Prediction)], false);
    w.advance(1.0 / RATE);
    w.push_predicted(
        2,
        &[item(a, Vec3::ZERO, Quat::IDENTITY, InterpMode::Prediction)],
        &[item(b, v(50.0, 0.0, 0.0), about_y(2.0), InterpMode::Prediction)],
        false,
    );
    assert_eq!(w.correction_offset(b), Some(0.0));
    assert!(shown(&w, a).is_none());
    // A new entity does not slerp or streak from anywhere.
    let t = shown(&w, b).unwrap();
    assert_eq!(t.pos, v(50.0, 0.0, 0.0));
    assert!(t.rot.angle_to(about_y(2.0)) < 1e-5);
    let events = w.take_lifecycle();
    assert!(events.contains(&ViewLifecycle::Despawned(a)) && events.contains(&ViewLifecycle::Spawned(b)));
}

#[test]
fn snapshot_mode_slerps_confirmed_frames() {
    let e = ent(5, 0);
    let mut w = world(ViewConfig::default());
    let at = |yaw: f32| item(e, Vec3::ZERO, about_y(yaw), InterpMode::Snapshot);
    for tick in 1..=10u64 {
        w.push_verified(tick, &[at(tick as f32 * 0.1)]);
    }
    for _ in 0..30 {
        w.advance(1.0 / 120.0);
    }
    // Shown a little late, never ahead of the newest confirmed frame, and always a pure rotation about y.
    let t = shown(&w, e).unwrap();
    let yaw = 2.0 * t.rot.y.atan2(t.rot.w);
    assert!(yaw > 0.0 && yaw <= 1.0 + 1e-4, "{yaw}");
    assert!((t.rot.dot(t.rot) - 1.0).abs() < 1e-4);
}

#[test]
fn fixed_point_conversion_gives_a_unit_quaternion() {
    let q = FPQuat::from_axis_angle(FPVec3::new(fp!(0.0), fp!(1.0), fp!(0.0)), fp!(1.0));
    let f = fp_to_quat(q);
    assert!((f.dot(f) - 1.0).abs() < 1e-5);
    assert!(f.angle_to(about_y(1.0)) < 1e-3);
    let p = fp_to_vec3(FPVec3::new(fp!(1.5), fp!(-2.25), fp!(3.0)));
    assert_eq!(p, v(1.5, -2.25, 3.0));
}
