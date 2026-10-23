#![cfg(feature = "animated-models")]
#![allow(clippy::float_arithmetic)]

use egui::{accesskit::Role, Key};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, TextureId, TouchPhase};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    animated_panel::preview_camera_image, animated_preview::PreviewCamera, game::EditorGame,
    Editor, EditorApp,
};
use orr_model::animation::PlaybackState;
use orr_package::{Project, Runtime};
use orr_reflect::Guid;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
struct CameraUi {
    camera: PreviewCamera,
    image: Option<Rect>,
}

fn harness() -> Harness<'static, CameraUi> {
    Harness::builder().with_size([420.0, 320.0]).build_ui_state(
        |ui, state: &mut CameraUi| {
            let response = preview_camera_image(
                ui,
                TextureId::User(42),
                egui::vec2(240.0, 180.0),
                &mut state.camera,
            );
            state.image = Some(response.rect);
        },
        CameraUi::default(),
    )
}

fn drag(harness: &mut Harness<'_, CameraUi>, button: PointerButton, delta: egui::Vec2) {
    let start = harness.state().image.unwrap().center();
    harness.event(Event::PointerMoved(start));
    harness.step();
    harness.event(Event::PointerButton {
        pos: start,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    let steps = 4;
    for step in 1..=steps {
        harness.event(Event::PointerMoved(
            start + delta * (step as f32 / steps as f32),
        ));
        harness.step();
    }
    let end = start + delta;
    harness.event(Event::PointerButton {
        pos: end,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(2);
}

fn wheel(harness: &mut Harness<'_, CameraUi>, point: Pos2, delta: f32) {
    harness.event(Event::PointerMoved(point));
    harness.step();
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    harness.run_steps(2);
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    for _ in 0..3 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn type_into(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
}

fn install(root: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let project = Project::open_for_install(root, Runtime::content_only().engine_version).unwrap();
    project.install(&[source]).unwrap();
}

fn app(root: &Path) -> Harness<'static, EditorApp> {
    let scene = root.join("arena.yaml");
    fs::write(
        &scene,
        include_str!("../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    editor.sync();
    assert!(editor.spawn_arena_player(1, [20.0, 0.0]));
    editor.sync();
    assert!(editor.save());
    editor.sync();
    editor.select(None);
    Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .with_size([1800.0, 1800.0])
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()))
}

fn drag_app(harness: &mut Harness<'_, EditorApp>, button: PointerButton, delta: egui::Vec2) {
    harness
        .get_by_label("Animated model preview camera")
        .scroll_to_me();
    harness.run_steps(3);
    let image = harness.get_by_label("Animated model preview camera").rect();
    let start = image.center();
    harness.event(Event::PointerMoved(start));
    harness.step();
    harness.event(Event::PointerButton {
        pos: start,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    let steps = 4;
    for step in 1..=steps {
        harness.event(Event::PointerMoved(
            start + delta * (step as f32 / steps as f32),
        ));
        harness.step();
    }
    harness.event(Event::PointerButton {
        pos: start + delta,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(2);
}

fn wheel_app(harness: &mut Harness<'_, EditorApp>, point: Pos2, delta: f32) {
    harness.event(Event::PointerMoved(point));
    harness.step();
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    harness.run_steps(2);
}

fn clear_pointer(harness: &mut Harness<'_, EditorApp>) {
    harness.event(Event::PointerMoved(egui::pos2(1790.0, 1790.0)));
    harness.run_steps(2);
}

#[derive(Debug, PartialEq, Eq)]
struct PreviewCrop {
    size: [u32; 2],
    rgba: Vec<u8>,
}

fn preview_crop(
    frame_size: [u32; 2],
    raw: &[u8],
    rect: Rect,
    pixels_per_point: f32,
) -> PreviewCrop {
    let [frame_width, frame_height] = frame_size;
    let x0 = (rect.left() * pixels_per_point)
        .floor()
        .clamp(0.0, frame_width as f32) as u32;
    let y0 = (rect.top() * pixels_per_point)
        .floor()
        .clamp(0.0, frame_height as f32) as u32;
    let x1 = (rect.right() * pixels_per_point)
        .ceil()
        .clamp(0.0, frame_width as f32) as u32;
    let y1 = (rect.bottom() * pixels_per_point)
        .ceil()
        .clamp(0.0, frame_height as f32) as u32;
    assert!(x1 > x0 && y1 > y0, "preview image intersects the App frame");
    let mut rgba = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for row in y0..y1 {
        let start = ((row * frame_width + x0) * 4) as usize;
        let end = ((row * frame_width + x1) * 4) as usize;
        rgba.extend_from_slice(&raw[start..end]);
    }
    PreviewCrop {
        size: [x1 - x0, y1 - y0],
        rgba,
    }
}

fn save_preview_crop(name: &str, crop: &PreviewCrop) {
    let Some(directory) = std::env::var_os("ORR_ANIMATION_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    fs::create_dir_all(directory).unwrap();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [crop.size[0] as usize, crop.size[1] as usize],
        &crop.rgba,
    );
    orr_editor::app::write_png(&directory.join(format!("{name}.png")), &image).unwrap();
}

fn render_preview_crop(harness: &mut Harness<'_, EditorApp>) -> (PreviewCrop, Rect) {
    harness
        .get_by_label("Animated model preview camera")
        .scroll_to_me();
    harness.run_steps(3);
    let rect = harness.get_by_label("Animated model preview camera").rect();
    let pixels_per_point = harness.ctx.pixels_per_point();
    let frame = harness.render().expect("render production App preview");
    let crop = preview_crop(
        [frame.width(), frame.height()],
        frame.as_raw(),
        rect,
        pixels_per_point,
    );
    (crop, rect)
}

struct AppSnapshot {
    scene: PathBuf,
    sidecar: PathBuf,
    scene_bytes: Vec<u8>,
    sidecar_bytes: Vec<u8>,
    checksum: u64,
    history: orr_editor::model::History,
    guid: Guid,
    preview_time: f32,
}

fn assert_app_state_unchanged(harness: &Harness<'_, EditorApp>, snapshot: &AppSnapshot) {
    let app = harness.state();
    assert_eq!(app.editor.checksum(), snapshot.checksum);
    assert_eq!(app.editor.history(), &snapshot.history);
    assert_eq!(fs::read(&snapshot.scene).unwrap(), snapshot.scene_bytes);
    assert_eq!(fs::read(&snapshot.sidecar).unwrap(), snapshot.sidecar_bytes);
    let bindings = app.animated_models.bindings.as_ref().unwrap();
    assert!(!bindings.dirty());
    let player = app.animated_models.player(&snapshot.guid).unwrap();
    assert_eq!(player.state(), PlaybackState::Paused);
    assert_eq!(player.time(), snapshot.preview_time);
}

#[test]
fn animated_preview_image_routes_orbit_pan_zoom_and_reset_without_gpu() {
    let mut harness = harness();
    harness.run_steps(2);
    let default = PreviewCamera::default();

    drag(&mut harness, PointerButton::Primary, egui::vec2(36.0, 24.0));
    assert_ne!(harness.state().camera, default, "left drag orbits");
    harness.get_by_label("Reset preview camera").click();
    harness.run_steps(2);
    assert_eq!(harness.state().camera, default);

    drag(
        &mut harness,
        PointerButton::Secondary,
        egui::vec2(30.0, -18.0),
    );
    assert_ne!(harness.state().camera, default, "right drag pans");
    harness.get_by_label("Reset preview camera").click();
    harness.run_steps(2);
    assert_eq!(harness.state().camera, default);

    drag(&mut harness, PointerButton::Middle, egui::vec2(-24.0, 15.0));
    assert_ne!(harness.state().camera, default, "middle drag pans");
    harness.get_by_label("Reset preview camera").click();
    harness.run_steps(2);
    assert_eq!(harness.state().camera, default);

    let image = harness.state().image.unwrap();
    wheel(&mut harness, image.center(), 90.0);
    assert_ne!(harness.state().camera, default, "wheel over image zooms");
    harness.get_by_label("Reset preview camera").click();
    harness.run_steps(2);
    assert_eq!(harness.state().camera, default);

    let outside = egui::pos2(image.right() + 20.0, image.center().y);
    wheel(&mut harness, outside, 90.0);
    assert_eq!(
        harness.state().camera,
        default,
        "wheel outside image is ignored"
    );
}

#[test]
#[ignore = "explicit production EditorApp GPU acceptance; requires ORR_REQUIRE_GPU=1"]
fn production_animated_camera_gestures_are_isolated_from_host_and_sidecar() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("animated camera fixture");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    install(&root);
    let scene = root.join("arena.yaml");
    let sidecar = root.join("arena.animation.json");
    let mut harness = app(&root);
    harness.run_steps(3);
    assert!(harness.state().has_gpu());

    click(&mut harness, "Animated model authoring");
    type_into(
        &mut harness,
        "Animation sidecar path",
        sidecar.to_str().unwrap(),
    );
    type_into(&mut harness, "Animation scene relative path", "arena.yaml");
    click(&mut harness, "Create animation bindings");
    type_into(&mut harness, "Animated package name", "sample-animation");
    type_into(
        &mut harness,
        "Animated asset path in package",
        "animated.glb",
    );
    click(&mut harness, "Load animated model");
    assert!(harness.state().animated_models.error().is_none());

    click(&mut harness, "player_0");
    harness
        .get_by_role_and_label(Role::ComboBox, "Animation clip")
        .click();
    harness.run_steps(3);
    click(&mut harness, "1: pulse (1.00s)");
    click(&mut harness, "Once");
    click(&mut harness, "Assign animated clip to selection");
    let guid = harness.state().editor.selected_guids()[0].clone();
    click(&mut harness, "Play animation preview");
    click(&mut harness, "Pause animation preview");
    assert_eq!(
        harness
            .state()
            .animated_models
            .player(&guid)
            .unwrap()
            .state(),
        PlaybackState::Paused
    );
    click(&mut harness, "Save animation bindings");
    let preview_time = harness
        .state()
        .animated_models
        .player(&guid)
        .unwrap()
        .time();
    let snapshot = AppSnapshot {
        scene: scene.clone(),
        sidecar: sidecar.clone(),
        scene_bytes: fs::read(&scene).unwrap(),
        sidecar_bytes: fs::read(&sidecar).unwrap(),
        checksum: harness.state().editor.checksum(),
        history: harness.state().editor.history().clone(),
        guid,
        preview_time,
    };
    assert!(!harness
        .state()
        .animated_models
        .bindings
        .as_ref()
        .unwrap()
        .dirty());

    clear_pointer(&mut harness);
    let (baseline, _) = render_preview_crop(&mut harness);
    save_preview_crop("baseline", &baseline);

    drag_app(&mut harness, PointerButton::Primary, egui::vec2(36.0, 24.0));
    clear_pointer(&mut harness);
    let (orbited, _) = render_preview_crop(&mut harness);
    save_preview_crop("orbit", &orbited);
    assert_ne!(orbited, baseline, "orbit changes the production GPU view");
    assert_app_state_unchanged(&harness, &snapshot);

    drag_app(
        &mut harness,
        PointerButton::Secondary,
        egui::vec2(30.0, -18.0),
    );
    clear_pointer(&mut harness);
    let (panned, image) = render_preview_crop(&mut harness);
    save_preview_crop("pan", &panned);
    assert_ne!(panned, orbited, "pan changes the production GPU view");
    assert_app_state_unchanged(&harness, &snapshot);

    wheel_app(&mut harness, image.center(), 90.0);
    clear_pointer(&mut harness);
    let (zoomed, _) = render_preview_crop(&mut harness);
    save_preview_crop("zoom", &zoomed);
    assert_ne!(zoomed, panned, "wheel zoom changes the production GPU view");
    assert_app_state_unchanged(&harness, &snapshot);

    click(&mut harness, "Reset preview camera");
    clear_pointer(&mut harness);
    let (reset, image) = render_preview_crop(&mut harness);
    save_preview_crop("reset", &reset);
    assert_eq!(reset, baseline, "reset reproduces the initial GPU view");
    assert_app_state_unchanged(&harness, &snapshot);

    let outside = egui::pos2(image.right() + 20.0, image.center().y);
    wheel_app(&mut harness, outside, 90.0);
    clear_pointer(&mut harness);
    assert_eq!(
        render_preview_crop(&mut harness).0,
        baseline,
        "wheel outside the image does not change the GPU view"
    );
    assert_app_state_unchanged(&harness, &snapshot);
}
