//! Actual CLI and EditorApp widgets/GPU for existing Room/Yard consumers.
//! Field values and selection are seeded through public test APIs; Load, Assign,
//! Save and Play are real egui widgets. Reopening uses existing admission APIs.
#![cfg(all(feature = "room-animated-models", feature = "package-manager"))]
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
#[allow(dead_code)]
#[path = "../../orr_model/tests/common/animation_clip_sequence_oracle.rs"]
mod oracle;
use egui_kittest::{Harness, kittest::Queryable};
use egui_wgpu::{WgpuSetupCreateNew, wgpu};
use orr_editor::{
    Editor, EditorApp, HostSpec, Target,
    app::{LBL_PAUSE, LBL_PLAY},
    game::EditorGame,
    model_bindings::{self, ModelKind},
};
use orr_package::{Project, Runtime};
use orr_reflect::{Guid, Scene};
use orr_sample::{
    room_game::{KEY, PLAYER, RoomActor, RoomConfig, RoomEscapeV1},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
const PACKAGE: &str = "sequence-acceptance";
const ASSET: &str = "action.animated.json";
fn write(root: &Path, name: &str, bytes: &[u8]) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}
fn software_setup(evidence: PathBuf) -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _| {
        let adapter = adapters
            .iter()
            .find(|a| {
                let i = a.get_info();
                i.backend == wgpu::Backend::Vulkan && i.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "mandatory software Vulkan Cpu adapter unavailable".to_owned())?;
        let i = adapter.get_info();
        // Selector may be invoked more than once: retain its first observed
        // adapter rather than overwrite failure evidence.
        let path = evidence.join("adapter.json");
        if !path.exists() {
            write(&evidence,"adapter.json",&serde_json::to_vec_pretty(&json!({"name":i.name,"backend":format!("{:?}",i.backend),"device_type":format!("{:?}",i.device_type)})).unwrap());
        }
        Ok(adapter)
    }));
    setup.into()
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        h.state_mut().editor.sync();
        h.run_steps(2);
        if h.state().editor.down().is_none()
            && h.state().editor.yard_rows_coherent()
            && h.state().editor.snapshot().is_some_and(|s| {
                s.timeline().is_some() == (h.state().editor.mode() == orr_editor::Mode::Play)
                    && s.tick() == s.predicted().tick()
            })
        {
            return;
        }
        assert!(Instant::now() < end, "coherent host frame deadline");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    settle(h);
}
fn frame(h: &mut Harness<'_, EditorApp>) -> (u64, u64, Vec<u8>) {
    settle(h);
    let s = h.state().editor.snapshot().unwrap();
    let tick = s.predicted().tick();
    let checksum = s.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("sequence-authority-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(f) = client.local_frames.pop_front() {
            if f.frame.tick() == tick && f.frame.checksum() == checksum {
                return (tick, checksum, f.frame.to_bytes());
            }
        }
        assert!(Instant::now() < end, "authoritative Frame bytes deadline");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn capture(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) -> Vec<u8> {
    settle(h);
    let composed = h.render().expect("real egui GPU composition");
    write(root, &format!("{name}-ui.rgba"), composed.as_raw());
    let pixels = h
        .state()
        .viewport3d_gpu()
        .expect("actual EditorApp main viewport")
        .gpu()
        .read_rgba8();
    let (w, height) = h.state().ui.viewport_px;
    write(root, &format!("{name}.rgba"), &pixels);
    let mut ppm = format!("P6\n{w} {height}\n255\n").into_bytes();
    for p in pixels.as_chunks::<4>().0 {
        ppm.extend_from_slice(&p[..3]);
    }
    write(root, &format!("{name}.ppm"), &ppm);
    pixels
}
fn room(root: &Path) -> Vec<Guid> {
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
        .unwrap()
        .to_yaml();
    write(root, "room.scene.yaml", text.as_bytes());
    write(root,"orr.project.json",&serde_json::to_vec(&json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap());
    let scene = PreparedScene::parse(&text).unwrap();
    scene
        .frame()
        .entities()
        .filter(|e| [PLAYER, KEY].contains(&scene.frame().get::<RoomActor>(*e).unwrap().kind))
        .map(|e| scene.index().guid(e).unwrap().clone())
        .collect()
}
fn harness(root: &Path, is_room: bool, evidence: &Path) -> Harness<'static, EditorApp> {
    let (mut editor, models) = if is_room {
        let (_, path, scene, models) = PreparedProject::open(root).unwrap().into_parts();
        (
            Editor::start(&HostSpec::PreparedRoom {
                scene: path,
                text: scene.text().into(),
                listen: None,
                debug_hooks: false,
            })
            .unwrap(),
            Some(models),
        )
    } else {
        (
            Editor::open_game(&root.join("yard.scene.yaml"), EditorGame::Yard3D).unwrap(),
            None,
        )
    };
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 2.0, 0.0], 0.0, 0.2, 12.0);
    Harness::builder()
        .with_size([1500.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(software_setup(evidence.into()))
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            if let Some(models) = models {
                app.models.install_room(models).unwrap();
            }
            app
        })
}
fn checkpoint(h: &mut Harness<'_, EditorApp>, tick: u64) -> (u64, u64, Vec<u8>) {
    settle(h);
    let current = h.state().editor.snapshot().unwrap().predicted().tick();
    if current < tick {
        h.state_mut()
            .editor
            .step((tick - current).try_into().unwrap());
    } else {
        h.state_mut().editor.seek(tick);
    }
    let frame = frame(h);
    assert_eq!(frame.0, tick, "requested exact authoritative tick");
    frame
}
#[test]
#[ignore = "mandatory absolute CLI, evidence directory and software Vulkan widget acceptance"]
fn actual_cli_room_yard_widgets_save_reopen_play_and_authority_parity() {
    assert_eq!(
        std::env::var("ORR_SEQUENCE_REQUIRE_SOFTWARE").as_deref(),
        Ok("1")
    );
    let cli =
        PathBuf::from(std::env::var_os("ORR_ANIMATION_SEQUENCE_CLI").expect("absolute built CLI"));
    assert!(cli.is_absolute() && cli.is_file());
    let evidence = PathBuf::from(
        std::env::var_os("ORR_SEQUENCE_EDITOR_EVIDENCE")
            .expect("fresh absolute evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let source = evidence.join("inputs");
    fs::create_dir(&source).unwrap();
    write(
        &source,
        "source.animated.json",
        &oracle::fixture().to_bytes().unwrap(),
    );
    write(&source,"recipe.sequence.json",&serde_json::to_vec(&json!({"format":"orr_animation_clip_sequence","version":1,"source":"source.animated.json","first_clip":0,"second_clip":1,"transition_seconds":1,"name":"A then B"})).unwrap());
    let result = Command::new(&cli)
        .arg("--recipe")
        .arg(source.join("recipe.sequence.json"))
        .args(["--asset-id", "sequences/action.sequence.json", "--output"])
        .arg(source.join(ASSET))
        .output()
        .unwrap();
    write(&evidence, "cli.stdout", &result.stdout);
    write(&evidence, "cli.stderr", &result.stderr);
    write(&evidence,"started.json",&serde_json::to_vec_pretty(&json!({"status":"partial","cli":cli,"exit":result.status.code(),"actual_widgets":["Load model asset","Assign animated model","Save model bindings",LBL_PLAY],"api_seeded":["selection","package and asset text","clip index"],"scope":"Room and Yard; Mesh excluded"})).unwrap());
    assert!(result.status.success());
    let authored = fs::read(source.join(ASSET)).unwrap();
    let hash: String = Sha256::digest(&authored)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    write(
        &source,
        "foreground.glb",
        include_bytes!("../../../assets/imported_scene_demo/foreground.glb"),
    );
    write(&source,"orr.package.json",&serde_json::to_vec(&json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"*","capabilities":["models","animation"],"dependencies":{},"files":[ASSET,"foreground.glb"]})).unwrap());
    let mut roots = Vec::new();
    for is_room in [false, true] {
        let name = if is_room { "room" } else { "yard" };
        let root = evidence.join(name);
        fs::create_dir(&root).unwrap();
        let guids = if is_room {
            room(&root)
        } else {
            write(
                &root,
                "yard.scene.yaml",
                include_str!("../../../scenes/yard3d_authoring.scene.yaml")
                    .replace("kind: dynamic", "kind: static")
                    .replace("inv_mass: 1", "inv_mass: 0")
                    .as_bytes(),
            );
            Vec::new()
        };
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
        if is_room {
            // Room admission needs a real initial binding. Start with unrelated
            // static art, never the sequence that the widgets must assign.
            let static_asset =
                model_bindings::load_asset(&root, PACKAGE, "foreground.glb").unwrap();
            let binding = model_bindings::Binding::from_asset(
                PACKAGE.into(),
                "foreground.glb".into(),
                &static_asset,
                model_bindings::LocalTransform::default(),
            )
            .unwrap();
            let document = model_bindings::Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings: [(guids[0].to_string(), binding)].into_iter().collect(),
            };
            write(
                &root,
                "room.models.json",
                &serde_json::to_vec(&document).unwrap(),
            );
        }
        roots.push((root, is_room, guids));
    }
    fs::rename(&source, evidence.join("inputs-hidden")).unwrap();
    assert!(!source.exists());
    for (root, is_room, guids) in roots {
        let catalog = model_bindings::list_assets(&root).unwrap();
        assert!(
            catalog
                .iter()
                .any(|a| a.package == PACKAGE && a.asset == ASSET && a.kind == ModelKind::Animated)
        );
        let loaded =
            model_bindings::load_asset_for_kind(&root, PACKAGE, ASSET, ModelKind::Animated)
                .unwrap();
        assert_eq!(loaded.source_hash(), hash);
        let model = loaded.animated_model().unwrap();
        assert_eq!(model.source().dependencies.len(), 2);
        let clip = (model.source().clips.len() - 1) as u32;
        let mut h = harness(&root, is_room, &root);
        settle(&mut h);
        // The saved Room is admitted with real static art. Temporarily detach
        // the presentation panel for a genuinely presentation-off baseline.
        let initial_panel = std::mem::take(&mut h.state_mut().models);
        assert!(h.state_mut().editor.start_play());
        let off = checkpoint(&mut h, 135);
        h.state_mut().editor.stop();
        settle(&mut h);
        h.state_mut().models = initial_panel;
        if is_room {
            assert!(
                h.state()
                    .models
                    .bindings
                    .as_ref()
                    .unwrap()
                    .document()
                    .bindings
                    .values()
                    .all(|b| b.kind == ModelKind::Static && b.asset == "foreground.glb")
            );
        }
        assert_eq!(clip, 2, "literal fixture appends exactly clip 2");
        let mut intended_bindings = std::collections::BTreeMap::new();
        for index in 0..2 {
            if is_room {
                h.state_mut()
                    .editor
                    .select(Some(Target::Guid(guids[index].clone())));
            } else {
                assert!(
                    h.state_mut()
                        .editor
                        .select_named(["box_left", "box_right"][index])
                );
            }
            settle(&mut h);
            let target = h.state().editor.selected_guid().unwrap().to_string();
            assert!(
                intended_bindings
                    .insert(target, if index == 0 { 2 } else { 0 })
                    .is_none(),
                "each intended actor is distinct"
            );
            if index == 0 {
                click(&mut h, "Static model binding");
                if !is_room {
                    click(&mut h, "Create model bindings");
                }
            }
            h.state_mut().models.package = PACKAGE.into();
            h.state_mut().models.asset = ASSET.into();
            click(&mut h, "Animated");
            click(&mut h, "Load model asset");
            h.state_mut().models.animation.clip_index = if index == 0 { clip } else { 0 };
            click(&mut h, "Once");
            click(&mut h, "Assign animated model");
            assert!(
                h.state().models.error().is_none(),
                "{:?}",
                h.state().models.error()
            );
        }
        click(&mut h, "Save model bindings");
        assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
        let saved = h
            .state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .clone();
        assert_eq!(saved.bindings.len(), 2);
        for (target, expected_clip) in &intended_bindings {
            let binding = &saved.bindings[target];
            assert_eq!(binding.package, PACKAGE, "target {target} package");
            assert_eq!(binding.asset, ASSET, "target {target} asset");
            assert_eq!(binding.kind, ModelKind::Animated, "target {target} kind");
            let animation = binding.animation.expect("intended animated binding");
            assert_eq!(
                animation.clip_index, *expected_clip,
                "target {target} exact clip selection"
            );
            assert_eq!(
                animation.playback,
                model_bindings::PlaybackMode::Once,
                "target {target} playback selection"
            );
            assert_eq!(binding.source_hash, hash);
        }
        drop(h);
        let mut h = harness(&root, is_room, &root);
        settle(&mut h);
        if !is_room {
            let app = h.state_mut();
            app.models.open_for_editor(&app.editor, false).unwrap();
        }
        settle(&mut h);
        assert_eq!(
            h.state().models.bindings.as_ref().unwrap().document(),
            &saved
        );
        click(&mut h, LBL_PLAY);
        click(&mut h, LBL_PAUSE);
        let on = checkpoint(&mut h, 135);
        let first = capture(&mut h, &root, "transition-quarter");
        write(&root, "checkpoint.frame", &on.2);
        write(&root,"partial.json",&serde_json::to_vec_pretty(&json!({"status":"partial","tick":on.0,"checksum":on.1,"bindings":2,"cooked_provenance_entries":2,"binding_source_hash":hash,"presentation_off_checksum":off.1})).unwrap());
        assert_eq!(
            on, off,
            "presentation does not mutate authoritative Frame or checksum"
        );
        let placements = h
            .state()
            .models
            .animated_placements(&h.state().editor)
            .unwrap();
        assert_eq!(placements.len(), 2);
        // Each independently assigned instance is checked against literal f64
        // fixture algebra, not sample_clip compared against itself.
        for p in &placements {
            let guid = h
                .state()
                .editor
                .rows()
                .iter()
                .find(|r| r.entity == p.entity)
                .unwrap()
                .guid
                .as_ref()
                .unwrap()
                .to_string();
            let binding = &saved.bindings[&guid];
            let expected = if binding.animation.unwrap().clip_index == clip {
                oracle::sequence_positions(2.25)
            } else {
                oracle::source_positions(0, 2.25)
            };
            for (v, w) in p.model.deform(&p.pose).unwrap()[0]
                .vertices
                .iter()
                .zip(expected)
            {
                for (actual, expected) in v.position.iter().zip(w) {
                    assert!((f64::from(*actual) - expected).abs() < 4e-5);
                }
            }
        }
        checkpoint(&mut h, 165);
        let last = capture(&mut h, &root, "transition-three-quarter");
        assert_eq!(first.len(), last.len());
        let changed = first
            .as_chunks::<4>().0.iter()
            .zip(last.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 20, "actual main viewport motion: {changed}");
        checkpoint(&mut h, 135);
        let replay = capture(&mut h, &root, "seek-quarter");
        assert_eq!(first, replay, "paused deterministic viewport seek");
        write(&root,"acceptance.json",&serde_json::to_vec_pretty(&json!({"status":"passed","actual_EditorApp_widgets_and_GPU":true,"catalog":true,"save_reopen":true,"instances":2,"visible_changed_pixels":changed,"authority_byte_parity":true,"inputs_hidden":true,"visible_oracle_scope":"direct renderer suite proves pixels independently; this suite proves native viewport integration and independent per-instance deformation; procedural Room/Yard geometry remains present"})).unwrap());
    }
}
