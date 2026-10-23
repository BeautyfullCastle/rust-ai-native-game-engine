#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers, PointerButton};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    Editor, EditorApp, HostSpec, Mode,
};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
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
        .with_size([1500.0, 1000.0])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(20);
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
fn pixels(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str) -> Vec<u8> {
    settle(h);
    let viewport = h.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    let bytes = viewport.gpu().read_rgba8();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &bytes);
    orr_editor::app::write_png(&dir.join(name), &image).unwrap();
    bytes
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
fn drag(h: &mut Harness<'_, EditorApp>, button: PointerButton, delta: egui::Vec2) {
    let start = egui::pos2(650.0, 400.0);
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: start,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    h.event(Event::PointerMoved(start + delta));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: start + delta,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(h);
}
#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_CAMERA_CAPTURE_DIR"]
fn viewport_camera_stage_apply_cancel_history_save_reopen_and_play() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_CAMERA_CAPTURE_DIR").expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    fixture(&root);
    let preserved = [
        "room.scene.yaml",
        "room.models.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ]
    .map(|name| (name, fs::read(root.join(name)).unwrap()));
    let camera_bytes = fs::read(root.join("room.camera.json")).unwrap();
    let mut h = harness(&root);
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let authored = h.state().room_camera.as_ref().unwrap().document().clone();
    let initial = pixels(&mut h, &evidence, "01-authored.png");
    drag(&mut h, PointerButton::Secondary, egui::vec2(65.0, 30.0));
    drag(&mut h, PointerButton::Middle, egui::vec2(25.0, -18.0));
    h.event(Event::PointerMoved(egui::pos2(650.0, 400.0)));
    h.run_steps(2);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 90.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(30);
    settle(&mut h);
    let live = h.state().editor.camera3d;
    assert_ne!(live.target, authored.target);
    assert_ne!(live.yaw, authored.yaw);
    assert_ne!(live.distance, authored.distance);
    let navigated = pixels(&mut h, &evidence, "02-session-navigation.png");
    assert_ne!(initial, navigated);
    click(&mut h, "Stage viewport camera");
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &authored
    );
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        fs::read(root.join("room.camera.json")).unwrap(),
        camera_bytes
    );
    shot(&mut h, &evidence, "03-staged-controls.png");
    click(&mut h, "Cancel staged viewport camera");
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &authored
    );
    assert!(h
        .get_by_role_and_label(Role::Button, "Apply staged viewport camera")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Stage viewport camera");
    click(&mut h, "Apply staged viewport camera");
    let captured = h.state().room_camera.as_ref().unwrap().document().clone();
    assert_ne!(captured, authored);
    assert!(h.state().room_camera.as_ref().unwrap().dirty());
    assert_eq!(captured.target, live.target);
    let applied = pixels(&mut h, &evidence, "04-applied-same-framing.png");
    assert_eq!(applied, navigated);
    assert_eq!(
        fs::read(root.join("room.camera.json")).unwrap(),
        camera_bytes
    );
    click(&mut h, "Undo captured camera");
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &authored
    );
    assert_eq!(pixels(&mut h, &evidence, "05-capture-undone.png"), initial);
    click(&mut h, "Redo captured camera");
    assert_eq!(pixels(&mut h, &evidence, "06-capture-redone.png"), applied);
    click(&mut h, "Save captured camera");
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    let saved = fs::read(root.join("room.camera.json")).unwrap();
    assert_ne!(saved, camera_bytes);
    // An unrelated camera authoring caller cannot reuse capture-specific history.
    let mut external = captured.clone();
    external.yaw += 0.1;
    h.state_mut()
        .room_camera
        .as_mut()
        .unwrap()
        .apply_document(external.clone())
        .unwrap();
    h.state_mut().editor.install_room_camera(external).unwrap();
    settle(&mut h);
    assert!(h.query_by_label("Undo captured camera").is_none());
    assert!(h.query_by_label("Redo captured camera").is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &captured
    );
    assert_eq!(pixels(&mut h, &evidence, "07-reopened-camera.png"), applied);
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &captured
    );
    shot(&mut h, &evidence, "08-play-authored-camera.png");
    click(&mut h, LBL_STOP);
    assert_eq!(
        pixels(&mut h, &evidence, "09-stop-authored-camera.png"),
        applied
    );
    for (name, bytes) in preserved {
        assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
    }
    assert_eq!(fs::read(root.join("room.camera.json")).unwrap(), saved);
    assert_eq!(h.state().editor.checksum(), checksum);
    let adapter = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .adapter_name()
        .to_owned();
    assert!(adapter.to_lowercase().contains("llvmpipe"));
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"real_orbit_pan_zoom_events":true,"stage_cancel_no_authoring_mutation":true,"apply_preserves_viewport_pixels":true,"capture_undo_redo":true,"external_camera_change_invalidates_history":true,"save_reopen_play_stop":true,"scene_models_lock_history_unchanged":true})).unwrap()).unwrap();
}
