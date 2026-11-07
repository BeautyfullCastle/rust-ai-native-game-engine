use super::*;
use crate::animation::{Interpolation, Trs};
fn fixture(step: bool) -> AnimatedModel {
    let base = AnimatedModel::from_bytes(include_bytes!(
        "../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    let mut s = base.source().clone();
    let times = vec![0.0, 1.0, 2.0, 3.0];
    let interpolation = if step {
        Interpolation::Step
    } else {
        Interpolation::Linear
    };
    s.clips[0].channels = vec![
        AnimationChannel {
            node: 0,
            interpolation,
            times: times.clone(),
            values: ChannelValues::Translation(
                times.iter().map(|t| [1.0 + 2.0 * t, -t, 0.5 * t]).collect(),
            ),
        },
        AnimationChannel {
            node: 0,
            interpolation,
            times: times.clone(),
            values: ChannelValues::Rotation(
                times
                    .iter()
                    .map(|t| {
                        let a = t * std::f32::consts::PI / 12.0;
                        [0.0, a.sin(), 0.0, a.cos()]
                    })
                    .collect(),
            ),
        },
        AnimationChannel {
            node: 0,
            interpolation,
            times: times.clone(),
            values: ChannelValues::Scale(
                times.iter().map(|t| [1.0 + t * 0.25, 1.0, 1.0]).collect(),
            ),
        },
    ];
    AnimatedModel::new(s).unwrap()
}
fn recipe(start: f64, end: f64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"format":"orr_animation_clip_trim","version":1,"source":"input.animated.json","clip":0,"start_seconds":start,"end_seconds":end,"name":"excerpt"})).unwrap()
}
fn trim(source: &AnimatedModel, start: f64, end: f64) -> AnimatedModel {
    let bytes = source.to_bytes().unwrap();
    trim_clip(&recipe(start, end), "variants/excerpt.trim.json", |p| {
        assert_eq!(p, "input.animated.json");
        Ok(bytes.clone())
    })
    .unwrap()
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00002, "{a} != {b}");
}
fn oracle(t: f32) -> Trs {
    let a = t * std::f32::consts::PI / 12.0;
    Trs {
        translation: [1.0 + 2.0 * t, -t, 0.5 * t],
        rotation: [0.0, a.sin(), 0.0, a.cos()],
        scale: [1.0 + t * 0.25, 1.0, 1.0],
    }
}
#[test]
fn linear_nonkey_boundaries_and_interior_pose_match_independent_trs_slerp_oracle() {
    let source = fixture(false);
    let changed = trim(&source, 0.5, 2.5);
    let index = source.source().clips.len() as u32;
    assert_eq!(changed.source().clips[index as usize].duration(), 2.0);
    for t in [0.0, 0.25, 0.5, 1.0, 1.5, 1.75, 2.0] {
        let pose = changed.sample_clip(index, t).unwrap();
        let expected = oracle(t + 0.5);
        let actual = pose.local()[0];
        for i in 0..3 {
            close(actual.translation[i], expected.translation[i]);
            close(actual.scale[i], expected.scale[i]);
        }
        for i in 0..4 {
            close(actual.rotation[i], expected.rotation[i]);
        }
        let old = source.sample_clip(0, t + 0.5).unwrap();
        for (a, b) in pose.local().iter().zip(old.local()) {
            for i in 0..3 {
                close(a.translation[i], b.translation[i]);
            }
        }
    }
}
#[test]
fn step_exact_keys_and_nonkey_boundaries_preserve_right_continuous_sampling() {
    let source = fixture(true);
    let changed = trim(&source, 0.5, 2.5);
    let clip = source.source().clips.len() as u32;
    for (t, expectedtime) in [
        (0.0, 0.0),
        (0.499, 0.0),
        (0.5, 1.0),
        (1.499, 1.0),
        (1.5, 2.0),
        (2.0, 2.0),
    ] {
        let p = changed.sample_clip(clip, t).unwrap();
        let expected = oracle(expectedtime);
        for i in 0..3 {
            close(p.local()[0].translation[i], expected.translation[i]);
            close(p.local()[0].scale[i], expected.scale[i]);
        }
        for i in 0..4 {
            close(p.local()[0].rotation[i], expected.rotation[i]);
        }
    }
}
#[test]
fn original_clips_rig_geometry_materials_and_images_are_exactly_preserved() {
    let source = fixture(false);
    let before = source.to_bytes().unwrap();
    let changed = trim(&source, 0.5, 2.5);
    let mut expected = source.source().clone();
    expected.asset_id = changed.source().asset_id.clone();
    expected.dependencies = changed.source().dependencies.clone();
    for (p, new) in expected
        .primitives
        .iter_mut()
        .zip(&changed.source().primitives)
    {
        p.id = new.id.clone();
    }
    expected
        .clips
        .push(changed.source().clips.last().unwrap().clone());
    assert_eq!(&expected, changed.source());
    assert_eq!(source.to_bytes().unwrap(), before);
    assert_eq!(changed.source().dependencies.len(), 2);
    assert_eq!(
        changed.source().dependencies[0].sha256,
        digest(&recipe(0.5, 2.5))
    );
    assert_eq!(changed.source().dependencies[1].sha256, digest(&before));
}
#[test]
fn strict_recipe_rejects_malformed_wire_before_resolver_access() {
    let base = String::from_utf8(recipe(0.5, 2.5)).unwrap();
    for bad in [
        "[]".into(),
        "null".into(),
        base.replace("\"version\":1", "\"version\":1,\"version\":1"),
        base.replace("\"version\":1", "\"version\":2"),
        base.replace("\"version\":1", "\"unknown\":0,\"version\":1"),
        base.replace("input.animated.json", "../input.animated.json"),
        format!("{base}null"),
    ] {
        let mut calls = 0;
        assert!(trim_clip(bad.as_bytes(), "out.trim.json", |_| {
            calls += 1;
            Err("must not resolve".into())
        })
        .is_err());
        assert_eq!(calls, 0);
    }
    for (start, end) in [
        (-0.1, 1.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (0.0, 4000.0),
        (1.0, 1.0 + 1e-10),
    ] {
        let mut calls = 0;
        assert!(trim_clip(&recipe(start, end), "out.trim.json", |_| {
            calls += 1;
            Err("must not resolve".into())
        })
        .is_err());
        assert_eq!(calls, 0);
    }
    assert!(trim_clip(
        &vec![b' '; MAX_RECIPE_BYTES + 1],
        "out.trim.json",
        |_| panic!("noIO")
    )
    .is_err());
}
#[test]
fn source_range_clip_name_and_asset_identity_rejections_leave_input_unchanged() {
    let original = fixture(false);
    let mut same = original.source().clone();
    let old = same.asset_id.clone();
    same.asset_id = "out.trim.json".into();
    for p in &mut same.primitives {
        p.id = format!("out.trim.json{}", p.id.strip_prefix(&old).unwrap());
    }
    let same = AnimatedModel::new(same).unwrap().to_bytes().unwrap();
    assert!(trim_clip(&recipe(0.0, 1.0), "out.trim.json", |_| Ok(same.clone())).is_err());

    let source = fixture(false);
    let bytes = source.to_bytes().unwrap();
    for (start, end) in [(0.0, 3.1), (3.0, 3.1)] {
        assert!(trim_clip(&recipe(start, end), "out.trim.json", |_| Ok(bytes.clone())).is_err());
    }
    let mut value: serde_json::Value = serde_json::from_slice(&recipe(0.0, 1.0)).unwrap();
    for clip in [3, 31] {
        value["clip"] = serde_json::json!(clip);
        assert!(trim_clip(
            &serde_json::to_vec(&value).unwrap(),
            "out.trim.json",
            |_| Ok(bytes.clone())
        )
        .is_err());
    }
    value["clip"] = serde_json::json!(0);
    value["name"] = serde_json::json!(source.source().clips[0].name);
    assert!(trim_clip(
        &serde_json::to_vec(&value).unwrap(),
        "out.trim.json",
        |_| Ok(bytes.clone())
    )
    .is_err());
    assert!(
        trim_clip(&recipe(0.0, 1.0), "wrong.animated.json", |_| panic!(
            "invalididbeforeIO"
        ))
        .is_err()
    );
    assert_eq!(source.to_bytes().unwrap(), bytes);
}
#[test]
fn clip_and_key_budgets_are_enforced_before_appending() {
    let base = fixture(false);
    let mut s = base.source().clone();
    s.clips = (0..MAX_CLIPS)
        .map(|i| {
            let mut c = s.clips[0].clone();
            c.name = format!("clip{i}");
            c
        })
        .collect();
    let full = AnimatedModel::new(s).unwrap().to_bytes().unwrap();
    assert!(trim_clip(&recipe(0.0, 1.0), "out.trim.json", |_| Ok(full.clone())).is_err());
    let mut s = base.source().clone();
    let count = MAX_ANIMATION_KEYS;
    s.clips.truncate(1);
    s.clips[0].channels.truncate(1);
    s.clips[0].channels[0].times = (0..count).map(|i| i as f32 / 1000.0).collect();
    s.clips[0].channels[0].values = ChannelValues::Translation(vec![[0.0; 3]; count]);
    let full = AnimatedModel::new(s).unwrap().to_bytes().unwrap();
    assert!(trim_clip(&recipe(0.0, 1.0), "out.trim.json", |_| Ok(full.clone())).is_err());
}
#[test]
fn resolver_once_deterministic_cooked_reload_and_short_channel_clamping() {
    let source = fixture(false);
    let mut s = source.source().clone();
    s.clips[0].channels[0].times = vec![1.0, 2.0];
    s.clips[0].channels[0].values = ChannelValues::Translation(vec![[1.0; 3], [2.0; 3]]);
    let original = AnimatedModel::new(s).unwrap();
    let bytes = original.to_bytes().unwrap();
    let mut calls = 0;
    let a = trim_clip(&recipe(0.5, 2.5), "out.trim.json", |_| {
        calls += 1;
        Ok(bytes.clone())
    })
    .unwrap();
    assert_eq!(calls, 1);
    let b = trim_clip(&recipe(0.5, 2.5), "out.trim.json", |_| Ok(bytes.clone())).unwrap();
    assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
    let index = original.source().clips.len() as u32;
    assert_eq!(
        a.sample_clip(index, 0.0).unwrap().local()[0].translation,
        [1.0; 3]
    );
    assert_eq!(
        a.sample_clip(index, 2.0).unwrap().local()[0].translation,
        [2.0; 3]
    );
    assert_eq!(
        AnimatedModel::from_bytes(&a.to_bytes().unwrap())
            .unwrap()
            .source(),
        a.source()
    );
}
