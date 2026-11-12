use super::*;
use orr_model::animation::{AnimatedModel, AnimationChannel, ChannelValues, Interpolation};
use std::sync::Arc;
fn model(duration: f32) -> Arc<AnimatedModel> {
    let imported = orr_model::animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("embedded GLB"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.clips.truncate(1);
    source.clips[0].channels = vec![AnimationChannel {
        node: 0,
        interpolation: Interpolation::Linear,
        times: if duration == 0.0 {
            vec![0.0]
        } else {
            vec![0.0, duration]
        },
        values: ChannelValues::Translation(if duration == 0.0 {
            vec![[0.0; 3]]
        } else {
            vec![[0.0; 3], [1.0, 0.0, 0.0]]
        }),
    }];
    Arc::new(AnimatedModel::new(source).unwrap())
}
fn guid(n: u32) -> Guid {
    Guid::parse(&format!("e_{n:08x}")).unwrap()
}
#[test]
fn literal_steps_negative_clamp_and_precision_loss() {
    assert_eq!(
        target_time(0.0, 1.0, PlaybackMode::Once, PreviewStep::Next)
            .unwrap()
            .to_bits(),
        0x3c888889
    );
    assert_eq!(
        target_time(0.01, 1.0, PlaybackMode::Loop, PreviewStep::Previous).unwrap(),
        0.0
    );
    assert_eq!(
        target_time(0.0, 1.0, PlaybackMode::Loop, PreviewStep::Previous).unwrap(),
        0.0
    );
    assert!(target_time(1048576.0, 2097152.0, PlaybackMode::Once, PreviewStep::Next).is_err());
    assert!(
        target_time(
            1048576.0,
            2097152.0,
            PlaybackMode::Loop,
            PreviewStep::Previous
        )
        .is_err()
    );
    assert!(target_time(1048576.0, 1048576.0, PlaybackMode::Once, PreviewStep::Next).is_ok());
    for invalid in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(target_time(invalid, 1.0, PlaybackMode::Once, PreviewStep::Next).is_err());
    }
}
#[test]
fn speed_independent_per_guid_steps_pause_and_stopped_selects_clip() {
    let model = model(1.0);
    let a = guid(1);
    let b = guid(2);
    let mut previews = PreviewPlayers::default();
    previews
        .bind(&a, model.clone(), 0, PlaybackMode::Once, 0.05)
        .unwrap();
    previews
        .bind(&b, model, 0, PlaybackMode::Once, 4.0)
        .unwrap();
    let b_before = previews.player(&b).unwrap().clone();
    previews.play(&a).unwrap();
    previews.step(&a, PreviewStep::Next).unwrap();
    assert_eq!(previews.player(&b).unwrap(), &b_before);
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
    previews.step(&b, PreviewStep::Next).unwrap();
    assert_eq!(
        previews.player(&a).unwrap().time(),
        previews.player(&b).unwrap().time()
    );
    let time = previews.player(&a).unwrap().time();
    previews.tick(1.0).unwrap();
    assert_eq!(previews.player(&a).unwrap().time(), time);
    previews.stop(&a).unwrap();
    previews.step(&a, PreviewStep::Previous).unwrap();
    assert_eq!(previews.player(&a).unwrap().clip(), Some(0));
    assert_eq!(previews.player(&a).unwrap().time(), 0.0);
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
}
#[test]
fn once_end_short_loop_and_zero_duration_keep_existing_seek_semantics() {
    let id = guid(1);
    for mode in [PlaybackMode::Once, PlaybackMode::Loop] {
        for duration in [0.0, 0.01, 1.0] {
            let model = model(duration);
            let mut p = PreviewPlayers::default();
            p.bind(&id, model.clone(), 0, mode, 1.0).unwrap();
            let mut expected = p.player(&id).unwrap().clone();
            expected.seek(&model, f32::from_bits(0x3c888889)).unwrap();
            expected.pause();
            p.step(&id, PreviewStep::Next).unwrap();
            assert_eq!(p.player(&id).unwrap(), &expected);
            if duration == 1.0 && mode == PlaybackMode::Once {
                p.seek(&id, 1.0).unwrap();
                p.step(&id, PreviewStep::Next).unwrap();
                assert_eq!(p.player(&id).unwrap().state(), PlaybackState::Finished);
                p.step(&id, PreviewStep::Previous).unwrap();
                assert!(p.player(&id).unwrap().time() < 1.0);
                assert_eq!(p.player(&id).unwrap().state(), PlaybackState::Paused);
            }
        }
    }
}
#[test]
fn failed_step_preserves_player_and_pose() {
    let id = guid(1);
    let mut p = PreviewPlayers::default();
    p.bind(&id, model(2097152.0), 0, PlaybackMode::Once, 1.0)
        .unwrap();
    p.seek(&id, 1048576.0).unwrap();
    let before = p.player(&id).unwrap().clone();
    let pose = p.pose(&id).unwrap().unwrap();
    assert!(p.step(&id, PreviewStep::Next).is_err());
    assert_eq!(p.player(&id).unwrap(), &before);
    assert_eq!(p.pose(&id).unwrap().unwrap().local(), pose.local());
    assert!(p.step(&guid(99), PreviewStep::Next).is_err());
    assert_eq!(p.player(&id).unwrap(), &before);
}

#[test]
fn stepped_nonzero_clip_pose_matches_independent_direct_sampling() {
    let mut source = model(1.0).source().clone();
    let mut second = source.clips[0].clone();
    second.name = "distinct inspection clip".into();
    second.channels[0].values = ChannelValues::Translation(vec![[0.0; 3], [2.0, 1.0, 0.0]]);
    source.clips.push(second);
    let model = Arc::new(AnimatedModel::new(source).unwrap());
    let id = guid(1);
    let mut previews = PreviewPlayers::default();
    previews
        .bind(&id, model.clone(), 1, PlaybackMode::Once, 2.0)
        .unwrap();
    let mut expected_time = 0.0_f32;
    for _ in 0..15 {
        expected_time = (f64::from(expected_time) + 1.0 / 60.0) as f32;
        previews.step(&id, PreviewStep::Next).unwrap();
        let actual = previews.pose(&id).unwrap().unwrap();
        let expected = model.sample_clip(1, expected_time).unwrap();
        assert_eq!(actual.local(), expected.local());
        assert_eq!(
            model.deform(&actual).unwrap(),
            model.deform(&expected).unwrap()
        );
    }
    assert_ne!(
        model.sample_clip(0, expected_time).unwrap().local(),
        model.sample_clip(1, expected_time).unwrap().local()
    );
    assert_eq!(previews.player(&id).unwrap().state(), PlaybackState::Paused);
}
