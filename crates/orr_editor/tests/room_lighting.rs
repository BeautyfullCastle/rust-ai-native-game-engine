//! Actual EditorApp acceptance for reconstructed Room lighting authoring and drawing.
#![cfg(all(
    feature = "room-lighting",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{room_lighting_panel::Panel, Editor, EditorApp, HostSpec, Mode};
use orr_sample::{
    project_create::{create, CreateOptions},
    room_lighting::Document,
    room_project::{CheckpointSupport, PreparedProject},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn project_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    original: BTreeMap<PathBuf, Vec<u8>>,
}

impl Fixture {
    fn new(template: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("lighting room");
        create(&CreateOptions {
            output: root.clone(),
            template: template.into(),
            seed: "editor-lighting-acceptance".into(),
        })
        .unwrap();
        let path = root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["entry"]["lighting"] = serde_json::json!("room.lighting.json");
        fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        fs::write(
            root.join("room.lighting.json"),
            Document::default().to_bytes().unwrap(),
        )
        .unwrap();
        let original = project_files(&root);
        Self {
            _temp: temp,
            root,
            original,
        }
    }

    fn open(&self) -> PreparedProject {
        PreparedProject::open_with_presentation(
            &self.root,
            false,
            CheckpointSupport::Disabled,
            cfg!(feature = "room-character"),
            true,
        )
        .unwrap()
    }

    fn editor(&self) -> EditorApp {
        self.build_app(None)
    }

    fn build_app(&self, render_state: Option<egui_wgpu::RenderState>) -> EditorApp {
        let mut prepared = self.open();
        let lighting = prepared.take_lighting().unwrap();
        let camera = prepared.take_camera().unwrap();
        #[cfg(feature = "room-character")]
        let character = prepared.take_character();
        let (_, scene_path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: scene_path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        // Match startup: admission must work before an artificial test sync.
        editor
            .install_room_lighting(lighting.document.clone())
            .unwrap();
        let panel = Panel::new_for_editor(lighting, &editor).unwrap();
        editor.install_room_camera(camera.document.clone()).unwrap();
        #[cfg(feature = "room-character")]
        if let Some(character) = character.as_ref() {
            editor
                .install_room_character(character.document.clone())
                .unwrap();
        }
        let mut app = EditorApp::new(editor, render_state);
        app.models.install_room(models).unwrap();
        app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
        app.room_lighting = Some(panel);
        #[cfg(feature = "room-character")]
        if let Some(character) = character {
            app.room_character = Some(orr_editor::room_character_panel::Panel::new(character));
        }
        app
    }

    fn app(&self, gpu: bool) -> Harness<'_, EditorApp> {
        let builder = Harness::builder()
            .with_size([1500.0, 1200.0])
            .with_step_dt(1.0 / 60.0);
        if gpu {
            builder
                .wgpu()
                .build_eframe(|cc| self.build_app(cc.wgpu_render_state.clone()))
        } else {
            builder.build_eframe(|_| self.editor())
        }
    }

    fn assert_preserved(&self) {
        let mut current = project_files(&self.root);
        current.remove(Path::new("room.lighting.json"));
        let mut original = self.original.clone();
        original.remove(Path::new("room.lighting.json"));
        assert_eq!(
            current, original,
            "lighting must preserve scene, sidecars and packages"
        );
    }
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
}

fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("room-lighting-frame-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching lighting frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn type_number(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn document(h: &Harness<'_, EditorApp>) -> Document {
    h.state().editor.room_lighting_document().unwrap().clone()
}

fn apply_enabled(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("Room lighting").click();
    h.run_steps(3);
    h.get_by_label("Point light enabled").click();
    h.run_steps(3);
    assert_eq!(
        document(h),
        Document::default(),
        "widgets stage before Apply"
    );
    h.get_by_label("Apply lighting").click();
    settle(h);
    assert_eq!(document(h), Document::enabled_default());
}

fn capture(h: &Harness<'_, EditorApp>, label: &str) -> Vec<u8> {
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("production main viewport");
    eprintln!(
        "room lighting viewport adapter: {}",
        viewport.gpu().adapter_name()
    );
    let rgba = viewport.gpu().read_rgba8();
    let size = h.state().ui.viewport_px;
    assert_eq!(rgba.len(), (size.0 * size.1 * 4) as usize);
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| *p != &rgba[..4])
            .count()
            > 100
    );
    if let Some(directory) = std::env::var_os("ORR_ROOM_LIGHTING_CAPTURE_DIR") {
        let path = PathBuf::from(directory);
        fs::create_dir_all(&path).unwrap();
        let file = fs::File::create(path.join(format!("{label}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
    }
    rgba
}

fn changed_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count()
}

fn live_workflow(template: &str, gpu: bool) {
    let fixture = Fixture::new(template);
    let mut h = fixture.app(gpu);
    settle(&mut h);
    let original_frame = frame_bytes(&mut h.state_mut().editor);
    let original_checksum = h.state().editor.checksum();
    let original_history_len = h.state().editor.history().entries.len();
    let capture_prefix = template.replace('-', "_");
    // Keep the panel open for every capture so layout changes cannot fake light changes.
    h.get_by_label("Room lighting").click();
    h.run_steps(3);
    let off = gpu.then(|| capture(&h, &format!("{capture_prefix}-off")));
    h.get_by_label("Point light enabled").click();
    h.run_steps(3);
    assert_eq!(document(&h), Document::default());
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    h.get_by_label("Apply lighting").click();
    settle(&mut h);
    assert_eq!(document(&h), Document::enabled_default());
    assert_eq!(
        h.state().room_lighting.as_ref().unwrap().document(),
        &document(&h)
    );
    let on = gpu.then(|| capture(&h, &format!("{capture_prefix}-on")));
    if let (Some(off), Some(on)) = (&off, &on) {
        assert!(
            changed_pixels(off, on) > 40,
            "lighting must change actual viewport pixels"
        );
        let (static_count, animated_count) = h
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts();
        assert!(static_count > 0, "static imported content must be admitted");
        if template.contains("character") {
            assert!(
                animated_count > 0,
                "the character must reach the actual GPU cache"
            );
        }
    }
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    h.get_by_label("Undo lighting edit").click();
    settle(&mut h);
    assert_eq!(document(&h), Document::default());
    if let Some(off) = &off {
        assert_eq!(&capture(&h, &format!("{capture_prefix}-undo")), off);
    }
    h.get_by_label("Redo lighting edit").click();
    settle(&mut h);
    assert_eq!(document(&h), Document::enabled_default());
    if let Some(on) = &on {
        assert_eq!(&capture(&h, &format!("{capture_prefix}-redo")), on);
    }

    type_number(&mut h, "Position X", "-3");
    type_number(&mut h, "Position Y", "2");
    type_number(&mut h, "Position Z", "3");
    assert_eq!(document(&h), Document::enabled_default());
    h.get_by_label("Apply lighting").click();
    settle(&mut h);
    let moved = document(&h);
    assert_eq!(moved.point_light.unwrap().position, [-3.0, 2.0, 3.0]);
    let moved_pixels = gpu.then(|| capture(&h, &format!("{capture_prefix}-moved")));
    if let (Some(on), Some(moved)) = (&on, &moved_pixels) {
        assert!(
            changed_pixels(on, moved) > 20,
            "authored position must affect lighting"
        );
    }
    h.get_by_label("Save lighting").click();
    settle(&mut h);
    assert_eq!(
        Document::parse(&fs::read(fixture.root.join("room.lighting.json")).unwrap()).unwrap(),
        moved
    );
    // Save must retain the independent history without adding scene undo entries.
    h.get_by_label("Undo lighting edit").click();
    settle(&mut h);
    assert_eq!(document(&h), Document::enabled_default());
    h.get_by_label("Redo lighting edit").click();
    settle(&mut h);
    assert_eq!(document(&h), moved);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert_eq!(h.state().editor.checksum(), original_checksum);
    assert_eq!(
        h.state().editor.history().entries.len(),
        original_history_len
    );
    fixture.assert_preserved();
    drop(h);

    let mut h = fixture.app(gpu);
    settle(&mut h);
    assert_eq!(document(&h), moved);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    h.get_by_label("Room lighting").click();
    h.run_steps(3);
    if let Some(moved_pixels) = &moved_pixels {
        assert_eq!(
            &capture(&h, &format!("{capture_prefix}-reopen")),
            moved_pixels
        );
    }
    let mut baseline = fixture.editor();
    baseline
        .editor
        .install_room_lighting(Document::default())
        .unwrap();
    let saved_bytes = fs::read(fixture.root.join("room.lighting.json")).unwrap();
    type_number(&mut h, "Intensity", "9");
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    h.state_mut().editor.pause();
    settle(&mut h);
    h.get_by_label("Apply lighting").click();
    h.get_by_label("Save lighting").click();
    settle(&mut h);
    assert_eq!(document(&h), moved);
    assert_eq!(
        fs::read(fixture.root.join("room.lighting.json")).unwrap(),
        saved_bytes
    );
    {
        let app = h.state_mut();
        let panel = app.room_lighting.as_mut().unwrap();
        assert!(panel
            .apply_for_editor(&mut app.editor, Document::default())
            .is_err());
        assert!(panel.undo_for_editor(&mut app.editor).is_err());
        assert!(panel.redo_for_editor(&mut app.editor).is_err());
        assert!(panel.save_for_editor(&app.editor).is_err());
    }
    h.state_mut().editor.seek(0);
    settle(&mut h);
    h.state_mut().editor.step(24);
    baseline.editor.step(24);
    settle(&mut h);
    baseline.editor.sync();
    assert_eq!(h.state().editor.timeline().unwrap().tick, 24);
    assert_eq!(
        frame_bytes(&mut h.state_mut().editor),
        frame_bytes(&mut baseline.editor)
    );
    if gpu {
        let paused_frame = frame_bytes(&mut h.state_mut().editor);
        let _ = capture(&h, &format!("{capture_prefix}-step"));
        assert_eq!(frame_bytes(&mut h.state_mut().editor), paused_frame);
    }
    h.state_mut().editor.seek(7);
    baseline.editor.seek(7);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 7);
    assert_eq!(
        frame_bytes(&mut h.state_mut().editor),
        frame_bytes(&mut baseline.editor)
    );
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert_eq!(h.state().editor.checksum(), original_checksum);
    fixture.assert_preserved();
}

#[test]
fn generated_room_lighting_actual_editor_apply_undo_redo_save_reopen() {
    live_workflow("room-escape-3d-v1", false);
}

#[cfg(feature = "room-character")]
#[test]
fn character_room_lighting_actual_editor_lifecycle() {
    live_workflow("room-escape-character-3d-v1", false);
}

#[test]
#[ignore = "requires mandatory real GPU; captures production EditorApp viewport"]
fn generated_room_lighting_actual_viewport_gpu_static_and_character() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    live_workflow("room-escape-3d-v1", true);
    #[cfg(feature = "room-character")]
    live_workflow("room-escape-character-3d-v1", true);
}

#[test]
fn lighting_source_restart_and_scene_replacement_retire_stale_panel() {
    let fixture = Fixture::new("room-escape-3d-v1");
    let mut h = fixture.app(false);
    settle(&mut h);
    apply_enabled(&mut h);
    let token = h.state().editor.room_lighting_source_token();
    let frame = frame_bytes(&mut h.state_mut().editor);
    assert!(!h
        .state_mut()
        .editor
        .open_path(&fixture.root.join("absent.yaml")));
    settle(&mut h);
    assert!(Arc::ptr_eq(
        &token,
        &h.state().editor.room_lighting_source_token()
    ));
    assert!(h.state_mut().editor.restart());
    settle(&mut h);
    assert!(!Arc::ptr_eq(
        &token,
        &h.state().editor.room_lighting_source_token()
    ));
    assert_eq!(document(&h), Document::enabled_default());
    assert_eq!(frame_bytes(&mut h.state_mut().editor), frame);
    let _ = h.get_by_label("Lighting source changed; reopen the complete project");
    let bytes = fs::read(fixture.root.join("room.lighting.json")).unwrap();
    h.get_by_label("Save lighting").click();
    settle(&mut h);
    assert_eq!(
        fs::read(fixture.root.join("room.lighting.json")).unwrap(),
        bytes
    );
    let other = fixture.root.join("other.yaml");
    fs::copy(fixture.root.join("room.scene.yaml"), &other).unwrap();
    assert!(h.state_mut().editor.open_path(&other));
    settle(&mut h);
    assert!(h.state().editor.room_lighting_document().is_none());
    assert!(h.state().room_lighting.is_none());
    assert!(h
        .state_mut()
        .editor
        .open_path(&fixture.root.join("room.scene.yaml")));
    settle(&mut h);
    assert!(h.state().room_lighting.is_none());
}

#[test]
fn lighting_replacement_before_first_panel_frame_cannot_rebind() {
    let fixture = Fixture::new("room-escape-3d-v1");
    for restart in [false, true] {
        let mut app = fixture.editor();
        let token = app.editor.room_lighting_source_token();
        let bytes = fs::read(fixture.root.join("room.lighting.json")).unwrap();
        let frame = frame_bytes(&mut app.editor);
        if restart {
            assert!(app.editor.restart());
        } else {
            let other = fixture.root.join("before-first-frame.yaml");
            fs::copy(fixture.root.join("room.scene.yaml"), &other).unwrap();
            assert!(app.editor.open_path(&other));
            assert!(app.editor.open_path(&fixture.root.join("room.scene.yaml")));
            app.editor
                .install_room_lighting(Document::default())
                .unwrap();
        }
        app.editor.sync();
        assert!(!Arc::ptr_eq(
            &token,
            &app.editor.room_lighting_source_token()
        ));
        let panel = app.room_lighting.as_mut().unwrap();
        assert!(panel
            .apply_for_editor(&mut app.editor, Document::enabled_default())
            .is_err());
        assert!(panel.undo_for_editor(&mut app.editor).is_err());
        assert!(panel.redo_for_editor(&mut app.editor).is_err());
        assert!(panel.save_for_editor(&app.editor).is_err());
        assert_eq!(
            app.editor.room_lighting_document(),
            Some(&Document::default())
        );
        assert_eq!(frame_bytes(&mut app.editor), frame);
        assert_eq!(
            fs::read(fixture.root.join("room.lighting.json")).unwrap(),
            bytes
        );
    }
}
