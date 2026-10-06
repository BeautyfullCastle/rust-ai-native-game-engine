//! Installed skeletal content through the real Yard host, EditorApp and egui texture.
#![cfg(feature = "animated-models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::Harness;
use orr_editor::{
    app::BottomTab,
    game::EditorGame,
    model_bindings::{ModelKind, PlaybackMode},
    Editor, EditorApp,
};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn scene(root: &Path) -> PathBuf {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[
            assets.join("animation_demo").canonicalize().unwrap(),
            assets.join("imported_scene_demo").canonicalize().unwrap(),
        ])
        .unwrap();
    let scene = root.join("yard.scene.yaml");
    // Fixed bodies isolate deformation from physics motion in the pixel comparisons.
    let mut text = include_str!("../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    let fourth = text
        .split("  e_00000003:\n")
        .nth(1)
        .unwrap()
        .replace("name: box_right", "name: model_static")
        .replace("pos: [2, 3, 0]", "pos: [0, 2, -3]");
    text.push_str("  e_00000004:\n");
    text.push_str(&fourth);
    std::fs::write(&scene, text).unwrap();
    scene
}
fn editor(scene: &Path) -> Editor {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 2.0, 0.0], 0.0, 0.2, 12.0);
    editor
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
    assert!(h.state().editor.yard_rows_coherent());
}
fn assign(app: &mut EditorApp, name: &str, kind: ModelKind, clip: u32) {
    assert!(app.editor.select_named(name));
    app.editor.sync();
    app.models.kind = kind;
    app.models.package = if kind == ModelKind::Static {
        "sample-imported-scene"
    } else {
        "sample-animation"
    }
    .into();
    app.models.asset = if kind == ModelKind::Static {
        "foreground.glb"
    } else {
        "animated.glb"
    }
    .into();
    app.models.animation.clip_index = clip;
    app.models.animation.playback = PlaybackMode::Loop;
    app.models.assign(&app.editor).unwrap();
}
fn open(app: &mut EditorApp) {
    app.models.open_for_editor(&app.editor, true).unwrap();
}
/// ERP acknowledgements and frame delivery are asynchronous. Wait for the
/// snapshot mode fence rather than assuming one `sync` observes Stop's frame.
fn sync_clock(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        editor.sync();
        if editor.snapshot().is_some_and(|snapshot| {
            snapshot.timeline().is_some() == (editor.mode() == orr_editor::Mode::Play)
                && snapshot.tick() == snapshot.predicted().tick()
        }) && editor.yard_rows_coherent()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "coherent host mode/frame did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn frame_bytes(editor: &mut Editor) -> (u64, u64, Vec<u8>) {
    sync_clock(editor);
    let s = editor.snapshot().unwrap();
    let tick = s.predicted().tick();
    let checksum = s.predicted().checksum();
    let client = editor.agent_client("yard-animation-byte-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return (tick, checksum, frame.frame.to_bytes());
            }
        }
        assert!(
            Instant::now() < end,
            "matching immutable host snapshot did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn trajectory(app: &mut EditorApp) -> Vec<(u64, u64, Vec<u8>)> {
    let mut bytes = vec![frame_bytes(&mut app.editor)];
    assert!(app.editor.start_play());
    bytes.push(frame_bytes(&mut app.editor));
    for step in [12, 18, 30] {
        app.editor.step(step);
        bytes.push(frame_bytes(&mut app.editor));
        let _ = app.models.animated_placements(&app.editor).unwrap();
    }
    app.editor.seek(12);
    bytes.push(frame_bytes(&mut app.editor));
    assert_eq!(bytes[2], bytes[5]);
    app.editor.seek(0);
    bytes.push(frame_bytes(&mut app.editor));
    assert_eq!(bytes[1], bytes[6]);
    assert!(app.editor.stop().is_some());
    bytes.push(frame_bytes(&mut app.editor));
    assert_eq!(bytes[0], bytes[7]);
    bytes
}
#[test]
fn off_static_animated_and_mixed_bindings_preserve_every_host_checkpoint_byte() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    let off = trajectory(&mut app);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Static, 0);
    assert_eq!(off, trajectory(&mut app));
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    assert_eq!(off, trajectory(&mut app));
    assign(&mut app, "box_right", ModelKind::Static, 0);
    assert_eq!(off, trajectory(&mut app));
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    assert_eq!(off, trajectory(&mut app));
}
#[test]
fn independent_clip_indices_sample_one_shared_asset_and_edit_mutations_are_rejected_in_play() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    let rest = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(rest.len(), 2);
    assert!(std::sync::Arc::ptr_eq(&rest[0].model, &rest[1].model));
    assert_eq!(rest[0].pose, rest[1].pose);
    assert!(app.editor.start_play());
    app.editor.sync();
    app.editor.step(30);
    app.editor.sync();
    let poses = app.models.animated_placements(&app.editor).unwrap();
    assert_ne!(poses[0].pose, poses[1].pose);
    let clock = app.editor.snapshot().unwrap();
    let time = clock.tick() as f32 / clock.tick_rate() as f32;
    for (placement, clip) in poses.iter().zip([0, 1]) {
        assert_eq!(
            placement.pose,
            placement.model.sample_clip(clip, time).unwrap()
        );
    }
    assert_eq!(
        app.models.animated_placements(&app.editor).unwrap()[0].pose,
        poses[0].pose
    );
    assert!(app.models.assign(&app.editor).is_err());
    assert!(app.models.reload(&app.editor).is_err());
    assert!(app.models.undo(&app.editor).is_err());
    assert!(app.models.redo(&app.editor).is_err());
    app.editor.seek(0);
    app.editor.sync();
    let zero = app.models.animated_placements(&app.editor).unwrap();
    for (placement, clip) in zero.iter().zip([0, 1]) {
        assert_eq!(
            placement.pose,
            placement.model.sample_clip(clip, 0.0).unwrap()
        );
    }
    app.editor.stop();
    sync_clock(&mut app.editor);
    let stopped = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(stopped[0].pose, rest[0].pose);
    assert_eq!(stopped[1].pose, rest[1].pose);
}
fn pixels(h: &Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    let gpu = h.state().viewport3d_gpu().expect("real main viewport");
    let bytes = gpu.gpu().read_rgba8();
    let size = h.state().ui.viewport_px;
    assert_eq!(bytes.len(), (size.0 * size.1 * 4) as usize);
    if let Some(dir) = std::env::var_os("ORR_YARD_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(Path::new(&dir).join(format!("{name}.ppm"))).unwrap();
        write!(f, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
        for p in bytes.chunks_exact(4) {
            f.write_all(&p[..3]).unwrap();
        }
    }
    bytes
}
#[test]
fn production_main_viewport_animated_ticks_pause_seek_stop_save_reopen_and_mixed_content() {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => eprintln!(
            "Yard animated adapter: {} software={}",
            gpu.adapter_name(),
            gpu.is_software()
        ),
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "mandatory GPU unavailable: {e}"
            );
            return;
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let editor = editor(&scene);
    let mut h = Harness::builder()
        .with_size([1200.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.ui.bottom_tab = BottomTab::History;
            app
        });
    settle(&mut h);
    let procedural = pixels(&h, "animated-procedural");
    let camera = h.state().editor.camera3d;
    open(h.state_mut());
    assign(h.state_mut(), "box_left", ModelKind::Animated, 0);
    assign(h.state_mut(), "box_right", ModelKind::Animated, 1);
    // Third presentation binding is static while the ground remains procedural.
    assign(h.state_mut(), "model_static", ModelKind::Static, 0);
    h.state_mut().editor.select(None);
    h.state_mut().editor.sync();
    settle(&mut h);
    let size = h.state().ui.viewport_px;
    let rest = pixels(&h, "animated-rest");
    assert_ne!(procedural, rest);
    let composed = h.render().unwrap();
    assert!(composed.as_raw().chunks_exact(4).any(|p| p[0] > 80));
    {
        let a = h.state_mut();
        a.models.bindings.as_mut().unwrap().save().unwrap();
        assert!(a.editor.save());
    }
    settle(&mut h);
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    assert_eq!(size, h.state().ui.viewport_px);
    let zero = pixels(&h, "animated-play0");
    let ui_zero = h.render().expect("compose native animated viewport");
    h.state_mut().editor.step(30);
    settle(&mut h);
    assert_eq!(size, h.state().ui.viewport_px);
    let middle = pixels(&h, "animated-tick30");
    assert_ne!(zero, middle);
    let ui_middle = h
        .render()
        .expect("compose deformed native animated viewport");
    let rect = h.state().ui.viewport_rect.unwrap();
    let changed = ui_middle
        .enumerate_pixels()
        .filter(|(x, y, pixel)| {
            let point = egui::pos2(*x as f32, *y as f32);
            rect.shrink(4.0).contains(point)
                && point.y > rect.min.y + 40.0
                && *pixel != ui_zero.get_pixel(*x, *y)
        })
        .count();
    assert!(
        changed > 20,
        "skeletal deformation reaches the egui native viewport texture: {changed}"
    );
    settle(&mut h);
    assert_eq!(middle, pixels(&h, "animated-paused30"));
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(zero, pixels(&h, "animated-seek0"));
    h.state_mut().editor.seek(30);
    settle(&mut h);
    assert_eq!(middle, pixels(&h, "animated-seek30"));
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(size, h.state().ui.viewport_px);
    assert_eq!(rest, pixels(&h, "animated-stop-rest"));
    assert!(h.state_mut().editor.open_path(&scene));
    settle(&mut h);
    {
        let a = h.state_mut();
        a.models.open_for_editor(&a.editor, false).unwrap();
    }
    settle(&mut h);
    assert_eq!(rest, pixels(&h, "animated-reopened-rest"));
    assert_eq!(camera, h.state().editor.camera3d);
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
    assert_eq!(
        h.state()
            .models
            .animated_placements(&h.state().editor)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn invalid_clip_assignment_and_reopen_preserve_document_history_and_cached_pose() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    app.models.bindings.as_mut().unwrap().save().unwrap();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let cached = app.models.animated_placements(&app.editor).unwrap();
    app.models.animation.clip_index = 31;
    let error = app.models.assign(&app.editor).unwrap_err();
    assert!(error.contains("clip"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    assert!(std::sync::Arc::ptr_eq(
        &cached[0].model,
        &app.models.animated_placements(&app.editor).unwrap()[0].model
    ));
    let sidecar = app.models.bindings.as_ref().unwrap().path.clone();
    let mut invalid = serde_json::to_value(&before).unwrap();
    invalid["bindings"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["animation"]["clip_index"] = 31.into();
    std::fs::write(&sidecar, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(app
        .models
        .open_for_editor(&app.editor, false)
        .unwrap_err()
        .contains("clip"));
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    assert!(std::sync::Arc::ptr_eq(
        &cached[0].model,
        &app.models.animated_placements(&app.editor).unwrap()[0].model
    ));
    app.models.undo(&app.editor).unwrap();
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    app.models.redo(&app.editor).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
}

#[test]
fn changed_package_after_clip_inspection_requires_explicit_reload_before_assignment() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    app.models.load_candidate(&app.editor).unwrap();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    let revision = temp.path().join("animation-revision");
    std::fs::create_dir(&revision).unwrap();
    for entry in std::fs::read_dir(original).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), revision.join(entry.file_name())).unwrap();
        }
    }
    let manifest = revision.join("orr.package.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["version"] = "1.0.1".into();
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let installer =
        Project::open_for_install(temp.path(), Runtime::content_only().engine_version).unwrap();
    installer.remove("sample-animation").unwrap();
    installer.install(&[revision]).unwrap();
    assert!(app
        .models
        .assign(&app.editor)
        .unwrap_err()
        .contains("changed"));
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    app.models.load_candidate(&app.editor).unwrap();
    app.models.assign(&app.editor).unwrap();
    assert_ne!(app.models.bindings.as_ref().unwrap().document(), &before);
}

#[test]
fn animated_kind_clip_loop_assignment_uses_real_inspector_widgets() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let editor = editor(&scene);
    let mut h = Harness::builder()
        .with_size([1500.0, 1400.0])
        .build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_left"));
    settle(&mut h);
    fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
        h.get_by_label(label).scroll_to_me();
        h.run_steps(2);
        h.get_by_label(label).click();
        settle(h);
    }
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = "sample-animation".into();
    h.state_mut().models.asset = "animated.glb".into();
    click(&mut h, "Animated");
    click(&mut h, "Load model asset");
    h.state_mut().models.animation.clip_index = 1;
    click(&mut h, "Loop");
    click(&mut h, "Assign animated model");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    let binding = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap();
    assert_eq!(binding.kind, ModelKind::Animated);
    assert_eq!(binding.animation.unwrap().clip_index, 1);
    assert_eq!(binding.animation.unwrap().playback, PlaybackMode::Loop);
    click(&mut h, "Save model bindings");
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    for label in [
        "Assign animated model",
        "Load model asset",
        "Undo model",
        "Redo model",
        "Save model bindings",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}

#[test]
fn mixed_viewport_rejects_foreign_pose_and_combined_limits_before_resize_or_visible_mutation() {
    use orr_editor::viewport3d::{AnimatedPlacement, Viewport3dGpu};
    let gpu = match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => gpu,
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "mandatory GPU unavailable: {e}"
            );
            return;
        }
    };
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    assign(&mut app, "box_right", ModelKind::Static, 0);
    let static_models = app.models.placements(&app.editor);
    let animated = app.models.animated_placements(&app.editor).unwrap();
    let hidden: Vec<_> = static_models
        .iter()
        .map(|p| p.entity)
        .chain(animated.iter().map(|p| p.entity))
        .collect();
    let list = app.editor.yard_frame().list(&hidden, None);
    let camera = app.editor.camera3d.camera();
    let mut viewport = Viewport3dGpu::new(&gpu, (160, 128));
    viewport
        .render_mixed((160, 128), &list, &camera, &static_models, &animated)
        .unwrap();
    let pixels = viewport.read_rgba8();
    let generation = viewport.target().generation();
    let foreign =
        orr_model::animation::AnimatedModel::new(animated[0].model.source().clone()).unwrap();
    let bad = AnimatedPlacement {
        entity: animated[0].entity,
        model: animated[0].model.clone(),
        instance: animated[0].instance,
        pose: foreign.rest_pose().unwrap(),
    };
    assert!(viewport
        .render_mixed((170, 140), &list, &camera, &static_models, &[bad])
        .is_err());
    assert_eq!(viewport.target().generation(), generation);
    assert_eq!(viewport.read_rgba8(), pixels);
    let too_many: Vec<_> = (0..256)
        .map(|_| AnimatedPlacement {
            entity: animated[0].entity,
            model: animated[0].model.clone(),
            instance: animated[0].instance,
            pose: animated[0].pose.clone(),
        })
        .collect();
    assert!(viewport
        .render_mixed((170, 140), &list, &camera, &static_models, &too_many)
        .unwrap_err()
        .contains("256"));
    let too_many_assets: Vec<_> = (0..8)
        .map(|_| {
            let model = std::sync::Arc::new(
                orr_model::animation::AnimatedModel::new(animated[0].model.source().clone())
                    .unwrap(),
            );
            AnimatedPlacement {
                entity: animated[0].entity,
                pose: model.rest_pose().unwrap(),
                model,
                instance: animated[0].instance,
            }
        })
        .collect();
    assert!(viewport
        .render_mixed((170, 140), &list, &camera, &static_models, &too_many_assets)
        .unwrap_err()
        .contains("eight"));
    assert_eq!(viewport.target().generation(), generation);
    assert_eq!(viewport.read_rgba8(), pixels);
    viewport
        .render_mixed((160, 128), &list, &camera, &static_models, &animated)
        .unwrap();
    assert_eq!(viewport.read_rgba8(), pixels);
}

#[test]
fn assignment_rejects_invalid_external_rest_bounds_even_when_clip_zero_is_valid() {
    use orr_model::animation::{
        AnimatedModel, AnimationChannel, ChannelValues, Interpolation, Trs,
    };
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    let mut source = orr_model::animation_import::import_path(&root, "animated.glb")
        .unwrap()
        .source()
        .clone();
    // Internal nodes/vertices each stay within their legal source ranges,
    // but their sum exceeds the external world bound after local scale.
    source.nodes[0].rest.translation[0] = 1.0e6;
    for primitive in &mut source.primitives {
        for vertex in &mut primitive.vertices {
            vertex.vertex.position[0] += 999_000.0;
        }
    }
    source.clips[0].channels.push(AnimationChannel {
        node: 0,
        interpolation: Interpolation::Step,
        times: vec![0.0],
        values: ChannelValues::Translation(vec![[-999_000.0, 0.2, 0.0]]),
    });
    let model = AnimatedModel::new(source).unwrap();
    let transform = Trs {
        translation: [-2.0, 1.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1000.0; 3],
    }
    .matrix();
    let zero = model.sample_clip(0, 0.0).unwrap();
    let rest = model.rest_pose().unwrap();
    assert!(orr_render::SkinnedInstance {
        pose: &zero,
        transform
    }
    .validate_for(&model)
    .is_ok());
    assert!(orr_render::SkinnedInstance {
        pose: &rest,
        transform
    }
    .validate_for(&model)
    .is_err());
    let revision = temp.path().join("rest-bounds-package");
    std::fs::create_dir(&revision).unwrap();
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), revision.join(entry.file_name())).unwrap();
        }
    }
    std::fs::write(
        revision.join("external-rest.json"),
        model.to_bytes().unwrap(),
    )
    .unwrap();
    let manifest = revision.join("orr.package.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["version"] = "1.0.1".into();
    value["files"]
        .as_array_mut()
        .unwrap()
        .push("external-rest.json".into());
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let project =
        Project::open_for_install(temp.path(), Runtime::content_only().engine_version).unwrap();
    project.remove("sample-animation").unwrap();
    project.install(&[revision]).unwrap();
    app.models.asset = "external-rest.json".into();
    app.models.transform.scale = [1000.0; 3];
    assert!(app.models.assign(&app.editor).is_err());
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
}
