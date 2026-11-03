//! Actual Room camera authoring acceptance through the production EditorApp.
//! This is headless egui input and real wgpu readback, not native OS input or a
//! hardware-GPU claim. The receipt records the adapter actually selected.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::wgpu;
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    model::History,
    Editor, EditorApp, HostSpec, Mode,
};
use orr_model_bindings::model_bindings::{
    self, Binding, Document as ModelDocument, LocalTransform,
};
use orr_sample::{
    room_camera::{Document as CameraDocument, Projection},
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

fn fixture(root: &Path) {
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
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": 2,
            "engine": "*",
            "entry": {
                "game": "room-escape-v1",
                "scene": "room.scene.yaml",
                "models": "room.models.json",
                "camera": "room.camera.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
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
        serde_json::to_vec(&ModelDocument {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
}

fn harness(root: &Path) -> (Harness<'static, EditorApp>, wgpu::AdapterInfo) {
    // Every call opens a new PreparedProject. Nothing from the first editor's
    // in-memory camera/history is injected into the reopened editor.
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
    editor.install_room_camera(camera.document.clone()).unwrap();
    let adapter = Arc::new(Mutex::new(None));
    let observed = Arc::clone(&adapter);
    let h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        // Use the normal headless harness selection, without forcing Vulkan
        // or substituting a mock renderer when no real adapter is available.
        .wgpu()
        .build_eframe(move |cc| {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .expect("actual wgpu adapter required");
            let info = state.adapter.get_info();
            eprintln!(
                "Room camera redo adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            *observed.lock().unwrap() = Some(info);
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        });
    let info = adapter.lock().unwrap().take().expect("adapter recorded");
    (h, info)
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn open_camera(h: &mut Harness<'_, EditorApp>) {
    settle(h);
    h.get_by_label("Room camera").scroll_to_me();
    h.run_steps(20);
    h.get_by_label("Room camera").click();
    h.run_steps(20);
    settle(h);
    assert!(h
        .query_by_role_and_label(Role::Button, "Apply camera")
        .is_some());
}

fn click(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(20);
    assert!(
        !h.get_by_role_and_label(role, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(role, label).click_accesskit();
    settle(h);
}

fn disabled(h: &Harness<'_, EditorApp>, label: &str) {
    assert!(
        h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "{label} must be disabled"
    );
}

fn assert_camera(h: &Harness<'_, EditorApp>, expected: &CameraDocument) {
    assert_eq!(h.state().room_camera.as_ref().unwrap().document(), expected);
    assert_eq!(h.state().editor.camera3d, expected.orbit());
    let size = h.state().viewport3d_gpu().unwrap().gpu().target().size();
    assert_eq!(
        h.state().editor.presentation_camera3d(size).unwrap(),
        expected.camera(&expected.orbit(), size).unwrap()
    );
}

fn pixels(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str, size: (u32, u32)) -> Vec<u8> {
    settle(h);
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("production Room viewport");
    assert!(
        viewport.last_texture().is_some(),
        "a Room frame was rendered"
    );
    assert_eq!(viewport.gpu().target().size(), size, "fixed viewport size");
    let bytes = viewport.gpu().read_rgba8();
    assert_eq!(bytes.len(), (size.0 * size.1 * 4) as usize);
    assert!(bytes.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    assert!(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[..3] != bytes[..3]),
        "actual Room viewport must not be a uniform clear-color image"
    );
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &bytes);
    orr_editor::app::write_png(&dir.join(name), &image).unwrap();
    bytes
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count()
}

fn assert_same_pixels(actual: &[u8], expected: &[u8], stage: &str) {
    assert_eq!(changed_pixels(actual, expected), 0, "{stage}");
}

fn shot(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str) {
    let image = h.render().unwrap();
    orr_editor::app::write_png(
        &dir.join(name),
        &egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
    )
    .unwrap();
}

struct Preserved {
    checksum: u64,
    history: History,
    models: ModelDocument,
    files: Vec<(&'static str, Vec<u8>)>,
}

impl Preserved {
    fn new(h: &Harness<'_, EditorApp>, root: &Path) -> Self {
        Self {
            checksum: h.state().editor.checksum(),
            history: h.state().editor.history().clone(),
            models: h
                .state()
                .models
                .bindings
                .as_ref()
                .unwrap()
                .document()
                .clone(),
            files: [
                "room.scene.yaml",
                "room.models.json",
                "orr.project.json",
                "orr.packages.lock.json",
            ]
            .into_iter()
            .map(|name| (name, fs::read(root.join(name)).unwrap()))
            .collect(),
        }
    }

    fn assert_unchanged(&self, h: &Harness<'_, EditorApp>, root: &Path) {
        // The simulation may advance during Play, but its authored history,
        // scene files and presentation bindings must still remain unchanged.
        if h.state().editor.mode() == Mode::Edit {
            assert_eq!(h.state().editor.checksum(), self.checksum);
        }
        assert_eq!(h.state().editor.history(), &self.history);
        assert!(!h.state().editor.is_dirty());
        let bindings = h.state().models.bindings.as_ref().unwrap();
        assert_eq!(bindings.document(), &self.models);
        assert!(!bindings.dirty());
        for (name, bytes) in &self.files {
            assert_eq!(&fs::read(root.join(name)).unwrap(), bytes, "changed {name}");
        }
    }
}

#[test]
#[ignore = "actual EditorApp wgpu acceptance; requires fresh absolute ORR_ROOM_CAMERA_REDO_DIR"]
fn room_camera_apply_undo_redo_save_reopen_pixels_and_play_gating() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_CAMERA_REDO_DIR").expect("fresh evidence directory required"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).expect("evidence directory must not already exist");
    let root = evidence.join("room");
    fixture(&root);
    let original_bytes = fs::read(root.join("room.camera.json")).unwrap();
    let original = CameraDocument::parse(&original_bytes).unwrap();
    let (mut h, adapter) = harness(&root);
    open_camera(&mut h);
    let preserved = Preserved::new(&h, &root);
    let size = h.state().viewport3d_gpu().unwrap().gpu().target().size();
    assert!(size.0 > 0 && size.1 > 0);
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),
        adapter.name
    );
    assert_camera(&h, &original);
    disabled(&h, "Undo camera edit");
    disabled(&h, "Redo camera edit");
    let before = pixels(&mut h, &evidence, "01-original.png", size);

    // Use actual camera authoring widgets. A candidate alone cannot change
    // the authored document, live camera, pixels, disk or scene history.
    assert!(matches!(
        original.projection,
        Projection::Orthographic { .. }
    ));
    click(&mut h, Role::RadioButton, "Perspective");
    assert_camera(&h, &original);
    assert!(h.state().room_camera.as_ref().unwrap().dirty());
    preserved.assert_unchanged(&h, &root);
    assert_eq!(
        fs::read(root.join("room.camera.json")).unwrap(),
        original_bytes
    );
    assert_same_pixels(
        &pixels(&mut h, &evidence, "02-staged-only.png", size),
        &before,
        "staging does not change live viewport",
    );
    let mut edited = original.clone();
    edited.projection = Projection::Perspective {
        fov_y_degrees: 60.0,
    };
    click(&mut h, Role::Button, "Apply camera");
    assert_camera(&h, &edited);
    assert!(h.state().room_camera.as_ref().unwrap().dirty());
    preserved.assert_unchanged(&h, &root);
    let applied = pixels(&mut h, &evidence, "03-applied.png", size);
    let changed = changed_pixels(&before, &applied);
    assert!(
        changed > 100,
        "camera Apply must visibly change the actual Room viewport"
    );
    disabled(&h, "Redo camera edit");
    assert_eq!(
        fs::read(root.join("room.camera.json")).unwrap(),
        original_bytes
    );

    click(&mut h, Role::Button, "Undo camera edit");
    assert_camera(&h, &original);
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    preserved.assert_unchanged(&h, &root);
    assert_same_pixels(
        &pixels(&mut h, &evidence, "04-undone.png", size),
        &before,
        "Undo restores the original live viewport exactly",
    );
    shot(&mut h, &evidence, "05-redo-enabled-controls.png");
    click(&mut h, Role::Button, "Redo camera edit");
    assert_camera(&h, &edited);
    assert!(h.state().room_camera.as_ref().unwrap().dirty());
    preserved.assert_unchanged(&h, &root);
    assert_same_pixels(
        &pixels(&mut h, &evidence, "06-redone.png", size),
        &applied,
        "actual Redo button restores the applied live viewport exactly",
    );
    assert_eq!(
        fs::read(root.join("room.camera.json")).unwrap(),
        original_bytes
    );

    // The primary Redo above must traverse the real accessibility button and
    // EditorApp's normal reset-live path. The public no-op API checks below
    // additionally prove that repeated exhausted history cannot mutate state.
    for _ in 0..3 {
        disabled(&h, "Redo camera edit");
        assert!(!h.state_mut().room_camera.as_mut().unwrap().redo());
        settle(&mut h);
        assert_camera(&h, &edited);
        assert!(h.state().room_camera.as_ref().unwrap().dirty());
        preserved.assert_unchanged(&h, &root);
    }
    assert_same_pixels(
        &pixels(&mut h, &evidence, "07-empty-redo-no-change.png", size),
        &applied,
        "repeated empty redo does not change pixels",
    );
    click(&mut h, Role::Button, "Save camera");
    assert_camera(&h, &edited);
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    let saved = fs::read(root.join("room.camera.json")).unwrap();
    assert_eq!(saved, edited.to_bytes().unwrap());
    assert_ne!(saved, original_bytes);
    preserved.assert_unchanged(&h, &root);

    // Populate redo before Play, so disabled cannot merely mean empty history.
    click(&mut h, Role::Button, "Undo camera edit");
    assert_camera(&h, &original);
    assert!(h.state().room_camera.as_ref().unwrap().dirty());
    assert!(!h
        .get_by_role_and_label(Role::Button, "Redo camera edit")
        .accesskit_node()
        .is_disabled());
    click(&mut h, Role::Button, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_camera(&h, &original);
    for label in [
        "Apply camera",
        "Undo camera edit",
        "Redo camera edit",
        "Save camera",
    ] {
        disabled(&h, label);
    }
    preserved.assert_unchanged(&h, &root);
    assert_eq!(fs::read(root.join("room.camera.json")).unwrap(), saved);
    shot(&mut h, &evidence, "08-play-redo-disabled.png");
    click(&mut h, Role::Button, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_camera(&h, &original);
    preserved.assert_unchanged(&h, &root);
    click(&mut h, Role::Button, "Redo camera edit");
    assert_camera(&h, &edited);
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    assert_same_pixels(
        &pixels(&mut h, &evidence, "09-stop-redo.png", size),
        &applied,
        "Play/Stop preserves the camera redo entry",
    );
    assert_eq!(fs::read(root.join("room.camera.json")).unwrap(), saved);
    preserved.assert_unchanged(&h, &root);
    drop(h);

    let (mut reopened, reopened_adapter) = harness(&root);
    open_camera(&mut reopened);
    assert_eq!(reopened_adapter.name, adapter.name);
    assert_eq!(reopened_adapter.backend, adapter.backend);
    assert_eq!(reopened_adapter.device_type, adapter.device_type);
    assert_camera(&reopened, &edited);
    assert!(!reopened.state().room_camera.as_ref().unwrap().dirty());
    disabled(&reopened, "Undo camera edit");
    disabled(&reopened, "Redo camera edit");
    preserved.assert_unchanged(&reopened, &root);
    assert_eq!(fs::read(root.join("room.camera.json")).unwrap(), saved);
    assert_same_pixels(
        &pixels(&mut reopened, &evidence, "10-fresh-reopened.png", size),
        &applied,
        "fresh PreparedProject and EditorApp reproduce saved framing exactly",
    );
    shot(&mut reopened, &evidence, "11-reopened-controls.png");
    fs::write(
        evidence.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 1,
            "production_editor_app": true,
            "gpu_readback": true,
            "adapter": adapter.name,
            "adapter_backend": format!("{:?}", adapter.backend),
            "adapter_device_type": format!("{:?}", adapter.device_type),
            "software_gpu": adapter.device_type == wgpu::DeviceType::Cpu,
            "physical_os_input": false,
            "input": "egui_kittest actual camera widgets and accessibility button actions",
            "viewport_size": [size.0, size.1],
            "changed_pixels_after_apply": changed,
            "original_camera": original,
            "edited_camera": edited,
            "scene_checksum": format!("0x{:016x}", preserved.checksum),
            "staging_changes_neither_document_nor_live_camera": true,
            "actual_apply_undo_redo_buttons": true,
            "live_orbit_and_projection_exact": true,
            "undo_redo_pixels_exact": true,
            "repeated_empty_redo_no_change": true,
            "populated_redo_disabled_in_play": true,
            "play_stop_retains_redo": true,
            "save_and_fresh_prepared_project_reopen_pixels_exact": true,
            "scene_models_manifest_lock_history_unchanged": true,
            "hardware_gpu_or_native_window_claim": false
        }))
        .unwrap(),
    )
    .unwrap();
}
