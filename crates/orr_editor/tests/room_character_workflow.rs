//! New generated room → actual EditorApp authoring, save/reopen, keyboard play and restart.
//! cargo test -p orr_editor --features room-project,project-create --test new_room_project
//! Explicit GPU: ORR_REQUIRE_GPU=1 with -- --ignored --exact generated_room_gpu_uv_models
#![cfg(all(
    feature = "room-character",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{kittest::Queryable, Harness};

use orr_editor::{
    editor::input::Phase, game::EditorGame, Editor, EditorApp, HostSpec, Mode, Target,
};
use orr_fp::{FPVec3, FP};
use orr_reflect::Guid;
use orr_sample::{
    project_create::{create, CreateOptions},
    room_game::{RoomActor, RoomRun, EXIT, KEY, PLAYER},
    room_project::{PreparedProject, PreparedScene},
};

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
            template: "room-escape-character-3d-v1".into(),
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
        let mut prepared = PreparedProject::open_with_capabilities(
            &self.root,
            false,
            orr_sample::room_project::CheckpointSupport::Disabled,
            true,
        )
        .unwrap();
        let camera = prepared
            .take_camera()
            .expect("generated template declares camera");
        let character = prepared.take_character().unwrap();
        let (_, path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor
            .install_room_character(character.document.clone())
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
                app.room_character = Some(orr_editor::room_character_panel::Panel::new(character));
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
            })
        } else {
            builder.build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app.room_character = Some(orr_editor::room_character_panel::Panel::new(character));
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
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

fn poses(h: &Harness<'_, EditorApp>) -> Vec<orr_model::animation::Matrix4> {
    h.state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap()[0]
        .pose
        .global()
        .to_vec()
}
fn workflow(gpu: bool) {
    let fixture = Fixture::new();
    let mut h = fixture.app(gpu);
    settle(&mut h);
    let initial = frame_bytes(&mut h.state_mut().editor);
    h.state_mut()
        .editor
        .select(Some(Target::Guid(fixture.guids[&PLAYER].clone())));
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.kind = orr_model_bindings::model_bindings::ModelKind::Animated;
        app.models.package = "sample-room-character".into();
        app.models.asset = "courier.orrmodel.json".into();
        app.models.animation = orr_model_bindings::model_bindings::AnimationDescriptor {
            clip_index: 0,
            playback: orr_model_bindings::model_bindings::PlaybackMode::Loop,
        };
        app.models.transform.scale = [1.1; 3];
    }
    h.get_by_label("Static model binding").click();
    settle(&mut h);
    h.get_by_label("Load model asset").click();
    settle(&mut h);
    h.get_by_label("Assign animated model").click();
    settle(&mut h);
    h.get_by_label("Static model binding").click();
    settle(&mut h);
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&fixture.guids[&PLAYER].to_string()]
            .transform
            .scale,
        [1.1; 3]
    );
    let original = h.state().editor.room_character().unwrap().clone();
    let original_models = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let rest = poses(&h);
    if gpu {
        capture(&h, "character-rest");
    }
    {
        let app = h.state_mut();
        let panel = app.room_character.as_mut().unwrap();
        let mut invalid = original.clone();
        invalid.carrying = 31;
        assert!(panel
            .apply_document(invalid, &mut app.editor, &mut app.models)
            .is_err());
        assert_eq!(panel.document(), &original);
        assert_eq!(
            app.models.bindings.as_ref().unwrap().document(),
            &original_models
        );
    }
    h.get_by_label("Room character").click();
    settle(&mut h);
    for (label, clip) in [
        ("Searching clip", 1),
        ("Carrying key clip", 2),
        ("Escaped clip", 0),
    ] {
        h.get_by_label(label).click();
        h.run_steps(2);
        h.get_by_label(format!("{label} {clip}").as_str()).click();
        h.run_steps(2);
    }
    h.get_by_label("Apply character clips").click();
    settle(&mut h);
    assert_eq!(h.state().editor.room_character().unwrap().searching, 1);
    assert_eq!(h.state().editor.room_character().unwrap().carrying, 2);
    assert_eq!(h.state().editor.room_character().unwrap().escaped, 0);
    h.get_by_label("Undo character edit").click();
    settle(&mut h);
    assert_eq!(h.state().editor.room_character().unwrap(), &original);
    h.get_by_label("Redo character edit").click();
    settle(&mut h);
    assert_eq!(h.state().editor.room_character().unwrap().searching, 1);
    h.get_by_label("Save character and model bindings").click();
    settle(&mut h);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), initial);
    drop(h);
    let mut h = fixture.app(gpu);
    settle(&mut h);
    assert_eq!(h.state().editor.room_character().unwrap().searching, 1);
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
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let authored = frame_bytes(&mut h.state_mut().editor);
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "live play", |app| app.editor.can_take_control());
    h.get_by_label("Take control").click();
    wait(&mut h, "claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    assert_eq!(
        orr_sample::room_character::state(h.state().editor.snapshot().unwrap().predicted()),
        orr_sample::room_character::State::Searching
    );
    let searching = poses(&h);
    assert_ne!(searching, rest);
    if gpu {
        capture(&h, "character-searching");
    }
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "key pickup", |app| run(app).key_collected == 1);
    assert_eq!(
        orr_sample::room_character::state(h.state().editor.snapshot().unwrap().predicted()),
        orr_sample::room_character::State::Carrying
    );
    assert_eq!(
        h.state().models.placements(&h.state().editor).len(),
        0,
        "only hidden key is static"
    );
    if gpu {
        capture(&h, "character-carrying");
    }
    key(&mut h, egui::Key::E, false);
    key(&mut h, egui::Key::W, true);
    wait(&mut h, "move near exit", |app| {
        app.models.animated_placements(&app.editor).unwrap()[0].instance[3][2] < -0.2
    });
    key(&mut h, egui::Key::W, false);
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "escape", |app| run(app).won == 1);
    key(&mut h, egui::Key::E, false);
    assert_eq!(
        orr_sample::room_character::state(h.state().editor.snapshot().unwrap().predicted()),
        orr_sample::room_character::State::Escaped
    );
    if gpu {
        capture(&h, "character-escaped");
    }
    h.state_mut().editor.pause();
    settle(&mut h);
    let paused = poses(&h);
    settle(&mut h);
    assert_eq!(poses(&h), paused);
    h.state_mut().editor.seek(0);
    wait(&mut h, "seek zero", |app| {
        app.editor.timeline().is_some_and(|t| t.tick == 0)
    });
    assert_eq!(run(h.state()), RoomRun::default());
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
    let zero = poses(&h);
    settle(&mut h);
    assert_eq!(poses(&h), zero);
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "stop", |app| {
        app.editor.mode() == Mode::Edit
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_none())
    });
    assert_eq!(poses(&h), rest);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored);
}

fn capture(h: &Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    let gpu = h.state().viewport3d_gpu().expect("production 3D viewport");
    println!(
        "Room character editor GPU adapter: {}",
        gpu.gpu().adapter_name()
    );
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
fn character_edit_undo_redo_reopen_and_authoritative_play_states() {
    workflow(false);
}
#[test]
#[ignore = "explicit production editor GPU acceptance; requires ORR_REQUIRE_GPU=1"]
fn character_actual_editor_gpu_three_states() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    workflow(true);
}

#[test]
fn same_path_role_replacement_and_descriptor_retirement_fail_closed() {
    let fixture = Fixture::new();
    let mut h = fixture.app(false);
    settle(&mut h);
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let bindings = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let path = fixture.root.join("room.scene.yaml");
    let original = fs::read(&path).unwrap();
    let mut scene = orr_reflect::Scene::parse(
        std::str::from_utf8(&original).unwrap(),
        &orr_sample::room_project::types(),
    )
    .unwrap();
    let player_components = scene.entities[&fixture.guids[&PLAYER]].components.clone();
    let key_components = scene.entities[&fixture.guids[&KEY]].components.clone();
    scene
        .entities
        .get_mut(&fixture.guids[&PLAYER])
        .unwrap()
        .components = key_components;
    scene
        .entities
        .get_mut(&fixture.guids[&KEY])
        .unwrap()
        .components = player_components;
    // Canonical Room gameplay remains valid, but the persisted character GUID
    // now names KEY. This is a real scene replacement, not fabricated Frame flags.
    PreparedScene::parse(&scene.to_yaml()).unwrap();
    fs::write(&path, scene.to_yaml()).unwrap();
    assert!(h.state_mut().editor.restart());
    wait(&mut h, "same-path role replacement", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref() == Some(&fixture.guids[&PLAYER]))
                .is_some_and(|row| {
                    app.editor
                        .snapshot()
                        .unwrap()
                        .predicted()
                        .get::<RoomActor>(row.entity)
                        .is_some_and(|actor| actor.kind == KEY)
                })
    });
    let error = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .err()
        .expect("non-player character must reject");
    assert!(error.contains("current PLAYER"), "{error}");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    let mut renamed = orr_reflect::Scene::parse(
        std::str::from_utf8(&original).unwrap(),
        &orr_sample::room_project::types(),
    )
    .unwrap();
    let player = renamed.entities.remove(&fixture.guids[&PLAYER]).unwrap();
    renamed.entities.insert(
        Guid::parse("e_11111111111111111111111111111111").unwrap(),
        player,
    );
    PreparedScene::parse(&renamed.to_yaml()).unwrap();
    fs::write(&path, renamed.to_yaml()).unwrap();
    assert!(h.state_mut().editor.restart());
    wait(&mut h, "same-path player GUID replacement", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .rows()
                .iter()
                .all(|row| row.guid.as_ref() != Some(&fixture.guids[&PLAYER]))
    });
    let error = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .err()
        .expect("missing Room GUID must reject");
    assert!(error.contains("GUID is absent"), "{error}");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    fs::write(&path, &original).unwrap();
    assert!(h.state_mut().editor.open_path(&path));
    wait(&mut h, "same-path descriptor retirement", |app| {
        app.editor.yard_rows_coherent() && app.editor.room_character().is_none()
    });
    let error = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .err()
        .expect("no generic Yard fallback for Room");
    assert!(error.contains("installed character descriptor"), "{error}");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
}

#[test]
fn stale_room_character_binding_does_not_mutate_the_verified_cache() {
    let fixture = Fixture::new();
    let mut h = fixture.app(false);
    settle(&mut h);
    let frame = frame_bytes(&mut h.state_mut().editor);
    let model = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap()[0]
        .model
        .clone();
    let bindings = h.state().models.bindings.as_ref().unwrap();
    let path = bindings.path.clone();
    let original = bindings.document().clone();
    let mut stale = original.clone();
    stale
        .bindings
        .get_mut(&fixture.guids[&PLAYER].to_string())
        .unwrap()
        .source_hash = "0".repeat(64);
    h.state_mut().models.bindings = Some(
        orr_editor::model_bindings::Bindings::from_document(path.clone(), stale.clone()).unwrap(),
    );
    let error = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .err()
        .expect("stale binding must reject instead of showing a proxy");
    assert!(error.contains("binding/cache mismatch"), "{error}");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &stale
    );
    assert_eq!(frame_bytes(&mut h.state_mut().editor), frame);
    h.state_mut().models.bindings =
        Some(orr_editor::model_bindings::Bindings::from_document(path, original).unwrap());
    let restored = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&model, &restored[0].model));
}
