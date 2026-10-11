//! Explicit source-hidden exported App probe. Uses production App inputs/restart,
//! not fabricated RoomRun flags. A caller must provide an owned exported project.
use super::*;
use crate::room_character::State;

fn observed_character_pose(app: &App) -> orr_model::animation::Pose {
    let frame = FrameView::of(app.sim.frame());
    let placed = room_view::character_placement(
        frame,
        app.project.scene().index(),
        app.project.models(),
        room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    app.character_playback
        .pose(
            &app.project.character().unwrap().document,
            &placed.model,
            frame,
            crate::room_character::PlaybackContext {
                entity: placed.entity,
                tick_rate: TICK_RATE,
                rest: false,
                revision: 0,
            },
        )
        .unwrap()
        .clone()
}

#[cfg(all(feature = "project-create", target_os = "linux"))]
#[test]
fn character_crossfade_app_observes_steps_interruption_completion_and_restart() {
    use crate::room_character::{Document, PlaybackRate, PlaybackRates};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("crossfade");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: crate::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "crossfade-app".into(),
    })
    .unwrap();
    let path = root.join("room.character.json");
    let mut document = Document::parse(&std::fs::read(&path).unwrap()).unwrap();
    document.schema = 3;
    document.crossfade_ticks = Some(120);
    document.speeds = Some(PlaybackRates {
        searching: PlaybackRate::Half,
        carrying: PlaybackRate::Double,
        escaped: PlaybackRate::Quarter,
    });
    std::fs::write(path, document.to_bytes().unwrap()).unwrap();
    let project = PreparedProject::open_with_capabilities(
        &root,
        false,
        crate::room_project::CheckpointSupport::Disabled,
        true,
    )
    .unwrap();
    let mut baseline = project.scene().simulation().unwrap();
    let mut app = App::new(project).unwrap();
    let initial = app.sim.frame().to_bytes();
    let outgoing = observed_character_pose(&app);
    let advance = |app: &mut App, baseline: &mut Simulation<RoomEscapeV1>, input| {
        app.step_authoritative(input).unwrap();
        step(baseline, input);
        assert_eq!(app.sim.frame().to_bytes(), baseline.frame().to_bytes());
    };
    advance(
        &mut app,
        &mut baseline,
        RoomInput {
            buttons: INTERACT,
            ..Default::default()
        },
    );
    assert_eq!(
        crate::room_character::state(FrameView::of(app.sim.frame())),
        State::Carrying
    );
    assert_eq!(observed_character_pose(&app), outgoing);
    for _ in 0..6 {
        advance(&mut app, &mut baseline, RoomInput::default());
    }
    let target = room_view::character_placement(
        FrameView::of(app.sim.frame()),
        app.project.scene().index(),
        app.project.models(),
        room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    let visible = observed_character_pose(&app);
    assert_eq!(
        visible,
        target
            .model
            .blend_poses(&outgoing, &target.pose, 6.0 / 120.0)
            .unwrap()
    );
    assert_ne!(visible, target.pose);
    for _ in 0..8 {
        app.observe_character().unwrap();
        assert_eq!(observed_character_pose(&app), visible);
        assert_eq!(app.sim.frame().to_bytes(), baseline.frame().to_bytes());
    }
    for _ in 0..40 {
        advance(
            &mut app,
            &mut baseline,
            RoomInput {
                move_x: 1,
                ..Default::default()
            },
        );
    }
    let interrupted = observed_character_pose(&app);
    advance(
        &mut app,
        &mut baseline,
        RoomInput {
            buttons: INTERACT,
            ..Default::default()
        },
    );
    assert_eq!(
        crate::room_character::state(FrameView::of(app.sim.frame())),
        State::Escaped
    );
    assert_eq!(observed_character_pose(&app), interrupted);
    for _ in 0..120 {
        advance(&mut app, &mut baseline, RoomInput::default());
    }
    let target = room_view::character_placement(
        FrameView::of(app.sim.frame()),
        app.project.scene().index(),
        app.project.models(),
        room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(observed_character_pose(&app), target.pose);
    app.restart().unwrap();
    assert_eq!(app.sim.frame().to_bytes(), initial);
    assert_eq!(observed_character_pose(&app), outgoing);
}

#[test]
#[ignore = "explicit source-hidden exported App child; requires project and capture paths"]
fn exported_character_app_child() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = PathBuf::from(
        std::env::var_os("ORR_CHARACTER_EXPORTED_PROJECT").expect("explicit exported project"),
    );
    let captures = PathBuf::from(
        std::env::var_os("ORR_CHARACTER_APP_CAPTURES").expect("explicit capture directory"),
    );
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).exists(),
        "workspace must be hidden"
    );
    let project = PreparedProject::open_with_capabilities(
        &root,
        false,
        crate::room_project::CheckpointSupport::Disabled,
        true,
    )
    .unwrap();
    let mut app = App::new(project).unwrap();
    let initial = app.sim.frame().to_bytes();
    let gpu =
        orr_render::orr_rhi::Wgpu::headless(orr_render::orr_rhi::WgpuOptions::default()).unwrap();
    println!("Room character App GPU adapter: {}", gpu.adapter_name());
    let mut renderer = RoomRenderer::new(&gpu, (1024, 768), app.project.models()).unwrap();
    let capture = |app: &App, renderer: &mut RoomRenderer, name: &str, state: State| {
        let frame = orr_bridge::FrameView::of(app.sim.frame());
        assert_eq!(crate::room_character::state(frame), state);
        let bytes = app.sim.frame().to_bytes();
        renderer
            .render_with_playback(
                frame,
                app.project.scene().index(),
                app.project.models(),
                &room_view::camera((1024, 768)),
                &app.character_playback,
            )
            .unwrap();
        let first = renderer.read_rgba8();
        renderer
            .render_with_playback(
                frame,
                app.project.scene().index(),
                app.project.models(),
                &room_view::camera((1024, 768)),
                &app.character_playback,
            )
            .unwrap();
        assert_eq!(first, renderer.read_rgba8());
        assert_eq!(bytes, app.sim.frame().to_bytes());
        let file = std::fs::File::create(captures.join(format!("{name}.png"))).unwrap();
        let mut encoder = png17::Encoder::new(file, 1024, 768);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&first).unwrap();
        writer.finish().unwrap();
        first
    };
    let searching = capture(&app, &mut renderer, "app-searching", State::Searching);
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    })
    .unwrap();
    let carrying = capture(&app, &mut renderer, "app-carrying", State::Carrying);
    assert_ne!(searching, carrying);
    for _ in 0..40 {
        app.step_authoritative(RoomInput {
            move_x: 1,
            ..Default::default()
        })
        .unwrap();
    }
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    })
    .unwrap();
    let escaped = capture(&app, &mut renderer, "app-escaped", State::Escaped);
    assert_ne!(carrying, escaped);
    app.restart().unwrap();
    assert_eq!(app.sim.frame().to_bytes(), initial);
    let restarted = capture(&app, &mut renderer, "app-restart", State::Searching);
    assert_eq!(searching, restarted);
}

#[cfg(all(
    feature = "room-checkpoint",
    feature = "project-create",
    target_os = "linux"
))]
#[test]
fn checkpoint_restore_selects_carrying_without_an_animation_checkpoint_field() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("character");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: crate::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "restore".into(),
    })
    .unwrap();
    let project = PreparedProject::open_with_capabilities(
        &root,
        false,
        crate::room_project::CheckpointSupport::Disabled,
        true,
    )
    .unwrap();
    let restored = crate::room_checkpoint::restore(&project, true).unwrap();
    let frame = orr_bridge::FrameView::of(restored.frame());
    assert_eq!(frame.tick(), 0);
    assert_eq!(crate::room_character::state(frame), State::Carrying);
    let placed = crate::room_view::character_placement(
        frame,
        project.scene().index(),
        project.models(),
        crate::room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    let character = &project.character().unwrap().document;
    let expected = placed.model.sample_clip(character.carrying, 0.0).unwrap();
    assert_eq!(placed.pose.global(), expected.global());
    let fresh = crate::room_checkpoint::restore(&project, false).unwrap();
    assert_eq!(
        crate::room_character::state(orr_bridge::FrameView::of(fresh.frame())),
        State::Searching
    );
}

/// This is a production App probe inside the source-hidden export namespace.
#[test]
#[ignore = "explicit source-hidden crossfade App child; requires project and capture paths"]
fn exported_character_crossfade_app_child() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert!(
        !Path::new(env!("CARGO_MANIFEST_DIR")).exists(),
        "workspace must be hidden"
    );
    let root = PathBuf::from(std::env::var_os("ORR_CHARACTER_EXPORTED_PROJECT").unwrap());
    let captures = PathBuf::from(std::env::var_os("ORR_CHARACTER_APP_CAPTURES").unwrap());
    let project = PreparedProject::open_with_capabilities(
        &root,
        false,
        crate::room_project::CheckpointSupport::Disabled,
        true,
    )
    .unwrap();
    let document = project.character().unwrap().document.clone();
    let duration = document
        .crossfade_ticks
        .expect("schema3 authored crossfade");
    assert!(duration >= 4);
    let mut app = App::new(project).unwrap();
    let initial = app.sim.frame().to_bytes();
    let pose = observed_character_pose;
    let initial_pose = pose(&app);
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        crate::room_character::state(FrameView::of(app.sim.frame())),
        State::Carrying
    );
    assert_eq!(
        pose(&app),
        initial_pose,
        "first transition tick freezes the previous visible pose"
    );
    for _ in 0..duration / 2 {
        app.step_authoritative(RoomInput::default()).unwrap();
    }
    let frame = FrameView::of(app.sim.frame());
    let midpoint = pose(&app);
    let target = room_view::character_placement(
        frame,
        app.project.scene().index(),
        app.project.models(),
        room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    assert_ne!(midpoint, target.pose);
    assert_eq!(
        midpoint,
        target
            .model
            .blend_poses(
                &initial_pose,
                &target.pose,
                (duration / 2) as f32 / duration as f32
            )
            .unwrap()
    );
    let bytes = app.sim.frame().to_bytes();
    let gpu = Wgpu::headless(WgpuOptions::default()).unwrap();
    println!("Room crossfade App GPU adapter: {}", gpu.adapter_name());
    let mut renderer = RoomRenderer::new(&gpu, (1024, 768), app.project.models()).unwrap();
    let camera = room_view::camera((1024, 768));
    renderer
        .render_with_playback(
            frame,
            app.project.scene().index(),
            app.project.models(),
            &camera,
            &app.character_playback,
        )
        .unwrap();
    let blended = renderer.read_rgba8();
    renderer
        .render_with_playback(
            frame,
            app.project.scene().index(),
            app.project.models(),
            &camera,
            &app.character_playback,
        )
        .unwrap();
    assert_eq!(blended, renderer.read_rgba8());
    renderer
        .render(
            frame,
            app.project.scene().index(),
            app.project.models(),
            &camera,
        )
        .unwrap();
    let snapped = renderer.read_rgba8();
    let changed = blended
        .chunks_exact(4)
        .zip(snapped.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 20,
        "actual retained crossfade pose must affect pixels: {changed}"
    );
    assert_eq!(app.sim.frame().to_bytes(), bytes);
    assert_eq!(pose(&app), midpoint);
    for (name, pixels) in [
        ("app-crossfade-midpoint", blended),
        ("app-crossfade-snap", snapped),
    ] {
        let file = std::fs::File::create(captures.join(format!("{name}.png"))).unwrap();
        let mut encoder = png17::Encoder::new(file, 1024, 768);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }
    for _ in duration / 2..duration {
        app.step_authoritative(RoomInput::default()).unwrap();
    }
    let frame = FrameView::of(app.sim.frame());
    assert_eq!(
        pose(&app),
        room_view::character_placement(
            frame,
            app.project.scene().index(),
            app.project.models(),
            room_view::PresentationTime::default()
        )
        .unwrap()
        .unwrap()
        .pose
    );
    app.restart().unwrap();
    assert_eq!(app.sim.frame().to_bytes(), initial);
    assert_eq!(pose(&app), initial_pose);
    println!("Crossfade App midpoint changes {changed} pixels; repeated draw, completion and restart passed");
}
