//! Explicit source-hidden exported App probe. Uses production App inputs/restart,
//! not fabricated RoomRun flags. A caller must provide an owned exported project.
use super::*;
use crate::room_character::State;

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
    println!("Room character App GPU adapter: {}",gpu.adapter_name());
    let mut renderer = RoomRenderer::new(&gpu, (1024, 768), app.project.models()).unwrap();
    let capture = |app: &App, renderer: &mut RoomRenderer, name: &str, state: State| {
        let frame = orr_bridge::FrameView::of(app.sim.frame());
        assert_eq!(crate::room_character::state(frame), state);
        let bytes = app.sim.frame().to_bytes();
        renderer
            .render(
                frame,
                app.project.scene().index(),
                app.project.models(),
                &room_view::camera((1024, 768)),
            )
            .unwrap();
        let first = renderer.read_rgba8();
        renderer
            .render(
                frame,
                app.project.scene().index(),
                app.project.models(),
                &room_view::camera((1024, 768)),
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
    });
    let carrying = capture(&app, &mut renderer, "app-carrying", State::Carrying);
    assert_ne!(searching, carrying);
    for _ in 0..40 {
        app.step_authoritative(RoomInput {
            move_x: 1,
            ..Default::default()
        });
    }
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    });
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
