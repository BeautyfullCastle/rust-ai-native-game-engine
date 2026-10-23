#![cfg(feature = "package-manager")]
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
fn fixture(root: &Path, source: &Path) {
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
    fs::create_dir(source).unwrap();
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
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source.to_path_buf()])
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

const CANVAS: &str = "Static package model preview canvas";
const PACKAGE: &str = "sample-imported-scene";

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
    click(h, &format!("Preview static model {PACKAGE}/{asset}"));
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

#[test]
#[ignore = "requires software Vulkan and fresh ORR_PACKAGE_MODEL_PREVIEW_CAPTURE_DIR"]
fn verified_static_package_preview_is_read_only_and_assignment_still_reopens() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_PACKAGE_MODEL_PREVIEW_CAPTURE_DIR")
            .expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    let source = evidence.join("owned-package-copy");
    fixture(&root, &source);
    let original = files(&root);
    let source_bytes = files(&source);
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
    assert!(
        h.query_by_label(&format!("Preview static model {PACKAGE}/foreground.glb"))
            .is_none(),
        "unverified inventory cannot preview"
    );
    verify(&mut h);
    shot(&mut h, &evidence, "01-verified.png");

    preview(&mut h, "foreground.glb");
    shot(&mut h, &evidence, "02-foreground.png");
    let foreground = crop(&mut h, &evidence, "02-foreground-canvas.png");
    preview(&mut h, "background.glb");
    shot(&mut h, &evidence, "03-background.png");
    let background = crop(&mut h, &evidence, "03-background-canvas.png");
    let different_assets = differences(&foreground, &background);
    assert!(
        different_assets > 1000,
        "distinct assets must draw visibly different geometry/materials, got {different_assets}"
    );
    orbit(&mut h);
    shot(&mut h, &evidence, "04-orbit.png");
    let orbited = crop(&mut h, &evidence, "04-orbit-canvas.png");
    let orbit_pixels = differences(&background, &orbited);
    assert!(orbit_pixels > 1000, "orbit must visibly change the preview");
    let center = h.get_by_label(CANVAS).rect().center();
    h.event(Event::PointerMoved(center));
    h.step();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 100.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(40);
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
    let zoomed = crop(&mut h, &evidence, "05-zoom-canvas.png");
    let zoom_pixels = differences(&orbited, &zoomed);
    assert!(zoom_pixels > 1000, "zoom must visibly change the preview");
    click(&mut h, "Reset preview view");
    h.run_steps(10);
    let reset = crop(&mut h, &evidence, "06-reset-canvas.png");
    assert_eq!(
        reset, background,
        "Reset restores the exact default asset camera"
    );
    shot(&mut h, &evidence, "06-reset.png");
    click(&mut h, "Close static preview");
    assert!(h.query_by_label(CANVAS).is_none());
    assert!(h.query_by_label("Static package model preview").is_none());
    shot(&mut h, &evidence, "07-closed.png");
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
    assert_eq!(files(&source), source_bytes);

    // Tamper only with this test's own installed copy after Verify. The button
    // still exists, but the asynchronous loader must refuse the changed bytes.
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    let cached = root
        .join(".orr/packages/objects")
        .join(&lock.packages[PACKAGE].digest)
        .join("background.glb");
    let good_bytes = fs::read(&cached).unwrap();
    fs::write(&cached, b"corrupt isolated acceptance fixture").unwrap();
    click(
        &mut h,
        &format!("Preview static model {PACKAGE}/background.glb"),
    );
    wait(&mut h, |h| {
        h.query_by_label_contains("Static preview unavailable:")
            .is_some()
    });
    assert!(h.query_by_label(CANVAS).is_none());
    shot(&mut h, &evidence, "08-corrupt-refused.png");
    click(&mut h, "Close static preview");
    fs::write(cached, good_bytes).unwrap();
    verify(&mut h);
    assert_eq!(files(&root), original);

    // Assignment remains a separate existing action and changes only the model
    // sidecar after its explicit Save. It does not mutate scene history/Frame.
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
    click(&mut h, &format!("Use model {PACKAGE}/background.glb"));
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .asset,
        "background.glb"
    );
    h.state_mut().package_manager.open = false;
    settle(&mut h);
    if h.query_by_label("Save model bindings").is_none() {
        h.get_by_label("Static model binding").click_accesskit();
        settle(&mut h);
    }
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    shot(&mut h, &evidence, "09-assigned-saved.png");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    for name in [
        "room.scene.yaml",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ] {
        if let Some(bytes) = original.get(Path::new(name)) {
            assert_eq!(&fs::read(root.join(name)).unwrap(), bytes);
        }
    }
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .asset,
        "background.glb"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    shot(&mut h, &evidence, "10-reopened.png");
    assert_eq!(files(&source), source_bytes);
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "test": "verified_static_package_preview_is_read_only_and_assignment_still_reopens",
            "adapter": h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),
            "different_asset_pixels": different_assets,
            "orbit_changed_pixels": orbit_pixels,
            "zoom_changed_pixels": zoom_pixels,
            "reset_exact": true,
            "preview_without_selection": true,
            "preview_preserves_project_source_files_frame_history_bindings_and_editor_camera": true,
            "corrupt_cached_asset_refused": true,
            "use_model_save_reopen_preserved": true
        }))
        .unwrap(),
    )
    .unwrap();
}
