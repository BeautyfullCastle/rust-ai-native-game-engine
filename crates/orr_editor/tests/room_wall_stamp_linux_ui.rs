#![cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "room-authoring"
))]
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
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
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
        .with_size([1600.0, 1150.0])
        .with_step_dt(1.0 / 60.0)
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
    // Match render-target dimensions, not merely window dimensions: hierarchy
    // widths grow with labels/history and otherwise differ in a fresh app.
    let target = (900, 850);
    for _ in 0..6 {
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
    let viewport = h.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    assert_eq!(size, target, "stable GPU viewport dimensions");
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

fn named(h: &Harness<'_, EditorApp>, name: &str) -> BTreeSet<orr_reflect::Guid> {
    h.state()
        .editor
        .rows()
        .iter()
        .filter(|row| row.name.as_deref() == Some(name))
        .filter_map(|row| row.guid.clone())
        .collect()
}

fn disabled(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn select_sources(h: &mut Harness<'_, EditorApp>, sources: &[(String, orr_reflect::Guid)]) {
    click(h, "Read Room walls");
    for (name, guid) in sources {
        h.get_by_role_and_label(Role::CheckBox, &format!("{name} ({guid})"))
            .scroll_to_me();
        h.run_steps(20);
        h.get_by_role_and_label(Role::CheckBox, &format!("{name} ({guid})"))
            .click_accesskit();
        settle(h);
    }
}

fn assert_geometry(
    h: &mut Harness<'_, EditorApp>,
    guid: &orr_reflect::Guid,
    original: &[orr_reflect::Value; 3],
    offset: [i32; 2],
) {
    assert_eq!(component(h, guid, "RoomEscapeV1::Actor"), original[0]);
    assert_eq!(component(h, guid, "orr_physics3d::Collider"), original[2]);
    let mut expected = original[1].clone();
    let orr_reflect::Value::Struct(fields) = &mut expected else {
        panic!("Body struct")
    };
    let (_, position) = fields.iter_mut().find(|(name, _)| name == "pos").unwrap();
    let orr_reflect::Value::Vec3(position) = position else {
        panic!("Body.pos Vec3")
    };
    position.x += orr_fp::FP::from_int(offset[0]);
    position.z += orr_fp::FP::from_int(offset[1]);
    assert_eq!(component(h, guid, "orr_physics3d::Body"), expected);
}

// Real production input routing, rather than mutating physics or player position.
fn drive_right(h: &mut Harness<'_, EditorApp>) -> ([f32; 3], [f32; 3]) {
    click(h, LBL_PLAY);
    click(h, "Take control");
    let start = player_position(h);
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(h);
    (start, player_position(h))
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_WALL_STAMPS_LINUX_DIR"]
fn linux_room_wall_groups_ui_geometry_history_reopen_and_collision() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_STAMPS_LINUX_DIR")
            .expect("fresh retained evidence directory"),
    );
    assert!(evidence.is_absolute() && !evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Room fixture");
    fixture(&root);
    let preserved = [
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ]
    .map(|name| (name, fs::read(root.join(name)).unwrap()));
    let stamp = evidence.join("two walls 東京.room-stamp.json");
    let hidden = evidence.join("source-hidden.room-stamp.json");
    let mut h = harness(&root);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let initial_rows = h.state().editor.rows().len();
    menu_click(&mut h, "File", "Room level authoring…");
    let mut sources = Vec::new();
    for (name, z) in [("stamp_source_a", "5"), ("stamp_source_b", "7")] {
        for (label, value) in [
            ("Room wall name", name),
            ("Wall center X", "5"),
            ("Wall center Z", z),
            ("Wall half-width X", "0.15"),
            ("Wall half-depth Z", "1"),
        ] {
            field(&mut h, label, value);
        }
        click(&mut h, "Create Room wall");
        let guids = named(&h, name);
        assert_eq!(guids.len(), 1);
        sources.push((name.to_owned(), guids.into_iter().next().unwrap()));
    }
    // The authoring panel is outside this test's group workflow; close it to expose File.
    h.state_mut().room_authoring.open = false;
    settle(&mut h);
    menu_click(&mut h, "File", "Save Ctrl+S");
    assert!(!h.state().editor.is_dirty());
    let source_scene = fs::read(root.join("room.scene.yaml")).unwrap();
    let source_checksum = h.state().editor.checksum();
    let original: Vec<_> = sources
        .iter()
        .map(|(_, guid)| {
            [
                component(&mut h, guid, "RoomEscapeV1::Actor"),
                component(&mut h, guid, "orr_physics3d::Body"),
                component(&mut h, guid, "orr_physics3d::Collider"),
            ]
        })
        .collect();

    let (control_start, control_end) = drive_right(&mut h);
    assert!(
        control_end[0] > 2.5,
        "original route must cross future wall: {control_end:?}"
    );
    shot(&mut h, &evidence, "01-original-route-unblocked.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), source_checksum);
    let baseline_pixels = pixels(&mut h, &evidence, "02-source-walls-viewport.png");

    menu_click(&mut h, "File", "Room wall groups…");
    select_sources(&mut h, &sources);
    assert!(!disabled(&h, "Capture selected walls"));
    // Any document edit invalidates the capture selection, including undo/redo.
    menu_click(&mut h, "Edit", "Undo Ctrl+Z");
    assert!(disabled(&h, "Capture selected walls"));
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert!(disabled(&h, "Capture selected walls"));
    select_sources(&mut h, &sources);
    click(&mut h, "Capture selected walls");
    assert!(h
        .query_by_label("Captured Room wall stamp: 2 wall(s); save explicitly.")
        .is_some());
    field(&mut h, "Room wall stamp file", &stamp.display().to_string());
    let clean_history = h.state().editor.history().clone();
    click(&mut h, "Save Room wall stamp");
    let stamp_bytes = fs::read(&stamp).unwrap();
    click(&mut h, "Load Room wall stamp");
    assert!(h
        .query_by_label("Loaded Room wall stamp: 2 wall(s).")
        .is_some());
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(h.state().editor.history(), &clean_history);
    shot(&mut h, &evidence, "03-captured-saved-loaded.png");

    // Editing offsets is staging only. Invalid text cannot begin a transaction.
    field(&mut h, "Stamp offset X", "not-a-number");
    assert!(disabled(&h, "Instantiate Room wall stamp"));
    assert_eq!(h.state().editor.history(), &clean_history);
    field(&mut h, "Stamp offset X", "20");
    click(&mut h, "Instantiate Room wall stamp");
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(h.state().editor.history(), &clean_history);
    assert_eq!(h.state().editor.rows().len(), initial_rows + 2);
    assert_eq!(
        fs::read(root.join("room.scene.yaml")).unwrap(),
        source_scene
    );
    shot(&mut h, &evidence, "04-rejected-out-of-bounds.png");

    // Failed loads clear the in-memory template; stale data must not be instantiated.
    let invalid = evidence.join("invalid.room-stamp.json");
    fs::write(&invalid, b"{\"schema\":999,\"walls\":[]}").unwrap();
    field(
        &mut h,
        "Room wall stamp file",
        &invalid.display().to_string(),
    );
    click(&mut h, "Load Room wall stamp");
    assert!(disabled(&h, "Instantiate Room wall stamp"));
    field(&mut h, "Room wall stamp file", &stamp.display().to_string());
    click(&mut h, "Load Room wall stamp");
    // Loaded templates are reusable snapshots. Removing the file is not scene staleness.
    fs::rename(&stamp, &hidden).unwrap();
    click(&mut h, "Save Room wall stamp");
    assert!(h
        .query_by_label("Room wall stamp was removed by another editor")
        .is_some());
    assert_eq!(fs::read(&hidden).unwrap(), stamp_bytes);
    assert!(!stamp.exists());
    assert_eq!(h.state().editor.history(), &clean_history);

    field(&mut h, "Stamp offset X", "-3");
    field(&mut h, "Stamp offset Z", "-5");
    click(&mut h, "Instantiate Room wall stamp");
    let first_checksum = h.state().editor.checksum();
    assert_ne!(first_checksum, source_checksum);
    assert_eq!(
        h.state().editor.history().entries.len(),
        clean_history.entries.len() + 1
    );
    let mut first_guids = BTreeSet::new();
    for ((name, source), geometry) in sources.iter().zip(&original) {
        let group = named(&h, name);
        assert_eq!(group.len(), 2);
        let guid = group.iter().find(|guid| *guid != source).unwrap();
        assert_geometry(&mut h, guid, geometry, [-3, -5]);
        assert!(first_guids.insert(guid.clone()));
    }
    menu_click(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(h.state().editor.rows().len(), initial_rows + 2);
    menu_click(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), first_checksum);
    for ((name, source), geometry) in sources.iter().zip(&original) {
        for guid in named(&h, name).iter().filter(|guid| *guid != source) {
            assert!(first_guids.contains(guid));
            assert_geometry(&mut h, guid, geometry, [-3, -5]);
        }
    }
    shot(&mut h, &evidence, "05-instantiated-undo-redo.png");
    field(&mut h, "Stamp offset X", "-1");
    field(&mut h, "Stamp offset Z", "-5");
    click(&mut h, "Instantiate Room wall stamp");
    assert_eq!(
        h.state().editor.history().entries.len(),
        clean_history.entries.len() + 2
    );
    let mut all_guids = BTreeSet::new();
    for ((name, source), geometry) in sources.iter().zip(&original) {
        let group = named(&h, name);
        assert_eq!(group.len(), 3);
        for guid in &group {
            assert!(all_guids.insert(guid.clone()));
            let offset = if guid == source {
                [0, 0]
            } else if first_guids.contains(guid) {
                [-3, -5]
            } else {
                [-1, -5]
            };
            assert_geometry(&mut h, guid, geometry, offset);
        }
    }
    assert_eq!(all_guids.len(), 6);
    let final_checksum = h.state().editor.checksum();
    menu_click(&mut h, "File", "Save Ctrl+S");
    assert!(!h.state().editor.is_dirty());
    let saved_scene = fs::read(root.join("room.scene.yaml")).unwrap();
    h.state_mut().editor.select(None);
    let saved_pixels = pixels(&mut h, &evidence, "06-saved-expanded-viewport.png");
    assert_eq!(baseline_pixels.len(), saved_pixels.len());
    assert!(
        baseline_pixels != saved_pixels,
        "new geometry must change actual viewport rendering"
    );
    drop(h);

    // A brand-new EditorApp/host admits the expanded scene with no stamp source.
    let mut h = harness(&root);
    settle(&mut h);
    assert!(!stamp.exists());
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert_eq!(h.state().editor.rows().len(), initial_rows + 6);
    assert_eq!(fs::read(root.join("room.scene.yaml")).unwrap(), saved_scene);
    for ((name, source), geometry) in sources.iter().zip(&original) {
        let group = named(&h, name);
        assert_eq!(group.len(), 3);
        for guid in &group {
            assert!(all_guids.contains(guid));
            let offset = if guid == source {
                [0, 0]
            } else if first_guids.contains(guid) {
                [-3, -5]
            } else {
                [-1, -5]
            };
            assert_geometry(&mut h, guid, geometry, offset);
        }
    }
    let reopened_pixels = pixels(&mut h, &evidence, "07-fresh-source-hidden-viewport.png");
    assert!(
        saved_pixels == reopened_pixels,
        "fresh reopen preserves visible ordinary actors at identical viewport dimensions"
    );
    let (blocked_start, blocked_end) = drive_right(&mut h);
    assert_eq!(blocked_start, control_start);
    assert!(
        blocked_end[0] > blocked_start[0] + 0.05 && blocked_end[0] < 1.7,
        "new wall must block route crossed by control: {blocked_start:?} -> {blocked_end:?}"
    );
    shot(&mut h, &evidence, "08-instanced-wall-blocks-gameplay.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), final_checksum);
    // The ordinary Open scene dialog has a real Cancel action. Cancelling preserves
    // a current capture list; reopening identical bytes invalidates it by load epoch.
    menu_click(&mut h, "File", "Room wall groups…");
    select_sources(&mut h, &sources);
    let before_cancel = h.state().editor.history().clone();
    menu_click(&mut h, "File", "Open scene…");
    click(&mut h, "Cancel");
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert_eq!(h.state().editor.history(), &before_cancel);
    assert!(!disabled(&h, "Capture selected walls"));
    menu_click(&mut h, "File", "Open scene…");
    field(
        &mut h,
        "Scene path",
        &root.join("room.scene.yaml").display().to_string(),
    );
    click(&mut h, "OK");
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert!(disabled(&h, "Capture selected walls"));
    select_sources(&mut h, &sources);
    let alternate = evidence.join("same-content.scene.yaml");
    fs::write(&alternate, &saved_scene).unwrap();
    menu_click(&mut h, "File", "Open scene…");
    field(&mut h, "Scene path", &alternate.display().to_string());
    click(&mut h, "OK");
    assert_eq!(
        h.state().editor.path().as_deref(),
        Some(alternate.as_path())
    );
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert!(disabled(&h, "Capture selected walls"));
    shot(&mut h, &evidence, "09-path-epoch-stale-selection.png");
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
    assert!(
        adapter.to_lowercase().contains("llvmpipe"),
        "expected software Vulkan: {adapter}"
    );
    use sha2::{Digest, Sha256};
    let mut captures = Vec::new();
    for entry in fs::read_dir(&evidence).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|v| v.to_str()) == Some("png") {
            let bytes = fs::read(&path).unwrap();
            captures.push(
                serde_json::json!({"name":path.file_name().unwrap().to_str().unwrap(),
                "bytes":bytes.len(),"sha256":Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>()}),
            );
        }
    }
    captures.sort_by_key(|value| value["name"].as_str().unwrap().to_owned());
    fs::write(evidence.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"platform":"linux-x86_64","production_editor_app":true,
        "file_menu_wall_groups":true,"software_gpu":true,"adapter":adapter,
        "physical_os_input":false,"ui_input":"egui pointer/accessibility/keyboard events",
        "exact_collision_geometry_and_fresh_guids":true,"one_history_entry_per_instance":true,
        "checksum_stale_capture_rejected":true,"path_epoch_stale_capture_rejected":true,
        "open_scene_cancel_preserves_selection_and_history":true,"out_of_bounds_transaction_rolled_back":true,
        "invalid_load_clears_template":true,"removed_source_save_rejected":true,
        "loaded_snapshot_reusable_without_source":true,"repeated_instantiation":true,
        "undo_redo_save_fresh_reopen":true,"source_hidden_during_reopen":true,
        "visual_proof":{"viewport_changed":true,"reopen_pixels_equal":true},
        "gameplay_proof":{"control_start":control_start,"control_end":control_end,
            "blocked_start":blocked_start,"blocked_end":blocked_end,"input":"D via Take control"},
        "scene_checksum":format!("0x{final_checksum:016x}"),"captures":captures,
        "cancel_control":"Open scene dialog Cancel; wall groups panel itself has no cancel operation"
    })).unwrap()).unwrap();
}
