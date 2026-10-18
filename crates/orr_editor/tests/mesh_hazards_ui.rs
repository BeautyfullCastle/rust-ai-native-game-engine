#![cfg(feature = "mesh-hazards")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

//! Opt-in production EditorApp acceptance with synthetic egui input and software Vulkan.
//! This is not hardware-GPU or physical operating-system input acceptance.
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationStatus};
use orr_remote::mesh_navigation::{
    authoring::create_mesh_scene,
    hazards::{self, HazardsView},
    mesh_from_view,
    mission::{self, MeshMission, MissionPoint, MISSION_NAME},
    AgentSpec,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn fixture(path: &Path) {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [fp!(4), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [fp!(4), FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(1), FP::ZERO, fp!(2)],
            goal: [fp!(1), FP::ZERO, fp!(3)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(1),
        },
    )
    .unwrap();
    let mut config = MeshMission {
        enabled: 1,
        count: 1,
        graph_revision: graph.revision(),
        exit: MissionPoint {
            position: [fp!(1), FP::ZERO, fp!(3)],
            radius: fp!(0.25),
        },
        ..Default::default()
    };
    config.points[0] = config.exit;
    let value = document
        .types()
        .get(MISSION_NAME)
        .unwrap()
        .read(orr_reflect::bytemuck::bytes_of(&config));
    document
        .apply(
            orr_edit::Op::SetSingletonField {
                singleton: MISSION_NAME.into(),
                path: String::new(),
                value,
            },
            orr_edit::Origin::User,
        )
        .unwrap();
    fs::write(path, document.save_yaml()).unwrap();
}

fn harness(path: &Path) -> Harness<'static, EditorApp> {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("software Vulkan preflight");
    assert!(
        gpu.is_software(),
        "software GPU required for this acceptance"
    );
    eprintln!("Preflight software adapter: {}", gpu.adapter_name());
    let mut editor = Editor::open_game(path, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".to_owned())
    }));
    Harness::builder()
        .with_size([1500.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |context| EditorApp::new(editor, context.wgpu_render_state.clone()))
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(20);
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn menu(h: &mut Harness<'_, EditorApp>, name: &str, item: &str) {
    h.get_by_label(name).click_accesskit();
    h.run_steps(2);
    h.get_by_label(item).click_accesskit();
    settle(h);
}

fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(20);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}

fn view(h: &Harness<'_, EditorApp>) -> Option<HazardsView> {
    hazards::from_view(h.state().editor.snapshot().unwrap().predicted()).unwrap()
}

fn capture(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle(h);
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("real production 3D viewport");
    assert!(
        viewport
            .gpu()
            .adapter_name()
            .to_lowercase()
            .contains("llvmpipe"),
        "this acceptance explicitly requires software GPU"
    );
    eprintln!("software GPU: {}", viewport.gpu().adapter_name());
    let image = h.render().expect("production EditorApp composite");
    let rgba = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    orr_editor::app::write_png(&root.join(name), &rgba).unwrap();
}

#[test]
#[ignore = "opt-in software Vulkan EditorApp acceptance; set ORR_MESH_HAZARDS_CAPTURE_DIR"]
fn hazards_widgets_apply_undo_save_reopen_swept_loss_and_stop_reset() {
    assert_eq!(
        std::env::var("ORR_MESH_HAZARDS_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("ORR_MESH_HAZARDS_CAPTURE_DIR").unwrap());
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("fresh evidence directory");
    let scene = root.join("hazards.scene.yaml");
    fixture(&scene);
    let original = fs::read(&scene).unwrap();
    let mut h = harness(&scene);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let initial = h.state().editor.checksum();
    assert!(view(&h).is_none());
    click(&mut h, "Mesh hazards");
    click(&mut h, "Enable mesh hazards");
    for (label, value) in [
        ("Hazard count (1–8)", "1"),
        ("Hazard 1 X", "1"),
        ("Hazard 1 Y", "0"),
        ("Hazard 1 Z", "2.5"),
        ("Hazard 1 radius", "0"),
    ] {
        field(&mut h, label, value);
    }
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Apply mesh hazards");
    assert_eq!(h.state().editor.checksum(), initial);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(fs::read(&scene).unwrap(), original);
    assert!(h.query_by_label_contains("Apply hazards failed:").is_some());
    assert!(h
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    capture(&mut h, &root, "invalid-radius-warning.png");
    field(&mut h, "Hazard 1 radius", "0.1");
    click(&mut h, "Apply mesh hazards");
    let applied = h.state().editor.checksum();
    assert_ne!(applied, initial);
    let config = view(&h).unwrap().config;
    assert_eq!(config.count, 1);
    assert_eq!(config.spheres[0].center, [fp!(1), FP::ZERO, fp!(2.5)]);
    assert_eq!(config.spheres[0].radius, fp!(0.1));
    assert_eq!(view(&h).unwrap().run.lost, 0);
    menu(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(h.state().editor.checksum(), initial);
    assert!(view(&h).is_none());
    menu(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(h.state().editor.checksum(), applied);
    assert_eq!(view(&h).unwrap().config, config);
    menu(&mut h, "File", "Save Ctrl+S");
    let saved = fs::read(&scene).unwrap();
    assert_ne!(saved, original);
    capture(&mut h, &root, "authored-hazard.png");
    drop(h);

    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), applied);
    assert_eq!(view(&h).unwrap().config, config);
    // Exercise the production Open dialog as well as a fresh local host instance in this test process.
    menu(&mut h, "File", "Open scene…");
    field(&mut h, "Scene path", &scene.display().to_string());
    click(&mut h, "OK");
    assert_eq!(h.state().editor.game(), EditorGame::MeshHazardsV1);
    assert_eq!(h.state().editor.checksum(), applied);
    assert_eq!(view(&h).unwrap().config, config);
    click(&mut h, "Mesh hazards");
    capture(&mut h, &root, "reopened-hazard.png");
    let mesh = mesh_from_view(h.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(mesh.pin.start, [fp!(1), FP::ZERO, fp!(2)]);
    assert_eq!(mesh.pin.goal, [fp!(1), FP::ZERO, fp!(3)]);
    assert_eq!(mesh.pin.distance_per_tick, fp!(1));
    assert_eq!(mesh.agent.path.points, vec![mesh.pin.start, mesh.pin.goal]);
    assert_eq!(
        mesh.graph
            .movement_polyline(&mesh.agent.path, FP::ZERO, mesh.pin.distance_per_tick)
            .unwrap(),
        vec![mesh.pin.start, mesh.pin.goal]
    );
    // The one-tick path crosses the sphere; neither route endpoint is inside it.
    assert!((config.spheres[0].center[2] - mesh.pin.start[2]).abs() > config.spheres[0].radius);
    assert!((config.spheres[0].center[2] - mesh.pin.goal[2]).abs() > config.spheres[0].radius);
    click(&mut h, LBL_PLAY);
    eprintln!(
        "After Play: mode={:?} timeline={:?} status={:?}",
        h.state().editor.mode(),
        h.state().editor.timeline(),
        h.state().editor.status()
    );
    capture(&mut h, &root, "after-play-diagnostic.png");
    assert_eq!(
        h.state().editor.mode(),
        Mode::Play,
        "Play widget must enter Play"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while view(&h).unwrap().run.lost == 0 {
        assert!(
            Instant::now() < deadline,
            "swept hazard loss did not occur: timeline={:?}, status={:?}, position={:?}",
            h.state().editor.timeline(),
            h.state().editor.status(),
            mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
                .unwrap()
                .agent
                .position
        );
        settle(&mut h);
        std::thread::sleep(Duration::from_millis(10));
    }
    let lost = view(&h).unwrap().run;
    assert_eq!(lost.hit_index, 0);
    assert_eq!(lost.lost_tick, 1);
    let snapshot = h.state().editor.snapshot().unwrap();
    assert_eq!(
        mesh_from_view(snapshot.predicted()).unwrap().agent.status,
        NavigationStatus::Stopped
    );
    assert_eq!(
        mesh_from_view(snapshot.predicted()).unwrap().agent.position,
        mesh.pin.start
    );
    assert_eq!(
        mission::from_view(snapshot.predicted())
            .unwrap()
            .unwrap()
            .run
            .won,
        0
    );
    assert!(h
        .query_all_by_label_contains("Lost · hazard 1 · tick 1")
        .next()
        .is_some());
    capture(&mut h, &root, "swept-hazard-loss.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), applied);
    assert_eq!(view(&h).unwrap().run, Default::default());
    assert_eq!(view(&h).unwrap().config, config);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    capture(&mut h, &root, "stop-reset.png");
    fs::write(
        root.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "profile": "MeshHazardsV1", "software_gpu": true, "hardware_gpu": false,
            "physical_os_input": false, "production_editor_app": true,
            "invalid_radius_rejected_atomically": true, "widget_apply_undo_redo_save_reopen": true,
            "swept_loss_tick": lost.lost_tick, "endpoint_overlap": false,
            "mission_win_suppressed": true, "stop_restores_authored_checksum": true,
            "scene_checksum": applied,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn assert_disabled(h: &Harness<'_, EditorApp>, label: &str) {
    assert!(
        h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "{label} must be disabled"
    );
}

#[test]
#[ignore = "opt-in software Vulkan EditorApp creator acceptance; set ORR_MESH_HAZARDS_CAPTURE_DIR"]
fn hazard_creator_widgets_validate_create_preserve_and_reopen() {
    assert_eq!(
        std::env::var("ORR_MESH_HAZARDS_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let base = PathBuf::from(std::env::var_os("ORR_MESH_HAZARDS_CAPTURE_DIR").unwrap());
    assert!(base.is_absolute());
    let root = base.with_file_name(format!(
        "{}-creator",
        base.file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir(&root).expect("fresh creator evidence directory");
    let original_path = root.join("original.scene.yaml");
    fixture(&original_path);
    let original_bytes = fs::read(&original_path).unwrap();
    let mut h = harness(&original_path);
    settle(&mut h);
    let original_checksum = h.state().editor.checksum();
    let destination = root.join("created.scene.yaml");

    menu(&mut h, "File", "New hazard game…");
    assert_disabled(&h, "Create hazard game");
    // Attempt an accessibility action as well: a modal must guard the transport itself.
    if let Some(play) = h.query_by_role_and_label(Role::Button, LBL_PLAY) {
        play.click_accesskit();
    }
    settle(&mut h);
    capture(&mut h, &root, "creator-modal-play-guard.png");
    assert_eq!(
        h.state().editor.mode(),
        Mode::Edit,
        "creator modal must block Play action"
    );
    field(
        &mut h,
        "New hazard game scene path",
        &destination.display().to_string(),
    );
    click(&mut h, "Cancel new hazard game");
    assert!(!destination.exists());
    assert_eq!(h.state().editor.checksum(), original_checksum);
    assert_eq!(
        h.state().editor.path().as_deref(),
        Some(original_path.as_path())
    );

    menu(&mut h, "File", "New hazard game…");
    field(&mut h, "Game floor width", "invalid");
    assert_disabled(&h, "Create hazard game");
    field(&mut h, "Game floor width", "1");
    click(&mut h, "Create hazard game");
    assert!(!destination.exists());
    assert_eq!(h.state().editor.checksum(), original_checksum);
    assert!(h
        .query_all_by_label_contains("Current workspace retained;")
        .next()
        .is_some());
    capture(&mut h, &root, "creator-invalid-input.png");
    field(&mut h, "Game floor width", "6");
    field(&mut h, "Game floor depth", "4");
    field(&mut h, "Game collectible count", "1");
    field(&mut h, "Game distance per tick", "0.125");
    assert_eq!(
        h.get_by_label("Include starter hazard sphere")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    capture(&mut h, &root, "creator-ready-default-off.png");
    click(&mut h, "Create hazard game");
    assert!(h.query_by_label("Create hazard game").is_none());
    assert_eq!(h.state().editor.game(), EditorGame::MeshHazardsV1);
    assert_eq!(
        h.state().editor.path().as_deref(),
        Some(destination.as_path())
    );
    assert!(view(&h).is_none(), "starter hazards must default off");
    assert!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
    let created_checksum = h.state().editor.checksum();
    let created_bytes = fs::read(&destination).unwrap();
    click(&mut h, "Mesh hazards");
    capture(&mut h, &root, "creator-default-off-created.png");

    // Existing destination must be rejected without replacing scene or active host.
    menu(&mut h, "File", "New hazard game…");
    field(
        &mut h,
        "New hazard game scene path",
        &destination.display().to_string(),
    );
    click(&mut h, "Create hazard game");
    assert!(h
        .query_all_by_label_contains("Current workspace retained;")
        .next()
        .is_some());
    assert_eq!(h.state().editor.checksum(), created_checksum);
    assert_eq!(
        h.state().editor.path().as_deref(),
        Some(destination.as_path())
    );
    assert_eq!(fs::read(&destination).unwrap(), created_bytes);
    capture(&mut h, &root, "creator-overwrite-refused.png");
    click(&mut h, "Cancel new hazard game");

    // Dirty drafts guard both Play and another creator before any file mutation.
    click(&mut h, "Enable mesh hazards");
    field(&mut h, "Hazard 1 X", "2");
    field(&mut h, "Hazard 1 Y", "0");
    field(&mut h, "Hazard 1 Z", "2");
    field(&mut h, "Hazard 1 radius", "0.25");
    assert_disabled(&h, LBL_PLAY);
    click(&mut h, "File");
    assert_disabled(&h, "New hazard game…");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), created_checksum);
    assert_eq!(fs::read(&destination).unwrap(), created_bytes);
    click(&mut h, "Apply mesh hazards");
    let authored = view(&h).unwrap().config;
    assert_eq!(authored.spheres[0].center, [fp!(2), FP::ZERO, fp!(2)]);
    assert_eq!(authored.spheres[0].radius, fp!(0.25));
    click(&mut h, "Mesh hazards");
    click(&mut h, "Mesh mission");
    let original_mission = mission::from_view(h.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    field(&mut h, "Collectible 1 radius", "0.25");
    field(&mut h, "Mission exit radius", "0.75");
    assert_disabled(&h, LBL_PLAY);
    click(&mut h, "Apply mesh mission");
    let authored_mission = mission::from_view(h.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    assert_eq!(authored_mission.points[0].radius, fp!(0.25));
    assert_eq!(authored_mission.exit.radius, fp!(0.75));
    menu(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        original_mission
    );
    menu(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        authored_mission
    );
    assert_eq!(view(&h).unwrap().config, authored);
    capture(&mut h, &root, "creator-mission-numeric-edited.png");
    menu(&mut h, "File", "Save Ctrl+S");
    let authored_checksum = h.state().editor.checksum();
    let authored_bytes = fs::read(&destination).unwrap();
    menu(&mut h, "File", "Open scene…");
    field(&mut h, "Scene path", &destination.display().to_string());
    click(&mut h, "OK");
    assert_eq!(h.state().editor.game(), EditorGame::MeshHazardsV1);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(view(&h).unwrap().config, authored);
    assert_eq!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        authored_mission
    );
    capture(&mut h, &root, "creator-authored-reopened.png");

    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    click(&mut h, "File");
    assert_disabled(&h, "New hazard game…");
    h.key_press(Key::Escape);
    settle(&mut h);
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(fs::read(&destination).unwrap(), authored_bytes);

    // A second small scene exercises the explicit optional starter sphere.
    let with_starter = root.join("with-starter.scene.yaml");
    menu(&mut h, "File", "New hazard game…");
    field(
        &mut h,
        "New hazard game scene path",
        &with_starter.display().to_string(),
    );
    click(&mut h, "Include starter hazard sphere");
    click(&mut h, "Create hazard game");
    assert_eq!(h.state().editor.game(), EditorGame::MeshHazardsV1);
    assert_eq!(
        h.state().editor.path().as_deref(),
        Some(with_starter.as_path())
    );
    assert_eq!(view(&h).unwrap().config.count, 1);
    assert_eq!(view(&h).unwrap().run, Default::default());
    assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
    assert_eq!(fs::read(&destination).unwrap(), authored_bytes);
    click(&mut h, "Mesh hazards");
    capture(&mut h, &root, "creator-optional-starter.png");
    drop(h);
    let mut h = harness(&destination);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(view(&h).unwrap().config, authored);
    fs::write(
        root.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "profile":"MeshHazardsV1", "production_editor_app":true,
            "software_gpu":true, "hardware_gpu":false, "physical_os_input":false,
            "creator_cancel_untouched":true, "invalid_inputs_rejected":true,
            "default_hazards_off":true, "optional_starter_sphere":true,
            "original_scene_unchanged":true, "overwrite_refused":true,
            "draft_play_modal_guards":true, "authored_save_reopen":true,
        "numeric_mission_edit_undo_redo_save_reopen":true,
            "authored_checksum":authored_checksum,
        }))
        .unwrap(),
    )
    .unwrap();
}


#[test]
#[ignore = "opt-in software Vulkan hazard viewport picking; set ORR_HAZARD_PICK_CAPTURE_DIR"]
fn hazard_center_viewport_pick_stages_only_selected_sphere_and_apply_undo_save_reopen() {
    use egui::{Event, PointerButton};
    assert_eq!(
        std::env::var("ORR_MESH_HAZARDS_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("ORR_HAZARD_PICK_CAPTURE_DIR").unwrap());
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("fresh hazard picking evidence directory");
    let scene = root.join("hazard-picking.scene.yaml");
    fixture(&scene);
    let mut h = harness(&scene);
    settle(&mut h);
    // Mesh scenes admit only their derived agent, so select that real frame entity.
    let selected = orr_editor::Target::Entity(mesh_from_view(h.state().editor.snapshot().unwrap().predicted()).unwrap().agent.entity);
    h.state_mut().editor.select(Some(selected.clone()));
    settle(&mut h);
    assert_eq!(h.state().editor.selection(), Some(&selected));

    click(&mut h, "Mesh hazards");
    click(&mut h, "Enable mesh hazards");
    field(&mut h, "Hazard count (1–8)", "2");
    for (label, value) in [
        ("Hazard 1 X", "0.5"),
        ("Hazard 1 Y", "0"),
        ("Hazard 1 Z", "0.5"),
        ("Hazard 1 radius", "0.2"),
        ("Hazard 2 X", "0.75"),
        ("Hazard 2 Y", "0"),
        ("Hazard 2 Z", "0.75"),
        ("Hazard 2 radius", "0.3"),
    ] {
        field(&mut h, label, value);
    }
    click(&mut h, "Apply mesh hazards");
    menu(&mut h, "File", "Save Ctrl+S");
    let baseline = view(&h).unwrap().config;
    let checksum = h.state().editor.checksum();
    let saved = fs::read(&scene).unwrap();
    let mission_before = mission::from_view(h.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    let pin_before = orr_reflect::bytemuck::bytes_of(
        &mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .pin,
    )
    .to_vec();
    let history_before = h.state().editor.history().entries.len();
    let selection_before = h.state().editor.selection().cloned();
    assert!(selection_before.is_some(), "Escape regression requires a real selected mesh agent");

    click(&mut h, "Pick hazard 2 center in viewport");
    assert!(h.query_by_label("Cancel hazard center picking").is_some());
    assert_disabled(&h, LBL_PLAY);
    h.get_by_label("File").click_accesskit();
    h.run_steps(2);
    assert_disabled(&h, "Save Ctrl+S");
    assert_disabled(&h, "Open scene…");
    #[cfg(feature = "mesh-hazards-export")]
    assert_disabled(&h, "Export hazard game…");
    capture(&mut h, &root, "01-picking-guards.png");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Cancel hazard center picking").is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.selection().cloned(), selection_before);
    assert_eq!(fs::read(&scene).unwrap(), saved);

    // Focus loss cancels an armed pick without touching the host or its selection.
    click(&mut h, "Pick hazard 2 center in viewport");
    h.input_mut().focused = false;
    h.event(Event::WindowFocused(false));
    settle(&mut h);
    h.input_mut().focused = true;
    h.event(Event::WindowFocused(true));
    settle(&mut h);
    assert!(h.query_by_label("Cancel hazard center picking").is_none());
    assert_eq!(h.state().editor.checksum(), checksum);

    click(&mut h, "Pick hazard 2 center in viewport");
    // A real primary-pointer event hits the admitted floor through the camera.
    let rect = h.state().ui.viewport_rect.unwrap();
    let size = h.state().ui.viewport_px;
    let camera = h.state().editor.presentation_camera3d(size).unwrap();
    let pixel = camera.world_to_screen([1.0, 0.0, 2.5], size).unwrap();
    let pointer = rect.min + egui::vec2(pixel[0], pixel[1]) * (rect.width() / size.0 as f32);
    assert!(rect.contains(pointer));
    h.event(Event::PointerMoved(pointer));
    h.run_steps(2);
    h.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert!(h.query_by_label("Cancel hazard center picking").is_none());
    assert_eq!(
        h.state().editor.checksum(),
        checksum,
        "picking only stages form values"
    );
    assert_eq!(h.state().editor.history().entries.len(), history_before);
    assert_eq!(
        h.state().editor.selection().cloned(),
        selection_before,
        "pick must not fall through into generic selection"
    );
    assert_eq!(view(&h).unwrap().config, baseline);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    assert_disabled(&h, LBL_PLAY);
    capture(&mut h, &root, "02-picked-draft.png");
    click(&mut h, "Apply mesh hazards");
    let applied = view(&h).unwrap().config;
    assert_eq!(applied.spheres[0], baseline.spheres[0]);
    assert_eq!(applied.spheres[1].radius, baseline.spheres[1].radius);
    assert!((applied.spheres[1].center[0] - fp!(1)).abs() < fp!(0.01));
    assert_eq!(applied.spheres[1].center[1], FP::ZERO);
    assert!((applied.spheres[1].center[2] - fp!(2.5)).abs() < fp!(0.01));
    assert_eq!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        mission_before
    );
    assert_eq!(
        orr_reflect::bytemuck::bytes_of(
            &mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
                .unwrap()
                .pin
        ),
        pin_before
    );
    let applied_checksum = h.state().editor.checksum();
    assert_ne!(applied_checksum, checksum);
    menu(&mut h, "Edit", "Undo Ctrl+Z");
    assert_eq!(view(&h).unwrap().config, baseline);
    menu(&mut h, "Edit", "Redo Ctrl+Y");
    assert_eq!(view(&h).unwrap().config, applied);
    menu(&mut h, "File", "Save Ctrl+S");
    capture(&mut h, &root, "03-picked-applied.png");
    drop(h);
    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), applied_checksum);
    assert_eq!(view(&h).unwrap().config, applied);
    click(&mut h, "Mesh hazards");
    capture(&mut h, &root, "04-picked-reopened.png");
    let authored_bytes = fs::read(&scene).unwrap();
    let mesh = mesh_from_view(h.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(mesh.pin.start, [fp!(1), FP::ZERO, fp!(2)]);
    assert_eq!(mesh.pin.goal, [fp!(1), FP::ZERO, fp!(3)]);
    click(&mut h, LBL_PLAY);
    let deadline = Instant::now() + Duration::from_secs(15);
    while view(&h).unwrap().run.lost == 0 {
        assert!(
            Instant::now() < deadline,
            "viewport-authored route sphere did not cause loss"
        );
        settle(&mut h);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(view(&h).unwrap().run.hit_index, 1);
    assert_eq!(view(&h).unwrap().run.lost_tick, 1);
    assert_eq!(
        mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .agent
            .status,
        NavigationStatus::Stopped
    );
    assert_eq!(
        mission::from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .run
            .won,
        0
    );
    capture(&mut h, &root, "05-picked-route-loss.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), applied_checksum);
    assert_eq!(view(&h).unwrap().config, applied);
    assert_eq!(view(&h).unwrap().run, Default::default());
    assert_eq!(fs::read(&scene).unwrap(), authored_bytes);
    capture(&mut h, &root, "06-picked-stop-reset.png");

    fs::write(root.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"profile":"MeshHazardsV1","production_editor_app":true,"software_gpu":true,"hardware_gpu":false,"physical_os_input":false,
        "viewport_pointer_pick":true,"selected_sphere_only":true,"radius_preserved":true,"staging_does_not_mutate_host":true,
        "play_save_open_guards":true,"escape_cancel_preserves_selection":true,"focus_loss_cancels":true,
        "no_selection_fallthrough":true,"mission_and_agent_unchanged":true,"apply_undo_redo_save_reopen":true,"viewport_authored_sphere_2_loss_tick_1":true,"stop_restores_authored_state":true,
        "source_checksum":checksum,"applied_checksum":applied_checksum,
    })).unwrap()).unwrap();
}
