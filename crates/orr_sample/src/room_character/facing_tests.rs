use super::*;
use orr_fp::FP;
fn enabled(axis: ForwardAxis) -> Document {
    Document {
        schema: 7,
        motion: Some(Motion {
            searching_idle: 0,
            carrying_idle: 1,
            escaped_once: true,
            pickup: None,
            facing: Some(axis),
        }),
        ..crossfade_document(0)
    }
}
fn observed(
    tick: u64,
    x: i64,
    z: i64,
    key: u32,
    won: u32,
    revision: u64,
    rest: bool,
) -> (Frame, PlaybackContext) {
    use crate::room_game::{RoomConfig, RoomEscapeV1};
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
    let mut f = sim.frame().clone();
    f.set_tick(tick);
    f.set_singleton(RoomRun {
        key_collected: key,
        won,
        ..Default::default()
    });
    let entity = f
        .entities()
        .find(|&e| f.get::<RoomActor>(e).is_some_and(|a| a.kind == PLAYER))
        .unwrap();
    let b = f.get_mut::<orr_physics3d::Body>(entity).unwrap();
    b.pos.x = FP::from_raw(x);
    b.pos.z = FP::from_raw(z);
    (
        f,
        PlaybackContext {
            entity,
            revision,
            rest,
            ..context()
        },
    )
}
fn yaw(
    c: &mut PlaybackCursor,
    d: &Document,
    m: &Arc<AnimatedModel>,
    f: &Frame,
    ctx: PlaybackContext,
) -> f32 {
    let before = f.to_bytes();
    c.observe(d, m, FrameView::of(f), ctx).unwrap();
    let y = c.facing_yaw(d, m, FrameView::of(f), ctx).unwrap();
    assert_eq!(f.to_bytes(), before);
    y
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}
#[test]
fn cardinal_and_diagonal_displacement_respect_all_forward_axes() {
    let m = Arc::new(model());
    for (axis, offset) in [
        (ForwardAxis::PositiveZ, 0.0),
        (ForwardAxis::NegativeZ, std::f32::consts::PI),
        (ForwardAxis::PositiveX, std::f32::consts::FRAC_PI_2),
        (ForwardAxis::NegativeX, -std::f32::consts::FRAC_PI_2),
    ] {
        for (x, z, angle) in [
            (0, 1, 0.0),
            (1, 0, std::f32::consts::FRAC_PI_2),
            (0, -1, std::f32::consts::PI),
            (-1, 0, -std::f32::consts::FRAC_PI_2),
            (1, 1, std::f32::consts::FRAC_PI_4),
        ] {
            let d = enabled(axis);
            let mut c = PlaybackCursor::default();
            let (f, ctx) = observed(10, 0, 0, 0, 0, 0, false);
            assert_eq!(yaw(&mut c, &d, &m, &f, ctx), 0.0);
            let (f, ctx) = observed(11, x, z, 0, 0, 0, false);
            let actual = yaw(&mut c, &d, &m, &f, ctx);
            close(actual.sin(), (angle - offset).sin());
            close(actual.cos(), (angle - offset).cos());
        }
    }
}
#[test]
fn idle_and_repeated_draw_hold_the_last_heading() {
    let d = enabled(ForwardAxis::PositiveZ);
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    for (t, x) in [(10, 0), (11, 1), (12, 1)] {
        let (f, ctx) = observed(t, x, 0, 0, 0, 0, false);
        let actual = yaw(&mut c, &d, &m, &f, ctx);
        if t > 10 {
            close(actual, std::f32::consts::FRAC_PI_2);
            assert_eq!(yaw(&mut c, &d, &m, &f, ctx), actual);
        }
    }
    let (f, ctx) = observed(20, 1, 0, 0, 0, 0, false);
    close(yaw(&mut c, &d, &m, &f, ctx), std::f32::consts::FRAC_PI_2);
}
#[test]
fn forward_gaps_use_net_displacement_but_discontinuities_reset_heading() {
    let d = enabled(ForwardAxis::PositiveZ);
    let m = Arc::new(model());
    for case in 0..6 {
        let mut c = PlaybackCursor::default();
        let (f, ctx) = observed(10, 0, 0, 0, 0, 0, false);
        yaw(&mut c, &d, &m, &f, ctx);
        let (f, ctx) = observed(11, 1, 0, 0, 0, 0, false);
        yaw(&mut c, &d, &m, &f, ctx);
        let (t, rev, rest) = match case {
            0 => (14, 0, false),
            1 => (5, 0, false),
            2 => (14, 1, false),
            3 => (12, 0, true),
            _ => (12, 0, false),
        };
        let (f, ctx) = observed(t, 2, 0, 0, 0, rev, rest);
        let replacement = Arc::new(model());
        let mut changed = d.clone();
        changed.crossfade_ticks = Some(1);
        assert_eq!(
            yaw(
                &mut c,
                if case == 5 { &changed } else { &d },
                if case == 4 { &replacement } else { &m },
                &f,
                ctx
            ),
            if case == 0 {
                std::f32::consts::FRAC_PI_2
            } else {
                0.0
            }
        );
    }
}
#[test]
fn extreme_raw_fp_differences_never_overflow_or_produce_nonfinite_heading() {
    let d = enabled(ForwardAxis::PositiveZ);
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    let (f, ctx) = observed(10, i64::MIN, i64::MAX, 0, 0, 0, false);
    yaw(&mut c, &d, &m, &f, ctx);
    let (f, ctx) = observed(11, i64::MAX, i64::MIN, 0, 0, 0, false);
    let actual = yaw(&mut c, &d, &m, &f, ctx);
    assert!(actual.is_finite());
    close(actual, 3.0 * std::f32::consts::FRAC_PI_4);
}
#[test]
fn strict_schema7_axis_wire_and_legacy_motion_absence() {
    let legacy6 = Document {
        schema: 6,
        motion: Some(Motion {
            searching_idle: 0,
            carrying_idle: 1,
            escaped_once: true,
            pickup: Some(Pickup {
                clip: 0,
                speed: PlaybackRate::Normal,
            }),
            facing: None,
        }),
        ..crossfade_document(0)
    };
    // Full pre-facing schema-6 encoder bytes (a157530), including final newline.
    assert_eq!(
        legacy6.to_bytes().unwrap(),
        br#"{
  "schema": 6,
  "player": "e_00000001",
  "searching": 0,
  "carrying": 1,
  "escaped": 2,
  "speeds": {
    "searching": "1x",
    "carrying": "1x",
    "escaped": "1x"
  },
  "crossfade_ticks": 0,
  "motion": {
    "searching_idle": 0,
    "carrying_idle": 1,
    "escaped_once": true,
    "pickup": {
      "clip": 0,
      "speed": "1x"
    }
  }
}
"#
    );
    let d = enabled(ForwardAxis::PositiveZ);
    let bytes = d.to_bytes().unwrap();
    assert_eq!(Document::parse(&bytes).unwrap(), d);
    let wire = String::from_utf8(bytes.clone()).unwrap();
    for bad in [
        wire.replace(
            "\"facing\": \"positive-z\"",
            "\"facing\": \"positive-z\", \"facing\": \"negative-z\"",
        ),
        wire.replace(
            "\"facing\": \"positive-z\"",
            "\"facing\": \"positive-z\", \"axis\": \"positive-x\"",
        ),
    ] {
        assert!(Document::parse(bad.as_bytes()).is_err());
    }

    for bad in [
        serde_json::json!(null),
        serde_json::json!("PositiveZ"),
        serde_json::json!("positive-y"),
        serde_json::json!(1),
        serde_json::json!([]),
        serde_json::json!({"axis":"positive-z"}),
    ] {
        let mut w = serde_json::to_value(&d).unwrap();
        w["motion"]["facing"] = bad;
        assert!(Document::parse(&serde_json::to_vec(&w).unwrap()).is_err());
    }
    let mut legacy = d.clone();
    legacy.schema = 5;
    assert!(legacy.validate().is_err());
    legacy.motion.as_mut().unwrap().facing = None;
    let old = legacy.to_bytes().unwrap();
    assert!(!String::from_utf8(old.clone()).unwrap().contains("facing"));
    assert_eq!(Document::parse(&old).unwrap(), legacy);
    legacy.schema = 7;
    assert!(legacy.validate().is_err());
}
#[test]
fn pickup_and_escape_pose_policies_do_not_override_displacement_heading() {
    let mut d = enabled(ForwardAxis::PositiveZ);
    d.motion.as_mut().unwrap().pickup = Some(Pickup {
        clip: 0,
        speed: PlaybackRate::Normal,
    });
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    for (t, x, key, won) in [(10, 0, 0, 0), (11, 1, 1, 0), (12, 2, 1, 1), (13, 2, 1, 1)] {
        let (f, ctx) = observed(t, x, 0, key, won, 0, false);
        let angle = yaw(&mut c, &d, &m, &f, ctx);
        if t > 10 {
            close(angle, std::f32::consts::FRAC_PI_2);
        }
    }
}
#[test]
fn wrong_source_or_tick_lookup_rejects_and_failed_observe_preserves_heading() {
    let d = enabled(ForwardAxis::PositiveZ);
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    let (f, ctx) = observed(10, 0, 0, 0, 0, 0, false);
    assert!(c.facing_yaw(&d, &m, FrameView::of(&f), ctx).is_err());
    yaw(&mut c, &d, &m, &f, ctx);
    let (next, ctx) = observed(11, 1, 0, 0, 0, 0, false);
    yaw(&mut c, &d, &m, &next, ctx);
    let mut bad = d.clone();
    bad.searching = 31;
    assert!(c.observe(&bad, &m, FrameView::of(&next), ctx).is_err());
    close(
        c.facing_yaw(&d, &m, FrameView::of(&next), ctx).unwrap(),
        std::f32::consts::FRAC_PI_2,
    );
    assert!(c.facing_yaw(&d, &m, FrameView::of(&f), ctx).is_err());
}
