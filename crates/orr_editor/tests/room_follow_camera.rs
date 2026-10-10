//! EditorApp acceptance for authored Room follow camera and live frame routing.
#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    editor::input::Phase, game::EditorGame, Editor, EditorApp, HostSpec, Mode, Target,
};
use orr_fp::{FPVec3, FP};
use orr_model::MaterialOverride;
use orr_model_bindings::model_bindings::{self, Document as ModelDocument, ModelKind};
use orr_reflect::{Guid, Scene, SceneIndex};
use orr_sample::{
    project_create::{create, CreateOptions},
    room_camera::Document as CameraDocument,
    room_game::{RoomActor, RoomRun, EXIT, KEY, PLAYER},
    room_project::{CheckpointSupport, PreparedProject, PreparedScene},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

macro_rules! save_composed {
    ($image:expr, $label:expr) => {{
        if let Some(directory) = std::env::var_os("ORR_ROOM_FOLLOW_CAPTURE_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            let image = $image;
            let file =
                fs::File::create(directory.join(format!("{}-composed.png", $label))).unwrap();
            let mut encoder = png::Encoder::new(file, image.width(), image.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(image.as_raw()).unwrap();
            writer.finish().unwrap();
        }
    }};
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    guids: BTreeMap<u32, Guid>,
    index: SceneIndex,
    model_bytes: Vec<u8>,
    preserved_sidecars: BTreeMap<String, Vec<u8>>,
}
impl Fixture {
    fn new(template: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("generated follow room");
        create(&CreateOptions {
            output: root.clone(),
            template: template.into(),
            seed: "editor-follow-camera-acceptance".into(),
        })
        .unwrap();
        let loaded =
            model_bindings::load_asset(&root, "sample-imported-scene", "foreground.glb").unwrap();
        let slot = loaded.static_model().unwrap().source().primitives[0].material;
        let models_path = root.join("room.models.json");
        let mut models: ModelDocument =
            serde_json::from_slice(&fs::read(&models_path).unwrap()).unwrap();
        models.version = 3;
        let mut static_index = 0;
        for binding in models.bindings.values_mut() {
            if binding.kind == ModelKind::Static {
                let colors = [[0.18, 0.36, 0.72], [0.92, 0.16, 0.43]];
                binding.material_override = Some(MaterialOverride {
                    material_slot: slot,
                    base_color_factor: colors[static_index.min(1)],
                });
                static_index += 1;
            }
        }
        assert!(
            static_index > 0,
            "fixture needs at least one static material binding"
        );
        fs::write(&models_path, serde_json::to_vec_pretty(&models).unwrap()).unwrap();
        let model_bytes = fs::read(&models_path).unwrap();
        let mut preserved_sidecars =
            BTreeMap::from([("room.models.json".into(), model_bytes.clone())]);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("orr.project.json")).unwrap()).unwrap();
        if let Some(character) = manifest["entry"]["character"].as_str() {
            preserved_sidecars.insert(
                character.to_string(),
                fs::read(root.join(character)).unwrap(),
            );
        }
        let scene =
            PreparedScene::parse(&fs::read_to_string(root.join("room.scene.yaml")).unwrap())
                .unwrap();
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
            index: scene.index().clone(),
            model_bytes,
            preserved_sidecars,
        }
    }

    fn app(&self, gpu: bool) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open_with_capabilities(
            &self.root,
            false,
            CheckpointSupport::Disabled,
            cfg!(feature = "room-character"),
        )
        .unwrap();
        let camera = prepared.take_camera().expect("template camera sidecar");
        #[cfg(feature = "room-character")]
        let character = prepared.take_character();
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
        #[cfg(feature = "room-character")]
        if let Some(character) = character.as_ref() {
            editor
                .install_room_character(character.document.clone())
                .unwrap();
        }
        editor.install_room_camera(camera.document.clone()).unwrap();
        let builder = Harness::builder()
            .with_size([1400.0, 1000.0])
            .with_step_dt(1.0 / 60.0);
        if gpu {
            builder.wgpu().build_eframe(move |cc| {
                let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                #[cfg(feature = "room-character")]
                if let Some(character) = character {
                    app.room_character =
                        Some(orr_editor::room_character_panel::Panel::new(character));
                }
                app
            })
        } else {
            builder.build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                #[cfg(feature = "room-character")]
                if let Some(character) = character {
                    app.room_character =
                        Some(orr_editor::room_character_panel::Panel::new(character));
                }
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

fn type_number(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn assert_viewport_controls(h: &Harness<'_, EditorApp>, following: bool) {
    fn collect<'a>(shape: &'a egui::epaint::Shape, labels: &mut Vec<&'a str>) {
        match shape {
            egui::epaint::Shape::Text(text) => {
                let label = text.galley.text();
                if label.starts_with("Yard3D · collider-proxy picking") {
                    labels.push(label);
                }
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, labels);
                }
            }
            _ => {}
        }
    }
    let mut labels = Vec::new();
    for clipped in &h.output().shapes {
        collect(&clipped.shape, &mut labels);
    }
    let expected = if following {
        "Yard3D · collider-proxy picking · right-drag orbit · middle-drag pan disabled while following"
    } else {
        "Yard3D · collider-proxy picking · right-drag orbit · middle-drag pan"
    };
    assert_eq!(
        labels,
        [expected],
        "actual viewport controls must match installed camera mode"
    );
}

fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("room-follow-frame-probe").unwrap();
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

fn player_position(app: &EditorApp, index: &SceneIndex, guid: &Guid) -> [f32; 3] {
    let entity = index.entity(guid).expect("PLAYER GUID stays indexed");
    let snapshot = app.editor.snapshot().unwrap();
    let position = orr_sample::yard3d_view::editor_body_poses(snapshot.predicted())
        .into_iter()
        .find(|(candidate, _)| *candidate == entity)
        .expect("current player pose")
        .1
        .pos;
    [position.x, position.y, position.z]
}

fn assert_camera_tracks_player(
    app: &EditorApp,
    index: &SceneIndex,
    guid: &Guid,
    offset: [f32; 3],
) -> [f32; 3] {
    let position = player_position(app, index, guid);
    let actual = app
        .editor
        .presentation_camera3d((1200, 800))
        .unwrap()
        .target;
    for axis in 0..3 {
        assert!(
            (actual[axis] - (position[axis] + offset[axis])).abs() < 0.00001,
            "camera axis {axis} was {:?}, PLAYER {:?} + {:?}",
            actual,
            position,
            offset
        );
    }
    actual
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

fn author_follow_through_widgets(h: &mut Harness<'_, EditorApp>, player_guid: &Guid) -> [f32; 3] {
    h.get_by_label("Room camera").click();
    h.run_steps(3);
    h.get_by_label("Follow PLAYER").click();
    h.run_steps(3);
    let following_label = format!("Following PLAYER {player_guid}");
    let _ = h.get_by_label(following_label.as_str());
    assert!(h
        .state()
        .editor
        .room_camera_document()
        .unwrap()
        .follow
        .is_none());
    assert_viewport_controls(h, false);
    // Candidates are isolated until Apply; the editor-owned document stays fixed.
    type_number(h, "Follow offset X", "1.25");
    type_number(h, "Follow offset Y", "-0.5");
    type_number(h, "Follow offset Z", "0.75");
    h.get_by_label("Apply camera").click();
    settle(h);
    let applied = h.state().editor.room_camera_document().unwrap().clone();
    let follow = applied.follow.as_ref().unwrap();
    assert_eq!(follow.player, player_guid.to_string());
    assert_eq!(follow.offset, [1.25, -0.5, 0.75]);
    assert_eq!(applied.schema, 2);
    assert_eq!(h.state().room_camera.as_ref().unwrap().document(), &applied);
    assert_viewport_controls(h, true);
    [1.25, -0.5, 0.75]
}

fn live_workflow(template: &str, gpu: bool) {
    let fixture = Fixture::new(template);
    let mut h = fixture.app(gpu);
    let capture_prefix = template.replace('-', "_");
    settle(&mut h);
    assert_viewport_controls(&h, false);
    let installed_models = h.state().models.bindings.as_ref().unwrap().document();
    assert_eq!(installed_models.version, 3);
    assert!(installed_models
        .bindings
        .values()
        .filter(|binding| binding.kind == ModelKind::Static)
        .all(|binding| binding.material_override.is_some()));
    let player_guid = fixture.guids[&PLAYER].clone();
    let before_camera_edit = frame_bytes(&mut h.state_mut().editor);
    let before_checksum = h.state().editor.checksum();
    let offset = author_follow_through_widgets(&mut h, &player_guid);
    assert_eq!(
        h.state().editor.room_camera_player_guid().unwrap(),
        player_guid.to_string()
    );
    let valid_document = h.state().editor.room_camera_document().unwrap().clone();
    let mut reassigned = valid_document.clone();
    reassigned.follow.as_mut().unwrap().player = fixture.guids[&KEY].to_string();
    assert!(h
        .state()
        .editor
        .validate_room_camera_document(&reassigned)
        .is_err());
    assert_eq!(
        h.state().editor.room_camera_document().unwrap(),
        &valid_document
    );
    let mut missing = valid_document.clone();
    missing.follow.as_mut().unwrap().player = "e_ffffffffffffffffffffffffffffffff".into();
    assert!(h
        .state()
        .editor
        .validate_room_camera_document(&missing)
        .is_err());
    let initial_view = if gpu {
        Some(capture(&h, &format!("{capture_prefix}-follow-initial")))
    } else {
        None
    };
    let initial_composed = if gpu {
        let image = h
            .render()
            .expect("compose initial follow-camera editor viewport");
        save_composed!(&image, format!("{capture_prefix}-follow-initial"));
        Some(image)
    } else {
        None
    };
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before_camera_edit);
    assert_eq!(h.state().editor.checksum(), before_checksum);
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);

    h.get_by_label("Undo camera edit").click();
    settle(&mut h);
    let undone = h.state().editor.room_camera_document().unwrap();
    assert_eq!(undone.schema, 1);
    assert!(undone.follow.is_none());
    assert_viewport_controls(&h, false);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before_camera_edit);
    let fixed_view = if gpu {
        Some(capture(&h, &format!("{capture_prefix}-fixed-negative")))
    } else {
        None
    };
    let fixed_composed = if gpu {
        let image = h.render().expect("compose fixed-camera negative control");
        save_composed!(&image, format!("{capture_prefix}-fixed-negative"));
        Some(image)
    } else {
        None
    };
    if let (Some(initial), Some(fixed), Some(rect)) = (
        initial_composed.as_ref(),
        fixed_composed.as_ref(),
        h.state().ui.viewport_rect,
    ) {
        let rect = rect.shrink(4.0);
        assert!(
            fixed
                .enumerate_pixels()
                .filter(|(x, y, pixel)| {
                    let point = egui::pos2(*x as f32, *y as f32);
                    rect.contains(point)
                        && point.y > rect.min.y + 40.0
                        && *pixel != initial.get_pixel(*x, *y)
                })
                .count()
                > 20,
            "fixed-camera control must change pixels inside the composed editor viewport"
        );
        assert_ne!(fixed_view.as_ref().unwrap(), initial_view.as_ref().unwrap());
    }
    h.get_by_label("Redo camera edit").click();
    settle(&mut h);
    assert_viewport_controls(&h, true);
    assert_eq!(
        h.state()
            .editor
            .room_camera_document()
            .unwrap()
            .follow
            .as_ref()
            .unwrap()
            .offset,
        offset
    );
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before_camera_edit);
    if gpu {
        assert_eq!(
            &capture(&h, &format!("{capture_prefix}-follow-redo")),
            initial_view.as_ref().unwrap(),
            "redo restores the exact source viewport pixels"
        );
        let image = h.render().expect("compose redone follow view");
        save_composed!(&image, format!("{capture_prefix}-follow-redo"));
        let rect = h.state().ui.viewport_rect.unwrap().shrink(4.0);
        let composed_difference = image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| {
                let point = egui::pos2(*x as f32, *y as f32);
                rect.contains(point)
                    && point.y > rect.min.y + 40.0
                    && *pixel != initial_composed.as_ref().unwrap().get_pixel(*x, *y)
            })
            .count();
        assert!(
            composed_difference < 20,
            "the same native viewport should compose to the same pixels apart from UI edges: {composed_difference}"
        );
    }
    h.get_by_label("Save camera").click();
    settle(&mut h);
    assert_eq!(
        CameraDocument::parse(&fs::read(fixture.root.join("room.camera.json")).unwrap()).unwrap(),
        h.state().editor.room_camera_document().unwrap().clone()
    );
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        fixture.model_bytes
    );
    for (name, bytes) in &fixture.preserved_sidecars {
        assert_eq!(
            fs::read(fixture.root.join(name)).unwrap().as_slice(),
            bytes.as_slice(),
            "camera authoring changed {name}"
        );
    }
    drop(h);

    // Reopen the generated project through its manifest and actual EditorApp widgets.
    let mut h = fixture.app(gpu);
    settle(&mut h);
    assert_eq!(
        h.state()
            .editor
            .room_camera_document()
            .unwrap()
            .follow
            .as_ref()
            .unwrap()
            .offset,
        offset
    );
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &serde_json::from_slice::<ModelDocument>(&fixture.model_bytes).unwrap()
    );
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);

    // Move the key and exit into a short, deterministic keyboard route. The player
    // remains the only gameplay entity whose camera target can change in Play.
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
    let authored_frame = frame_bytes(&mut h.state_mut().editor);
    let authored_checksum = h.state().editor.checksum();
    let authored_target =
        assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);

    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "live room play", |app| {
        app.editor.can_take_control()
    });
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    if gpu {
        let _ = capture(&h, &format!("{capture_prefix}-play-start"));
        let image = h.render().expect("compose play-start viewport");
        save_composed!(&image, format!("{capture_prefix}-play-start"));
    }
    h.get_by_label("Take control").click();
    wait(&mut h, "managed input claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "key collected", |app| {
        app.editor
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<RoomRun>()
            .key_collected
            == 1
    });
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    let key_view = if gpu {
        Some(capture(&h, &format!("{capture_prefix}-key-collected")))
    } else {
        None
    };
    if gpu {
        let image = h.render().expect("compose key-collected viewport");
        save_composed!(&image, format!("{capture_prefix}-key-collected"));
    }
    key(&mut h, egui::Key::E, false);
    key(&mut h, egui::Key::W, true);
    wait(&mut h, "player moved", |app| {
        player_position(app, &fixture.index, &player_guid)[2] < -0.2
    });
    let moved_target = assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    assert_ne!(
        moved_target, authored_target,
        "camera must follow the new frame immediately"
    );
    if gpu {
        let movement_view = capture(&h, &format!("{capture_prefix}-movement"));
        assert_ne!(movement_view, *key_view.as_ref().unwrap());
        let image = h.render().expect("compose moved-player viewport");
        save_composed!(&image, format!("{capture_prefix}-movement"));
    }
    key(&mut h, egui::Key::W, false);
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "exit reached", |app| {
        app.editor
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<RoomRun>()
            .won
            == 1
    });
    key(&mut h, egui::Key::E, false);
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    if gpu {
        let _ = capture(&h, &format!("{capture_prefix}-exit-won"));
        let image = h.render().expect("compose won exit viewport");
        save_composed!(&image, format!("{capture_prefix}-exit-won"));
    }

    h.state_mut().editor.pause();
    settle(&mut h);
    let tick_before_restart = h.state().editor.timeline().unwrap().tick;
    assert!(tick_before_restart > 0);
    h.get_by_label("Restart room").click();
    wait(&mut h, "room restarted", |app| {
        app.editor.mode() == Mode::Play
            && app.editor.can_take_control()
            && app.editor.snapshot().is_some_and(|snapshot| {
                snapshot.timeline().is_some_and(|timeline| timeline.playing)
                    && snapshot.predicted().singleton::<RoomRun>() == &RoomRun::default()
            })
    });
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    if gpu {
        let _ = capture(&h, &format!("{capture_prefix}-restart"));
        let image = h.render().expect("compose restarted viewport");
        save_composed!(&image, format!("{capture_prefix}-restart"));
    }
    h.state_mut().editor.pause();
    settle(&mut h);
    h.state_mut().editor.seek(0);
    wait(&mut h, "seek to zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0)
    });
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    if gpu {
        let _ = capture(&h, &format!("{capture_prefix}-seek-zero"));
        let image = h.render().expect("compose seek-zero viewport");
        save_composed!(&image, format!("{capture_prefix}-seek-zero"));
    }
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "stop returns to edit", |app| {
        app.editor.mode() == Mode::Edit
    });
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);
    if gpu {
        let _ = capture(&h, &format!("{capture_prefix}-stop"));
        let image = h.render().expect("compose stopped viewport");
        save_composed!(&image, format!("{capture_prefix}-stop"));
    }
}

#[test]
fn generated_room_follow_camera_widget_lifecycle_and_live_play_routes() {
    let templates: &[&str] = if cfg!(feature = "room-character") {
        &["room-escape-3d-v1", "room-escape-character-3d-v1"]
    } else {
        &["room-escape-3d-v1"]
    };
    for template in templates {
        live_workflow(template, false);
    }
}

fn capture(h: &Harness<'_, EditorApp>, label: &str) -> Vec<u8> {
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("production main viewport");
    eprintln!(
        "follow camera viewport adapter: {}",
        viewport.gpu().adapter_name()
    );
    let rgba = viewport.gpu().read_rgba8();
    let size = h.state().ui.viewport_px;
    assert_eq!(rgba.len(), (size.0 * size.1 * 4) as usize);
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| *p != &rgba[..4])
            .count()
            > 100,
        "main viewport must contain composed scene pixels"
    );
    if let Some(directory) = std::env::var_os("ORR_ROOM_FOLLOW_CAPTURE_DIR") {
        let path = PathBuf::from(directory);
        fs::create_dir_all(&path).unwrap();
        let file = fs::File::create(path.join(format!("{label}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
    }
    rgba
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink(), "fixture must not use symlinks");
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn install_fresh_panel(fixture: &Fixture, h: &mut Harness<'_, EditorApp>) {
    let mut prepared = PreparedProject::open_with_capabilities(
        &fixture.root,
        false,
        CheckpointSupport::Disabled,
        cfg!(feature = "room-character"),
    )
    .unwrap();
    let camera = prepared.take_camera().unwrap();
    h.state_mut()
        .editor
        .install_room_camera(camera.document.clone())
        .unwrap();
    h.state_mut().room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
    settle(h);
}

#[test]
fn camera_source_lifetime_restart_and_scene_replacement_fail_closed() {
    let fixture = Fixture::new("room-escape-3d-v1");
    let mut h = fixture.app(false);
    settle(&mut h);
    let player_guid = fixture.guids[&PLAYER].clone();
    let offset = author_follow_through_widgets(&mut h, &player_guid);
    h.get_by_label("Save camera").click();
    settle(&mut h);
    let camera = h.state().editor.room_camera_document().unwrap().clone();
    let scene_path = fixture.root.join("room.scene.yaml");
    let original_scene = fs::read(&scene_path).unwrap();
    let original_frame = frame_bytes(&mut h.state_mut().editor);

    // Failed Open is a no-op. Successful A -> B -> A opens rotate source identity,
    // so the original panel cannot revive when its path and content return.
    let token_a = h.state().editor.room_camera_source_token();
    assert!(!h
        .state_mut()
        .editor
        .open_path(&fixture.root.join("missing.scene.yaml")));
    settle(&mut h);
    assert!(h.state().editor.has_room_camera());
    assert!(Arc::ptr_eq(
        &token_a,
        &h.state().editor.room_camera_source_token()
    ));
    let path_b = fixture.root.join("room-b.scene.yaml");
    fs::copy(&scene_path, &path_b).unwrap();
    assert!(h.state_mut().editor.open_path(&path_b));
    settle(&mut h);
    assert!(!h.state().editor.has_room_camera());
    assert!(h.state().room_camera.is_none());
    assert_viewport_controls(&h, false);
    let token_b = h.state().editor.room_camera_source_token();
    assert!(!Arc::ptr_eq(&token_a, &token_b));
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert!(h.state_mut().editor.open_path(&scene_path));
    settle(&mut h);
    assert!(!h.state().editor.has_room_camera());
    let token_a_again = h.state().editor.room_camera_source_token();
    assert!(!Arc::ptr_eq(&token_a, &token_a_again));
    assert!(!Arc::ptr_eq(&token_b, &token_a_again));
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert!(
        h.state().room_camera.is_none(),
        "A to B to A does not revive the old panel"
    );

    // Reopening the complete project provides a new panel. A successful host
    // restart retains the authored follow document while retiring that panel token.
    install_fresh_panel(&fixture, &mut h);
    let token_before_restart = h.state().editor.room_camera_source_token();
    assert!(h.state_mut().editor.restart());
    settle(&mut h);
    assert!(h.state().editor.has_room_camera());
    assert_eq!(h.state().editor.room_camera_document().unwrap(), &camera);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    let token_after_restart = h.state().editor.room_camera_source_token();
    assert!(!Arc::ptr_eq(&token_before_restart, &token_after_restart));
    let _ = h.get_by_label("Camera source changed; reopen the complete project");

    // A replacement scene that moves the persisted player GUID onto KEY is a
    // valid Room scene, but the old follow binding must refuse to downgrade.
    let mut reassigned = Scene::parse(
        std::str::from_utf8(&original_scene).unwrap(),
        &orr_sample::room_project::types(),
    )
    .unwrap();
    let player_components = reassigned.entities[&fixture.guids[&PLAYER]]
        .components
        .clone();
    let key_components = reassigned.entities[&fixture.guids[&KEY]].components.clone();
    reassigned
        .entities
        .get_mut(&fixture.guids[&PLAYER])
        .unwrap()
        .components = key_components;
    reassigned
        .entities
        .get_mut(&fixture.guids[&KEY])
        .unwrap()
        .components = player_components;
    let reassigned_text = reassigned.to_yaml();
    PreparedScene::parse(&reassigned_text).unwrap();
    fs::write(&scene_path, reassigned_text).unwrap();
    let frame_before_rejected_restart = frame_bytes(&mut h.state_mut().editor);
    let retained_token = h.state().editor.room_camera_source_token();
    assert!(!h.state_mut().editor.restart());
    settle(&mut h);
    assert!(h.state().editor.has_room_camera());
    assert_eq!(h.state().editor.room_camera_document().unwrap(), &camera);
    assert_eq!(
        frame_bytes(&mut h.state_mut().editor),
        frame_before_rejected_restart
    );
    assert!(Arc::ptr_eq(
        &retained_token,
        &h.state().editor.room_camera_source_token()
    ));
    assert_camera_tracks_player(h.state(), &fixture.index, &player_guid, offset);

    // Removing/reassigning the GUID entirely is another invalid source lifetime.
    fs::write(&scene_path, &original_scene).unwrap();
    let mut missing = Scene::parse(
        std::str::from_utf8(&original_scene).unwrap(),
        &orr_sample::room_project::types(),
    )
    .unwrap();
    let player_entity = missing.entities.remove(&fixture.guids[&PLAYER]).unwrap();
    missing.entities.insert(
        Guid::parse("e_11111111111111111111111111111111").unwrap(),
        player_entity,
    );
    let missing_text = missing.to_yaml();
    PreparedScene::parse(&missing_text).unwrap();
    fs::write(&scene_path, missing_text).unwrap();
    assert!(!h.state_mut().editor.restart());
    settle(&mut h);
    assert!(h.state().editor.has_room_camera());
    assert_eq!(h.state().editor.room_camera_document().unwrap(), &camera);
    assert_eq!(
        frame_bytes(&mut h.state_mut().editor),
        frame_before_rejected_restart
    );
    fs::write(&scene_path, &original_scene).unwrap();
}

#[test]
fn camera_save_rejects_project_directory_inode_aba() {
    let fixture = Fixture::new("room-escape-3d-v1");
    let mut h = fixture.app(false);
    settle(&mut h);
    let player_guid = fixture.guids[&PLAYER].clone();
    let _ = author_follow_through_widgets(&mut h, &player_guid);
    h.get_by_label("Save camera").click();
    settle(&mut h);
    let camera_path = fixture.root.join("room.camera.json");
    let manifest_path = fixture.root.join("orr.project.json");
    let camera_before = fs::read(&camera_path).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();

    let renamed = fixture.root.with_file_name("original project directory");
    fs::rename(&fixture.root, &renamed).unwrap();
    copy_tree(&renamed, &fixture.root);
    assert_eq!(fs::read(&camera_path).unwrap(), camera_before);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
    h.get_by_label("Save camera").click();
    settle(&mut h);
    let _ = h.get_by_label("Camera project directory changed; reopen before saving");
    assert_eq!(fs::read(&camera_path).unwrap(), camera_before);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
}

#[test]
#[ignore = "requires explicit real GPU; captures actual EditorApp main viewport"]
fn generated_room_follow_camera_actual_viewport_gpu_static_and_character() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    live_workflow("room-escape-3d-v1", true);
    #[cfg(feature = "room-character")]
    live_workflow("room-escape-character-3d-v1", true);
}

#[test]
fn prepared_room_first_restart_reopens_disk_without_replacing_initial_admission() {
    let fixture = Fixture::new("room-escape-3d-v1");
    let scene_path = fixture.root.join("room.scene.yaml");
    let admitted_text = fs::read_to_string(&scene_path).unwrap();
    let admitted = PreparedScene::parse(&admitted_text).unwrap();
    let player_guid = fixture.guids[&PLAYER].clone();
    // Disk changes after project admission but before host startup. First start
    // must still consume its captured, validated bytes; reconnect must not.
    let mut replaced = Scene::parse(&admitted_text, &orr_sample::room_project::types()).unwrap();
    let actor = replaced.entities.remove(&player_guid).unwrap();
    replaced.entities.insert(
        Guid::parse("e_11111111111111111111111111111111").unwrap(),
        actor,
    );
    PreparedScene::parse(&replaced.to_yaml()).unwrap();
    fs::write(&scene_path, replaced.to_yaml()).unwrap();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: scene_path.clone(),
        text: admitted_text.clone(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    // Match main.rs: install the saved camera directly after Editor::start,
    // without a test-only sync that would mask the initial history invalidation.
    let mut camera = CameraDocument::readable_default();
    camera.schema = 2;
    camera.follow = Some(orr_sample::room_camera::Follow {
        player: player_guid.to_string(),
        offset: [0.25, 1.0, -0.5],
    });
    assert!(
        !editor.yard_rows_coherent(),
        "initial history retires startup rows"
    );
    editor.install_room_camera(camera.clone()).unwrap();
    assert!(editor.yard_rows_coherent());
    assert_eq!(
        editor.snapshot().unwrap().predicted().checksum(),
        admitted.frame().checksum()
    );
    assert_eq!(frame_bytes(&mut editor), admitted.frame().to_bytes());
    assert_eq!(editor.room_camera_document(), Some(&camera));
    let source = editor.room_camera_source_token();
    assert!(
        !editor.restart(),
        "first host restart must re-read the now-incompatible saved scene"
    );
    editor.sync();
    assert_eq!(frame_bytes(&mut editor), admitted.frame().to_bytes());
    assert_eq!(editor.room_camera_document(), Some(&camera));
    assert!(Arc::ptr_eq(&source, &editor.room_camera_source_token()));

    // Reconnect preserves the original bounded regular-file input contract.
    // Neither a large file nor a symlink is followed into a new host.
    fs::File::create(&scene_path)
        .unwrap()
        .set_len(orr_sample::room_project::MAX_SCENE_BYTES + 1)
        .unwrap();
    assert!(!editor.restart());
    assert_eq!(frame_bytes(&mut editor), admitted.frame().to_bytes());
    fs::remove_file(&scene_path).unwrap();
    let link_target = fixture.root.join("linked.scene.yaml");
    fs::write(&link_target, &admitted_text).unwrap();
    std::os::unix::fs::symlink(&link_target, &scene_path).unwrap();
    assert!(!editor.restart());
    assert_eq!(frame_bytes(&mut editor), admitted.frame().to_bytes());
    fs::remove_file(&scene_path).unwrap();

    // A valid changed source is reloaded on the next retry, with the same GUID
    // but a new authoritative position. The follow camera must move with it.
    let mut moved = Scene::parse(&admitted_text, &orr_sample::room_project::types()).unwrap();
    let body = &mut moved
        .entities
        .get_mut(&player_guid)
        .unwrap()
        .components
        .iter_mut()
        .find(|(name, _)| name == "orr_physics3d::Body")
        .unwrap()
        .1;
    let orr_reflect::Value::Struct(fields) = body else {
        panic!("reflected Body");
    };
    fields.iter_mut().find(|(name, _)| name == "pos").unwrap().1 =
        orr_reflect::Value::Vec3(FPVec3::new(FP::ONE, FP::HALF, FP::ZERO));
    let moved_text = moved.to_yaml();
    let moved_scene = PreparedScene::parse(&moved_text).unwrap();
    fs::write(&scene_path, moved_text).unwrap();
    assert!(editor.restart());
    editor.sync();
    assert_eq!(frame_bytes(&mut editor), moved_scene.frame().to_bytes());
    assert!(!Arc::ptr_eq(&source, &editor.room_camera_source_token()));
    assert_eq!(
        editor.presentation_camera3d((1024, 768)).unwrap().target,
        [1.25, 1.5, -0.5]
    );
    assert_eq!(editor.room_camera_document(), Some(&camera));
}
