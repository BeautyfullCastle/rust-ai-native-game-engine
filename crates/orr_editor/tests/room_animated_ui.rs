#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "room-animated-models"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    editor::Mode,
    game::EditorGame,
    model_bindings::{Binding, ModelKind, PlaybackMode},
    Editor, EditorApp, HostSpec,
};
use orr_package::{Lock, Project};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, RoomRun, PLAYER},
    room_project::{compiled_runtime, PreparedProject},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const ANIMATION_PACKAGE: &str = "sample-animation";
const ANIMATION_ASSET: &str = "animated.glb";

struct Fixture {
    _temp: tempfile::TempDir,
    output: PathBuf,
    other: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let output = std::env::var_os("ORR_ROOM_ANIMATED_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("animated Room project"));
        assert!(
            output.is_absolute(),
            "Room animated output must be absolute"
        );
        assert!(!output.exists(), "Room animated output must be new");
        assert!(output.parent().is_some_and(Path::is_dir));
        let other = temp.path().join("other Room project");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .expect("repository animated package fixture exists");
        orr_sample::windows_project::create_room_project(&output, "room-animated-ui")
            .expect("create initial admitted Room project");
        orr_sample::windows_project::create_room_project(&other, "room-animated-ui-other")
            .expect("create second admitted Room project");
        Self {
            _temp: temp,
            output,
            other,
            source,
        }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.output).unwrap();
        let camera = prepared
            .take_camera()
            .expect("admitted Room project declares its camera");
        let (_, path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        assert_eq!(editor.game(), EditorGame::RoomEscape);
        editor.install_room_camera(camera.document.clone()).unwrap();
        Harness::builder()
            .with_size([1800.0, 1200.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
            })
    }
}

fn hardware_vulkan_setup() -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan
                    && matches!(
                        info.device_type,
                        wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
                    )
            })
            .cloned()
            .ok_or_else(|| "No hardware Vulkan adapter is available".to_owned())
    }));
    setup.into()
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {reason}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_role(harness: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_role_and_label(role, label).click_accesskit();
    settle(harness);
}

fn set_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    let input = harness.get_by_role_and_label(Role::TextInput, label);
    input.scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        harness.get_by_label(menu).click();
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    harness.get_by_label(item).click();
    settle(harness);
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn player_guid(root: &Path) -> orr_reflect::Guid {
    let prepared = PreparedProject::open(root).unwrap();
    let scene = prepared.scene();
    let frame = scene.frame();
    let index = scene.index();
    let player = frame
        .entities()
        .find_map(|entity| {
            let actor = frame.get::<RoomActor>(entity)?;
            (actor.kind == PLAYER).then(|| index.guid(entity).cloned().unwrap())
        })
        .expect("admitted Room project contains a PLAYER");
    player
}

fn player_binding<'a>(app: &'a EditorApp, player: &orr_reflect::Guid) -> Option<&'a Binding> {
    app.models
        .bindings
        .as_ref()?
        .document()
        .bindings
        .get(&player.to_string())
}

fn lock(root: &Path) -> Lock {
    Project::open(root, compiled_runtime())
        .unwrap()
        .verify()
        .unwrap()
}

fn model_bindings_path(root: &Path) -> PathBuf {
    root.join("room.models.json")
}

fn room_run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .expect("Room editor has an authoritative snapshot")
        .predicted()
        .singleton::<RoomRun>()
}

fn paused_room_frame_at_tick_30(harness: &mut Harness<'_, EditorApp>) -> (u64, RoomRun) {
    click(harness, orr_editor::app::LBL_PLAY);
    wait_for(harness, "Room simulation start", |app| {
        app.editor.mode() == Mode::Play && app.editor.timeline().is_some()
    });
    click(harness, orr_editor::app::LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    let starting_tick = harness.state().editor.timeline().unwrap().tick;
    assert!(
        starting_tick <= 30,
        "simulation advanced beyond comparison tick"
    );
    for expected_tick in starting_tick..30 {
        click(harness, orr_editor::app::LBL_STEP);
        assert_eq!(
            harness.state().editor.timeline().unwrap().tick,
            expected_tick + 1,
            "one UI Step advances exactly one Room tick"
        );
        assert!(!harness.state().editor.sim().playing);
    }
    let snapshot = harness.state().editor.snapshot().unwrap();
    assert_eq!(snapshot.tick(), 30);
    let checksum = snapshot.predicted().checksum();
    let state = room_run(harness.state());
    (checksum, state)
}

#[test]
fn room_animated_package_ui_save_reopen_playback_and_export() {
    let acceptance_requested = [
        "ORR_REQUIRE_GPU",
        "ORR_ROOM_ANIMATED_ACCEPTANCE",
        "ORR_ROOM_ANIMATED_OUTPUT",
        "ORR_ROOM_ANIMATED_CAPTURE_DIR",
        "ORR_ROOM_ANIMATED_EXPORT_BUNDLE",
        "ORR_WINDOWS_EXPORT_RUNTIME",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !acceptance_requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_ROOM_ANIMATED_ACCEPTANCE=1 for actual Room animated-model acceptance"
        );
        return;
    }

    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "actual Room animated-model acceptance requires WGPU_BACKEND=vulkan"
    );
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("Room animated-model acceptance requires a hardware Vulkan GPU");
    assert!(
        !gpu.is_software(),
        "software GPU is not acceptance evidence"
    );
    eprintln!(
        "Room animated-model independent GPU preflight: {}",
        gpu.adapter_name()
    );
    let expected_adapter = gpu.adapter_name();
    drop(gpu);

    let fixture = Fixture::new();
    let initial_lock = lock(&fixture.output);
    let initial_lock_bytes = fs::read(fixture.output.join("orr.packages.lock.json")).unwrap();
    let initial_bindings_bytes = fs::read(model_bindings_path(&fixture.output)).unwrap();
    let initial_scene_checksum = PreparedProject::open(&fixture.output)
        .unwrap()
        .scene()
        .frame()
        .checksum();
    let player = player_guid(&fixture.output);
    let player_key = player.to_string();
    let initial_model_bindings = PreparedProject::open(&fixture.output)
        .unwrap()
        .models()
        .document
        .bindings
        .clone();
    let initial_player_binding = initial_model_bindings.get(&player_key).cloned();
    let expected_static_placements = initial_model_bindings
        .iter()
        .filter(|(guid, binding)| {
            guid.as_str() != player_key.as_str() && binding.kind == ModelKind::Static
        })
        .count();
    let mut harness = fixture.app();
    settle(&mut harness);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    assert_eq!(harness.state().editor.game(), EditorGame::RoomEscape);
    assert!(harness.state().room_camera.is_some());
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp Room viewport")
        .gpu();
    let viewport_adapter = viewport.adapter_name();
    eprintln!("Room animated-model EditorApp GPU: {viewport_adapter}");
    assert_eq!(viewport_adapter, expected_adapter);
    let viewport_size = viewport.target().size();
    let viewport_pixels = viewport.read_rgba8();
    assert!(viewport_size.0 > 0 && viewport_size.1 > 0);
    assert_eq!(
        viewport_pixels.len(),
        (viewport_size.0 * viewport_size.1 * 4) as usize
    );
    assert!(viewport_pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] > 0));

    let initial_run = room_run(harness.state());
    let (baseline_sim_checksum, baseline_run) = paused_room_frame_at_tick_30(&mut harness);
    assert_eq!(
        baseline_run, initial_run,
        "no-input Room simulation stays unchanged"
    );
    click(&mut harness, orr_editor::app::LBL_STEP);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 31);
    let baseline_step_checksum = harness
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    click(&mut harness, orr_editor::app::LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);

    menu_click(&mut harness, "File", "Content packages…");
    let source = fixture.source.canonicalize().unwrap();
    let output = fixture.output.canonicalize().unwrap();
    assert!(harness
        .query_by_label(&format!("Current Room project: {}", output.display()))
        .is_some());
    set_field(
        &mut harness,
        "Local content package directory",
        &source.display().to_string(),
    );
    click(&mut harness, "Verify installed packages");
    wait_for(&mut harness, "existing package verification", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Installed package bytes match")
    });
    assert!(harness
        .query_by_label("Use animated model sample-animation/animated.glb")
        .is_none());
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );

    click(&mut harness, "Preview package addition");
    wait_for(&mut harness, "animation package preview", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Preview ready. No package was added")
    });
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );
    assert!(!lock(&fixture.output)
        .packages
        .contains_key(ANIMATION_PACKAGE));
    click(&mut harness, "Add previewed package");
    wait_for(&mut harness, "animation package installation", |app| {
        !app.package_manager.busy() && app.package_manager.status.starts_with("Package added.")
    });
    let installed_lock = lock(&fixture.output);
    assert_eq!(
        installed_lock.packages.len(),
        initial_lock.packages.len() + 1
    );
    for (name, old_package) in &initial_lock.packages {
        assert_eq!(installed_lock.packages[name], *old_package);
    }
    let animation_package = &installed_lock.packages[ANIMATION_PACKAGE];
    assert!(animation_package
        .manifest
        .capabilities
        .contains("animation"));
    assert!(animation_package.files.contains_key(ANIMATION_ASSET));
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        initial_bindings_bytes
    );
    click(&mut harness, "Verify installed packages");
    wait_for(
        &mut harness,
        "installed animation byte verification",
        |app| {
            !app.package_manager.busy()
                && app
                    .package_manager
                    .status
                    .starts_with("Installed package bytes match")
        },
    );
    assert!(harness
        .query_by_label("Use animated model sample-animation/animated.glb")
        .is_some());

    harness
        .state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(player.clone())));
    settle(&mut harness);
    assert!(!harness
        .get_by_label("Use animated model sample-animation/animated.glb")
        .accesskit_node()
        .is_disabled());
    click(
        &mut harness,
        "Use animated model sample-animation/animated.glb",
    );
    assert!(
        harness.state().models.error().is_none(),
        "animated model request refused: {:?}; package status: {}",
        harness.state().models.error(),
        harness.state().package_manager.status
    );
    wait_for(&mut harness, "animated Room PLAYER binding", |app| {
        player_binding(app, &player).is_some_and(|binding| {
            binding.kind == ModelKind::Animated
                && binding.package == ANIMATION_PACKAGE
                && binding.asset == ANIMATION_ASSET
        })
    });
    let default_binding = player_binding(harness.state(), &player)
        .expect("verified package action assigns the selected PLAYER")
        .clone();
    assert_eq!(default_binding.package_digest, animation_package.digest);
    assert_eq!(
        default_binding.source_hash,
        animation_package.files[ANIMATION_ASSET]
    );
    assert_eq!(default_binding.animation.unwrap().clip_index, 0);
    assert_eq!(
        default_binding.animation.unwrap().playback,
        PlaybackMode::Loop
    );
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    assert!(harness.state().models.bindings.as_ref().unwrap().dirty());

    if harness.query_by_label("Animation clip").is_none() {
        click(&mut harness, "Static model binding");
    }
    let second_clip_label = {
        let placements = harness
            .state()
            .models
            .animated_placements(&harness.state().editor)
            .unwrap();
        let clip = &placements[0].model.source().clips[1];
        format!("1: {} ({:.2}s)", clip.name, clip.duration())
    };
    click_role(&mut harness, Role::ComboBox, "Animation clip");
    harness.run_steps(2);
    harness.get_by_label(&second_clip_label).click();
    settle(&mut harness);
    click(&mut harness, "Once");
    click(&mut harness, "Assign animated model");
    wait_for(&mut harness, "clip and playback reassignment", |app| {
        player_binding(app, &player).is_some_and(|binding| {
            binding.animation.is_some_and(|animation| {
                animation.clip_index == 1 && animation.playback == PlaybackMode::Once
            })
        })
    });
    let chosen_binding = player_binding(harness.state(), &player).unwrap().clone();
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    click(&mut harness, "Undo model");
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&default_binding)
    );
    click(&mut harness, "Undo model");
    assert_eq!(
        player_binding(harness.state(), &player),
        initial_player_binding.as_ref()
    );
    click(&mut harness, "Redo model");
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&default_binding)
    );
    click(&mut harness, "Redo model");
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&chosen_binding)
    );
    click(&mut harness, "Save model bindings");
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());
    let saved_binding_bytes = fs::read(model_bindings_path(&fixture.output)).unwrap();
    assert_ne!(saved_binding_bytes, initial_bindings_bytes);

    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.other.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "opening a different Room project", |app| {
        !app.windows_projects.busy()
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.other.join("room.scene.yaml"),
            )
    });
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "reopening the animated Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Opened Room project:")
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.output.join("room.scene.yaml"),
            )
            && app.editor.game() == EditorGame::RoomEscape
            && !app.editor.is_dirty()
            && app.editor.checksum() == initial_scene_checksum
            && app.models.placements(&app.editor).len() == expected_static_placements
            && app
                .models
                .animated_placements(&app.editor)
                .is_ok_and(|placements| placements.len() == 1)
            && app.room_camera.is_some()
    });
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&chosen_binding)
    );
    let reopened = PreparedProject::open(&fixture.output).unwrap();
    let reopened_lock = lock(&fixture.output);
    assert_eq!(
        reopened_lock.packages[ANIMATION_PACKAGE],
        installed_lock.packages[ANIMATION_PACKAGE]
    );
    assert_eq!(
        reopened.models().document.bindings[&player_key],
        chosen_binding
    );
    for (guid, binding) in &initial_model_bindings {
        if guid != &player_key {
            assert_eq!(
                reopened.models().document.bindings[guid],
                *binding,
                "other model bindings are preserved"
            );
        }
    }
    assert!(reopened
        .models()
        .assets
        .contains_key(&(ANIMATION_PACKAGE.into(), ANIMATION_ASSET.into())));
    assert_eq!(reopened.scene().frame().checksum(), initial_scene_checksum);
    assert!(reopened.camera().is_some());

    let rest_pose = harness
        .state()
        .models
        .animated_placements(&harness.state().editor)
        .unwrap()
        .into_iter()
        .find(|placement| {
            harness
                .state()
                .editor
                .rows()
                .iter()
                .any(|row| row.guid.as_ref() == Some(&player) && row.entity == placement.entity)
        })
        .expect("reopened PLAYER has an animated placement")
        .pose;

    let (replayed_sim_checksum, replayed_run) = paused_room_frame_at_tick_30(&mut harness);
    assert_eq!(replayed_sim_checksum, baseline_sim_checksum);
    assert_eq!(replayed_run, baseline_run);
    assert_eq!(replayed_run, initial_run);
    let snapshot = harness.state().editor.snapshot().unwrap();
    let tick = snapshot.tick();
    let tick_rate = snapshot.tick_rate();
    let time = tick as f32 / tick_rate as f32;
    let placements = harness
        .state()
        .models
        .animated_placements(&harness.state().editor)
        .unwrap();
    let animated_player = placements
        .iter()
        .find(|placement| {
            harness
                .state()
                .editor
                .rows()
                .iter()
                .any(|row| row.guid.as_ref() == Some(&player) && row.entity == placement.entity)
        })
        .expect("animated PLAYER placement follows the Room snapshot");
    let animated_player_entity = animated_player.entity;
    let animated_player_pose = animated_player.pose.clone();
    let animated_player_model = animated_player.model.clone();
    let clip_index = chosen_binding.animation.unwrap().clip_index;
    assert_ne!(
        animated_player_pose, rest_pose,
        "authoritative ticks animate the PLAYER pose"
    );
    assert_eq!(
        animated_player_pose,
        animated_player_model.sample_clip(clip_index, time).unwrap(),
        "rendered pose is sampled from the current authoritative tick"
    );
    assert_eq!(harness.state().editor.timeline().unwrap().tick, tick);
    assert!(!harness.state().editor.sim().playing);
    let tick_30_checksum = snapshot.predicted().checksum();
    let animated_tick_frame = harness
        .render()
        .expect("render the animated pose through the actual EditorApp GPU path");
    assert!(animated_tick_frame.width() >= 800 && animated_tick_frame.height() >= 600);
    click(&mut harness, orr_editor::app::LBL_STEP);
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, tick + 1);
    let stepped_snapshot = harness.state().editor.snapshot().unwrap();
    let stepped_time = stepped_snapshot.tick() as f32 / stepped_snapshot.tick_rate() as f32;
    let stepped_player = harness
        .state()
        .models
        .animated_placements(&harness.state().editor)
        .unwrap()
        .into_iter()
        .find(|placement| placement.entity == animated_player_entity)
        .expect("stepped animation remains bound to the same PLAYER");
    assert_eq!(
        stepped_player.pose,
        stepped_player
            .model
            .sample_clip(clip_index, stepped_time)
            .unwrap()
    );
    assert_eq!(
        harness
            .state()
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        baseline_step_checksum
    );
    assert_eq!(room_run(harness.state()), initial_run);
    click(&mut harness, orr_editor::app::LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);

    if let Some(runtime) = std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").map(PathBuf::from) {
        assert!(runtime.is_absolute());
        assert!(runtime.is_file());
        menu_click(&mut harness, "File", "Export Windows game…");
        set_field(
            &mut harness,
            "Runtime executable",
            &runtime.display().to_string(),
        );
        click(&mut harness, "Check runtime SHA-256");
        wait_for(&mut harness, "trusted runtime measurement", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let export_output = std::env::var_os("ORR_ROOM_ANIMATED_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("room-animated-export"));
        assert!(export_output.is_absolute());
        assert!(!export_output.exists());
        set_field(
            &mut harness,
            "Export destination",
            &export_output.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App animated Room export", |app| {
            !app.windows_export.busy()
        });
        assert!(harness.state().windows_export.last_output.is_some());
        assert!(path_matches(
            harness.state().windows_export.last_output.as_deref(),
            &export_output
        ));
        let manifest: orr_sample::windows_export::ExportManifest =
            serde_json::from_slice(&fs::read(export_output.join("orr.export.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest.payload.initial_checksum,
            format!("0x{initial_scene_checksum:016x}")
        );
        assert_eq!(
            manifest.payload.packages[ANIMATION_PACKAGE],
            installed_lock.packages[ANIMATION_PACKAGE].digest
        );
        assert_eq!(
            manifest.payload.runtime.sha256,
            harness.state().windows_export.runtime_sha256
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str())
        );
    } else {
        assert!(
            std::env::var_os("ORR_ROOM_ANIMATED_EXPORT_BUNDLE").is_none(),
            "an export destination requires ORR_WINDOWS_EXPORT_RUNTIME"
        );
    }

    if let Some(capture_root) = std::env::var_os("ORR_ROOM_ANIMATED_CAPTURE_DIR") {
        let capture_root = PathBuf::from(capture_root);
        assert!(capture_root.is_absolute());
        fs::create_dir(&capture_root).expect("Room animated capture directory must be new");
        let frame = harness
            .render()
            .expect("capture the actual hardware-rendered EditorApp UI");
        assert!(frame.width() >= 800 && frame.height() >= 600);
        frame
            .save(capture_root.join("room-animated-ui.png"))
            .unwrap();
        animated_tick_frame
            .save(capture_root.join("room-animated-play-tick30.png"))
            .unwrap();
        fs::write(
            capture_root.join("acceptance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "project": fixture.output,
                "game": "RoomEscape",
                "adapter": viewport_adapter,
                "package": ANIMATION_PACKAGE,
                "asset": ANIMATION_ASSET,
                "player": player.to_string(),
                "clip_index": clip_index,
                "playback": "once",
                "sample_tick": tick,
                "sample_tick_rate": tick_rate,
                "sim_checksum_tick_30": format!("0x{tick_30_checksum:016x}"),
                "scene_checksum": format!("0x{initial_scene_checksum:016x}"),
                "model_placements": expected_static_placements,
                "animated_placements": 1,
                "total_model_placements": expected_static_placements + 1,
                "camera_installed": true,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
