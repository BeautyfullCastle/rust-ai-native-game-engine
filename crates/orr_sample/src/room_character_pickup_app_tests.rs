use super::*;
use crate::room_character::{Motion, Pickup, PlaybackRate, PlaybackRates};

#[test]
fn pickup_app_observes_event_resumes_carrying_and_restarts_without_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("pickup app");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: crate::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "pickup-app".into(),
    })
    .unwrap();
    let path = root.join("room.character.json");
    let mut document =
        crate::room_character::Document::parse(&std::fs::read(&path).unwrap()).unwrap();
    document.schema = 6;
    document.speeds = Some(PlaybackRates::default());
    document.crossfade_ticks = Some(0);
    document.motion = Some(Motion {
        searching_idle: document.searching,
        carrying_idle: document.carrying,
        escaped_once: false,
        pickup: Some(Pickup {
            clip: document.escaped,
            speed: PlaybackRate::Normal,
        }),
    });
    std::fs::write(path, document.to_bytes().unwrap()).unwrap();
    let project = PreparedProject::open_with_capabilities(
        &root,
        crate::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap();
    let binding = &project.models().document.bindings[&document.player];
    let model = project.models().assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .unwrap()
        .clone();
    let mut baseline = project.scene().simulation().unwrap();
    let mut app = App::new(project).unwrap();
    let initial = app.sim.frame().to_bytes();
    let initial_digest = observed_character_digest(
        &app.project,
        FrameView::of(app.sim.frame()),
        &app.character_playback,
    )
    .unwrap();
    std::fs::rename(&root, root.with_extension("hidden")).unwrap();
    for cycle in 0..2 {
        for tick in 1..=72u64 {
            let input = RoomInput {
                buttons: if tick == 8 { INTERACT } else { 0 },
                ..Default::default()
            };
            app.step_authoritative(input).unwrap();
            step(&mut baseline, input);
            assert_eq!(app.sim.frame().to_bytes(), baseline.frame().to_bytes());
            assert_eq!(
                app.sim.frame().singleton::<RoomRun>().key_collected,
                u32::from(tick >= 8)
            );
            assert_eq!(app.sim.frame().singleton::<RoomRun>().won, 0);
            let (clip, time) = if tick < 8 {
                (document.searching, (tick as f64 / 60.0) as f32)
            } else if tick - 8 < 60 {
                (document.escaped, ((tick - 8) as f64 / 60.0) as f32)
            } else {
                (
                    document.carrying,
                    orr_model_bindings::animation_time::sample_clip_time_at_rate(
                        tick,
                        60,
                        model.source().clips[document.carrying as usize].duration(),
                        orr_model_bindings::model_bindings::PlaybackMode::Loop,
                        PlaybackRate::Normal,
                    )
                    .unwrap(),
                )
            };
            let expected = character_pose_digest(&model.sample_clip(clip, time).unwrap());
            let bytes = app.sim.frame().to_bytes();
            assert_eq!(
                observed_character_digest(
                    &app.project,
                    FrameView::of(app.sim.frame()),
                    &app.character_playback
                )
                .unwrap(),
                Some(expected)
            );
            app.observe_character().unwrap();
            assert_eq!(app.sim.frame().to_bytes(), bytes);
        }
        app.restart().unwrap();
        baseline = app.project.scene().simulation().unwrap();
        assert_eq!(app.sim.frame().to_bytes(), initial, "restart cycle {cycle}");
        assert_eq!(
            observed_character_digest(
                &app.project,
                FrameView::of(app.sim.frame()),
                &app.character_playback
            )
            .unwrap(),
            initial_digest
        );
    }
}
