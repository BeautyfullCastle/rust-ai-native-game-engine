#![cfg(all(feature = "package-manager", feature = "room-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{Editor, EditorApp, HostSpec};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture(root: &Path, sources: &Path) {
    fs::create_dir(sources).unwrap();
    let source = sources.join("static");
    let animation = sources.join("animated");
    fs::create_dir(root).unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    fs::create_dir(&source).unwrap();
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    for name in [
        "orr.package.json",
        "foreground.glb",
        "background.glb",
        "LICENSE.txt",
        "generate.py",
    ] {
        fs::copy(shipped.join(name), source.join(name)).unwrap();
    }
    fs::create_dir(&animation).unwrap();
    let shipped_animation =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    for name in ["orr.package.json", "animated.glb", "LICENSE.txt"] {
        fs::copy(shipped_animation.join(name), animation.join(name)).unwrap();
    }
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source, animation])
    .unwrap();
    let loaded =
        model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        "sample-imported-scene".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
        })
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            )
        })
        .collect();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
}
fn harness(root: &Path) -> Harness<'static, EditorApp> {
    let mut prepared = PreparedProject::open(root).unwrap();
    let camera = prepared.take_camera().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.select(None);
    editor.install_room_camera(camera.document.clone()).unwrap();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                adapter.get_info().backend == wgpu::Backend::Vulkan
                    && adapter.get_info().device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}

const CANVAS: &str = "Animated package model preview canvas";
const PACKAGE: &str = "sample-animation";

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    h.state_mut().editor.sync();
    h.run_steps(5);
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(8);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(h);
}

fn wait(h: &mut Harness<'_, EditorApp>, condition: impl Fn(&Harness<'_, EditorApp>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition(h) {
        settle(h);
        assert!(Instant::now() < deadline, "package preview timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(h);
}

fn verify(h: &mut Harness<'_, EditorApp>) {
    click(h, "Verify installed packages");
    wait(h, |h| !h.state().package_manager.busy());
    assert!(h
        .state()
        .package_manager
        .status
        .starts_with("Installed package bytes match"));
}

fn preview(h: &mut Harness<'_, EditorApp>, asset: &str) {
    click(h, &format!("Preview animated model {PACKAGE}/{asset}"));
    wait(h, |h| h.query_by_label(CANVAS).is_some());
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
}

fn shot(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) {
    let frame = h.render().expect("production EditorApp GPU frame");
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied(
            [frame.width() as usize, frame.height() as usize],
            frame.as_raw(),
        ),
    )
    .unwrap();
}

fn crop(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    let rect = h.get_by_label(CANVAS).rect();
    let scale = h.ctx.pixels_per_point();
    let frame = h.render().expect("production EditorApp preview canvas");
    let x = (rect.left() * scale).round() as u32;
    let y = (rect.top() * scale).round() as u32;
    let width = (rect.width() * scale).round() as u32;
    let height = (rect.height() * scale).round() as u32;
    assert!(width > 0 && height > 0 && width <= 1024 && height <= 1024);
    assert!(x + width <= frame.width() && y + height <= frame.height());
    let mut rgba = Vec::new();
    for row in y..y + height {
        let start = ((row * frame.width() + x) * 4) as usize;
        rgba.extend_from_slice(&frame.as_raw()[start..start + width as usize * 4]);
    }
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
    )
    .unwrap();
    rgba
}

fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count()
}

fn orbit(h: &mut Harness<'_, EditorApp>) {
    let start = h.get_by_label(CANVAS).rect().center();
    h.event(Event::PointerMoved(start));
    h.step();
    h.event(Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for step in 1..=5 {
        h.event(Event::PointerMoved(
            start + egui::vec2(15.0 * step as f32, 7.0 * step as f32),
        ));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: start + egui::vec2(75.0, 35.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
}

fn set_field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}

fn sample(h: &mut Harness<'_, EditorApp>, seconds: &str) {
    set_field(h, "Preview seconds", seconds);
    click(h, "Sample preview pose");
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_PACKAGE_ANIMATED_PREVIEW_CAPTURE_DIR"]
fn verified_animated_package_preview_samples_explicit_poses_without_authoring() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_PACKAGE_ANIMATED_PREVIEW_CAPTURE_DIR")
            .expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    let sources = evidence.join("owned-package-copies");
    fixture(&root, &sources);
    let original = files(&root);
    let source_bytes = files(&sources);
    let mut h = harness(&root);
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let camera = h.state().editor.camera3d;
    let bindings = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(h.state().editor.selection().is_none());
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(&mut h);
    assert!(h
        .query_by_label(&format!("Preview animated model {PACKAGE}/animated.glb"))
        .is_none());
    verify(&mut h);
    assert!(
        h.query_by_label("Preview animated model sample-imported-scene/foreground.glb")
            .is_none(),
        "models-only packages cannot offer animated preview"
    );
    shot(&mut h, &evidence, "01-verified.png");
    preview(&mut h, "animated.glb");
    assert!(h
        .query_by_label("Accepted preview pose: clip 0 at 0.000s")
        .is_some());
    shot(&mut h, &evidence, "02-default-pose.png");
    let initial = crop(&mut h, &evidence, "02-default-canvas.png");

    h.get_by_role_and_label(Role::ComboBox, "Preview animation clip")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_label("1: pulse (1.00s)").click();
    settle(&mut h);
    // Selecting another named clip changes only the draft, until Sample.
    assert!(h
        .query_by_label("Accepted preview pose: clip 0 at 0.000s")
        .is_some());
    assert_eq!(
        crop(&mut h, &evidence, "03-unsampled-draft-canvas.png"),
        initial
    );
    sample(&mut h, "0");
    assert!(h
        .query_by_label("Accepted preview pose: clip 1 at 0.000s")
        .is_some());
    let first = crop(&mut h, &evidence, "04-clip-one-start-canvas.png");
    sample(&mut h, "0.5");
    assert!(h
        .query_by_label("Accepted preview pose: clip 1 at 0.500s")
        .is_some());
    let second = crop(&mut h, &evidence, "05-clip-one-midpoint-canvas.png");
    let time_pixels = differences(&first, &second);
    assert!(
        time_pixels > 1000,
        "two explicit times must visibly differ, got {time_pixels}"
    );
    shot(&mut h, &evidence, "05-clip-one-midpoint.png");
    // Let UI time advance. A sampled pose is not a live player.
    h.run_steps(120);
    assert_eq!(crop(&mut h, &evidence, "06-held-pose-canvas.png"), second);

    for (index, seconds) in [
        ("999", "0.5"),
        ("1", "NaN"),
        ("1", "-1"),
        ("1", "2"),
        ("1", "not-a-time"),
    ] {
        set_field(&mut h, "Preview clip index", index);
        sample(&mut h, seconds);
        assert!(h
            .query_by_label_contains("Preview sample refused:")
            .is_some());
        assert!(h
            .query_by_label("Accepted preview pose: clip 1 at 0.500s")
            .is_some());
        assert_eq!(
            crop(&mut h, &evidence, "07-invalid-retained-canvas.png"),
            second,
            "invalid index={index} time={seconds} must retain exact accepted image"
        );
    }
    shot(&mut h, &evidence, "07-invalid-retained.png");
    // Restore valid drafts without resampling; orbit and Reset affect only camera.
    set_field(&mut h, "Preview clip index", "1");
    set_field(&mut h, "Preview seconds", "0.5");
    orbit(&mut h);
    let orbited = crop(&mut h, &evidence, "08-orbit-canvas.png");
    let orbit_pixels = differences(&second, &orbited);
    assert!(orbit_pixels > 1000);
    click(&mut h, "Reset preview view");
    h.run_steps(10);
    assert_eq!(crop(&mut h, &evidence, "09-reset-canvas.png"), second);
    assert!(h
        .query_by_label("Accepted preview pose: clip 1 at 0.500s")
        .is_some());
    shot(&mut h, &evidence, "09-reset.png");

    // Static replacement uses the same one-window/resource slot and clears all
    // animated draft/error/pose state. Reopening animation starts clip0/time0.
    click(
        &mut h,
        "Preview static model sample-imported-scene/foreground.glb",
    );
    wait(&mut h, |h| {
        h.query_by_label("Static package model preview canvas")
            .is_some()
    });
    assert!(h.query_by_label(CANVAS).is_none());
    assert!(h.query_by_label("Sample preview pose").is_none());
    assert!(h
        .query_by_label_contains("Preview sample refused:")
        .is_none());
    shot(&mut h, &evidence, "10-static-replacement.png");
    preview(&mut h, "animated.glb");
    assert!(h
        .query_by_label("Accepted preview pose: clip 0 at 0.000s")
        .is_some());
    assert_eq!(
        crop(&mut h, &evidence, "11-animation-replaced-canvas.png"),
        initial
    );
    click(&mut h, "Close animated preview");
    assert!(h.query_by_label(CANVAS).is_none());
    assert!(h.query_by_label("Animated package model preview").is_none());
    preview(&mut h, "animated.glb");
    assert!(h
        .query_by_label("Accepted preview pose: clip 0 at 0.000s")
        .is_some());
    assert_eq!(
        crop(&mut h, &evidence, "12-animation-reopened-canvas.png"),
        initial
    );
    click(&mut h, "Close animated preview");
    shot(&mut h, &evidence, "13-closed.png");
    assert!(h.state().editor.selection().is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(files(&root), original);
    assert_eq!(files(&sources), source_bytes);

    // Existing explicit assignment remains independent of the preview's clip1.
    let prepared = PreparedProject::open(&root).unwrap();
    let scene = prepared.scene();
    let player = scene
        .frame()
        .entities()
        .find(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    let guid = scene.index().guid(player).unwrap().clone();
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    click(
        &mut h,
        &format!("Use animated model {PACKAGE}/animated.glb"),
    );
    let binding = &h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()];
    assert_eq!(binding.kind, model_bindings::ModelKind::Animated);
    assert_eq!(binding.animation.unwrap().clip_index, 0);
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    h.state_mut().package_manager.open = false;
    settle(&mut h);
    if h.query_by_label("Save model bindings").is_none() {
        h.get_by_label("Static model binding").click_accesskit();
        settle(&mut h);
    }
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    shot(&mut h, &evidence, "14-assigned-saved.png");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    let saved = files(&root);
    assert_eq!(saved.len(), original.len());
    for (path, bytes) in &original {
        if path != Path::new("room.models.json") {
            assert_eq!(&saved[path], bytes);
        }
    }
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    let binding = &h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()];
    assert_eq!(binding.kind, model_bindings::ModelKind::Animated);
    assert_eq!(binding.animation.unwrap().clip_index, 0);
    assert_eq!(h.state().editor.checksum(), checksum);
    shot(&mut h, &evidence, "15-reopened.png");
    assert_eq!(files(&sources), source_bytes);
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "test": "verified_animated_package_preview_samples_explicit_poses_without_authoring",
            "adapter": h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),
            "clip_index": 1,
            "clip_name": "pulse",
            "sample_times_seconds": [0.0, 0.5],
            "time_changed_pixels": time_pixels,
            "orbit_changed_pixels": orbit_pixels,
            "unsampled_draft_preserved_exact": true,
            "sample_held_without_live_playback": true,
            "invalid_index_and_times_retained_exact_pose": true,
            "reset_exact": true,
            "replacement_and_close_reset_clip_and_pose": true,
            "preview_without_selection": true,
            "preview_preserves_files_frame_history_bindings_and_editor_camera": true,
            "animated_use_model_clip_zero_save_reopen_preserved": true
        }))
        .unwrap(),
    )
    .unwrap();
}
