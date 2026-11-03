//! Opt-in real EditorApp/offscreen GPU consumer. This is not native-window evidence.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use crate::{game::EditorGame, navigation_panel::Endpoint};
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use std::path::{Path, PathBuf};

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    let mut previous = h.get_by_label(label).rect();
    let mut stable = 0;
    for _ in 0..90 {
        h.run_steps(1);
        let current = h.get_by_label(label).rect();
        if (current.min - previous.min).length_sq() < 0.0001
            && (current.max - previous.max).length_sq() < 0.0001
        {
            stable += 1;
        } else {
            stable = 0;
        }
        previous = current;
        if stable >= 4 {
            break;
        }
    }
    assert!(
        stable >= 4 && h.ctx.content_rect().contains_rect(previous),
        "{label} must settle on screen"
    );
    h.get_by_label(label).click();
    settle(h);
}
fn harness(scene: &Path, receipt: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 0.0, 0.0], 0.65, 0.95, 13.0);
    let receipt = receipt.to_owned();
    Harness::builder()
        .with_size([1440.0, 1200.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("ORR_REQUIRE_GPU: actual GPU harness required")
                .adapter
                .get_info();
            std::fs::write(
                receipt,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "adapter":info.name,"backend":format!("{:?}",info.backend),
                    "device_type":format!("{:?}",info.device_type),"offscreen":true,
                    "source_sha":std::env::var("NAVIGATION_ENDPOINT_SOURCE_SHA").unwrap()
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}
fn attach(h: &mut Harness<'_, EditorApp>, scene: &Path) {
    h.state_mut()
        .terrain
        .document
        .set_scene(Some(scene))
        .unwrap();
    h.state_mut()
        .terrain
        .document
        .open_local("endpoint.orrt")
        .unwrap();
    settle(h);
}
fn pointer(h: &mut Harness<'_, EditorApp>, point: egui::Pos2) {
    for pressed in [true, false] {
        h.event(egui::Event::PointerMoved(point));
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
    }
    settle(h);
}
fn pick(h: &mut Harness<'_, EditorApp>, label: &str, endpoint: Endpoint, world: [f32; 3]) {
    click(h, label);
    assert_eq!(h.state().navigation_endpoint.armed(), Some(endpoint));
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let size = app.ui.viewport_px;
    let pixel = app
        .editor
        .presentation_camera3d(size)
        .unwrap()
        .world_to_screen(world, size)
        .unwrap();
    let point = rect.min
        + egui::vec2(
            pixel[0] * rect.width() / size.0 as f32,
            pixel[1] * rect.height() / size.1 as f32,
        );
    assert!(rect.shrink(12.0).contains(point));
    pointer(h, point);
    assert_eq!(
        h.state().navigation_endpoint.armed(),
        None,
        "{}",
        h.state().navigation_endpoint.status
    );
}
fn frame(h: &mut Harness<'_, EditorApp>) -> orr_ecs::Frame {
    settle(h);
    let snapshot = h.state().editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("endpoint-consumer-authority")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(received) = client.local_frames.pop_front() {
            if received.frame.tick() == tick && received.frame.checksum() == checksum {
                return (*received.frame).clone();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "authoritative Frame matching view timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn capture(h: &mut Harness<'_, EditorApp>, dir: &Path, phase: &str) {
    settle(h);
    let image = h.render().expect("required GPU composition");
    image.save(dir.join(format!("{phase}-app.png"))).unwrap();
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("production viewport GPU")
        .gpu();
    let pixels = gpu.read_rgba8();
    let size = gpu.target().size();
    let mut encoder = png::Encoder::new(
        std::fs::File::create(dir.join(format!("{phase}-viewport.png"))).unwrap(),
        size.0,
        size.1,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    let frame = frame(h);
    let bytes = frame.to_bytes();
    std::fs::write(dir.join(format!("{phase}.frame")), &bytes).unwrap();
    std::fs::write(dir.join(format!("{phase}.json")),serde_json::to_vec_pretty(&serde_json::json!({
        "phase":phase,"tick":frame.tick(),"frame_sha256":digest(&bytes),"viewport_sha256":digest(&pixels),
        "start":h.state().navigation.start,"goal":h.state().navigation.goal,
        "pick_status":h.state().navigation_endpoint.status,"host_history_entries":h.state().editor.history().entries.len(),
        "terrain_can_undo":h.state().terrain.document.can_undo(),"source_sha":std::env::var("NAVIGATION_ENDPOINT_SOURCE_SHA").unwrap()
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "requires ORR_REQUIRE_GPU=1, fresh NAVIGATION_ENDPOINT_CAPTURE_DIR and NAVIGATION_ENDPOINT_SOURCE_SHA; run explicitly --ignored --exact"]
fn actual_viewport_endpoint_picks_build_save_reopen_and_step() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "GPU acceptance cannot silently skip"
    );
    let source = std::env::var("NAVIGATION_ENDPOINT_SOURCE_SHA").expect("source SHA required");
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    let evidence = PathBuf::from(
        std::env::var_os("NAVIGATION_ENDPOINT_CAPTURE_DIR")
            .expect("fresh evidence directory required"),
    );
    if evidence.exists() {
        assert_eq!(
            std::fs::read_dir(&evidence).unwrap().count(),
            0,
            "evidence directory must be empty"
        );
    } else {
        std::fs::create_dir_all(&evidence).unwrap();
    }
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let terrain = orr_terrain::Terrain::new(
        "endpoint/diagnostic".into(),
        9,
        9,
        [FP::from_int(-4); 2],
        FP::ONE,
        vec![FP::ZERO; 81],
        vec![false; 64],
    )
    .unwrap()
    .cook();
    let terrain_path = root.path().join("endpoint.orrt");
    std::fs::write(&terrain_path, &terrain).unwrap();
    let scene_before = std::fs::read(&scene).unwrap();
    let mut h = harness(&scene, &evidence.join("adapter.json"));
    attach(&mut h, &scene);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    let before = frame(&mut h).to_bytes();
    let history = h.state().editor.history().clone();
    capture(&mut h, &evidence, "01-before");
    // Intentionally off-center interior hits. Expected snapped centers below are
    // literal independent values, not derived by the production picking helper.
    pick(
        &mut h,
        "Pick navigation start",
        Endpoint::Start,
        [-2.8, 0.0, -0.7],
    );
    pick(
        &mut h,
        "Pick navigation goal",
        Endpoint::Goal,
        [2.2, 0.0, 0.3],
    );
    assert_eq!(h.state().navigation.start, ["-2.5", "-0.5"]);
    assert_eq!(h.state().navigation.goal, ["2.5", "0.5"]);
    assert_eq!(frame(&mut h).to_bytes(), before);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().terrain.document.bytes().unwrap(), terrain);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(std::fs::read(&scene).unwrap(), scene_before);
    assert_eq!(std::fs::read(&terrain_path).unwrap(), terrain);
    capture(&mut h, &evidence, "02-picked-draft");
    // An armed geometric miss is owned by the endpoint tool, never an edit.
    click(&mut h, "Pick navigation goal");
    let selected = h.state().editor.selection().cloned();
    let vertex = h.state().terrain.selected_vertex;
    let cell = h.state().terrain.selected_cell;
    let miss = h.state().ui.viewport_rect.unwrap().left_top() + egui::vec2(3.0, 3.0);
    pointer(&mut h, miss);
    assert_eq!(h.state().navigation.goal, ["2.5", "0.5"]);
    assert_eq!(h.state().editor.selection(), selected.as_ref());
    assert_eq!(h.state().terrain.selected_vertex, vertex);
    assert_eq!(h.state().terrain.selected_cell, cell);
    assert_eq!(h.state().terrain.document.bytes().unwrap(), terrain);
    assert_eq!(frame(&mut h).to_bytes(), before);
    assert_eq!(h.state().editor.history(), &history);
    click(&mut h, "Build route");
    assert!(
        h.state().editor.admitted_navigation().admitted,
        "{:?}",
        h.state().navigation.error
    );
    assert_eq!(
        h.state().editor.history().entries.len(),
        history.entries.len() + 1
    );
    let built = frame(&mut h).to_bytes();
    // Undo/Redo live in the Edit menu; use its real global shortcuts.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    settle(&mut h);
    assert_eq!(frame(&mut h).to_bytes(), before);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Y);
    settle(&mut h);
    assert_eq!(frame(&mut h).to_bytes(), built);
    assert_eq!(
        h.state().editor.history().entries.len(),
        history.entries.len() + 1
    );
    capture(&mut h, &evidence, "03-built-redone");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    settle(&mut h);
    assert_ne!(
        std::fs::read(&scene).unwrap(),
        scene_before,
        "Save shortcut must persist scene pin"
    );
    assert_eq!(std::fs::read(&terrain_path).unwrap(), terrain);
    drop(h);
    let mut h = harness(&scene, &evidence.join("reopen-adapter.json"));
    attach(&mut h, &scene);
    assert_eq!(
        frame(&mut h).to_bytes(),
        built,
        "fresh host reload must restore exact authoritative edit Frame"
    );
    assert_eq!(h.state().navigation.start, ["-2.5", "-0.5"]);
    assert_eq!(h.state().navigation.goal, ["2.5", "0.5"]);
    click(&mut h, LBL_PLAY);
    click(&mut h, LBL_PAUSE);
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(!h.state().editor.timeline().unwrap().playing);
    let initial = frame(&mut h);
    let initial_nav =
        orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&initial))
            .unwrap();
    assert_eq!(initial.tick(), 0);
    assert_eq!(
        initial_nav.navigator.position().map(FP::raw),
        [-163840, 0, -32768]
    );
    let path = initial_nav.navigator.path().unwrap();
    assert_eq!(
        path.waypoints().first().unwrap().map(FP::raw),
        [-163840, 0, -32768]
    );
    assert_eq!(
        path.waypoints().last().unwrap().map(FP::raw),
        [163840, 0, 32768]
    );
    click(&mut h, LBL_STEP);
    let first = frame(&mut h);
    let first_nav =
        orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&first))
            .unwrap();
    assert_eq!(first.tick(), 1);
    assert_ne!(
        first_nav.navigator.position(),
        initial_nav.navigator.position()
    );
    assert_eq!(first_nav.navigator.position()[1], FP::ZERO);
    assert_eq!(
        first_nav.navigator.status(),
        orr_navigation::NavigationStatus::Moving
    );
    capture(&mut h, &evidence, "04-paused-step");
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(frame(&mut h).to_bytes(), initial.to_bytes());
    click(&mut h, LBL_STEP);
    assert_eq!(
        frame(&mut h).to_bytes(),
        first.to_bytes(),
        "same paused widget Step replays deterministically"
    );
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(frame(&mut h).to_bytes(), built);
    let mut files = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&evidence).unwrap() {
        let path = entry.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        files.insert(
            path.file_name().unwrap().to_string_lossy().into_owned(),
            serde_json::json!({"bytes":bytes.len(),"sha256":digest(&bytes)}),
        );
    }
    assert_eq!(
        files.len(),
        18,
        "four app/viewport/Frame/receipt captures and two adapter receipts"
    );
    std::fs::write(evidence.join("complete.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "complete":true,"source_sha":source,"files":files,"actual_pointer_picks":2,"owned_invalid_clicks":1,
        "build_history_delta":1,"undo_redo":true,"save_fresh_reopen":true,"deterministic_paused_step":true,"stop_restores_edit_frame":true,
        "expected_start_raw":[-163840,-32768],"expected_goal_raw":[163840,32768],
        "scope":"Actual offscreen EditorApp GPU widgets and viewport pointer input; not native-window evidence. Picks only fill drafts and do not promise connectivity; Build is authoritative."
    })).unwrap()).unwrap();
}
