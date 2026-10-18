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
// Linear line color [1.0, 0.85, 0.2] is encoded as sRGB [255, 237, 124]
// by the production viewport. This excludes the Yard box's orange fill.
fn is_outline_pixel(pixel: &[u8; 4]) -> bool {
    pixel[..3]
        .iter()
        .zip([255u8, 237, 124])
        .all(|(actual, expected)| actual.abs_diff(expected) <= 2)
}

// Check the rendered yellow wire against the twelve projected collider edges,
// not the old max-extent selection cube. No screenshot postprocessing is used.
fn assert_thin_collider_outline(h: &Harness<'_, EditorApp>, rgba: &[u8]) -> usize {
    let size = h.state().ui.viewport_px;
    let camera = h.state().editor.presentation_camera3d(size).unwrap();
    let corners: Vec<_> = [3.6, 4.0]
        .into_iter()
        .flat_map(|x| {
            [-0.5, 3.5]
                .into_iter()
                .flat_map(move |y| [2.8, 3.2].into_iter().map(move |z| [x, y, z]))
        })
        .map(|p| camera.world_to_screen(p, size).unwrap())
        .collect();
    let mut edges = Vec::new();
    for a in 0..8usize {
        for bit in [1, 2, 4] {
            let b = a ^ bit;
            if a < b {
                edges.push((corners[a], corners[b]));
            }
        }
    }
    assert_eq!(rgba.len(), size.0 as usize * size.1 as usize * 4);
    let mut yellow = 0;
    for (i, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if !is_outline_pixel(pixel) {
            continue;
        }
        let p = [
            (i % size.0 as usize) as f32 + 0.5,
            (i / size.0 as usize) as f32 + 0.5,
        ];
        let distance = edges
            .iter()
            .map(|(a, b)| {
                let d = [b[0] - a[0], b[1] - a[1]];
                let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1])
                    / (d[0] * d[0] + d[1] * d[1]))
                    .clamp(0.0, 1.0);
                ((p[0] - a[0] - t * d[0]).powi(2) + (p[1] - a[1] - t * d[1]).powi(2)).sqrt()
            })
            .fold(f32::INFINITY, f32::min);
        assert!(
            distance < 4.0,
            "yellow pixel {p:?} is {distance} pixels from collider edges"
        );
        assert!(
            p[0] > size.0 as f32 * 0.025
                && p[0] < size.0 as f32 * 0.975
                && p[1] > size.1 as f32 * 0.025
                && p[1] < size.1 as f32 * 0.975
        );
        yellow += 1;
    }
    assert!(
        yellow > 30,
        "expected visible yellow collider wire, found {yellow}"
    );
    yellow
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
    assert_eq!(
        h.state()
            .editor
            .row_of(selection.as_ref().unwrap())
            .unwrap()
            .guid
            .as_ref(),
        Some(&wall)
    );
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
    let outline_pixels = assert_thin_collider_outline(&h, &framed);
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
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"accurate_collider_outline":true,"yellow_outline_pixels":outline_pixels,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"button_frames_full_collider_with_margin":true,"viewport_f_frames":true,"unfocused_f_ignored":true,"held_pointer_f_ignored":true,"no_selection_and_play_disabled":true,"yaw_pitch_preserved":true,"authoring_frame_history_selection_files_unchanged":true})).unwrap()).unwrap();
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_YARD_SELECTION_OUTLINE_DIR"]
fn yard_rotated_box_and_capsule_outlines_follow_colliders() {
    let evidence = PathBuf::from(std::env::var_os("ORR_YARD_SELECTION_OUTLINE_DIR").unwrap());
    assert!(!evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let source = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
    let (ground, actors) = source.split_once("  e_00000002:").unwrap();
    let actors = actors.replace(
        "rot: { kind: unit, x: 0, y: 0, z: 0, w: 1 }",
        "rot: { kind: unit, x: 0, y: 0.6, z: 0, w: 0.8 }",
    );
    let (left, right) = actors.split_once("  e_00000003:").unwrap();
    let left = left.replace(
        "half_extents: [0.5, 0.5, 0.5]",
        "half_extents: [0.3, 1.5, 0.7]",
    );
    let right = right.replace(
        "kind: box, half_extents: [0.5, 0.5, 0.5]",
        "kind: capsule, half_length: 1.2, radius: 0.35",
    );
    let text = format!("{ground}  e_00000002:{left}  e_00000003:{right}");
    let path = evidence.join("yard.scene.yaml");
    fs::write(&path, &text).unwrap();
    let mut editor = Editor::open_game(&path, orr_editor::game::EditorGame::Yard3D).unwrap();
    editor.sync();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| {
                a.get_info().backend == wgpu::Backend::Vulkan
                    && a.get_info().device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let tick = h.state().editor.snapshot().unwrap().predicted().tick();
    let mut counts = Vec::new();
    for (guid, name) in [
        ("e_00000002", "01-rotated-box.png"),
        ("e_00000003", "02-capsule.png"),
    ] {
        let guid = orr_reflect::Guid::parse(guid).unwrap();
        let row = h
            .state()
            .editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref() == Some(&guid))
            .unwrap();
        let entity = row.entity;
        let label = row.label();
        h.get_by_label(&label).click_accesskit();
        settle(&mut h);
        let selection = h.state().editor.selection().cloned();
        assert_eq!(
            h.state()
                .editor
                .row_of(selection.as_ref().unwrap())
                .unwrap()
                .guid
                .as_ref(),
            Some(&guid)
        );
        click(&mut h, "Frame selected collider");
        let item = *h
            .state()
            .editor
            .yard_frame()
            .items
            .iter()
            .find(|i| i.entity == entity)
            .unwrap();
        let size = h.state().ui.viewport_px;
        let camera = h.state().editor.presentation_camera3d(size).unwrap();
        let mut local = Vec::new();
        assert!(matches!(
            (guid.as_str(), item.style.shape),
            ("e_00000002", orr_view::Shape3::Box { .. })
                | ("e_00000003", orr_view::Shape3::Capsule { .. })
        ));
        match item.style.shape {
            orr_view::Shape3::Box { half } => {
                assert!(item.transform.rot.to_array()[1].abs() > 0.5);
                for axis in 0..3 {
                    for a in [-1.0, 1.0] {
                        for b in [-1.0, 1.0] {
                            for n in 0..=256 {
                                let mut p = [0.0; 3];
                                p[axis] = half[axis] * (n as f32 / 128.0 - 1.0);
                                p[(axis + 1) % 3] = a * half[(axis + 1) % 3];
                                p[(axis + 2) % 3] = b * half[(axis + 2) % 3];
                                local.push(p);
                            }
                        }
                    }
                }
            }
            orr_view::Shape3::Capsule {
                half_length,
                radius,
            } => {
                for sign in [-1.0, 1.0] {
                    for n in 0..=256 {
                        let a = n as f32 / 256.0 * std::f32::consts::TAU;
                        local.push([radius * a.cos(), sign * half_length, radius * a.sin()]);
                        let a = a * 0.5;
                        for axis in [0, 2] {
                            let mut p = [0.0; 3];
                            p[axis] = radius * a.cos();
                            p[1] = sign * (half_length + radius * a.sin());
                            local.push(p);
                        }
                    }
                    for axis in [0, 2] {
                        for n in 0..=256 {
                            let mut p = [0.0; 3];
                            p[axis] = sign * radius;
                            p[1] = half_length * (n as f32 / 128.0 - 1.0);
                            local.push(p);
                        }
                    }
                }
            }
            _ => panic!("fixture shape changed"),
        }
        let projected: Vec<_> = local
            .into_iter()
            .map(|p| {
                let rotated = orr_render::math3::quat_rotate(item.transform.rot.to_array(), p);
                let position = item.transform.pos.to_array();
                camera
                    .world_to_screen(
                        std::array::from_fn(|axis| position[axis] + rotated[axis]),
                        size,
                    )
                    .unwrap()
            })
            .collect();
        let rgba = pixels(&mut h, &evidence, name);
        let mut count = 0;
        for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
            if !is_outline_pixel(pixel) {
                continue;
            }
            let x = (index % size.0 as usize) as f32 + 0.5;
            let y = (index / size.0 as usize) as f32 + 0.5;
            assert!(
                projected
                    .iter()
                    .any(|p| (p[0] - x).powi(2) + (p[1] - y).powi(2) < 16.0),
                "wire pixel {x},{y} outside projected collider curves"
            );
            assert!(
                x > size.0 as f32 * 0.025
                    && x < size.0 as f32 * 0.975
                    && y > size.1 as f32 * 0.025
                    && y < size.1 as f32 * 0.975
            );
            count += 1;
        }
        assert!(count > 30);
        counts.push(count);
        assert_eq!(h.state().editor.selection(), selection.as_ref());
        assert_eq!(h.state().editor.checksum(), checksum);
        assert_eq!(h.state().editor.history(), &history);
        assert_eq!(
            h.state().editor.snapshot().unwrap().predicted().tick(),
            tick
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    assert!(adapter.to_lowercase().contains("llvmpipe"));
    fs::write(evidence.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"rotated_box_outline":true,"capsule_outline":true,"yellow_pixel_counts":counts,"frame_history_selection_scene_unchanged":true})).unwrap()).unwrap();
}
