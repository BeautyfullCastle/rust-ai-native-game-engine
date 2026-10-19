//! Explicit raw-GLB v1 to v2 package update through the production EditorApp.
//! Preview is read-only; hidden-source fresh reopen and actual KEY play/restart
//! preserve authoritative simulation bytes with default renderer options.
#![cfg(feature = "package-manager")]
#![recursion_limit = "256"]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_bridge::PlayerSlot;
use orr_editor::{editor::input::Phase, Editor, EditorApp, HostSpec, Mode};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomRun, INTERACT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::TickInputs;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
fn hash(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325_u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
fn fixture(root: &Path, raw: &Path, sources: &Path) {
    for dir in [root, raw, sources] {
        fs::create_dir(dir).unwrap();
    }
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    let mut camera = orr_sample::room_camera::Document::readable_default();
    camera.target = [1.0, 0.5, 0.0];
    camera.projection = orr_sample::room_camera::Projection::Orthographic { half_height: 4.0 };
    fs::write(root.join("room.camera.json"), camera.to_bytes().unwrap()).unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(std::slice::from_ref(&original))
    .unwrap();
    let loaded = model_bindings::load_asset(root, EXISTING, "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        EXISTING.into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| [PLAYER, KEY].contains(&a.kind))
        })
        .map(|e| (scene.index().guid(e).unwrap().to_string(), binding.clone()))
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
    let v1 = fs::read(original.join("foreground.glb")).unwrap();
    fs::write(raw.join("v1.glb"), &v1).unwrap();
    // Preserve the embedded binary chunk while visibly changing material and geometry.
    let json_len = u32::from_le_bytes(v1[12..16].try_into().unwrap()) as usize;
    let end = 20 + json_len;
    let mut json: serde_json::Value = serde_json::from_slice(&v1[20..end]).unwrap();
    json["nodes"][0]["scale"] = serde_json::json!([0.5, 1.6, 1.2]);
    json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
        serde_json::json!([0.15, 0.9, 0.25, 1.0]);
    let mut document = serde_json::to_vec(&json).unwrap();
    while !document.len().is_multiple_of(4) {
        document.push(b' ');
    }
    let total = 20 + document.len() + v1.len() - end;
    let mut v2 = Vec::new();
    v2.extend_from_slice(b"glTF");
    v2.extend_from_slice(&2_u32.to_le_bytes());
    v2.extend_from_slice(&(total as u32).to_le_bytes());
    v2.extend_from_slice(&(document.len() as u32).to_le_bytes());
    v2.extend_from_slice(&0x4e4f_534a_u32.to_le_bytes());
    v2.extend_from_slice(&document);
    v2.extend_from_slice(&v1[end..]);
    fs::write(raw.join("v2.glb"), &v2).unwrap();
    fs::copy(original.join("LICENSE.txt"), raw.join("LICENSE.txt")).unwrap();
    assert_ne!(v1, v2);
    for bytes in [&v1, &v2] {
        orr_model::import::import_with_resolver("model.glb", bytes, |_| {
            panic!("self-contained GLB")
        })
        .unwrap();
    }
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
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
            egui_kittest::wgpu::create_render_state(
                setup.into(),
                egui_wgpu::RendererOptions::default(),
            ),
        ))
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}

const CANVAS: &str = "Static package model preview canvas";
const PACKAGE: &str = "raw-package-update";
const EXISTING: &str = "sample-imported-scene";

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

fn viewport_pixels(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    // Normalize actual render targets; never resample or crop the comparison.
    let target = (900, 850);
    for _ in 0..8 {
        settle(h);
        let actual = h.state().viewport3d_gpu().unwrap().gpu().target().size();
        if actual == target {
            break;
        }
        let window = h.ctx.viewport_rect().size();
        let ratio = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(h);
    let gpu = h.state().viewport3d_gpu().unwrap().gpu();
    assert_eq!(gpu.target().size(), target);
    let bytes = gpu.read_rgba8();
    assert_eq!(bytes.len(), (target.0 * target.1 * 4) as usize);
    assert!(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != &bytes[..4])
            .count()
            > 1000,
        "nonblank production viewport required"
    );
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &bytes),
    )
    .unwrap();
    bytes
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

fn run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}
fn key(h: &mut Harness<'_, EditorApp>, pressed: bool) {
    h.event(Event::Key {
        key: egui::Key::E,
        physical_key: Some(egui::Key::E),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}
fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("package-update-frame-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
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
fn import(h: &mut Harness<'_, EditorApp>, raw: &Path, sources: &Path, version: &str) {
    set_field(
        h,
        "Raw GLB file",
        &raw.join(format!("v{version}.glb")).display().to_string(),
    );
    set_field(
        h,
        "License file",
        &raw.join("LICENSE.txt").display().to_string(),
    );
    set_field(
        h,
        "Package destination directory",
        &sources.join(format!("v{version}")).display().to_string(),
    );
    set_field(h, "Package name", PACKAGE);
    set_field(h, "Package version", &format!("{version}.0.0"));
    assert!(h
        .get_by_role_and_label(Role::Button, "Import raw GLB package")
        .accesskit_node()
        .is_disabled());
    h.get_by_role_and_label(
        Role::CheckBox,
        "I have reviewed rights to distribute this content",
    )
    .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(
        Role::CheckBox,
        "I have reviewed rights to distribute this content",
    )
    .click_accesskit();
    settle(h);
    click(h, "Import raw GLB package");
    wait(h, |h| !h.state().package_manager.busy());
    assert!(
        h.state()
            .package_manager
            .status
            .starts_with("Imported raw package"),
        "{}",
        h.state().package_manager.status
    );
}
fn saved_model_controls(h: &mut Harness<'_, EditorApp>) {
    h.state_mut().package_manager.open = false;
    settle(h);
    if h.query_by_label("Save model bindings").is_none() {
        h.get_by_label("Static model binding").click_accesskit();
        settle(h);
    }
}
fn open_packages(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(h);
}
#[test]
#[ignore = "requires software Vulkan and a fresh absolute package update capture directory"]
fn raw_glb_update_key_preview_save_reopen_collect_restart() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_PACKAGE_UPDATE_CAPTURE_DIR").expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    let raw = evidence.join("raw-inputs");
    let sources = evidence.join("package-sources");
    fixture(&root, &raw, &sources);
    let initial = files(&root);
    let raw_bytes = files(&raw);
    let mut h = harness(&root);
    settle(&mut h);
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    assert!(
        adapter.to_lowercase().contains("llvmpipe"),
        "mandatory software Vulkan: {adapter}"
    );
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
    let authored_frame = frame_bytes(&mut h.state_mut().editor);
    let prepared = PreparedProject::open(&root).unwrap();
    let scene = prepared.scene();
    let entity = scene
        .frame()
        .entities()
        .find(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| a.kind == KEY)
        })
        .unwrap();
    let guid = scene.index().guid(entity).unwrap().clone();
    open_packages(&mut h);
    h.get_by_label("Import a raw GLB package").click_accesskit();
    settle(&mut h);
    import(&mut h, &raw, &sources, "1");
    assert_eq!(files(&root), initial);
    assert_eq!(
        fs::read(sources.join("v1/model.glb")).unwrap(),
        raw_bytes[Path::new("v1.glb")]
    );
    click(&mut h, "Preview package addition");
    wait(&mut h, |h| !h.state().package_manager.busy());
    assert_eq!(files(&root), initial);
    click(&mut h, "Add previewed package");
    wait(&mut h, |h| !h.state().package_manager.busy());
    assert!(
        h.state()
            .package_manager
            .status
            .starts_with("Package added."),
        "{}",
        h.state().package_manager.status
    );
    let old_lock = orr_editor::package_manager::inspect(&root).unwrap();
    assert_eq!(old_lock.direct[PACKAGE], "1.0.0");
    fs::copy(
        root.join("orr.packages.lock.json"),
        evidence.join("lock-before-update.json"),
    )
    .unwrap();
    verify(&mut h);
    preview(&mut h, "model.glb");
    let v1_pixels = crop(&mut h, &evidence, "01-v1-canvas.png");
    shot(&mut h, &evidence, "01-v1-preview.png");
    click(&mut h, "Close static preview");
    assert!(h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .all(|b| b.package != PACKAGE));
    // Deliberately retain an old Undo entry and transient loaded v1 candidate,
    // while restoring and saving the original, wholly unassigned Room document.
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    click(&mut h, &format!("Use model {PACKAGE}/model.glb"));
    saved_model_controls(&mut h);
    click(&mut h, "Save model bindings");
    open_packages(&mut h);
    verify(&mut h);
    click(&mut h, &format!("Use model {EXISTING}/foreground.glb"));
    saved_model_controls(&mut h);
    click(&mut h, "Save model bindings");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    {
        let app = h.state_mut();
        app.models.package = PACKAGE.into();
        app.models.asset = "model.glb".into();
        app.models.load_candidate(&app.editor).unwrap();
    }
    h.state_mut().editor.select(None);
    open_packages(&mut h);
    import(&mut h, &raw, &sources, "2");
    assert_eq!(
        fs::read(sources.join("v2/model.glb")).unwrap(),
        raw_bytes[Path::new("v2.glb")]
    );
    let before_update = files(&root);
    let source_bytes = files(&sources);
    click(&mut h, "Preview update");
    wait(&mut h, |h| !h.state().package_manager.busy());
    assert!(
        !h.get_by_role_and_label(Role::Button, "Update previewed package")
            .accesskit_node()
            .is_disabled(),
        "{}",
        h.state().package_manager.status
    );
    shot(&mut h, &evidence, "02-update-preview.png");
    assert_eq!(files(&root), before_update);
    assert_eq!(files(&sources), source_bytes);
    assert_eq!(files(&raw), raw_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    // Changing the source invalidates the retained preview; no implicit update.
    set_field(
        &mut h,
        "Local content package directory",
        &sources.join("v1").display().to_string(),
    );
    assert!(h
        .get_by_role_and_label(Role::Button, "Update previewed package")
        .accesskit_node()
        .is_disabled());
    set_field(
        &mut h,
        "Local content package directory",
        &sources.join("v2").display().to_string(),
    );
    assert!(h
        .get_by_role_and_label(Role::Button, "Update previewed package")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Preview update");
    wait(&mut h, |h| !h.state().package_manager.busy());
    assert_eq!(files(&root), before_update);
    click(&mut h, "Update previewed package");
    wait(&mut h, |h| !h.state().package_manager.busy());
    let updated = orr_editor::package_manager::inspect(&root).unwrap();
    assert_eq!(
        updated.direct[PACKAGE],
        "2.0.0",
        "{}",
        h.state().package_manager.status
    );
    assert_ne!(
        updated.packages[PACKAGE].digest,
        old_lock.packages[PACKAGE].digest
    );
    assert_eq!(updated.packages.len(), old_lock.packages.len());
    for (name, package) in &old_lock.packages {
        if name != PACKAGE {
            assert_eq!(updated.packages[name], *package);
        }
    }
    for (path, bytes) in &before_update {
        if path != Path::new("orr.packages.lock.json") {
            assert_eq!(
                files(&root).get(path),
                Some(bytes),
                "existing file changed: {}",
                path.display()
            );
        }
    }
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    fs::copy(
        root.join("orr.packages.lock.json"),
        evidence.join("lock-after-update.json"),
    )
    .unwrap();
    shot(&mut h, &evidence, "03-updated.png");
    // Neither an old Undo revision nor a retained v1 candidate may reintroduce v1.
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    let update_files = files(&root);
    {
        let app = h.state_mut();
        assert!(app.models.undo(&app.editor).is_err());
        assert!(app.models.assign(&app.editor).is_err());
    }
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(files(&root), update_files);
    verify(&mut h);
    preview(&mut h, "model.glb");
    let v2_pixels = crop(&mut h, &evidence, "04-v2-canvas.png");
    let version_changed_pixels = differences(&v1_pixels, &v2_pixels);
    assert!(
        version_changed_pixels > 1000,
        "visibly different v2: {version_changed_pixels}"
    );
    shot(&mut h, &evidence, "04-v2-preview.png");
    click(&mut h, "Close static preview");
    assert_eq!(files(&root), update_files);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    let current = model_bindings::load_asset(&root, PACKAGE, "model.glb").unwrap();
    assert_eq!(current.package_digest(), updated.packages[PACKAGE].digest);
    click(&mut h, &format!("Use model {PACKAGE}/model.glb"));
    let assigned = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()]
        .clone();
    let mut expected = bindings.clone();
    expected.bindings.insert(guid.to_string(), assigned.clone());
    assert_eq!(assigned.package, PACKAGE);
    assert_eq!(assigned.asset, "model.glb");
    assert_eq!(assigned.transform, LocalTransform::default());
    assigned.validate(&current).unwrap();
    assert_eq!(assigned.package_digest, updated.packages[PACKAGE].digest);
    saved_model_controls(&mut h);
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    h.state_mut().editor.select(None);
    settle(&mut h);
    let saved_pixels = viewport_pixels(&mut h, &evidence, "05-saved-viewport.png");
    shot(&mut h, &evidence, "05-saved.png");
    let expected_files = files(&root);
    let hidden_source = evidence.join("hidden-package-sources");
    let hidden_raw = evidence.join("hidden-raw-inputs");
    fs::rename(&sources, &hidden_source).unwrap();
    fs::rename(&raw, &hidden_raw).unwrap();
    assert!(!sources.exists() && !raw.exists());
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    let reopened = model_bindings::load_binding(&root, &assigned).unwrap();
    assert_eq!(reopened.source_hash(), current.source_hash());
    let reopened_pixels = viewport_pixels(&mut h, &evidence, "06-reopened-viewport.png");
    assert_eq!(
        saved_pixels, reopened_pixels,
        "fresh default-renderer host/app/cache reopen"
    );
    shot(&mut h, &evidence, "06-reopened.png");
    assert_eq!(files(&root), expected_files);
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    // Actual app controls and physical E event. The canonical KEY is within the
    // simulation's interaction distance; presentation size never affects it.
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, |h| h.state().editor.can_take_control());
    click(&mut h, "Take control");
    wait(&mut h, |h| h.state().editor.input_phase() == Phase::Active);
    let before_run = run(h.state());
    assert_eq!(before_run.key_collected, 0);
    key(&mut h, true);
    wait(&mut h, |h| run(h.state()).key_collected == 1);
    key(&mut h, false);
    wait(&mut h, |h| run(h.state()).previous_buttons == 0);
    h.state_mut().editor.pause();
    settle(&mut h);
    let live = frame_bytes(&mut h.state_mut().editor);
    let live_checksum = h.state().editor.snapshot().unwrap().predicted().checksum();
    let collected_run = run(h.state());
    assert_eq!(collected_run.key_collected, 1);
    let key_entity = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.guid.as_ref() == Some(&guid))
        .unwrap()
        .entity;
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 1);
    assert!(placements.iter().all(|p| p.entity != key_entity));
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    let collected_pixels = viewport_pixels(&mut h, &evidence, "11-collected-viewport.png");
    let collect_changed = differences(&reopened_pixels, &collected_pixels);
    assert!(
        collect_changed > 1000,
        "actual updated KEY must disappear: {collect_changed}"
    );
    shot(&mut h, &evidence, "11-collected.png");
    let stopped_tick = h.state().editor.timeline().unwrap().tick;
    let sequence = h.state().editor.snapshot().unwrap().seq();
    click(&mut h, "Restart room");
    wait(&mut h, |h| {
        h.state().editor.mode() == Mode::Play
            && h.state().editor.can_take_control()
            && h.state().editor.yard_rows_coherent()
            && h.state().editor.snapshot().is_some_and(|s| {
                s.seq() > sequence
                    && s.timeline().is_some_and(|t| t.playing)
                    && *s.predicted().singleton::<RoomRun>() == RoomRun::default()
            })
    });
    let stopped = h.state().editor.last_stopped().unwrap().clone();
    assert_eq!(stopped.tick, stopped_tick);
    assert_eq!(stopped.checksum, live_checksum);
    h.state_mut().editor.pause();
    settle(&mut h);
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    assert_eq!(run(h.state()), RoomRun::default());
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    let restarted_pixels = viewport_pixels(&mut h, &evidence, "12-restarted-viewport.png");
    assert_eq!(
        restarted_pixels, reopened_pixels,
        "Restart restores exact updated model and initial gameplay view"
    );
    shot(&mut h, &evidence, "12-restarted.png");
    // Replay the exact recorded GUI-admitted samples, never guessed durations,
    // in an independent simulation that has never loaded any model binding.
    let reader = orr_session::ReplayReader::<RoomEscapeV1>::parse(&stopped.replay).unwrap();
    let mut baseline = prepared.scene().simulation().unwrap();
    let mut saw_interact = false;
    let mut saw_release = false;
    let mut checked = 0;
    if let Some((_, sum)) = reader.checksums.iter().find(|(at, _)| *at == 0) {
        assert_eq!(
            baseline.frame().checksum(),
            *sum,
            "presentation-free initial checksum"
        );
        checked += 1;
    }
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).unwrap();
        assert!(commands.is_empty());
        saw_interact |= inputs[0].buttons & INTERACT != 0;
        saw_release |= saw_interact && inputs[0].buttons == 0;
        let mut samples = TickInputs::new(tick, 1);
        samples.set_input(PlayerSlot(0), inputs[0]);
        baseline.step(&samples);
        if let Some((_, sum)) = reader.checksums.iter().find(|(at, _)| *at == tick) {
            assert_eq!(
                baseline.frame().checksum(),
                *sum,
                "presentation-free checksum at {tick}"
            );
            checked += 1;
        }
    }
    assert!(saw_interact && saw_release);
    assert!(checked > 0, "recorded checksum coverage must be nonempty");
    assert_eq!(
        checked,
        reader.checksums.len(),
        "every recorded checksum must be checked"
    );
    assert_eq!(baseline.frame().to_bytes(), live);
    assert_eq!(baseline.frame().checksum(), live_checksum);
    assert_eq!(*baseline.frame().singleton::<RoomRun>(), collected_run);
    fs::write(evidence.join("collected-authoritative.frame"), &live).unwrap();
    fs::write(
        evidence.join("presentation-free.frame"),
        baseline.frame().to_bytes(),
    )
    .unwrap();
    fs::write(evidence.join("actual-gui.orrp"), &stopped.replay).unwrap();
    click(&mut h, orr_editor::app::LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    assert_eq!(files(&root), expected_files);
    assert_eq!(files(&hidden_source), source_bytes);
    assert_eq!(files(&hidden_raw), raw_bytes);
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"status":"passed","test":"raw_glb_update_key_preview_save_reopen_collect_restart","mandatory_test_count":1,
        "test_source_fnv1a64":hash(include_bytes!("package_update_preview_ui.rs")),
        "package":PACKAGE,"old_version":old_lock.direct[PACKAGE],"new_version":lock.direct[PACKAGE],
        "old_digest":old_lock.packages[PACKAGE].digest,"new_digest":lock.packages[PACKAGE].digest,
        "raw_v1_fnv1a64":hash(&raw_bytes[Path::new("v1.glb")]),"raw_v2_fnv1a64":hash(&raw_bytes[Path::new("v2.glb")]),
        "raw_import_v1_v2":true,"preview_read_only":true,"explicit_update":true,"other_packages_unchanged":true,
        "source_change_invalidates_preview":true,"retained_candidate_rejected":true,"old_undo_rejected":true,
        "assigned_guid":guid.to_string(),"assigned_role":"KEY","assigned_binding":assigned,"source_hash":reopened.source_hash(),
        "adapter":h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),"renderer_options":"default","predictable_texture_filtering":false,"dithering":true,
        "version_changed_pixels":version_changed_pixels,"collect_changed_pixels":collect_changed,
        "sources_hidden":true,"fresh_reopen_binding_exact":true,"fresh_reopen_viewport_pixels_exact":true,"normalized_viewport_size":[900,850],
        "actual_play_take_control_keyboard_interact":true,"key_placements_before_collect":2,"key_placements_after_collect":1,"key_placements_after_restart":2,
        "key_collected":collected_run.key_collected,"restart_key_collected":0,"restart_viewport_pixels_exact":true,
        "presentation_free_frame_bytes_exact":true,"presentation_free_checksums_exact":true,"recorded_ticks":reader.last_tick(),"recorded_checksums_checked":checked,
        "authoritative_checksum":format!("{live_checksum:016x}"),"source_and_installed_files_preserved":true
    })).unwrap()).unwrap();
}
