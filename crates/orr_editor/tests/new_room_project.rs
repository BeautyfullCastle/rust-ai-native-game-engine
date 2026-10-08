//! New generated room → actual EditorApp authoring, save/reopen, keyboard play and restart.
//! cargo test -p orr_editor --features room-project,project-create --test new_room_project
//! Explicit GPU: ORR_REQUIRE_GPU=1 with -- --ignored --exact generated_room_gpu_uv_models
#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{Harness, kittest::Queryable};
use orr_bridge::PlayerSlot;
use orr_editor::{
    Editor, EditorApp, HostSpec, Mode, Target, editor::input::Phase, game::EditorGame,
};
use orr_fp::{FP, FPVec3};
use orr_reflect::Guid;
use orr_sample::{
    project_create::{CreateOptions, create},
    room_game::{EXIT, INTERACT, KEY, PLAYER, RoomActor, RoomEscapeV1, RoomRun},
    room_project::{PreparedProject, PreparedScene},
};
use orr_sim::TickInputs;
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
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("new generated room");
        let report = create(&CreateOptions {
            output: root.clone(),
            template: "room-escape-3d-v1".into(),
            seed: "editor-room-acceptance".into(),
        })
        .unwrap();
        let text = fs::read_to_string(root.join("room.scene.yaml")).unwrap();
        let scene = PreparedScene::parse(&text).unwrap();
        assert_eq!(scene.frame().checksum(), report.initial_checksum);
        let mut guids = BTreeMap::new();
        for entity in scene.frame().entities() {
            let kind = scene.frame().get::<RoomActor>(entity).unwrap().kind;
            if [PLAYER, KEY, EXIT].contains(&kind) {
                guids.insert(kind, scene.index().guid(entity).unwrap().clone());
            }
        }
        Self {
            _temp: temp,
            root,
            guids,
        }
    }
    fn app(&self, gpu: bool) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.root).unwrap();
        let camera=prepared.take_camera().expect("generated template declares camera");
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
        let builder = Harness::builder()
            .with_size([1400.0, 1000.0])
            .with_step_dt(1.0 / 60.0);
        if gpu {
            builder.wgpu().build_eframe(move |cc| {
                let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera=Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
            })
        } else {
            builder.build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app.room_camera=Some(orr_editor::room_camera_panel::Panel::new(camera));
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
    assert!(
        h.state_mut()
            .editor
            .set_yard_transform(position, [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE])
    );
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
fn generated_room_editor_save_reopen_keyboard_win_restart() {
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
fn generated_room_gpu_uv_models() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let fixture = Fixture::new();
    let mut h = fixture.app(true);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let checksum = h.state().editor.checksum();
    let frame = frame_bytes(&mut h.state_mut().editor);
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 2);
    assert!(
        placements
            .iter()
            .flat_map(|p| &p.model.source().primitives)
            .flat_map(|p| &p.vertices)
            .any(|v| v.uv[0] > 0.0 || v.uv[1] > 0.0)
    );
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
