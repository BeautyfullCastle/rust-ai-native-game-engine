//! Actual app acceptance: dedicated ignored GPU test is mandatory in the issue's
//! producer batch, not silently satisfied by generic CPU CI.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use crate::game::EditorGame;
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain};
use std::path::{Path, PathBuf};

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    // ScrollIntoView animates. A pointer click queues coordinates, so wait for
    // the actual target to stop moving rather than clicking its old position.
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
        stable >= 4,
        "widget {label} did not settle after ScrollIntoView"
    );
    assert!(
        h.ctx.content_rect().contains_rect(previous),
        "widget {label} must be on screen before its physical click: {previous:?}"
    );
    h.get_by_label(label).click();
    settle(h);
}
fn bytes(h: &Harness<'_, EditorApp>) -> Vec<u8> {
    h.state().terrain.document.bytes().unwrap().to_vec()
}
fn fixture(root: &Path, navigation: bool) -> PathBuf {
    let scene = root.join("scene.yaml");
    std::fs::write(
        &scene,
        if navigation {
            include_str!("../../../../scenes/navigation_blank.scene.yaml")
        } else {
            include_str!("../../../../scenes/yard3d_authoring.scene.yaml")
        },
    )
    .unwrap();
    // Deliberately nonflat: the finished corridor has y=(x+3)/4 at z=0.
    // Endpoints (-3,0,0) and (3,1.5,0) cannot pass a wrong-axis/flat oracle.
    let mut heights = vec![FP::ZERO; 81];
    heights[4 * 9 + 7] = FP::from_raw(98304);
    // The +X-owned goal cell lies beyond the finite ramp. An existing exterior
    // landing keeps that cell walkable without changing the ramp footprint.
    for z in 3..=5 {
        heights[z * 9 + 8] = FP::from_raw(98304);
    }
    let t = Terrain::new(
        "ramp/diagnostic".into(),
        9,
        9,
        [FP::from_int(-4); 2],
        FP::ONE,
        heights,
        vec![false; 64],
    )
    .unwrap();
    std::fs::write(root.join("ramp.orrt"), t.cook()).unwrap();
    scene
}
fn open(scene: &Path, game: EditorGame) -> Editor {
    let mut editor = Editor::open_game(scene, game).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 0.5, 0.0], 0.65, 0.95, 13.0);
    editor
}
fn attach(h: &mut Harness<'_, EditorApp>, scene: &Path) {
    let a = h.state_mut();
    a.terrain.document.set_scene(Some(scene)).unwrap();
    a.terrain.document.open_local("ramp.orrt").unwrap();
    settle(h);
}
fn arm_diagnostic(h: &Harness<'_, EditorApp>, label: &str, phase: &str) -> serde_json::Value {
    let a = h.state();
    serde_json::json!({"phase":phase,"label":label,"mode":format!("{:?}",a.editor.mode()),"authoring_active":a.terrain.authoring_active(&a.editor),"rows_coherent":a.editor.yard_rows_coherent(),"can_mutate":a.editor.can_mutate(),"scene_load_epoch":a.editor.scene_load_epoch(),"host_epoch":a.editor.sim().epoch,"scene_path":a.editor.path(),"document_scene":a.terrain.document.scene(),"scene_matches":a.terrain.scene_matches(&a.editor),"read_only":a.terrain.document.read_only(),"focused":h.ctx.input(|input|input.focused),"snapshot":a.editor.snapshot().map(|snapshot|serde_json::json!({"tick":snapshot.predicted().tick(),"checksum":format!("{:016x}",snapshot.predicted().checksum()),"has_timeline":snapshot.timeline().is_some()})),"status":a.terrain_ramp.status,"endpoints":a.terrain_ramp.endpoints,"selected_vertex":a.terrain.selected_vertex,"selected_cell":a.terrain.selected_cell,"terrain_mode":format!("{:?}",a.terrain.selection_mode),"terrain_error":a.terrain.error(),"log":a.editor.log().iter().map(|m|serde_json::json!({"error":m.error,"text":m.text})).collect::<Vec<_>>()})
}
fn wait_for_stopped_terrain(h: &mut Harness<'_, EditorApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    eprintln!(
        "ramp-stop {}",
        arm_diagnostic(h, "post-Stop", "before-readiness")
    );
    loop {
        h.state_mut().editor.sync();
        h.run_steps(1);
        let app = h.state();
        let ready = app.editor.mode() == Mode::Edit
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_none())
            && app.terrain.authoring_active(&app.editor);
        if ready {
            eprintln!("ramp-stop {}", arm_diagnostic(h, "post-Stop", "ready"));
            return;
        }
        assert!(
            app.editor.down().is_none() && std::time::Instant::now() < deadline,
            "coherent Edit terrain did not become ready after Stop: {}",
            arm_diagnostic(h, "post-Stop", "refused")
        );
    }
}
fn prepare(h: &mut Harness<'_, EditorApp>) {
    // CPU guard tests use the production arm/pick adapter, not private pin injection.
    for (label, vertex) in [("Pick ramp start", [1, 4]), ("Pick ramp end", [7, 4])] {
        eprintln!("ramp-arm {}", arm_diagnostic(h, label, "before-arm-click"));
        click(h, label);
        let diagnostic = arm_diagnostic(h, label, "after-arm-click");
        eprintln!("ramp-arm {diagnostic}");
        let a = h.state_mut();
        let size = (800, 600);
        let camera = a.editor.camera3d.camera();
        let world = a
            .terrain
            .document
            .terrain()
            .unwrap()
            .vertex_position(vertex[1] * 9 + vertex[0])
            .unwrap()
            .map(FP::to_f32);
        let pixel = camera.world_to_screen(world, size).unwrap();
        assert!(
            a.terrain_ramp_pick(&camera, pixel, size),
            "arm/pick refused; pre-pick state: {diagnostic}; post-pick status: {}",
            a.terrain_ramp.status
        );
    }
    h.state_mut().terrain_ramp_stage().unwrap();
}

#[test]
fn app_context_refusals_preserve_document_history_and_original_files() {
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path(), false);
    let editor = open(&scene, EditorGame::Yard3D);
    let mut h = Harness::builder()
        .with_size([1440.0, 1100.0])
        .build_eframe(move |_| EditorApp::new(editor, None));
    attach(&mut h, &scene);
    let original = bytes(&h);
    let history = h.state().editor.history().clone();
    // Test synchronous Apply guards without giving frame sync a chance to clear.
    for case in 0..5 {
        h.state_mut().terrain_ramp.half_width = 1;
        h.state_mut().terrain.selected_vertex = [0, 0];
        h.state_mut().terrain.selected_cell = [0, 0];
        prepare(&mut h);
        match case {
            0 => h.state_mut().terrain.selected_vertex = [2, 2],
            1 => h.state_mut().terrain.selected_cell = [2, 2],
            2 => h.state_mut().terrain_ramp.half_width = 2,
            3 => {
                h.state_mut().terrain.selection_mode =
                    crate::terrain_pick::TerrainSelectionMode::Cell
            }
            _ => h.state_mut().terrain_ramp.endpoints[0] = Some([2, 4]),
        }
        assert!(h.state_mut().terrain_ramp_apply().is_err(), "case {case}");
        assert_eq!(bytes(&h), original);
        assert!(!h.state().terrain.document.can_undo());
        assert_eq!(h.state().editor.history(), &history);
    }
    prepare(&mut h);
    // Native egui-winit supplies both RawInput.focused and the focus event.
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.run_steps(2);
    assert!(!h.ctx.input(|input| input.focused));
    assert!(h.state().terrain_ramp.staged.is_none());
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    h.run_steps(2);
    assert!(h.ctx.input(|input| input.focused));
    prepare(&mut h);
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    let changed = bytes(&h);
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), changed);
    assert!(h.state_mut().terrain.document.undo().unwrap());
    assert_eq!(bytes(&h), original);
    prepare(&mut h);
    h.state_mut()
        .terrain
        .document
        .set_render_admission_error(Some("injected admission failure".into()));
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    assert_eq!(
        std::fs::read(root.path().join("ramp.orrt")).unwrap(),
        original
    );
    h.state_mut()
        .terrain
        .document
        .set_render_admission_error(None);
    prepare(&mut h);
    std::fs::write(root.path().join("other.orrt"), &original).unwrap();
    h.state_mut()
        .terrain
        .document
        .open_local("other.orrt")
        .unwrap();
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    h.state_mut()
        .terrain
        .document
        .open_local("ramp.orrt")
        .unwrap();
    prepare(&mut h);
    let old_epoch = h.state().editor.scene_load_epoch();
    assert!(h.state_mut().editor.open_path(&scene));
    h.state_mut().editor.sync();
    assert_ne!(
        h.state().editor.scene_load_epoch(),
        old_epoch,
        "same-scene reload changes the editor scene-load epoch"
    );
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    prepare(&mut h);
    assert!(h.state_mut().editor.select_named("box_left"));
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    h.state_mut().editor.select(None);
    prepare(&mut h);
    h.state_mut().editor.play();
    h.state_mut().editor.sync();
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    h.state_mut().editor.stop();
    wait_for_stopped_terrain(&mut h);
    prepare(&mut h);
    let other_scene = root.path().join("other.scene.yaml");
    std::fs::write(
        &other_scene,
        include_str!("../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    assert!(h.state_mut().editor.open_path(&other_scene));
    h.state_mut().editor.sync();
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert_eq!(bytes(&h), original);
    assert!(h.state_mut().editor.open_path(&scene));
    h.state_mut().editor.sync();
    prepare(&mut h);
    h.state_mut().terrain_ramp_apply().unwrap();
    let ramp = bytes(&h);
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(h.state_mut().terrain.document.undo().unwrap());
    prepare(&mut h);
    assert_eq!(
        h.state()
            .terrain_ramp
            .staged
            .as_ref()
            .unwrap()
            .candidate
            .changed_vertices(),
        0
    );
    h.state_mut().terrain_ramp_apply().unwrap();
    assert_eq!(bytes(&h), ramp);
    assert!(
        h.state_mut().terrain.document.redo().unwrap(),
        "no-op adapter retains existing redo branch"
    );
    h.state_mut().terrain.document.discard();
    h.state_mut()
        .terrain
        .document
        .open_local("ramp.orrt")
        .unwrap();
    prepare(&mut h);
    let terrain = h.state().terrain.document.terrain().unwrap().clone();
    let project = root.path().join("package-project");
    let installed = orr_terrain_view::package::install_and_reload(
        &terrain,
        &project,
        &root.path().join("package-source"),
        "1.0.0",
    )
    .unwrap();
    let package_file = project
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join(orr_terrain_view::package::ASSET_PATH);
    let package_before = std::fs::read(&package_file).unwrap();
    h.state_mut()
        .terrain
        .document
        .open_package(
            &project,
            orr_terrain_view::package::PACKAGE_NAME,
            orr_terrain_view::package::ASSET_PATH,
        )
        .unwrap();
    assert!(h.state().terrain.document.read_only());
    assert!(h.state_mut().terrain_ramp_apply().is_err());
    assert!(h.state_mut().terrain_ramp_stage().is_err());
    assert_eq!(bytes(&h), original);
    assert_eq!(std::fs::read(package_file).unwrap(), package_before);
}

#[cfg(feature = "navigation")]
fn gpu_harness(scene: &Path, receipt: &Path) -> Harness<'static, EditorApp> {
    let editor = open(scene, EditorGame::NavigationYard3D);
    let receipt = receipt.to_owned();
    Harness::builder().with_size([1440.0,1200.0]).with_step_dt(1.0/60.0).wgpu().build_eframe(move |cc| {
        let info=cc.wgpu_render_state.as_ref().expect("producer GPU required").adapter.get_info();
        let data=serde_json::json!({"phase":"adapter-created","adapter":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"source_sha":std::env::var("TERRAIN_RAMP_SOURCE_SHA").ok()});
        std::fs::write(receipt,serde_json::to_vec_pretty(&data).unwrap()).unwrap();
        assert_eq!(info.backend,egui_wgpu::wgpu::Backend::Vulkan,"exact software Vulkan required");
        assert_eq!(info.device_type,egui_wgpu::wgpu::DeviceType::Cpu,"hardware results do not satisfy software receipt");
        EditorApp::new(editor,cc.wgpu_render_state.clone())
    })
}
#[cfg(feature = "navigation")]
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
#[cfg(feature = "navigation")]
fn authoritative_frame(h: &mut Harness<'_, EditorApp>) -> orr_ecs::Frame {
    use std::time::{Duration, Instant};
    settle(h);
    let snapshot = h.state().editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("terrain-ramp-authority-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(received) = client.local_frames.pop_front() {
            if received.frame.tick() == tick && received.frame.checksum() == checksum {
                return (*received.frame).clone();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative Frame deadline"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[cfg(feature = "navigation")]
fn capture(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str) -> Vec<u8> {
    settle(h);
    let composed = h.render().expect("composed EditorApp capture");
    composed.save(dir.join(format!("{name}-app.png"))).unwrap();
    std::fs::write(dir.join(format!("{name}-app.rgba")), composed.as_raw()).unwrap();
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("production viewport")
        .gpu();
    let pixels = gpu.read_rgba8();
    let size = gpu.target().size();
    let adapter = gpu.adapter_name();
    std::fs::write(dir.join(format!("{name}.rgba")), &pixels).unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut png = png::Encoder::new(file, size.0, size.1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    let frame = authoritative_frame(h);
    let frame_bytes = frame.to_bytes();
    std::fs::write(dir.join(format!("{name}.frame")), &frame_bytes).unwrap();
    let nav =
        orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&frame)).ok();
    let snapshot = h.state().editor.snapshot().unwrap();
    let receipt = serde_json::json!({"phase":name,"partial":true,"mode":format!("{:?}",h.state().editor.mode()),"tick":frame.tick(),"checksum":format!("{:016x}",frame.checksum()),"display_tick":snapshot.predicted().tick(),"display_checksum":format!("{:016x}",snapshot.predicted().checksum()),"terrain_revision":h.state().terrain.document.revision(),"terrain_bytes_sha256":h.state().terrain.document.bytes().map(digest),"terrain_can_undo":h.state().terrain.document.can_undo(),"terrain_can_redo":h.state().terrain.document.can_redo(),"terrain_dirty":h.state().terrain.document.dirty(),"terrain_error":h.state().terrain.error(),"position_raw":nav.as_ref().map(|n|n.navigator.position().map(FP::raw)),"status":nav.as_ref().map(|n|format!("{:?}",n.navigator.status())),"next_waypoint":nav.as_ref().map(|n|n.navigator.next_waypoint()),"path":nav.as_ref().and_then(|n|n.navigator.path()).map(|p|serde_json::json!({"corridor":p.corridor().iter().map(|k|k.0).collect::<Vec<_>>(),"waypoints_raw":p.waypoints().iter().map(|p|p.map(FP::raw)).collect::<Vec<_>>()})),"diagnostics":h.state().editor.log().iter().map(|m|serde_json::json!({"error":m.error,"text":m.text})).collect::<Vec<_>>(),"navigation_error":h.state().navigation.error,"route_stale":h.state().navigation.route_stale(&h.state().editor,&h.state().terrain),"viewport":size,"composed_size":composed.dimensions(),"adapter":adapter,"frame_sha256":digest(&frame_bytes),"rgba_sha256":digest(&pixels),"composed_rgba_sha256":digest(composed.as_raw()),"source_sha":std::env::var("TERRAIN_RAMP_SOURCE_SHA").expect("producer source SHA required")});
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(frame.tick(), snapshot.predicted().tick());
    assert_eq!(frame.checksum(), snapshot.predicted().checksum());
    pixels
}
#[cfg(feature = "navigation")]
fn pick(h: &mut Harness<'_, EditorApp>, label: &str, vertex: [u32; 2]) {
    click(h, label);
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let px = app.ui.viewport_px;
    let camera = app.editor.presentation_camera3d(px).unwrap();
    let world = app
        .terrain
        .document
        .terrain()
        .unwrap()
        .vertex_position(vertex[1] * 9 + vertex[0])
        .unwrap()
        .map(FP::to_f32);
    let screen = camera.world_to_screen(world, px).unwrap();
    let point = rect.min
        + egui::vec2(
            screen[0] * rect.width() / px.0 as f32,
            screen[1] * rect.height() / px.1 as f32,
        );
    assert!(
        rect.shrink(12.0).contains(point),
        "endpoint must be visibly inside viewport"
    );
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

#[test]
#[cfg(feature = "navigation")]
#[ignore = "producer-required: exact software Vulkan plus TERRAIN_RAMP_CAPTURE_DIR; run explicitly with --ignored --exact"]
fn actual_viewport_ramp_navigation_save_reopen_play_and_stale_refusal() {
    let evidence = PathBuf::from(
        std::env::var_os("TERRAIN_RAMP_CAPTURE_DIR")
            .expect("retained producer evidence directory required"),
    );
    std::fs::create_dir_all(&evidence).unwrap();
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path(), true);
    let mut h = gpu_harness(&scene, &evidence.join("adapter.json"));
    attach(&mut h, &scene);
    click(&mut h, "Terrain authoring");
    let initial = bytes(&h);
    let host_history = h.state().editor.history().clone();
    let before = capture(&mut h, &evidence, "01-before");
    pick(&mut h, "Pick ramp start", [1, 4]);
    pick(&mut h, "Pick ramp end", [7, 4]);
    capture(&mut h, &evidence, "01b-picked-endpoints");
    assert_eq!(
        h.state().terrain_ramp.endpoints,
        [Some([1, 4]), Some([7, 4])]
    );
    click(&mut h, "Stage ramp");
    let staged_raw = capture(&mut h, &evidence, "02-stage");
    assert_eq!(
        staged_raw, before,
        "preview must not replace the underlying terrain geometry"
    );
    h.get_by_label("Prospective ramp: 17 changed vertices; not applied");
    h.get_by_label("Apply ramp");
    let staged_ui = h.render().unwrap();
    let viewport = h.state().ui.viewport_rect.unwrap();
    let ppp = h.ctx.pixels_per_point();
    assert!(
        staged_ui
            .enumerate_pixels()
            .filter(
                |(x, y, p)| viewport.contains(egui::pos2(*x as f32 / ppp, *y as f32 / ppp))
                    && p[0] < 100
                    && p[1] > 150
                    && p[2] > 170
            )
            .count()
            > 100,
        "prospective cyan grid is visible inside the composed viewport, not panel text"
    );
    assert!(h.state().terrain_ramp.staged.is_some());
    assert_eq!(bytes(&h), initial);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(h.state().editor.history(), &host_history);
    assert_eq!(
        std::fs::read(root.path().join("ramp.orrt")).unwrap(),
        initial
    );
    click(&mut h, "Cancel ramp");
    assert!(h.state().terrain_ramp.staged.is_none());
    assert_eq!(bytes(&h), initial);
    // Cancel may clear endpoint intent, so select through the viewport again.
    pick(&mut h, "Pick ramp start", [1, 4]);
    pick(&mut h, "Pick ramp end", [7, 4]);
    click(&mut h, "Stage ramp");
    click(&mut h, "Apply ramp");
    let after = capture(&mut h, &evidence, "03-applied");
    let authored = bytes(&h);
    assert_ne!(before, after);
    assert_ne!(authored, initial);
    let line = [0, 16384, 32768, 49152, 65536, 81920, 98304];
    for z in 3..=5 {
        for x in 1..=7 {
            assert_eq!(
                h.state().terrain.document.terrain().unwrap().heights()[z * 9 + x].raw(),
                line[x - 1]
            );
        }
    }
    click(&mut h, "Terrain Undo");
    capture(&mut h, &evidence, "03b-undo");
    assert_eq!(bytes(&h), initial);
    assert!(!h.state().terrain.document.can_undo());
    click(&mut h, "Terrain Redo");
    capture(&mut h, &evidence, "03c-redo");
    assert_eq!(bytes(&h), authored);
    // Deliberately wrong, independently constructed fixtures must be visible.
    // These are negative controls only, undone before save/navigation admission.
    for (name, wrong_axis) in [
        ("negative-wrong-axis", true),
        ("negative-wrong-slope", false),
    ] {
        let mut edits = Vec::new();
        for z in 0..9 {
            for x in 0..9 {
                let raw = if x == 8 && (3..=5).contains(&z) {
                    // Preserve the identical exterior landing in both controls.
                    98304
                } else if wrong_axis {
                    if (3..=5).contains(&x) && (1..=7).contains(&z) {
                        line[(z - 1) as usize]
                    } else {
                        0
                    }
                } else if (3..=5).contains(&z) && (1..=7).contains(&x) {
                    line[(7 - x) as usize]
                } else {
                    0
                };
                edits.push(Edit::SetHeight {
                    x,
                    z,
                    height: FP::from_raw(raw),
                });
            }
        }
        h.state_mut().terrain.document.apply(&edits).unwrap();
        let negative = capture(&mut h, &evidence, name);
        let changed = after
            .chunks_exact(4)
            .zip(negative.chunks_exact(4))
            .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 12))
            .count();
        assert!(
            changed > 100,
            "diagnostic wrong axis/slope geometry must be visibly distinguishable: {changed}"
        );
        assert!(h.state_mut().terrain.document.undo().unwrap());
        assert_eq!(bytes(&h), authored);
    }
    click(&mut h, "Save terrain");
    assert_eq!(
        std::fs::read(root.path().join("ramp.orrt")).unwrap(),
        authored
    );
    click(&mut h, "Point navigation");
    {
        let app = h.state_mut();
        app.navigation.start = ["-3".into(), "0".into()];
        app.navigation.goal = ["3".into(), "0".into()];
        app.navigation.max_slope = "1".into();
        app.navigation.distance_per_tick = "0.125".into();
    }
    click(&mut h, "Build route");
    capture(&mut h, &evidence, "04-built");
    assert!(
        h.state().editor.admitted_navigation().admitted,
        "{:?}",
        h.state().navigation.error
    );
    assert!(h.state_mut().editor.save());
    drop(h);
    let mut h = gpu_harness(&scene, &evidence.join("reopen-adapter.json"));
    attach(&mut h, &scene);
    capture(&mut h, &evidence, "04b-reopened");
    assert_eq!(bytes(&h), authored);
    assert!(!h.state().terrain.document.can_undo());
    click(&mut h, LBL_PLAY);
    click(&mut h, LBL_PAUSE);
    h.state_mut().editor.seek(0);
    settle(&mut h);
    capture(&mut h, &evidence, "05-play-paused");
    assert_eq!(h.state().editor.snapshot().unwrap().predicted().tick(), 0);
    assert!(!h.state().editor.timeline().unwrap().playing);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    // Independent canonical strip geometry: cell(1,4) half0 through cell(7,4)
    // half0. Existing navigation uses edge midpoints, not a straight-line funnel.
    let expected_corridor: Vec<u32> = (66..=78).collect();
    let expected_waypoints = [
        [-196608, 0, 0],
        [-163840, 8192, 32768],
        [-131072, 16384, 32768],
        [-98304, 24576, 32768],
        [-65536, 32768, 32768],
        [-32768, 40960, 32768],
        [0, 49152, 32768],
        [32768, 57344, 32768],
        [65536, 65536, 32768],
        [98304, 73728, 32768],
        [131072, 81920, 32768],
        [163840, 90112, 32768],
        [196608, 98304, 32768],
        [196608, 98304, 0],
    ];
    let mut saw_middle = false;
    let mut previous_position: Option<[i64; 3]> = None;
    let mut trace = Vec::new();
    let mut render_drains = Vec::new();
    for step in 0..64 {
        let frame = authoritative_frame(&mut h);
        let frame_bytes = frame.to_bytes();
        let file = format!("step-{step:02}.frame");
        std::fs::write(evidence.join(&file), &frame_bytes).unwrap();
        let nav =
            orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&frame))
                .unwrap();
        trace.push(serde_json::json!({"step":step,"tick":frame.tick(),"checksum":format!("{:016x}",frame.checksum()),"file":file,"sha256":digest(&frame_bytes),"position_raw":nav.navigator.position().map(FP::raw),"status":format!("{:?}",nav.navigator.status()),"next_waypoint":nav.navigator.next_waypoint(),"path":nav.navigator.path().map(|p|serde_json::json!({"corridor":p.corridor().iter().map(|k|k.0).collect::<Vec<_>>(),"waypoints_raw":p.waypoints().iter().map(|p|p.map(FP::raw)).collect::<Vec<_>>()}))}));
        std::fs::write(
            evidence.join("traversal-partial.json"),
            serde_json::to_vec_pretty(&trace).unwrap(),
        )
        .unwrap();
        // Each UI pass submits the production viewport even when kittest does
        // not composite it. Drain once per simulation step, using h.render's
        // unchanged fail-fast timeout, rather than queueing hundreds of draws
        // between the named captures. Raw Frame evidence already exists above.
        render_drains
            .push(serde_json::json!({"step":step,"tick":frame.tick(),"phase":"before-render"}));
        std::fs::write(
            evidence.join("render-drains-partial.json"),
            serde_json::to_vec_pretty(&render_drains).unwrap(),
        )
        .unwrap();
        let rendered = h.render();
        render_drains.push(match &rendered {
            Ok(image)=>serde_json::json!({"step":step,"tick":frame.tick(),"phase":"after-render","ok":true,"dimensions":image.dimensions()}),
            Err(error)=>serde_json::json!({"step":step,"tick":frame.tick(),"phase":"after-render","ok":false,"error":error}),
        });
        std::fs::write(
            evidence.join("render-drains-partial.json"),
            serde_json::to_vec_pretty(&render_drains).unwrap(),
        )
        .unwrap();
        rendered.unwrap_or_else(|error| panic!("per-step production composition failed at step {step}, tick {} within the unchanged harness timeout: {error}",frame.tick()));
        if step == 1 {
            capture(&mut h, &evidence, "05b-middle");
        }
        let [x, y, z] = nav.navigator.position().map(FP::raw);
        let path = nav
            .navigator
            .path()
            .expect("actual route retained in authoritative Frame");
        assert_eq!(
            path.corridor().iter().map(|k| k.0).collect::<Vec<_>>(),
            expected_corridor
        );
        assert_eq!(
            path.waypoints()
                .iter()
                .map(|p| p.map(FP::raw))
                .collect::<Vec<_>>(),
            expected_waypoints
        );
        assert!((-196608..=196608).contains(&x));
        assert!(
            (0..=32768).contains(&z),
            "agent must remain in the independently derived half-cell corridor"
        );
        if nav.navigator.status() == orr_navigation::NavigationStatus::Arrived {
            assert_eq!(nav.navigator.next_waypoint(), 14);
            assert_eq!([x, y, z], [196608, 98304, 0]);
        } else {
            assert_eq!(
                nav.navigator.status(),
                orr_navigation::NavigationStatus::Moving
            );
            assert!((1..=13).contains(&nav.navigator.next_waypoint()));
        }
        let segment = (nav.navigator.next_waypoint() as usize)
            .saturating_sub(1)
            .min(expected_corridor.len() - 1);
        let key = expected_corridor[segment];
        let cell_x = i64::from((key / 2) % 8);
        let u = x - (cell_x - 4) * 65536;
        let v = z;
        assert!((0..=65536).contains(&u) && (0..=65536).contains(&v));
        assert!(
            if key % 2 == 0 { u <= v } else { v <= u },
            "raw point must lie in its independent canonical triangle"
        );
        if let Some(previous) = previous_position {
            assert!(
                x >= previous[0],
                "X advances nondecreasingly along this corridor"
            );
            assert_ne!(
                [x, y, z],
                previous,
                "each fixed-distance step makes progress"
            );
            if previous[0] == 196608 {
                assert!(z < previous[2], "final vertical leg approaches exact goal");
            }
        }
        previous_position = Some([x, y, z]);
        // Piecewise plane sampling floors; profile vertices are exact quarters.
        assert_eq!(
            y,
            (x + 196608).div_euclid(4),
            "authoritative height independently follows ramp"
        );
        if x > -196608 && x < 196608 {
            saw_middle = true;
        }
        if nav.navigator.status() == orr_navigation::NavigationStatus::Arrived {
            break;
        }
        h.state_mut().editor.step(1);
        settle(&mut h);
    }
    capture(&mut h, &evidence, "06-arrival");
    let frame = authoritative_frame(&mut h);
    let nav =
        orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&frame))
            .unwrap();
    assert!(saw_middle);
    assert_eq!(
        nav.navigator.status(),
        orr_navigation::NavigationStatus::Arrived
    );
    assert_eq!(nav.navigator.position().map(FP::raw), [196608, 98304, 0]);
    click(&mut h, LBL_STOP);
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 4,
            z: 4,
            height: FP::from_int(2),
        }])
        .unwrap();
    capture(&mut h, &evidence, "07-stale");
    click(&mut h, LBL_PLAY);
    capture(&mut h, &evidence, "08-stale-play-refused");
    assert_eq!(
        h.state().editor.mode(),
        Mode::Edit,
        "stale route must refuse Play"
    );
    let phases = [
        "01-before",
        "01b-picked-endpoints",
        "02-stage",
        "03-applied",
        "03b-undo",
        "03c-redo",
        "negative-wrong-axis",
        "negative-wrong-slope",
        "04-built",
        "04b-reopened",
        "05-play-paused",
        "05b-middle",
        "06-arrival",
        "07-stale",
        "08-stale-play-refused",
    ];
    let mut files = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&evidence).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() && path.file_name().unwrap() != "complete.json" {
            let data = std::fs::read(&path).unwrap();
            files.insert(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                serde_json::json!({"bytes":data.len(),"sha256":digest(&data)}),
            );
        }
    }
    std::fs::write(evidence.join("complete.json"),serde_json::to_vec_pretty(&serde_json::json!({"complete":true,"phases":phases,"files":files,"source_sha":std::env::var("TERRAIN_RAMP_SOURCE_SHA").unwrap(),"scope":"existing point-agent heightfield navigation, not polygon navmesh or rigid-body collision; paused deterministic steps, not timing/performance evidence","independent_raw_profile":line,"expected_corridor":expected_corridor,"expected_waypoints_raw":expected_waypoints,"pointer_endpoints":true,"stage_cancel_apply":true,"single_undo_redo":true,"save_fresh_reopen":true,"stale_play_refused":true,"authoritative_frame_trace":trace,"render_drains":render_drains})).unwrap()).unwrap();
}
