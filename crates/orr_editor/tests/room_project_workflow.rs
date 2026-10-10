//! Real authored-room editor workflow. GPU acceptance is explicit and never a skip-pass.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{kittest::Queryable, Harness};
use orr_bridge::PlayerSlot;
use orr_editor::{
    editor::input::Phase, game::EditorGame, Editor, EditorApp, HostSpec, Mode, Target,
};
use orr_fp::{FPVec3, FP};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::{Guid, Scene};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomRun, EXIT, INTERACT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::{Simulation, TickInputs};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    guids: BTreeMap<u32, Guid>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &text).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}
        })).unwrap()).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap();
        let loaded =
            model_bindings::load_asset(&root, "sample-imported-scene", "foreground.glb").unwrap();
        let binding = Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let scene = PreparedScene::parse(&text).unwrap();
        let mut guids = BTreeMap::new();
        for entity in scene.frame().entities() {
            let kind = scene.frame().get::<RoomActor>(entity).unwrap().kind;
            if [PLAYER, KEY, EXIT].contains(&kind) {
                guids.insert(kind, scene.index().guid(entity).unwrap().clone());
            }
        }
        let document = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: [PLAYER, KEY]
                .into_iter()
                .map(|kind| (guids[&kind].to_string(), binding.clone()))
                .collect(),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            guids,
        }
    }
    fn app(&self, gpu: bool) -> Harness<'static, EditorApp> {
        let prepared = PreparedProject::open(&self.root).unwrap();
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
        let builder = Harness::builder()
            .with_size([1400.0, 1000.0])
            .with_step_dt(1.0 / 60.0);
        if gpu {
            builder.wgpu().build_eframe(move |cc| {
                let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app
            })
        } else {
            builder.build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app
            })
        }
    }
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
}
fn wait(h: &mut Harness<'_, EditorApp>, description: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        settle(h);
        if ready(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out: {description}; phase={:?}; hint={}; log={:?}",
            h.state().editor.input_phase(),
            h.state().editor.input_hint(),
            h.state().editor.log()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn key(h: &mut Harness<'_, EditorApp>, key: egui::Key, pressed: bool) {
    h.event(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}
fn run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}
fn move_actor(h: &mut Harness<'_, EditorApp>, guid: &Guid, position: FPVec3) {
    h.state_mut()
        .editor
        .select(Some(Target::Guid(guid.clone())));
    settle(h);
    assert!(h
        .state_mut()
        .editor
        .set_yard_transform(position, [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE]));
    settle(h);
}
fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("room-workflow-frame-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(Instant::now() < deadline, "matching frame bytes missing");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn authored_xyz_models_save_reopen_real_keyboard_win_restart_and_stop() {
    let fixture = Fixture::new();
    let mut h = fixture.app(false);
    settle(&mut h);
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    // Change all three authored key coordinates and move the exit into a short W route.
    move_actor(
        &mut h,
        &fixture.guids[&KEY],
        FPVec3::new(FP::HALF, FP::ONE, -FP::HALF),
    );
    move_actor(
        &mut h,
        &fixture.guids[&EXIT],
        FPVec3::new(FP::ZERO, FP::HALF, -FP::ONE),
    );
    h.state_mut()
        .editor
        .select(Some(Target::Guid(fixture.guids[&KEY].clone())));
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.package = "sample-imported-scene".into();
        app.models.asset = "foreground.glb".into();
        app.models.transform.scale = [0.75; 3];
        app.models.assign(&app.editor).unwrap();
        app.models.bindings.as_mut().unwrap().save().unwrap();
    }
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let authored = frame_bytes(&mut h.state_mut().editor);
    let checksum = h.state().editor.checksum();
    let doc = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let scene_bytes = fs::read(fixture.root.join("room.scene.yaml")).unwrap();
    let model_bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    drop(h);
    let mut h = fixture.app(false);
    settle(&mut h);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
    assert_eq!(h.state().models.bindings.as_ref().unwrap().document(), &doc);
    let entity = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(&fixture.guids[&KEY]))
        .unwrap()
        .entity;
    let placed = h.state().models.placements(&h.state().editor);
    let key_model = placed
        .iter()
        .find(|placement| placement.entity == entity)
        .unwrap();
    assert_eq!(key_model.instance.translation, [0.5, 1.0, -0.5]);
    assert_eq!(key_model.instance.scale, [0.75; 3]);
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "live play", |app| app.editor.can_take_control());
    key(&mut h, egui::Key::E, true);
    settle(&mut h);
    h.get_by_label("Take control").click();
    wait(&mut h, "managed claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let claimed_tick = h.state().editor.timeline().unwrap().tick;
    wait(&mut h, "held E remains neutral after claim", |app| {
        app.editor.timeline().unwrap().tick >= claimed_tick + 3
    });
    assert_eq!(
        run(h.state()).key_collected,
        0,
        "pre-claim held E is not a fresh interaction"
    );
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    wait(&mut h, "focus loss releases", |app| {
        app.editor.input_phase() == Phase::Off && !app.editor.input_cleanup_pending()
    });
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    key(&mut h, egui::Key::E, true);
    settle(&mut h);
    assert_eq!(
        h.state().editor.input_phase(),
        Phase::Off,
        "focus return is not control authorization"
    );
    h.get_by_label("Take control").click();
    wait(&mut h, "fresh managed claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let reclaimed_tick = h.state().editor.timeline().unwrap().tick;
    wait(&mut h, "held E remains neutral after reclaim", |app| {
        app.editor.timeline().unwrap().tick >= reclaimed_tick + 3
    });
    assert_eq!(
        run(h.state()).key_collected,
        0,
        "reclaim requires a fresh physical release and press"
    );
    key(&mut h, egui::Key::E, false);
    settle(&mut h);
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "key pickup", |app| run(app).key_collected == 1);
    assert_eq!(
        h.state().models.placements(&h.state().editor).len(),
        1,
        "collected GUID remains authored but is hidden"
    );
    assert_eq!(h.state().models.bindings.as_ref().unwrap().document(), &doc);
    key(&mut h, egui::Key::E, false);
    key(&mut h, egui::Key::W, true);
    wait(&mut h, "W maps to negative Z", |app| {
        app.models.placements(&app.editor)[0].instance.translation[2] < -0.2
    });
    key(&mut h, egui::Key::W, false);
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "exit interaction wins", |app| run(app).won == 1);
    key(&mut h, egui::Key::E, false);
    h.state_mut().editor.pause();
    settle(&mut h);
    let live = frame_bytes(&mut h.state_mut().editor);
    let stopped = h.state_mut().editor.stop().unwrap().clone();
    settle(&mut h);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
    assert_eq!(h.state().editor.checksum(), checksum);
    // Replay the actual asynchronously admitted GUI input samples through both
    // standalone runtime session and deterministic simulation, never guessed durations.
    let reader = orr_session::ReplayReader::<RoomEscapeV1>::parse(&stopped.replay).unwrap();
    let mut runtime = prepared.scene().session().unwrap();
    let mut sim = prepared.scene().simulation().unwrap();
    let mut saw_w = false;
    let mut saw_e = false;
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).unwrap();
        assert!(commands.is_empty());
        let input = inputs[0];
        saw_w |= input.move_z == -1;
        saw_e |= input.buttons & INTERACT != 0;
        let mut samples = TickInputs::new(tick, 1);
        samples.set_input(PlayerSlot(0), input);
        sim.step(&samples);
        runtime.set_input(PlayerSlot(0), input);
        assert!(runtime.step_now().is_some());
        assert_eq!(sim.frame().to_bytes(), runtime.frame().to_bytes());
        if let Some((_, expected)) = reader.checksums.iter().find(|(at, _)| *at == sim.tick()) {
            assert_eq!(sim.frame().checksum(), *expected);
        }
    }
    assert!(saw_w && saw_e);
    assert_eq!(runtime.frame().to_bytes(), live);
    assert_eq!(runtime.frame().checksum(), stopped.checksum);
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "second play", |app| app.editor.can_take_control());
    h.get_by_label("Take control").click();
    wait(&mut h, "second managed claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "second key pickup", |app| {
        run(app).key_collected == 1
    });
    key(&mut h, egui::Key::E, false);
    wait(&mut h, "interaction released", |app| {
        run(app).previous_buttons == 0
    });
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "second exit win", |app| run(app).won == 1);
    key(&mut h, egui::Key::E, false);
    h.state_mut().editor.pause();
    settle(&mut h);
    let before_restart = h.state().editor.timeline().unwrap().tick;
    assert!(before_restart > 0);
    let old_sequence = h.state().editor.snapshot().unwrap().seq();
    h.get_by_label("Restart room").click();
    wait(&mut h, "restart live frame", |app| {
        app.editor.mode() == Mode::Play
            && app.editor.can_take_control()
            && app.editor.yard_rows_coherent()
            && app.editor.snapshot().is_some_and(|snapshot| {
                snapshot.seq() > old_sequence
                    && snapshot.timeline().is_some_and(|timeline| timeline.playing)
                    && *snapshot.predicted().singleton::<RoomRun>() == RoomRun::default()
            })
    });
    assert_eq!(
        h.state().editor.last_stopped().unwrap().tick,
        before_restart,
        "Restart room must really stop the won session before starting another"
    );
    assert_eq!(run(h.state()), RoomRun::default());
    h.state_mut().editor.pause();
    settle(&mut h);
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    assert_eq!(run(h.state()), RoomRun::default());
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
    assert_eq!(
        fs::read(fixture.root.join("room.scene.yaml")).unwrap(),
        scene_bytes
    );
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        model_bytes
    );
}

fn capture(h: &Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    let gpu = h.state().viewport3d_gpu().expect("production 3D viewport");
    let pixels = gpu.gpu().read_rgba8();
    let size = h.state().ui.viewport_px;
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != &pixels[..4])
            .count()
            > 100,
        "visible geometry required"
    );
    if let Some(directory) = std::env::var_os("ORR_ROOM_CAPTURE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let output = fs::File::create(Path::new(&directory).join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(output, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }
    pixels
}
#[test]
#[ignore = "explicit production editor GPU acceptance; requires ORR_REQUIRE_GPU=1"]
fn production_room_viewport_native_texture_has_imported_uv_model_contribution() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let fixture = Fixture::new();
    let mut h = fixture.app(true);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let checksum = h.state().editor.checksum();
    let frame = frame_bytes(&mut h.state_mut().editor);
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 2);
    assert!(placements
        .iter()
        .flat_map(|p| &p.model.source().primitives)
        .flat_map(|p| &p.vertices)
        .any(|v| v.uv[0] > 0.0 || v.uv[1] > 0.0));
    h.state_mut()
        .editor
        .select(Some(Target::Guid(fixture.guids[&KEY].clone())));
    settle(&mut h);
    let before = capture(&h, "room-editor-bound");
    let size = h.state().ui.viewport_px;
    let composed_before = h.render().expect("native viewport texture composition");
    for kind in [PLAYER, KEY] {
        h.state_mut()
            .editor
            .select(Some(Target::Guid(fixture.guids[&kind].clone())));
        settle(&mut h);
        let app = h.state_mut();
        app.models.package = "sample-imported-scene".into();
        app.models.asset = "foreground.glb".into();
        app.models.transform.translation = [1000.0; 3];
        app.models.assign(&app.editor).unwrap();
    }
    settle(&mut h);
    let after = capture(&h, "room-editor-models-offscreen");
    assert_eq!(h.state().ui.viewport_px, size);
    assert!(
        before
            .as_chunks::<4>()
            .0
            .iter()
            .zip(after.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count()
            > 20,
        "imported model must contribute visible pixels"
    );
    let composed_after = h.render().expect("native viewport after model change");
    let rect = h.state().ui.viewport_rect.unwrap().shrink(4.0);
    assert!(
        composed_after
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                let point = egui::pos2(*x as f32, *y as f32);
                rect.contains(point)
                    && point.y > rect.min.y + 40.0
                    && *p != composed_before.get_pixel(*x, *y)
            })
            .count()
            > 20,
        "registered texture must change inside the actual main viewport"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), frame);
}

#[test]
fn legacy_game_routing_remains_explicit_with_room_feature() {
    assert!(matches!(
        HostSpec::local("legacy.scene.yaml"),
        HostSpec::Local { .. }
    ));
    assert!(!EditorGame::PhysGame.is_room());
    assert!(!EditorGame::Arena.is_room());
    assert!(!EditorGame::Yard3D.is_room());
}

#[test]
fn room_camera_projection_restart_and_open_lifecycle_are_coherent() {
    let f=Fixture::new();
    let mut h=f.app(false);settle(&mut h);
    let doc=orr_sample::room_camera::Document::readable_default();
    let initial=h.state().editor.snapshot().unwrap().predicted().checksum();
    h.state_mut().editor.install_room_camera(doc.clone()).unwrap();
    for size in [(1,8192),(8192,1),(640,960),(1024,768)] {
        assert_eq!(h.state().editor.presentation_camera3d(size).unwrap(),doc.camera(&doc.orbit(),size).unwrap());
    }
    h.state_mut().editor.pan3d([40.0,20.0],(1024,768));
    h.state_mut().editor.zoom3d(2.0);
    h.state_mut().editor.orbit3d([10.0,20.0]);
    assert_ne!(h.state().editor.presentation_camera3d((1024,768)).unwrap(),doc.camera(&doc.orbit(),(1024,768)).unwrap());
    assert!(h.state_mut().editor.restart());settle(&mut h);
    assert_eq!(h.state().editor.presentation_camera3d((1024,768)).unwrap(),doc.camera(&doc.orbit(),(1024,768)).unwrap());
    assert_eq!(h.state().editor.snapshot().unwrap().predicted().checksum(),initial);
    let mut invalid=doc.clone();invalid.target[0]=f32::NAN;
    assert!(h.state_mut().editor.install_room_camera(invalid).is_err());
    assert!(h.state().editor.has_room_camera());
    assert!(!h.state_mut().editor.open_path(&f.root.join("missing.yaml")));
    assert!(h.state().editor.has_room_camera());
    let a=f.root.join("room.scene.yaml");let b=f.root.join("other.scene.yaml");
    fs::copy(&a,&b).unwrap();
    assert!(h.state_mut().editor.open_path(&b));settle(&mut h);
    assert!(!h.state().editor.has_room_camera());
    assert!(h.state_mut().editor.open_path(&a));settle(&mut h);
    assert!(!h.state().editor.has_room_camera(),"scene Open does not read project camera");
    h.state_mut().editor.install_room_camera(doc.clone()).unwrap();
    assert!(h.state_mut().editor.open_path(&a));settle(&mut h);
    assert!(!h.state().editor.has_room_camera(),"same-path scene Open also clears admitted presentation");
    h.state_mut().editor.install_room_camera(doc).unwrap();
    assert!(h.state_mut().editor.save_as(&f.root.join("saved.scene.yaml")));settle(&mut h);
    assert!(!h.state().editor.has_room_camera());
}

#[test]
#[ignore = "explicit authored-camera production GPU acceptance"]
fn authored_camera_viewport_projection_picking_and_invalid_preservation() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(),Ok("1"));
    let f=Fixture::new();let mut h=f.app(true);settle(&mut h);
    let doc=orr_sample::room_camera::Document::readable_default();
    h.state_mut().editor.install_room_camera(doc.clone()).unwrap();settle(&mut h);
    let before=capture(&h,"authored-camera-landscape");
    let checksum=h.state().editor.checksum();
    let size=h.state().ui.viewport_px;
    let camera=h.state().editor.presentation_camera3d(size).unwrap();
    assert_eq!(camera,doc.camera(&doc.orbit(),size).unwrap());
    let snapshot=h.state().editor.snapshot().unwrap();
    let frame=snapshot.predicted();
    let player=frame.iter::<RoomActor>().find(|(_,a)|a.kind==PLAYER).unwrap().0;
    let center=orr_sample::room_view::items(frame).into_iter().find(|item|item.entity==player).unwrap().transform.pos.to_array();
    let screen=camera.world_to_screen(center,size).unwrap();
    assert!(screen[0]>0.0&&screen[0]<size.0 as f32&&screen[1]>0.0&&screen[1]<size.1 as f32);
    assert_eq!(h.state().editor.pick3d(screen,size),Some(Target::Guid(f.guids[&PLAYER].clone())));
    let mut invalid=doc.clone();invalid.distance=0.0;
    assert!(h.state_mut().editor.install_room_camera(invalid).is_err());settle(&mut h);
    assert_eq!(capture(&h,"authored-camera-invalid-retained"),before);
    assert_eq!(h.state().editor.checksum(),checksum);
    h.state_mut().editor.orbit3d([30.0,10.0]);h.state_mut().editor.zoom3d(1.0);settle(&mut h);
    assert_ne!(capture(&h,"authored-camera-manual"),before);
    h.state_mut().editor.reset_room_camera();settle(&mut h);
    assert_eq!(capture(&h,"authored-camera-reset"),before);
    // A failed effective projection retains the last native image and must
    // not map clicks through a rejected camera onto that retained texture.
    h.state_mut().editor.select(None);
    h.state_mut().editor.camera3d.distance=f32::NAN;
    settle(&mut h);
    assert_eq!(capture(&h,"authored-camera-projection-error"),before);
    assert_eq!(h.state().editor.pick3d(screen,size),None);
    let rect=h.state().ui.viewport_rect.unwrap();
    let pos=egui::pos2(rect.min.x+screen[0]/size.0 as f32*rect.width(),rect.min.y+screen[1]/size.1 as f32*rect.height());
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true,false] {
        h.event(egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE});
        h.run_steps(2);
    }
    assert!(h.state().editor.selection().is_none());
    h.state_mut().editor.reset_room_camera();settle(&mut h);
    h.set_size(egui::vec2(1000.0,1500.0));settle(&mut h);
    let portrait=capture(&h,"authored-camera-portrait");
    let portrait_size=h.state().ui.viewport_px;
    assert!(portrait_size.1>portrait_size.0);
    let projected=h.state().editor.presentation_camera3d(portrait_size).unwrap();
    assert_eq!(projected,doc.camera(&doc.orbit(),portrait_size).unwrap());
    let pixel=projected.world_to_screen(center,portrait_size).unwrap();
    assert_eq!(h.state().editor.pick3d(pixel,portrait_size),Some(Target::Guid(f.guids[&PLAYER].clone())));
    assert!(portrait.len()>100);
    h.set_size(egui::vec2(1400.0,1000.0));settle(&mut h);
    assert_eq!(capture(&h,"authored-camera-resize-return"),before);
}

#[test]
fn external_erp_load_retires_authored_camera_without_reactivation() {
    let f=Fixture::new();let mut h=f.app(false);settle(&mut h);
    let doc=orr_sample::room_camera::Document::readable_default();
    h.state_mut().editor.install_room_camera(doc.clone()).unwrap();
    let a=f.root.join("room.scene.yaml");let b=f.root.join("external.scene.yaml");
    let text=fs::read_to_string(&a).unwrap();
    h.state_mut().editor.agent_client("camera-lifecycle-acceptance").unwrap().call("scene.load",serde_json::json!({"text":text,"path":a.display().to_string()})).unwrap();
    wait(&mut h,"external same-path camera retirement",|app|!app.editor.has_room_camera());
    h.state_mut().editor.install_room_camera(doc).unwrap();
    h.state_mut().editor.agent_client("camera-lifecycle-acceptance").unwrap().call("scene.save",serde_json::json!({"write":true,"path":b.display().to_string()})).unwrap();
    wait(&mut h,"external save-as camera retirement",|app|!app.editor.has_room_camera());
    h.state_mut().editor.agent_client("camera-lifecycle-acceptance").unwrap().call("scene.load",serde_json::json!({"text":text,"path":a.display().to_string()})).unwrap();
    settle(&mut h);assert!(!h.state().editor.has_room_camera());
}


/// Taps are delivered in one real egui input frame, then replayed through
/// the unmodified deterministic runtime. No direct interaction API is used.
#[test]
fn room_same_frame_interact_taps_collect_and_win_with_exact_replay() {
    for tap_count in [2, 5] {
        let fixture = Fixture::new();
        let mut h = fixture.app(false);
        settle(&mut h);
        move_actor(&mut h, &fixture.guids[&KEY], FPVec3::new(FP::HALF, FP::ONE, FP::ZERO));
        move_actor(&mut h, &fixture.guids[&EXIT], FPVec3::new(-FP::HALF, FP::ONE, FP::ZERO));
        assert!(h.state_mut().editor.save());
        settle(&mut h);
        let authored = PreparedScene::parse(&fs::read_to_string(fixture.root.join("room.scene.yaml")).unwrap()).unwrap();
        h.get_by_label(orr_editor::app::LBL_PLAY).click();
        wait(&mut h, "live play", |app| app.editor.can_take_control());
        h.get_by_label("Take control").click();
        wait(&mut h, "managed claim", |app| app.editor.input_phase() == Phase::Active);
        for _ in 0..tap_count {
            key(&mut h, egui::Key::E, true);
            key(&mut h, egui::Key::E, false);
        }
        wait(&mut h, "two queued taps collect key then exit", |app| run(app).won == 1);
        assert_eq!(run(h.state()).key_collected, 1);
        h.state_mut().editor.pause();
        settle(&mut h);
        let live = frame_bytes(&mut h.state_mut().editor);
        let stopped = h.state_mut().editor.stop().unwrap().clone();
        settle(&mut h);
        assert_eq!(h.state().editor.checksum(), authored.frame().checksum());
        let reader = orr_session::ReplayReader::<RoomEscapeV1>::parse(&stopped.replay).unwrap();
        let mut sim = authored.simulation().unwrap();
        let mut presses = 0;
        let mut held = false;
        for tick in 1..=reader.last_tick() {
            let (inputs, commands) = reader.tick(tick).unwrap();
            assert!(commands.is_empty());
            let down = inputs[0].buttons & INTERACT != 0;
            presses += u32::from(down && !held);
            held = down;
            let mut samples = TickInputs::new(tick, 1);
            samples.set_input(PlayerSlot(0), inputs[0]);
            sim.step(&samples);
            if let Some((_, expected)) = reader.checksums.iter().find(|(at, _)| *at == sim.tick()) {
                assert_eq!(sim.frame().checksum(), *expected);
            }
        }
        assert_eq!(presses, 2, "two physical taps become exactly two deterministic rising edges");
        assert_eq!(sim.frame().to_bytes(), live);
        assert_eq!(sim.frame().checksum(), stopped.checksum);
        // Stop/restart does not replay any queued or already-consumed interaction.
        h.get_by_label(orr_editor::app::LBL_PLAY).click();
        wait(&mut h, "second live play", |app| app.editor.can_take_control());
        h.get_by_label("Take control").click();
        wait(&mut h, "second claim", |app| app.editor.input_phase() == Phase::Active);
        let tick = h.state().editor.timeline().unwrap().tick;
        wait(&mut h, "neutral restarted frames", |app| app.editor.timeline().unwrap().tick >= tick + 4);
        assert_eq!(run(h.state()).key_collected, 0);
        assert_eq!(run(h.state()).won, 0);
        h.state_mut().editor.stop().unwrap();
    }
}

#[test]
fn room_interact_taps_are_suppressed_by_focus_loss_escape_and_dialog_capture() {
    for gate in ["focus", "escape", "dialog"] {
        let fixture = Fixture::new();
        let mut h = fixture.app(false);
        settle(&mut h);
        h.get_by_label(orr_editor::app::LBL_PLAY).click();
        wait(&mut h, "live play", |app| app.editor.can_take_control());
        h.get_by_label("Take control").click();
        wait(&mut h, "managed claim", |app| app.editor.input_phase() == Phase::Active);
        match gate {
            "focus" => {
                h.input_mut().focused = false;
                h.event(egui::Event::WindowFocused(false));
            }
            "escape" => { key(&mut h, egui::Key::Escape, true); }
            "dialog" => {
                h.state_mut().ui.dialog = Some(orr_editor::app::Dialog {
                    kind: orr_editor::app::DialogKind::SaveAs,
                    text: String::new(),
                });
            }
            _ => unreachable!(),
        }
        for _ in 0..3 {
            key(&mut h, egui::Key::E, true);
            key(&mut h, egui::Key::E, false);
        }
        wait(&mut h, "gate releases and cancels interaction", |app| {
            app.editor.input_phase() == Phase::Off && !app.editor.input_cleanup_pending()
        });
        assert_eq!(run(h.state()).key_collected, 0, "{gate} must suppress E events");
        h.input_mut().focused = true;
        h.event(egui::Event::WindowFocused(true));
        key(&mut h, egui::Key::Escape, false);
        if gate == "dialog" {
            h.get_by_label("Cancel").click();
        }
        settle(&mut h);
        assert_eq!(h.state().editor.input_phase(), Phase::Off);
        h.get_by_label("Take control").click();
        wait(&mut h, "explicit reclaimed control", |app| app.editor.input_phase() == Phase::Active);
        let tick = h.state().editor.timeline().unwrap().tick;
        wait(&mut h, "neutral after capture", |app| app.editor.timeline().unwrap().tick >= tick + 4);
        assert_eq!(run(h.state()).key_collected, 0, "{gate} events cannot leak into reclaim");
        key(&mut h, egui::Key::E, true);
        key(&mut h, egui::Key::E, false);
        wait(&mut h, "fresh authorized tap", |app| run(app).key_collected == 1);
        h.state_mut().editor.stop().unwrap();
    }
}
