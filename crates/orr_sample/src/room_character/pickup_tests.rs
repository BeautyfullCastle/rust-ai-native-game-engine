use super::*;
fn enabled() -> Document {
    Document {
        schema: 6,
        motion: Some(Motion {
            searching_idle: 0,
            carrying_idle: 1,
            escaped_once: true,
            pickup: Some(Pickup {
                clip: 0,
                speed: PlaybackRate::default(),
            }),
        }),
        ..crossfade_document(0)
    }
}
fn sample(
    c: &mut PlaybackCursor,
    d: &Document,
    m: &Arc<AnimatedModel>,
    tick: u64,
    key: u32,
    won: u32,
    revision: u64,
) -> Pose {
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
    let before = f.to_bytes();
    let pose = c
        .observe(
            d,
            m,
            FrameView::of(&f),
            PlaybackContext {
                entity,
                revision,
                ..context()
            },
        )
        .unwrap()
        .clone();
    assert_eq!(f.to_bytes(), before);
    pose
}
#[test]
fn codec_legacy_absence_and_strict_schema6() {
    let d = enabled();
    // Pinned pre-schema-6 (7d42997) full pretty JSON, including final newline.
    let legacy = Document {
        schema: 5,
        motion: Some(Motion {
            pickup: None,
            ..d.motion.unwrap()
        }),
        ..crossfade_document(0)
    };
    assert_eq!(
        legacy.to_bytes().unwrap(),
        br#"{
  "schema": 5,
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
    "escaped_once": true
  }
}
"#
    );
    let bytes = d.to_bytes().unwrap();
    assert_eq!(Document::parse(&bytes).unwrap(), d);
    for schema in 1..=5 {
        let old = match schema {
            1 => document(),
            2 => Document {
                schema: 2,
                speeds: Some(Default::default()),
                ..document()
            },
            3 | 4 => Document {
                schema,
                ..crossfade_document(0)
            },
            _ => Document {
                schema: 5,
                motion: Some(Motion {
                    pickup: None,
                    ..d.motion.unwrap()
                }),
                ..crossfade_document(0)
            },
        };
        let bytes = old.to_bytes().unwrap();
        assert!(!String::from_utf8(bytes.clone()).unwrap().contains("pickup"));
        assert_eq!(Document::parse(&bytes).unwrap(), old);
    }
    for v in [
        serde_json::json!(null),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!({"clip":32,"speed":"1x"}),
        serde_json::json!({"clip":0,"speed":"1x","extra":0}),
    ] {
        let mut w = serde_json::to_value(&d).unwrap();
        w["motion"]["pickup"] = v;
        assert!(Document::parse(&serde_json::to_vec(&w).unwrap()).is_err());
    }
    let mut old = d.clone();
    old.schema = 5;
    assert!(old.validate().is_err());
}
#[test]
fn known_entry_and_known_forward_gap_resume_carrying_at_completion() {
    let d = enabled();
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    sample(&mut c, &d, &m, 10, 0, 0, 0);
    assert_eq!(
        sample(&mut c, &d, &m, 11, 1, 0, 0),
        m.sample_clip(0, 0.0).unwrap()
    );
    assert_eq!(
        sample(&mut c, &d, &m, 71, 1, 0, 0),
        m.sample_clip(0, 1.0).unwrap()
    );
    assert_eq!(
        sample(&mut c, &d, &m, 131, 1, 0, 0),
        m.sample_clip(1, 131.0 / 60.0).unwrap()
    );
}
#[test]
fn late_entry_and_unknown_gap_skip() {
    let d = enabled();
    let m = Arc::new(model());
    for before in [None, Some(10)] {
        let mut c = PlaybackCursor::default();
        if let Some(t) = before {
            sample(&mut c, &d, &m, t, 0, 0, 0);
        }
        assert_eq!(
            sample(&mut c, &d, &m, 30, 1, 0, 0),
            m.sample_clip(1, 0.5).unwrap()
        );
    }
}
#[test]
fn escape_preempts_pickup() {
    let d = enabled();
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    sample(&mut c, &d, &m, 10, 0, 0, 0);
    sample(&mut c, &d, &m, 11, 1, 0, 0);
    assert_eq!(
        sample(&mut c, &d, &m, 12, 1, 1, 0),
        m.sample_clip(2, 0.0).unwrap()
    );
    assert_eq!(
        sample(&mut c, &d, &m, 72, 1, 1, 0),
        m.sample_clip(2, 1.0).unwrap()
    );
}
#[test]
fn restart_revision_and_rewind_skip_unknown_entry() {
    let d = enabled();
    let m = Arc::new(model());
    for mode in 0..3 {
        let mut c = PlaybackCursor::default();
        sample(&mut c, &d, &m, 10, 0, 0, 0);
        sample(&mut c, &d, &m, 11, 1, 0, 0);
        let (t, r) = match mode {
            0 => {
                c.reset();
                (12, 0)
            }
            1 => (12, 1),
            _ => (5, 0),
        };
        assert_eq!(
            sample(&mut c, &d, &m, t, 1, 0, r),
            m.sample_clip(1, t as f32 / 60.0).unwrap()
        );
    }
}
#[test]
fn repeated_draw_and_invalid_mapping_preserve_cursor() {
    let d = enabled();
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    sample(&mut c, &d, &m, 10, 0, 0, 0);
    let pose = sample(&mut c, &d, &m, 11, 1, 0, 0);
    assert_eq!(sample(&mut c, &d, &m, 11, 1, 0, 0), pose);
    let mut bad = d.clone();
    bad.motion.as_mut().unwrap().pickup.as_mut().unwrap().clip = 31;
    let f = frame(12, 1, 0);
    assert!(c.observe(&bad, &m, FrameView::of(&f), context()).is_err());
    assert_eq!(c.observed.as_ref().unwrap().current, pose);
}
#[test]
fn all_rates_use_once_timing() {
    for speed in PlaybackRate::ALL {
        let mut d = enabled();
        d.motion.as_mut().unwrap().pickup.as_mut().unwrap().speed = speed;
        let m = Arc::new(model());
        let mut c = PlaybackCursor::default();
        sample(&mut c, &d, &m, 10, 0, 0, 0);
        sample(&mut c, &d, &m, 11, 1, 0, 0);
        let end = (1..=1000)
            .find(|&t| {
                sample_clip_time_at_rate(t, 60, 2.0, PlaybackMode::Once, speed).unwrap() >= 2.0
            })
            .unwrap();
        let before = sample_clip_time_at_rate(end - 1, 60, 2.0, PlaybackMode::Once, speed).unwrap();
        assert!(before < 2.0);
        assert_eq!(
            sample(&mut c, &d, &m, 11 + end - 1, 1, 0, 0),
            m.sample_clip(0, before).unwrap()
        );
        let absolute = sample_clip_time_at_rate(
            11 + end,
            60,
            4.0,
            PlaybackMode::Loop,
            PlaybackRate::default(),
        )
        .unwrap();
        assert_eq!(
            sample(&mut c, &d, &m, 11 + end, 1, 0, 0),
            m.sample_clip(1, absolute).unwrap()
        );
    }
    let d = enabled();
    let mut source = model().source().clone();
    source.clips[0].channels[0].times = vec![0.0];
    source.clips[0].channels[0].values = ChannelValues::Translation(vec![[1.0, 0.0, 0.0]]);
    let m = Arc::new(AnimatedModel::new(source).unwrap());
    let mut c = PlaybackCursor::default();
    sample(&mut c, &d, &m, 10, 0, 0, 0);
    assert_eq!(
        sample(&mut c, &d, &m, 11, 1, 0, 0),
        m.sample_clip(1, 11.0 / 60.0).unwrap()
    );

    for speed in PlaybackRate::ALL {
        let mut d = enabled();
        d.motion.as_mut().unwrap().pickup.as_mut().unwrap().speed = speed;
        let m = Arc::new(model());
        let mut c = PlaybackCursor::default();
        sample(&mut c, &d, &m, 10, 0, 0, 0);
        sample(&mut c, &d, &m, 11, 1, 0, 0);
        let t = sample_clip_time_at_rate(30, 60, 2.0, PlaybackMode::Once, speed).unwrap();
        let expected = if t < 2.0 {
            m.sample_clip(0, t)
        } else {
            m.sample_clip(1, 41.0 / 60.0)
        }
        .unwrap();
        assert_eq!(sample(&mut c, &d, &m, 41, 1, 0, 0), expected);
    }
}

#[test]
fn same_clip_once_to_loop_is_semantic_crossfade_transition() {
    let mut d = enabled();
    d.crossfade_ticks = Some(2);
    d.motion.as_mut().unwrap().pickup.as_mut().unwrap().clip = 1;
    let m = Arc::new(model());
    let mut c = PlaybackCursor::default();
    sample(&mut c, &d, &m, 10, 0, 0, 0);
    sample(&mut c, &d, &m, 11, 1, 0, 0);
    let previous = sample(&mut c, &d, &m, 250, 1, 0, 0);
    let first_loop = sample(&mut c, &d, &m, 251, 1, 0, 0);
    assert_eq!(first_loop, previous);
    assert!(!c.observed.as_ref().unwrap().pickup_active);
    assert!(c.observed.as_ref().unwrap().outgoing.is_some());
    sample(&mut c, &d, &m, 252, 1, 0, 0);
    assert_eq!(
        sample(&mut c, &d, &m, 253, 1, 0, 0),
        m.sample_clip(1, 13.0 / 60.0).unwrap()
    );
}
