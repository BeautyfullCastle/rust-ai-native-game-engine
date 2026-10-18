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
fn assert_margin(h: &Harness<'_, EditorApp>) {
    let size = h.state().ui.viewport_px;
    let camera = h.state().editor.presentation_camera3d(size).unwrap();
    for x in [3.6, 4.0] {
        for y in [-0.5, 3.5] {
            for z in [2.8, 3.2] {
                let p = camera
                    .world_to_screen([x, y, z], size)
                    .expect("collider corner in front of camera");
                assert!(
                    p[0] > size.0 as f32 * 0.03
                        && p[0] < (size.0 as f32) * 0.97
                        && p[1] > size.1 as f32 * 0.03
                        && p[1] < (size.1 as f32) * 0.97,
                    "corner {p:?} lacks margin in {size:?}"
                );
            }
        }
    }
}
#[test]
#[ignore = "requires software Vulkan and fresh ORR_FRAME_SELECTION_DIR"]
fn collider_framing_button_and_viewport_f_preserve_authoring() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_FRAME_SELECTION_DIR").expect("fresh evidence directory"),
    );
    assert!(!evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Room fixture");
    fixture(&root);
    let mut h = harness(&root);
    settle(&mut h);
    h.state_mut().room_authoring.open = true;
    settle(&mut h);
    for (label, value) in [
        ("Room wall name", "height_barrier"),
        ("Wall center X", "3.8"),
        ("Wall center Z", "3"),
        ("Wall half-width X", "0.2"),
        ("Wall half-depth Z", "0.2"),
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
    read_wall(&mut h, &wall);
    field(&mut h, "Selected wall center Y", "1.5");
    field(&mut h, "Selected wall half-height Y", "2");
    click(&mut h, "Apply wall height");
    h.state_mut().room_authoring.open = false;
    settle(&mut h);
    menu_click(&mut h, "File", "Save Ctrl+S");
    h.state_mut().editor.select(None);
    settle(&mut h);
    h.state_mut().editor.camera3d.target = [-30.0, 0.0, -30.0];
    settle(&mut h);
    let row_label = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.guid.as_ref() == Some(&wall))
        .unwrap()
        .label();
    h.get_by_label(&row_label).click_accesskit();
    settle(&mut h);
    let selection = h.state().editor.selection().cloned();
    assert!(selection.is_some());
    let authored = h.state().room_camera.as_ref().unwrap().document().clone();
    let history = h.state().editor.history().clone();
    let checksum = h.state().editor.checksum();
    let tick = h.state().editor.snapshot().unwrap().predicted().tick();
    let files = [
        "room.scene.yaml",
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ]
    .map(|n| (n, fs::read(root.join(n)).unwrap()));
    let offscreen = pixels(&mut h, &evidence, "01-offscreen-selected.png");
    let yaw = h.state().editor.camera3d.yaw;
    let pitch = h.state().editor.camera3d.pitch;
    h.get_by_role_and_label(Role::Button, "Frame selected collider")
        .scroll_to_me();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Frame selected collider")
        .click();
    settle(&mut h);
    assert_margin(&h);
    assert_eq!(h.state().editor.camera3d.yaw, yaw);
    assert_eq!(h.state().editor.camera3d.pitch, pitch);
    let framed = pixels(&mut h, &evidence, "02-button-framed.png");
    assert_ne!(framed, offscreen);
    let framed_target = h.state().editor.camera3d.target;
    let framed_distance = h.state().editor.camera3d.distance;
    h.state_mut().editor.camera3d.target = [-30.0, 0.0, -30.0];
    settle(&mut h);
    // F is ignored while the viewport does not own focus.
    h.ctx.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert_eq!(h.state().editor.camera3d.target, [-30.0, 0.0, -30.0]);
    let point = h.state().ui.viewport_rect.unwrap().center();
    h.event(Event::PointerMoved(point));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert_margin(&h);
    assert_eq!(h.state().editor.camera3d.target, framed_target);
    assert_eq!(h.state().editor.camera3d.distance, framed_distance);
    shot(&mut h, &evidence, "03-keyboard-framed.png");
    h.state_mut().editor.camera3d.target = [-30.0, 0.0, -30.0];
    settle(&mut h);
    h.event(Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert_eq!(
        h.state().editor.camera3d.target,
        [-30.0, 0.0, -30.0],
        "held viewport pointer must block framing"
    );
    h.event(Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);

    assert_eq!(h.state().editor.selection().cloned(), selection);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().tick(),
        tick
    );
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &authored
    );
    assert!(!h.state().room_camera.as_ref().unwrap().dirty());
    h.state_mut().editor.select(None);
    settle(&mut h);
    assert!(h
        .get_by_role_and_label(Role::Button, "Frame selected collider")
        .accesskit_node()
        .is_disabled());
    click(&mut h, LBL_PLAY);
    assert!(h
        .get_by_role_and_label(Role::Button, "Frame selected collider")
        .accesskit_node()
        .is_disabled());
    click(&mut h, LBL_STOP);
    for (n, bytes) in files {
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
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"button_frames_full_collider_with_margin":true,"viewport_f_frames":true,"unfocused_f_ignored":true,"held_pointer_f_ignored":true,"no_selection_and_play_disabled":true,"yaw_pitch_preserved":true,"authoring_frame_history_selection_files_unchanged":true})).unwrap()).unwrap();
}
