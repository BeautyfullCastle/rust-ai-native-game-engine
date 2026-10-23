#![cfg(feature = "room-authoring")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    Editor, EditorApp, HostSpec,
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
#[allow(dead_code)]
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
fn menu_click(h: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if h.query_by_label(item).is_none() {
        h.get_by_label(menu).click();
        h.run_steps(2);
    }
    h.get_by_label(item).click();
    settle(h);
}
fn player_position(h: &Harness<'_, EditorApp>) -> [f32; 3] {
    let frame = h.state().editor.snapshot().unwrap().predicted();
    let entity = h
        .state()
        .editor
        .rows()
        .iter()
        .map(|row| row.entity)
        .find(|&e| frame.get::<RoomActor>(e).is_some_and(|a| a.kind == PLAYER))
        .unwrap();
    h.state()
        .models
        .placements(&h.state().editor)
        .into_iter()
        .find(|p| p.entity == entity)
        .unwrap()
        .instance
        .translation
}
fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(10);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}
fn component(
    h: &mut Harness<'_, EditorApp>,
    guid: &orr_reflect::Guid,
    name: &str,
) -> orr_reflect::Value {
    h.state_mut()
        .editor
        .field_of(&orr_editor::Target::Guid(guid.clone()), name, "")
        .unwrap()
}
fn read_wall(h: &mut Harness<'_, EditorApp>, guid: &orr_reflect::Guid) {
    if h.query_by_label("Read Room walls").is_none() {
        h.get_by_label("Manage existing Room walls")
            .click_accesskit();
        settle(h);
    }
    click(h, "Read Room walls");
    h.get_by_label(&format!("height_barrier · {guid}"))
        .click_accesskit();
    settle(h);
}
#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_WALL_HEIGHT_DIR"]
fn selected_wall_height_preserves_identity_and_blocks_player() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_HEIGHT_DIR").expect("fresh evidence directory"),
    );
    assert!(!evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Room fixture");
    fixture(&root);
    let preserved = [
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ]
    .map(|n| (n, fs::read(root.join(n)).unwrap()));
    let mut h = harness(&root);
    settle(&mut h);
    h.state_mut().room_authoring.open = true;
    settle(&mut h);
    for (label, value) in [
        ("Room wall name", "height_barrier"),
        ("Wall center X", "1.85"),
        ("Wall center Z", "0"),
        ("Wall half-width X", "0.15"),
        ("Wall half-depth Z", "1"),
    ] {
        field(&mut h, label, value);
    }
    click(&mut h, "Create Room wall");
    let wall = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.name.as_deref() == Some("height_barrier"))
        .unwrap()
        .guid
        .clone()
        .unwrap();
    let identities = h
        .state()
        .editor
        .rows()
        .iter()
        .map(|r| (r.guid.clone(), r.name.clone()))
        .collect::<Vec<_>>();
    let actor = component(&mut h, &wall, "RoomEscapeV1::Actor");
    let body = component(&mut h, &wall, "orr_physics3d::Body");
    let collider = component(&mut h, &wall, "orr_physics3d::Collider");
    let original = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    read_wall(&mut h, &wall);
    let before = pixels(&mut h, &evidence, "01-original-height.png");
    for (y, half) in [("3", "0.1"), ("5", "1"), ("0.5", "8.1")] {
        read_wall(&mut h, &wall);
        field(&mut h, "Selected wall center Y", y);
        field(&mut h, "Selected wall half-height Y", half);
        let button = h.get_by_role_and_label(Role::Button, "Apply wall height");
        assert!(
            !button.accesskit_node().is_disabled(),
            "parsed invalid case must reach authoritative admission"
        );
        button.click_accesskit();
        settle(&mut h);
        assert_eq!(h.state().editor.checksum(), original);
        assert_eq!(h.state().editor.history(), &history);
        assert_eq!(component(&mut h, &wall, "orr_physics3d::Body"), body);
        assert_eq!(
            component(&mut h, &wall, "orr_physics3d::Collider"),
            collider
        );
    }
    shot(&mut h, &evidence, "02-invalid-refused.png");
    read_wall(&mut h, &wall);
    field(&mut h, "Selected wall center Y", "1.5");
    field(&mut h, "Selected wall half-height Y", "2");
    click(&mut h, "Apply wall height");
    let changed = h.state().editor.checksum();
    assert_ne!(changed, original);
    assert_eq!(
        h.state().editor.history().entries.len(),
        history.entries.len() + 1
    );
    assert_eq!(component(&mut h, &wall, "RoomEscapeV1::Actor"), actor);
    let new_body = component(&mut h, &wall, "orr_physics3d::Body");
    let old_pos = body.field("pos").unwrap();
    let new_pos = new_body.field("pos").unwrap();
    if let (orr_reflect::Value::Vec3(a), orr_reflect::Value::Vec3(b)) = (old_pos, new_pos) {
        assert_eq!(a.x, b.x);
        assert_eq!(a.z, b.z);
    } else {
        panic!("positions are vectors");
    }
    assert_eq!(
        h.state()
            .editor
            .rows()
            .iter()
            .map(|r| (r.guid.clone(), r.name.clone()))
            .collect::<Vec<_>>(),
        identities
    );
    let taller = pixels(&mut h, &evidence, "03-taller-wall.png");
    let diff = before.iter().zip(&taller).filter(|(a, b)| a != b).count();
    assert!(diff > 100, "height visibly changes rendered wall");
    // A cached selection cannot authorize a height commit after another host edit.
    read_wall(&mut h, &wall);
    field(&mut h, "Selected wall half-height Y", "3");
    h.state_mut().editor.undo();
    settle(&mut h);
    let stale_checksum = h.state().editor.checksum();
    let stale_history = h.state().editor.history().clone();
    assert!(h
        .query_by_label("Apply wall height")
        .is_none_or(|node| node.accesskit_node().is_disabled()));
    assert_eq!(h.state().editor.checksum(), stale_checksum);
    assert_eq!(h.state().editor.history(), &stale_history);
    h.state_mut().room_authoring.open = false;
    settle(&mut h);
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), changed);
    menu_click(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(h.state().editor.checksum(), original);
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), changed);
    menu_click(&mut h, "File", "Save Ctrl+S");
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), changed);
    shot(&mut h, &evidence, "04-reopened-tall-wall.png");
    click(&mut h, LBL_PLAY);
    click(&mut h, "Take control");
    let start = player_position(&h);
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    let blocked = player_position(&h);
    assert!(blocked[0] > start[0] + 0.05 && blocked[0] < 1.7);
    shot(&mut h, &evidence, "05-play-collision.png");
    click(&mut h, LBL_STOP);
    for (n, bytes) in preserved {
        assert_eq!(fs::read(root.join(n)).unwrap(), bytes);
    }
    let adapter = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .adapter_name()
        .to_owned();
    assert!(adapter.to_lowercase().contains("llvmpipe"));
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"height_changed_bytes":diff,"invalid_height_refused_atomic":true,"stale_read_refused":true,"identity_xz_actors_models_preserved":true,"undo_redo_save_reopen":true,"play_collision":true})).unwrap()).unwrap();
}
