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
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomRun, KEY, PLAYER},
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
fn key(h: &mut Harness<'_, EditorApp>, key: egui::Key, pressed: bool) {
    h.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}
fn run_state(h: &Harness<'_, EditorApp>) -> RoomRun {
    *h.state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}
fn wait_for(h: &mut Harness<'_, EditorApp>, ready: impl Fn(&Harness<'_, EditorApp>) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready(h) {
        assert!(
            std::time::Instant::now() < deadline,
            "gameplay condition timeout"
        );
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_ACTOR_PLACEMENT_DIR"]
fn actor_ground_placement_preserves_staging_and_completes_room() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_ACTOR_PLACEMENT_DIR").expect("fresh evidence directory"),
    );
    assert!(!evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Room fixture");
    fixture(&root);
    let scene_before = fs::read(root.join("room.scene.yaml")).unwrap();
    let preserved = [
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
    let guids = h
        .state()
        .editor
        .rows()
        .iter()
        .filter_map(|r| r.guid.clone())
        .collect::<Vec<_>>();
    let selected = orr_editor::Target::Guid(guids[0].clone());
    h.state_mut().editor.select(Some(selected.clone()));
    settle(&mut h);
    h.state_mut().room_authoring.open = true;
    settle(&mut h);
    let start = egui::pos2(950.0, 90.0);
    let end = egui::pos2(250.0, 90.0);
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(end));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    click(&mut h, "Read Room layout");
    let fields = (
        h.state().room_authoring.player.clone(),
        h.state().room_authoring.key.clone(),
        h.state().room_authoring.exit.clone(),
    );
    click(&mut h, "Place Player in viewport");
    assert!(h
        .get_by_role_and_label(Role::Button, "Draw Room wall footprint")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Cancel actor placement");
    click(&mut h, "Draw Room wall footprint");
    for label in [
        "Place Player in viewport",
        "Place Key in viewport",
        "Place Exit in viewport",
    ] {
        assert!(h
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled());
    }
    click(&mut h, "Cancel wall footprint");
    assert_eq!(
        (
            h.state().room_authoring.player.clone(),
            h.state().room_authoring.key.clone(),
            h.state().room_authoring.exit.clone()
        ),
        fields
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.selection(), Some(&selected));
    click(&mut h, "Place Player in viewport");
    key(&mut h, egui::Key::Escape, true);
    h.step();
    key(&mut h, egui::Key::Escape, false);
    settle(&mut h);
    assert!(h.query_by_label("Cancel actor placement").is_none());
    assert_eq!(h.state().editor.selection(), Some(&selected));
    // Player-wall overlap refuses the entire layout, including the separately staged Key.
    click(&mut h, "Place Player in viewport");
    corner(&mut h, [4.8, 0.0, 0.0]);
    click(&mut h, "Place Key in viewport");
    corner(&mut h, [-1.0, 0.0, -1.0]);
    click(&mut h, "Apply Room layout");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    shot(&mut h, &evidence, "01-overlap-refused.png");
    click(&mut h, "Read Room layout");
    for (label, point) in [
        ("Place Player in viewport", [-1.0, 0.0, -1.0]),
        ("Place Key in viewport", [0.0, 0.0, -1.0]),
        ("Place Exit in viewport", [1.5, 0.0, -1.0]),
    ] {
        click(&mut h, label);
        corner(&mut h, point);
        assert_eq!(h.state().editor.checksum(), checksum);
        assert_eq!(h.state().editor.history(), &history);
        assert_eq!(h.state().editor.selection(), Some(&selected));
    }
    assert_eq!(
        fs::read(root.join("room.scene.yaml")).unwrap(),
        scene_before
    );
    let panel = &h.state().room_authoring;
    for (actual, wanted) in [
        (&panel.player, [-1.0, -1.0]),
        (&panel.key, [0.0, -1.0]),
        (&panel.exit, [1.5, -1.0]),
    ] {
        for (value, expected) in actual.iter().zip(wanted) {
            assert!((value.parse::<f32>().unwrap() - expected).abs() < 0.002);
        }
    }
    shot(&mut h, &evidence, "02-staged-roles.png");
    click(&mut h, "Apply Room layout");
    assert_ne!(h.state().editor.checksum(), checksum);
    let applied = h.state().editor.checksum();
    assert_eq!(
        h.state().editor.history().entries.len(),
        history.entries.len() + 1
    );
    h.state_mut().room_authoring.open = false;
    settle(&mut h);
    menu_click(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(h.state().editor.checksum(), checksum);
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), applied);
    menu_click(&mut h, "File", "Save Ctrl+S");
    assert!(!h.state().editor.is_dirty());
    shot(&mut h, &evidence, "03-applied-saved.png");
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), applied);
    for guid in &guids {
        assert!(h
            .state()
            .editor
            .rows()
            .iter()
            .any(|r| r.guid.as_ref() == Some(guid)));
    }
    shot(&mut h, &evidence, "04-reopened.png");
    click(&mut h, LBL_PLAY);
    click(&mut h, "Take control");
    wait_for(&mut h, |h| {
        h.state().editor.input_phase() == orr_editor::editor::input::Phase::Active
    });
    key(&mut h, egui::Key::E, true);
    wait_for(&mut h, |h| run_state(h).key_collected == 1);
    key(&mut h, egui::Key::E, false);
    h.run_steps(5);
    shot(&mut h, &evidence, "05-key-collected.png");
    key(&mut h, egui::Key::D, true);
    wait_for(&mut h, |h| player_position(h)[0] > 0.5);
    key(&mut h, egui::Key::D, false);
    h.run_steps(5);
    key(&mut h, egui::Key::E, true);
    wait_for(&mut h, |h| run_state(h).won == 1);
    key(&mut h, egui::Key::E, false);
    settle(&mut h);
    shot(&mut h, &evidence, "06-room-won.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), applied);
    for (name, bytes) in preserved {
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
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"three_role_ground_clicks":true,"actor_wall_mutual_exclusion":true,"staging_no_mutation":true,"overlap_refusal_atomic":true,"escape_preserves_selection":true,"apply_undo_redo_save_reopen":true,"play_key_pickup_then_exit_won":true,"guids_models_camera_preserved":true})).unwrap()).unwrap();
}
