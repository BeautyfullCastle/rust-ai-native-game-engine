#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_fp::{fp, FP};
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::{decode_asset, mesh_from_view, AgentSpec};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const BLANK_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "MeshNavigation EditorApp GPU: {} (software: {})",
                rhi.adapter_name(),
                rhi.is_software()
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but MeshNavigation GPU is unavailable: {error}"
            );
            eprintln!("SKIP: MeshNavigation EditorApp GPU unavailable: {error}");
            false
        }
    }
}

fn setup(scene: &Path) -> Harness<'static, EditorApp> {
    std::fs::write(scene, BLANK_SCENE).unwrap();
    let mut editor = Editor::open_game(scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |creation_context| {
            EditorApp::new(editor, creation_context.wgpu_render_state.clone())
        })
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, label: &str) {
    harness.get_by_label(menu).click();
    harness.run_steps(2);
    harness.get_by_label(label).click();
    settle(harness);
}

fn type_into(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    settle(harness);
}

fn wait_for_playing_frame(harness: &mut Harness<'_, EditorApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
        if harness
            .state()
            .editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("MeshNavigation host did not publish a playing Frame within three seconds");
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness.state().editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle(harness);
    let viewport_rect = harness
        .state()
        .ui
        .viewport_rect
        .expect("production 3D viewport has a layout rectangle");
    assert!(
        viewport_rect.width() >= 400.0,
        "MeshNavigation inspector must leave at least 400 points for the viewport"
    );
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production MeshNavigation 3D viewport rendered");
    let size = viewport.gpu().target().size();
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    if let Some(directory) = std::env::var_os("ORR_MESH_EDITOR_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let image =
            egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
        orr_editor::app::write_png(&directory.join(format!("{name}.png")), &image).unwrap();
        let frame = harness
            .render()
            .expect("full EditorApp window framebuffer is rendered");
        let window_image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width() as usize, frame.height() as usize],
            frame.as_raw(),
        );
        orr_editor::app::write_png(&directory.join(format!("window-{name}.png")), &window_image)
            .unwrap();
    }
    pixels
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(first, second)| {
            first[..3]
                .iter()
                .zip(second[..3].iter())
                .any(|(first_channel, second_channel)| first_channel.abs_diff(*second_channel) > 12)
        })
        .count()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_native_receipt(scene_bytes: &[u8], workflow: serde_json::Value) {
    let Some(directory) = std::env::var_os("ORR_MESH_EDITOR_CAPTURE_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("production.scene.yaml"), scene_bytes).unwrap();
    let scene_sha256 = hex_bytes(&Sha256::digest(scene_bytes));
    let mut receipt = workflow
        .as_object()
        .expect("native receipt workflow fields are a JSON object")
        .clone();
    receipt.insert(
        "scene_file".into(),
        serde_json::json!("production.scene.yaml"),
    );
    receipt.insert("scene_sha256".into(), serde_json::json!(scene_sha256));
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

#[test]
fn production_editor_authors_saves_reopens_and_runs_a_self_contained_mesh_scene() {
    if !gpu_available() {
        return;
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let original_scene = root.join("mesh-navigation.scene.yaml");
    let copied_scene = root.join("saved-copy.scene.yaml");
    let mesh_asset = root.join("ramp.orrmesh");
    let hidden_mesh_asset = root.join("ramp.orrmesh.hidden");
    let mut harness = setup(&original_scene);
    harness.run_steps(4);
    assert!(harness.state().has_gpu());

    click(&mut harness, "Static-mesh navigation");
    let baseline = viewport_pixels(&mut harness, "baseline");
    click(&mut harness, "Create ramp and bridge");
    harness.state_mut().mesh_navigation.mesh_output_path = mesh_asset.display().to_string();
    click(&mut harness, "Save mesh asset");
    assert!(mesh_asset.is_file());

    type_into(&mut harness, "Distance per tick", "1");
    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "1");

    let saved_graph = decode_asset(&std::fs::read(&mesh_asset).unwrap()).unwrap();
    type_into(&mut harness, "Start X", "1.5");
    type_into(&mut harness, "Start Y", "0");
    type_into(&mut harness, "Start Z", "1");
    type_into(&mut harness, "Goal X", "3");
    type_into(&mut harness, "Goal Y", "2");
    type_into(&mut harness, "Goal Z", "1");
    click(&mut harness, "Place mesh agent");
    let placed_view = mesh_view(&harness);
    assert_eq!(placed_view.graph.revision(), saved_graph.revision());
    assert_eq!(
        placed_view.agent.spec,
        AgentSpec {
            start: [fp!(1.5), FP::ZERO, fp!(1)],
            goal: [fp!(3), fp!(2), fp!(1)],
            max_vertical: fp!(2),
            distance_per_tick: fp!(1),
        }
    );
    assert_eq!(placed_view.agent.status, NavigationStatus::Moving);
    assert_eq!(harness.state().editor.history().entries.len(), 1);
    let placed_pixels = viewport_pixels(&mut harness, "placed");
    assert!(changed_pixel_count(&baseline, &placed_pixels) > 100);

    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert!(harness.state().mesh_navigation.model().is_none());
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert!(harness.state().mesh_navigation.model().is_some());
    assert_eq!(mesh_view(&harness).graph.revision(), saved_graph.revision());

    assert!(harness.state_mut().editor.save_as(&copied_scene));
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene = std::fs::read(&copied_scene).unwrap();
    assert!(!saved_scene.is_empty());
    std::fs::rename(&mesh_asset, &hidden_mesh_asset).unwrap();
    assert!(!mesh_asset.exists());
    drop(harness);

    let mut editor = Editor::open_game(&copied_scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let saved_checksum = editor.checksum();
    let mut harness = Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |creation_context| {
            EditorApp::new(editor, creation_context.wgpu_render_state.clone())
        });
    harness.run_steps(4);
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.checksum(), saved_checksum);
    let reopened_view = mesh_view(&harness);
    assert_eq!(reopened_view.graph.revision(), saved_graph.revision());
    assert_eq!(reopened_view.agent.spec, placed_view.agent.spec);
    let reopened_pixels = viewport_pixels(&mut harness, "reopened-source-hidden");
    assert!(changed_pixel_count(&placed_pixels, &reopened_pixels) < 20);

    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    wait_for_playing_frame(&mut harness);
    click(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.timeline().unwrap().playing);
    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    let paused_tick = harness.state().editor.timeline().unwrap().tick;
    assert_eq!(paused_tick, 0);
    let start_position = mesh_view(&harness).agent.position;
    let play_start_pixels = viewport_pixels(&mut harness, "play-start");
    click(&mut harness, LBL_STEP);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        paused_tick + 1
    );
    assert_ne!(mesh_view(&harness).agent.position, start_position);
    harness.state_mut().editor.step(32);
    settle(&mut harness);
    let arrived_view = mesh_view(&harness);
    assert_eq!(arrived_view.agent.status, NavigationStatus::Arrived);
    let arrived_tick = harness.state().editor.timeline().unwrap().tick;
    let arrival_steps = u32::try_from(arrived_tick).expect("test tick count fits a host step");
    let arrived_checksum = harness.state().editor.checksum();
    let playing_pixels = viewport_pixels(&mut harness, "play-arrived");
    assert!(changed_pixel_count(&play_start_pixels, &playing_pixels) > 20);
    assert_eq!(std::fs::read(&copied_scene).unwrap(), saved_scene);

    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    harness.state_mut().editor.step(arrival_steps);
    settle(&mut harness);
    let replayed_view = mesh_view(&harness);
    assert_eq!(replayed_view.agent.status, NavigationStatus::Arrived);
    assert_eq!(replayed_view.agent.position, arrived_view.agent.position);
    let replay_checksum = harness.state().editor.checksum();
    let replay_tick = harness.state().editor.timeline().unwrap().tick;
    assert_eq!(replay_tick, arrived_tick);
    assert_eq!(replay_checksum, arrived_checksum);
    write_native_receipt(
        &saved_scene,
        serde_json::json!({
            "graph_revision": hex_bytes(&replayed_view.graph.revision()),
            "spec": {
                "start_raw": replayed_view.agent.spec.start.map(FP::raw),
                "goal_raw": replayed_view.agent.spec.goal.map(FP::raw),
                "max_vertical_raw": replayed_view.agent.spec.max_vertical.raw(),
                "distance_per_tick_raw": replayed_view.agent.spec.distance_per_tick.raw(),
            },
            "arrival_tick": arrived_tick,
            "arrival_checksum": format!("0x{arrived_checksum:016x}"),
            "replay_tick": replay_tick,
            "replay_checksum": format!("0x{replay_checksum:016x}"),
            "edit_checksum": format!("0x{saved_checksum:016x}"),
        }),
    );
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
}
