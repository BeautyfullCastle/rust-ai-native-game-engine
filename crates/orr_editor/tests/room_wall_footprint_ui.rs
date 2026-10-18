#![cfg(feature = "room-authoring")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers, PointerButton};
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
fn corner(h: &mut Harness<'_, EditorApp>, world: [f32; 3]) {
    let rect = h.state().ui.viewport_rect.unwrap();
    let size = h.state().ui.viewport_px;
    let camera = h.state().editor.presentation_camera3d(size).unwrap();
    let pixel = camera.world_to_screen(world, size).unwrap();
    let point = rect.min + egui::vec2(pixel[0], pixel[1]) * (rect.width() / size.0 as f32);
    assert!(rect.contains(point));
    h.event(Event::PointerMoved(point));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    settle(h);
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
#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_WALL_FOOTPRINT_DIR"]
fn two_corner_wall_stages_creates_persists_and_blocks_player() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_FOOTPRINT_DIR").expect("fresh evidence directory"),
    );
    assert!(!evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Room fixture");
    fixture(&root);
    let scene_before = fs::read(root.join("room.scene.yaml")).unwrap();
    let unchanged = [
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ]
    .map(|name| (name, fs::read(root.join(name)).unwrap()));
    let mut h = harness(&root);
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let actors = h
        .state()
        .editor
        .rows()
        .iter()
        .filter_map(|r| r.guid.clone())
        .collect::<Vec<_>>();
    let selected = actors[0].clone();
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(selected.clone())));
    settle(&mut h);
    let position = player_position(&h);
    h.state_mut().room_authoring.open = true;
    h.state_mut().room_authoring.wall_name = "viewport_barrier".into();
    settle(&mut h);
    // Move the floating authoring window clear of the ground corners.
    let start = egui::pos2(950.0, 90.0);
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(egui::pos2(250.0, 90.0)));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: egui::pos2(250.0, 90.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    click(&mut h, "Draw Room wall footprint");
    h.event(Event::Key {
        key: egui::Key::Escape,
        physical_key: Some(egui::Key::Escape),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::Key {
        key: egui::Key::Escape,
        physical_key: Some(egui::Key::Escape),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert!(h.query_by_label("Cancel wall footprint").is_none());
    assert_eq!(
        h.state().editor.selection(),
        Some(&orr_editor::Target::Guid(selected.clone()))
    );
    click(&mut h, "Draw Room wall footprint");
    corner(&mut h, [position[0] + 1.7, 0.0, position[2] - 1.0]);
    assert!(h.query_by_label("Cancel wall footprint").is_some());
    shot(&mut h, &evidence, "01-first-corner.png");
    corner(&mut h, [position[0] + 2.0, 0.0, position[2] + 1.0]);
    assert!(h.query_by_label("Cancel wall footprint").is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state().editor.selection(),
        Some(&orr_editor::Target::Guid(selected))
    );
    assert_eq!(
        fs::read(root.join("room.scene.yaml")).unwrap(),
        scene_before
    );
    let center = h
        .state()
        .room_authoring
        .wall_center
        .clone()
        .map(|v| v.parse::<f32>().unwrap());
    let half = h
        .state()
        .room_authoring
        .wall_half_extents
        .clone()
        .map(|v| v.parse::<f32>().unwrap());
    assert!((center[0] - position[0] - 1.85).abs() < 0.002);
    assert!((center[1] - position[2]).abs() < 0.002);
    assert!((half[0] - 0.15).abs() < 0.002);
    assert!((half[1] - 1.0).abs() < 0.002);
    shot(&mut h, &evidence, "02-staged-footprint.png");
    click(&mut h, "Create Room wall");
    let wall = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.name.as_deref() == Some("viewport_barrier"))
        .unwrap()
        .guid
        .clone()
        .unwrap();
    assert!(!actors.contains(&wall));
    assert_eq!(
        h.state().editor.history().entries.len(),
        history.entries.len() + 1
    );
    h.state_mut().room_authoring.open = false;
    settle(&mut h);
    shot(&mut h, &evidence, "03-created-wall.png");
    menu_click(&mut h, "Edit", "Undo Ctrl+Z");
    assert!(!h
        .state()
        .editor
        .rows()
        .iter()
        .any(|r| r.guid.as_ref() == Some(&wall)));
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert!(h
        .state()
        .editor
        .rows()
        .iter()
        .any(|r| r.guid.as_ref() == Some(&wall)));
    menu_click(&mut h, "File", "Save Ctrl+S");
    assert!(!h.state().editor.is_dirty());
    for guid in &actors {
        assert!(h
            .state()
            .editor
            .rows()
            .iter()
            .any(|r| r.guid.as_ref() == Some(guid)));
    }
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert!(h
        .state()
        .editor
        .rows()
        .iter()
        .any(|r| r.guid.as_ref() == Some(&wall)));
    shot(&mut h, &evidence, "04-reopened-wall.png");
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
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < until {
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
    assert!(
        blocked[0] > start[0] + 0.05,
        "player moved: {start:?} -> {blocked:?}"
    );
    assert!(
        blocked[0] < position[0] + 1.7,
        "wall blocks PLAYER: {blocked:?}"
    );
    shot(&mut h, &evidence, "05-play-collision.png");
    click(&mut h, LBL_STOP);
    for (name, bytes) in unchanged {
        assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
    }
    let adapter = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .adapter_name()
        .to_owned();
    assert!(adapter.to_lowercase().contains("llvmpipe"));
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"two_corner_staging_only":true,"escape_preserves_selection":true,"fresh_wall_guid":wall.to_string(),"undo_redo_save_reopen":true,"play_collision":true,"actors_models_camera_preserved":true})).unwrap()).unwrap();
}
