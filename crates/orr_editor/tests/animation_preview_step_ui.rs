//! Actual installed Arena animation preview. Captures exclude all UI labels.
#![cfg(feature = "animated-models")]
#![allow(clippy::float_arithmetic)]

use egui::{Rect, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_editor::{
    Editor, EditorApp,
    animated_bindings::{self, Binding, Bindings, PlaybackMode, PlaybackSettings},
    game::EditorGame,
};
use orr_model::animation::PlaybackState;
use orr_package::{Project, Runtime};
use std::{fs, path::Path};

const NEXT: &str = "Next preview sample (1/60 s)";
const PREVIOUS: &str = "Previous preview sample (1/60 s)";

fn button(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(4);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    h.run_steps(4);
    assert!(
        h.state().animated_models.error().is_none(),
        "{:?}",
        h.state().animated_models.error()
    );
}

fn save_crop(root: &Path, name: &str, crop: &PreviewCrop) {
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [crop.size[0] as usize, crop.size[1] as usize],
        &crop.rgba,
    );
    orr_editor::app::write_png(&root.join(format!("{name}.png")), &image).unwrap();
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

#[test]
#[ignore = "actual offscreen Editor GPU; requires ORR_ANIMATION_STEP_CAPTURE_DIR"]
fn installed_nonzero_clip_steps_pause_and_rewind_without_authoritative_edits() {
    let captures = std::path::PathBuf::from(
        std::env::var_os("ORR_ANIMATION_STEP_CAPTURE_DIR").expect("explicit capture directory"),
    );
    assert!(captures.is_absolute());
    fs::create_dir_all(&captures).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let source = temp.path().join("original animation package");
    fs::create_dir(&source).unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    for name in ["orr.package.json", "animated.glb", "LICENSE.txt"] {
        fs::copy(original.join(name), source.join(name)).unwrap();
    }
    let project = Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    project.install(std::slice::from_ref(&source)).unwrap();
    project.verify().unwrap();
    fs::rename(&source, temp.path().join("hidden original package")).unwrap();
    let loaded = animated_bindings::load_asset(&root, "sample-animation", "animated.glb").unwrap();
    let model = loaded.model().clone();
    assert!(model.source().clips.len() > 1);
    let scene = root.join("arena.yaml");
    let sidecar = root.join("arena.animation.json");
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
    assert!(editor.select_named("player_0"));
    let guid = editor.selected_guids()[0].clone();
    // Test setup uses the same validated binding API as assignment, not a JSON bypass.
    // Speed 2 and nonzero clip intentionally differ from the panel's draft defaults.
    let mut bindings = Bindings::create(sidecar.clone(), "arena.yaml".into(), ".".into()).unwrap();
    let binding = Binding::from_asset(
        "sample-animation".into(),
        "animated.glb".into(),
        &loaded,
        1,
        PlaybackSettings {
            mode: PlaybackMode::Once,
            speed: 2.0,
        },
    )
    .unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&guid), &binding, &loaded)
        .unwrap();
    bindings.save().unwrap();
    let mut h = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .with_size([1800.0, 1800.0])
        .wgpu()
        .build_eframe(move |cc| {
            let state = cc.wgpu_render_state.as_ref().expect("GPU required");
            let info = state.adapter.get_info();
            eprintln!(
                "animation-step adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            let mut rendered = EditorApp::new(editor, cc.wgpu_render_state.clone());
            rendered.animated_models.bindings = Some(bindings);
            rendered.animated_models.reload().unwrap();
            rendered
        });
    h.run_steps(4);
    assert!(h.state().has_gpu());
    h.get_by_label("Animated model authoring").click_accesskit();
    h.run_steps(4);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let document = h
        .state()
        .animated_models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let scene_bytes = fs::read(&scene).unwrap();
    let binding_bytes = fs::read(&sidecar).unwrap();
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().clip(),
        Some(1)
    );
    assert_eq!(h.state().animated_models.player(&guid).unwrap().time(), 0.0);
    let baseline = render_preview_crop(&mut h).0;
    save_crop(&captures, "sample-zero", &baseline);
    let zero = model.deform(&model.sample_clip(1, 0.0).unwrap()).unwrap();
    let mut expected_time = 0.0_f32;
    for _ in 0..15 {
        expected_time = (f64::from(expected_time) + 1.0 / 60.0) as f32;
        button(&mut h, NEXT);
        assert_eq!(
            h.state().animated_models.player(&guid).unwrap().time(),
            expected_time
        );
    }
    let time = h.state().animated_models.player(&guid).unwrap().time();
    h.get_by_label(&format!("Preview sample time: {time} s"));
    assert!(
        (time - 0.25).abs() < 1e-6,
        "source time must ignore speed 2: {time}"
    );
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().state(),
        PlaybackState::Paused
    );
    let literal = model.deform(&model.sample_clip(1, 0.25).unwrap()).unwrap();
    let sampled = model.deform(&model.sample_clip(1, time).unwrap()).unwrap();
    assert_ne!(
        literal, zero,
        "independent direct sampling must actually deform this fixture"
    );
    // Same source-time rounding as the step player; direct sampling never uses its pose state.
    assert_ne!(sampled, zero);
    assert_eq!(
        sampled,
        model
            .deform(&model.sample_clip(1, expected_time).unwrap())
            .unwrap()
    );
    let stepped = render_preview_crop(&mut h).0;
    assert_eq!(baseline.size, stepped.size);
    let changed = baseline
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(stepped.rgba.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 20,
        "actual preview pixels must move, not only the time label: {changed}"
    );
    save_crop(&captures, "sample-fifteen", &stepped);
    h.run_steps(30);
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().time(),
        time
    );
    assert_eq!(
        render_preview_crop(&mut h).0,
        stepped,
        "paused preview remains pixel-stable"
    );
    // One extra Previous intentionally covers clamp-at-zero despite accumulated f32 rounding.
    for _ in 0..16 {
        expected_time = (f64::from(expected_time) - 1.0 / 60.0).max(0.0) as f32;
        button(&mut h, PREVIOUS);
        assert_eq!(
            h.state().animated_models.player(&guid).unwrap().time(),
            expected_time
        );
    }
    assert_eq!(h.state().animated_models.player(&guid).unwrap().time(), 0.0);
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().clip(),
        Some(1)
    );
    let rewound = render_preview_crop(&mut h).0;
    assert_eq!(rewound, baseline);
    save_crop(&captures, "sample-rewound", &rewound);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state()
            .animated_models
            .bindings
            .as_ref()
            .unwrap()
            .document(),
        &document
    );
    assert!(!h.state().animated_models.bindings.as_ref().unwrap().dirty());
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), binding_bytes);
    assert!(!source.exists());
}
