//! Acceptance coverage uses real Room inputs and the production App step/observer path.
//! No test installs a fake acquired key in a Frame.
use super::*;
use crate::authored_ui::Screen;
use crate::room_checkpoint::{challenge_digest, CheckpointSession};
use crate::room_checkpoint_store::{CheckpointKey, CheckpointPaths, CheckpointStore, PRIMARY_NAME};
use crate::room_project::CheckpointSupport;
use orr_games::room_escape_game::{RoomActor, PLAYER, WALL};
use orr_physics3d::Body;
use std::fs;

const GAME_ID: &str = "12345678-1234-4234-9234-123456789abc";
fn fixture(root: &Path, seed: &str) {
    crate::project_create::create_room_checkpoint(
        &crate::project_create::CreateOptions {
            output: root.to_owned(),
            template: crate::project_create::ROOM_UI_TEMPLATE.into(),
            seed: seed.into(),
        },
        GAME_ID,
    )
    .unwrap();
}
fn project(root: &Path) -> PreparedProject {
    PreparedProject::open_with_options(root, true, CheckpointSupport::MetadataOnly).unwrap()
}
fn key(project: &PreparedProject) -> CheckpointKey {
    CheckpointKey {
        game_id: project.checkpoint().unwrap().game_id_bytes().unwrap(),
        challenge: challenge_digest(project),
    }
}
fn app_with_store(root: &Path, data: &Path) -> App {
    let mut app = App::new(project(root)).unwrap();
    app.checkpoint = Some(CheckpointSession::from_store(CheckpointStore::open(
        CheckpointPaths::from_directory(data).unwrap(),
        key(&app.project),
    )));
    app
}
fn run_state(app: &App) -> RoomRun {
    *app.sim.frame().singleton::<RoomRun>()
}
fn player_position(app: &App) -> orr_fp::FPVec3 {
    let frame = app.sim.frame();
    let entity = frame
        .entities()
        .find(|&e| frame.get::<RoomActor>(e).unwrap().kind == PLAYER)
        .unwrap();
    frame.get::<Body>(entity).unwrap().pos
}
fn press(app: &mut App) {
    app.step_authoritative(RoomInput::default());
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    });
}
fn acquire(app: &mut App) {
    assert_eq!(run_state(app).key_collected, 0);
    // The authored key is within interaction range of the authored spawn.
    press(app);
    assert_eq!(run_state(app).key_collected, 1);
    assert_eq!(run_state(app).won, 0);
    assert_eq!(
        app.checkpoint.as_ref().unwrap().status(),
        "Key checkpoint saved"
    );
}
fn walk_to_exit_and_win(app: &mut App) {
    for _ in 0..45 {
        app.step_authoritative(RoomInput {
            move_x: 1,
            ..Default::default()
        });
    }
    press(app);
    assert_eq!(
        run_state(app).won,
        1,
        "real movement and a fresh interaction must reach the exit"
    );
}
fn raw_input(size: (u32, u32)) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0 as f32, size.1 as f32),
        )),
        ..Default::default()
    }
}
fn hud_text(app: &mut App) -> String {
    fn collect(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                out.push_str(text.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let checksum = app.sim.frame().checksum();
    let mut text = String::new();
    for output in app.ui_frame(raw_input((1024, 768))).unwrap() {
        for shape in &output.shapes {
            collect(&shape.shape, &mut text);
        }
        output.drop_without_applying_deltas();
    }
    assert_eq!(
        app.sim.frame().checksum(),
        checksum,
        "HUD must not mutate simulation"
    );
    text
}

#[test]
fn checkpoint_real_acquisition_restart_resume_reset_and_held_input() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("external-data");
    fixture(&root, "app-acceptance");
    let mut app = app_with_store(&root, &data);
    let initial = app.sim.frame().checksum();
    let spawn = player_position(&app);
    app.apply_checkpoint_action(CheckpointAction::NewGame)
        .unwrap();
    acquire(&mut app);
    let saved = fs::read(data.join(PRIMARY_NAME)).unwrap();
    walk_to_exit_and_win(&mut app);
    assert!(hud_text(&mut app).contains("WON"));
    assert_eq!(
        fs::read(data.join(PRIMARY_NAME)).unwrap(),
        saved,
        "winning is not another save payload"
    );
    app.keys.event(KeyCode::KeyE, true, false, false, true);
    app.apply_checkpoint_action(CheckpointAction::Restart)
        .unwrap();
    assert_eq!(app.sim.frame().checksum(), initial);
    assert_eq!(
        fs::read(data.join(PRIMARY_NAME)).unwrap(),
        saved,
        "Restart is explicitly non-durable"
    );
    assert_eq!(app.keys.sample(), RoomInput::default());
    assert!(!app.keys.event(KeyCode::KeyE, true, true, false, true));
    assert_eq!(app.keys.sample().buttons, 0);
    app.apply_checkpoint_action(CheckpointAction::Resume)
        .unwrap();
    assert_eq!(app.sim.tick(), 0);
    assert_eq!(player_position(&app), spawn);
    assert_eq!(
        run_state(&app),
        RoomRun {
            key_collected: 1,
            ..Default::default()
        }
    );
    assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
    let hud = hud_text(&mut app);
    assert!(
        hud.contains("ACQUIRED") && hud.contains("UNLOCKED"),
        "{hud}"
    );
    app.apply_checkpoint_action(CheckpointAction::NewGame)
        .unwrap();
    assert_eq!(app.sim.frame().checksum(), initial);
    assert!(!app.checkpoint.as_ref().unwrap().available());
    let cleared: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join(PRIMARY_NAME)).unwrap()).unwrap();
    assert_eq!(cleared["key_collected"], false);
    drop(app);
    let mut reopened = app_with_store(&root, &data);
    assert!(!reopened.checkpoint.as_ref().unwrap().available());
    reopened
        .apply_checkpoint_action(CheckpointAction::Resume)
        .unwrap();
    assert_eq!(run_state(&reopened).key_collected, 0);
    assert_eq!(reopened.ui.as_ref().unwrap().screen(), Screen::Title);
}

#[test]
fn checkpoint_failed_durable_reset_preserves_live_frame() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fixture(&root, "failed-reset");
    let mut app = app_with_store(&root, &temp.path().join("data"));
    app.apply_checkpoint_action(CheckpointAction::Restart)
        .unwrap();
    acquire(&mut app);
    app.step_authoritative(RoomInput {
        move_x: 1,
        ..Default::default()
    });
    let checksum = app.sim.frame().checksum();
    app.checkpoint = Some(CheckpointSession::from_store(Err(
        "injected unavailable store".into(),
    )));
    app.apply_checkpoint_action(CheckpointAction::NewGame)
        .unwrap();
    assert_eq!(app.sim.frame().checksum(), checksum);
    assert_eq!(run_state(&app).key_collected, 1);
    assert!(app
        .checkpoint
        .as_ref()
        .unwrap()
        .status()
        .contains("not saved"));
}

// Launch the same test executable, not a simulation-only persistence helper.
// Each phase constructs a fresh App and the normal environment-based session.
#[test]
fn checkpoint_app_fresh_process_roundtrip_and_nonprofile_routes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("data");
    let home = temp.path().join("home");
    fixture(&root, "process-roundtrip");
    for phase in ["nonprofile", "acquire", "resume-win", "reset", "empty"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("room_checkpoint_acceptance_tests::checkpoint_process_child")
            .arg("--nocapture")
            .env("ORR_CHECKPOINT_ACCEPTANCE_PHASE", phase)
            .env("ORR_CHECKPOINT_ACCEPTANCE_PROJECT", &root)
            .env("XDG_DATA_HOME", &data)
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "phase {phase}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("checkpoint acceptance child completed"));
        if phase == "nonprofile" {
            assert!(
                !data.exists(),
                "App construction, admission/editor session and headless must not touch profile"
            );
            assert!(!home.exists());
        }
    }
}
#[test]
fn checkpoint_process_child() {
    let Ok(phase) = std::env::var("ORR_CHECKPOINT_ACCEPTANCE_PHASE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("ORR_CHECKPOINT_ACCEPTANCE_PROJECT").unwrap());
    let mut app = App::new(project(&root)).unwrap();
    assert!(app.checkpoint.is_none());
    if phase == "nonprofile" {
        app.project.scene().session().unwrap();
        headless(
            &app.project,
            3,
            RoomInput {
                buttons: INTERACT,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(!PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()).exists());
        println!("checkpoint acceptance child completed: {phase}");
        return;
    }
    app.checkpoint = CheckpointSession::open(&app.project);
    match phase.as_str() {
        "acquire" => {
            assert!(!app.checkpoint.as_ref().unwrap().available());
            app.apply_checkpoint_action(CheckpointAction::Restart)
                .unwrap();
            acquire(&mut app);
        }
        "resume-win" => {
            assert!(app.checkpoint.as_ref().unwrap().available());
            let spawn = player_position(&app);
            app.apply_checkpoint_action(CheckpointAction::Resume)
                .unwrap();
            assert_eq!(
                run_state(&app),
                RoomRun {
                    key_collected: 1,
                    ..Default::default()
                }
            );
            assert_eq!(app.sim.tick(), 0);
            assert_eq!(player_position(&app), spawn);
            assert!(hud_text(&mut app).contains("UNLOCKED"));
            walk_to_exit_and_win(&mut app);
            assert!(hud_text(&mut app).contains("WON"));
        }
        "reset" => {
            assert!(app.checkpoint.as_ref().unwrap().available());
            app.apply_checkpoint_action(CheckpointAction::NewGame)
                .unwrap();
            assert_eq!(
                app.checkpoint.as_ref().unwrap().status(),
                "New Game checkpoint reset saved"
            );
            assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
            assert!(!app.checkpoint.as_ref().unwrap().available());
            assert_eq!(run_state(&app), RoomRun::default());
        }
        "empty" => assert!(!app.checkpoint.as_ref().unwrap().available()),
        _ => panic!("unknown child phase"),
    }
    println!("checkpoint acceptance child completed: {phase}");
}

/// Invoked by the export acceptance gate after hiding source/project/tool roots.
/// This exercises the production App policy offscreen, never claims a native window.
#[test]
#[ignore = "requires exported project and isolated external profile from the acceptance gate"]
fn checkpoint_exported_app_process() {
    let root =
        PathBuf::from(std::env::var_os("ORR_ROOM_CHECKPOINT_PROJECT").expect("exported project"));
    let mode = std::env::var("ORR_ROOM_CHECKPOINT_MODE").expect("acceptance mode");
    let mut app = App::new(project(&root)).unwrap();
    let initial = app.sim.frame().checksum();
    let paths = CheckpointPaths::from_environment(&key(&app.project)).unwrap();
    app.checkpoint = CheckpointSession::open(&app.project);
    match mode.as_str() {
        "acquire" => {
            assert!(!app.checkpoint.as_ref().unwrap().available());
            app.apply_checkpoint_action(CheckpointAction::Restart)
                .unwrap();
            acquire(&mut app);
        }
        "resume" => {
            assert!(app.checkpoint.as_ref().unwrap().available());
            let spawn = player_position(&app);
            app.apply_checkpoint_action(CheckpointAction::Resume)
                .unwrap();
            assert_eq!(app.sim.tick(), 0);
            assert_eq!(player_position(&app), spawn);
            assert_eq!(
                run_state(&app),
                RoomRun {
                    key_collected: 1,
                    ..Default::default()
                }
            );
            assert!(hud_text(&mut app).contains("UNLOCKED"));
            walk_to_exit_and_win(&mut app);
            assert!(hud_text(&mut app).contains("WON"));
        }
        "newgame" => {
            assert!(app.checkpoint.as_ref().unwrap().available());
            app.apply_checkpoint_action(CheckpointAction::NewGame)
                .unwrap();
            assert_eq!(
                app.checkpoint.as_ref().unwrap().status(),
                "New Game checkpoint reset saved"
            );
            assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
            assert_eq!(app.sim.frame().checksum(), initial);
            assert!(!app.checkpoint.as_ref().unwrap().available());
        }
        "empty" => {
            assert!(!app.checkpoint.as_ref().unwrap().available());
            assert_eq!(app.sim.frame().checksum(), initial);
        }
        _ => panic!("unknown acceptance mode {mode}"),
    }
    println!(
        "offscreen App acceptance mode={mode} key={} won={} tick={} profile={} status={}",
        run_state(&app).key_collected,
        run_state(&app).won,
        app.sim.tick(),
        paths.directory().display(),
        app.checkpoint.as_ref().unwrap().status()
    );
}

#[test]
fn checkpoint_semantic_digest_ignores_cosmetics_and_wall_guid_order_but_tracks_gameplay() {
    use orr_reflect::{Scene, Value};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("first-path");
    let renamed = temp.path().join("different-path");
    fixture(&root, "guid-seed-one");
    fixture(&renamed, "guid-seed-two");
    let prepared = project(&root);
    let digest = challenge_digest(&prepared);
    assert_eq!(
        digest,
        challenge_digest(&project(&renamed)),
        "paths, GUID namespace and creator seed are cosmetic"
    );
    let types = crate::room_project::types();
    let mut scene = Scene::parse(prepared.scene().text(), &types).unwrap();
    let walls: Vec<_> = scene
        .entities
        .iter()
        .filter(|(_, entity)| {
            entity
                .components
                .iter()
                .find(|(name, _)| name == crate::room_project::ACTOR)
                .and_then(|(_, actor)| actor.field("kind"))
                == Some(&Value::Int(i128::from(WALL)))
        })
        .map(|(guid, entity)| (guid.clone(), entity.clone()))
        .collect();
    assert_eq!(walls.len(), 4);
    for ((guid, _), (_, entity)) in walls.iter().zip(walls.iter().rev()) {
        scene.entities.insert(guid.clone(), entity.clone());
    }
    for entity in scene.entities.values_mut() {
        entity.name = Some("Cosmetic display name".into());
    }
    scene
        .header_comments
        .push("Cosmetic comment unrelated to gameplay".into());
    fs::write(prepared.path(), scene.to_yaml()).unwrap();
    let mut camera = prepared.camera().unwrap().document.clone();
    camera.yaw += 0.125;
    camera.distance += 1.0;
    fs::write(
        &prepared.camera().unwrap().path,
        serde_json::to_vec_pretty(&camera).unwrap(),
    )
    .unwrap();
    let mut ui = prepared.ui().unwrap().document.clone();
    for node in &mut ui.nodes {
        if let crate::authored_ui::Kind::Label {
            text,
            binding: None,
        } = &mut node.kind
        {
            *text = "Cosmetic title".into();
        }
    }
    fs::write(
        &prepared.ui().unwrap().path,
        serde_json::to_vec_pretty(&ui).unwrap(),
    )
    .unwrap();
    let mut models = prepared.models().document.clone();
    for binding in models.bindings.values_mut() {
        binding.transform.translation[0] += 0.25;
    }
    fs::write(
        &prepared.models().path,
        serde_json::to_vec_pretty(&models).unwrap(),
    )
    .unwrap();
    assert_eq!(
        digest,
        challenge_digest(&project(&root)),
        "wall iteration order, actor names, comments, camera and UI cannot invalidate key progress"
    );
    let original_scene = scene.clone();
    for kind in [
        PLAYER,
        orr_games::room_escape_game::KEY,
        orr_games::room_escape_game::EXIT,
        WALL,
    ] {
        let mut changed = original_scene.clone();
        let entity = changed
            .entities
            .values_mut()
            .find(|entity| {
                entity
                    .components
                    .iter()
                    .find(|(name, _)| name == crate::room_project::ACTOR)
                    .and_then(|(_, actor)| actor.field("kind"))
                    == Some(&Value::Int(i128::from(kind)))
            })
            .unwrap();
        let body = &mut entity
            .components
            .iter_mut()
            .find(|(name, _)| name == "orr_physics3d::Body")
            .unwrap()
            .1;
        let Value::Struct(fields) = body else {
            panic!("Body must be a struct")
        };
        let Value::Vec3(position) =
            &mut fields.iter_mut().find(|(name, _)| name == "pos").unwrap().1
        else {
            panic!("position must be vec3")
        };
        position.x += orr_fp::FP::from_ratio(1, 8);
        fs::write(prepared.path(), changed.to_yaml()).unwrap();
        assert_ne!(
            digest,
            challenge_digest(&project(&root)),
            "gameplay position change for role {kind} must invalidate old namespace"
        );
    }
    let mut changed = original_scene;
    let wall = changed
        .entities
        .values_mut()
        .find(|entity| {
            entity
                .components
                .iter()
                .find(|(name, _)| name == crate::room_project::ACTOR)
                .and_then(|(_, actor)| actor.field("kind"))
                == Some(&Value::Int(i128::from(WALL)))
        })
        .unwrap();
    let collider = &mut wall
        .components
        .iter_mut()
        .find(|(name, _)| name == "orr_physics3d::Collider")
        .unwrap()
        .1;
    let Value::Struct(fields) = collider else {
        panic!("Collider struct")
    };
    let Value::Variant(kind, shape) = &mut fields
        .iter_mut()
        .find(|(name, _)| name == "shape")
        .unwrap()
        .1
    else {
        panic!("Shape variant")
    };
    assert_eq!(kind, "box");
    let Value::Vec3(half) = &mut shape
        .iter_mut()
        .find(|(name, _)| name == "half_extents")
        .unwrap()
        .1
    else {
        panic!("half extents")
    };
    half.x += orr_fp::FP::from_ratio(1, 8);
    fs::write(prepared.path(), changed.to_yaml()).unwrap();
    assert_ne!(
        digest,
        challenge_digest(&project(&root)),
        "collider geometry changes challenge"
    );
}

#[test]
#[ignore = "mandatory GPU adapter and ORR_ROOM_CHECKPOINT_CAPTURE_DIR output directory"]
fn checkpoint_gpu_actual_app_acquire_resume_and_win_overlay() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let output = PathBuf::from(
        std::env::var_os("ORR_ROOM_CHECKPOINT_CAPTURE_DIR").expect("capture directory"),
    );
    fs::create_dir_all(&output).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("external-profile");
    fixture(&root, "actual-app-gpu");
    let mut app = app_with_store(&root, &data);
    app.apply_checkpoint_action(CheckpointAction::Restart)
        .unwrap();
    acquire(&mut app);
    drop(app);
    let mut app = app_with_store(&root, &data);
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("mandatory GPU adapter");
    println!("Room checkpoint offscreen adapter: {}", gpu.adapter_name());
    for phase in ["chooser", "resumed-unlocked", "won"] {
        if phase == "resumed-unlocked" {
            app.apply_checkpoint_action(CheckpointAction::Resume)
                .unwrap();
        }
        if phase == "won" {
            walk_to_exit_and_win(&mut app);
        }
        for size in [(1024, 768), (480, 800)] {
            let checksum = app.sim.frame().checksum();
            let mut renderer = RoomRenderer::new(&gpu, size, app.project.models()).unwrap();
            let camera = app
                .project
                .camera()
                .unwrap()
                .document
                .camera(&app.orbit, size)
                .unwrap();
            renderer
                .render(
                    FrameView::of(app.sim.frame()),
                    app.project.scene().index(),
                    app.project.models(),
                    &camera,
                )
                .unwrap();
            let baseline = renderer.read_rgba8();
            let mut overlay = crate::game_ui_gpu::GpuOverlay::new(&gpu, room_view::TARGET_FORMAT);
            // A new GPU overlay needs the full font texture epoch. Recreate only
            // presentation state; authoritative sim is untouched and supplies HUD.
            let prepared_ui = app.project.ui().unwrap();
            let mut ui = crate::collect_ui::CollectUi::new_for(
                prepared_ui.document.clone(),
                prepared_ui.font.clone(),
                crate::authored_ui::Profile::Room,
            )
            .unwrap();
            if phase != "chooser" {
                ui.apply(crate::authored_ui::Action::Continue);
            }
            app.ui = Some(ui);
            for paint in app.ui_frame(raw_input(size)).unwrap() {
                overlay
                    .paint(
                        &gpu,
                        renderer.target().render_view(),
                        size,
                        &app.ui.as_ref().unwrap().context,
                        paint,
                    )
                    .unwrap();
            }
            let pixels = renderer.read_rgba8();
            let changed = pixels
                .chunks_exact(4)
                .zip(baseline.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                changed > 400,
                "real App HUD pixels missing: {phase} {changed}"
            );
            assert_eq!(app.sim.frame().checksum(), checksum);
            let file = fs::File::create(
                output.join(format!("checkpoint-{phase}-{}x{}.png", size.0, size.1)),
            )
            .unwrap();
            let mut encoder = png17::Encoder::new(file, size.0, size.1);
            encoder.set_color(png17::ColorType::Rgba);
            encoder.set_depth(png17::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
    }
}

fn chooser_button_point(app: &mut App, label: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    let mut point = None;
    for output in app.ui_frame(raw_input((1024, 768))).unwrap() {
        point = point.or_else(|| {
            output
                .shapes
                .iter()
                .find_map(|shape| find(&shape.shape, label))
        });
        output.drop_without_applying_deltas();
    }
    point.unwrap_or_else(|| panic!("chooser button absent: {label}"))
}
fn chooser_click(app: &mut App, label: &str) {
    let point = chooser_button_point(app, label);
    for pressed in [true, false] {
        let mut raw = raw_input((1024, 768));
        raw.events = vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let outputs = app.ui_frame(raw).unwrap();
        if !pressed {
            assert_eq!(
                outputs.len(),
                2,
                "a real chooser click must create a fresh action frame: {label}"
            );
            assert!(
                outputs[0].shapes.is_empty(),
                "pre-action chooser must never paint over restored simulation"
            );
        }
        for output in outputs {
            output.drop_without_applying_deltas();
        }
    }
    assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
    assert!(!hud_text(app).contains("Room checkpoint"));
}
#[test]
fn checkpoint_actual_egui_resume_restart_newgame_and_held_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("external-profile");
    fixture(&root, "chooser-clicks");
    let mut app = app_with_store(&root, &data);
    let initial = app.sim.frame().checksum();
    chooser_click(&mut app, "New Game");
    acquire(&mut app);
    let saved = fs::read(data.join(PRIMARY_NAME)).unwrap();
    app.apply_ui_action(crate::authored_ui::Action::Menu)
        .unwrap();
    chooser_click(&mut app, "Restart session (does not clear checkpoint)");
    assert_eq!(app.sim.frame().checksum(), initial);
    assert_eq!(fs::read(data.join(PRIMARY_NAME)).unwrap(), saved);
    app.apply_ui_action(crate::authored_ui::Action::Menu)
        .unwrap();
    app.keys.event(KeyCode::KeyE, true, false, false, true);
    chooser_click(&mut app, "Resume");
    assert_eq!(
        run_state(&app),
        RoomRun {
            key_collected: 1,
            ..Default::default()
        }
    );
    assert_eq!(app.keys.sample(), RoomInput::default());
    assert!(hud_text(&mut app).contains("UNLOCKED"));
    app.apply_ui_action(crate::authored_ui::Action::Menu)
        .unwrap();
    // Egui buttons activate on release, never repeatedly while held.
    let point = chooser_button_point(&mut app, "New Game");
    let mut raw = raw_input((1024, 768));
    raw.events = vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ];
    for output in app.ui_frame(raw).unwrap() {
        output.drop_without_applying_deltas();
    }
    for _ in 0..4 {
        hud_text(&mut app);
        assert_eq!(run_state(&app).key_collected, 1);
        assert_eq!(fs::read(data.join(PRIMARY_NAME)).unwrap(), saved);
    }
    let mut raw = raw_input((1024, 768));
    raw.events.push(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    let outputs = app.ui_frame(raw).unwrap();
    assert_eq!(outputs.len(), 2);
    assert!(outputs[0].shapes.is_empty());
    for output in outputs {
        output.drop_without_applying_deltas();
    }
    assert_eq!(app.sim.frame().checksum(), initial);
    assert!(!app.checkpoint.as_ref().unwrap().available());
    for _ in 0..4 {
        hud_text(&mut app);
    }
    assert_eq!(
        app.sim.frame().checksum(),
        initial,
        "pointer release cannot replay NewGame"
    );
}

#[test]
fn checkpoint_process_lock_contention_and_readonly_reset_preserve_save() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let xdg = temp.path().join("data");
    fixture(&root, "process-contention");
    let prepared = project(&root);
    let paths =
        CheckpointPaths::from_environment_values(Some(xdg.as_os_str()), None, &key(&prepared))
            .unwrap();
    let mut app = app_with_store(&root, paths.directory());
    app.apply_checkpoint_action(CheckpointAction::Restart)
        .unwrap();
    acquire(&mut app);
    let saved = fs::read(paths.directory().join(PRIMARY_NAME)).unwrap();
    let child = || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("room_checkpoint_acceptance_tests::checkpoint_process_blocked_reset_child")
            .arg("--nocapture")
            .env("ORR_CHECKPOINT_ACCEPTANCE_PROJECT", &root)
            .env("ORR_CHECKPOINT_BLOCKED_RESET", "1")
            .env("XDG_DATA_HOME", &xdg)
            .env("HOME", temp.path().join("unused-home"))
            .output()
            .unwrap()
    };
    let locked = child();
    assert!(
        locked.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&locked.stdout),
        String::from_utf8_lossy(&locked.stderr)
    );
    assert!(String::from_utf8_lossy(&locked.stdout).contains("blocked reset preserved live frame"));
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        saved
    );
    drop(app);
    fs::set_permissions(paths.directory(), fs::Permissions::from_mode(0o500)).unwrap();
    let readonly = child();
    fs::set_permissions(paths.directory(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        readonly.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&readonly.stdout),
        String::from_utf8_lossy(&readonly.stderr)
    );
    assert!(
        String::from_utf8_lossy(&readonly.stdout).contains("blocked reset preserved live frame")
    );
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        saved
    );
    let mut reopened = app_with_store(&root, paths.directory());
    assert!(reopened.checkpoint.as_ref().unwrap().available());
    reopened
        .apply_checkpoint_action(CheckpointAction::NewGame)
        .unwrap();
    drop(reopened);
    assert!(!app_with_store(&root, paths.directory())
        .checkpoint
        .as_ref()
        .unwrap()
        .available());
}
#[test]
fn checkpoint_process_blocked_reset_child() {
    if std::env::var("ORR_CHECKPOINT_BLOCKED_RESET").as_deref() != Ok("1") {
        return;
    }
    let root = PathBuf::from(std::env::var_os("ORR_CHECKPOINT_ACCEPTANCE_PROJECT").unwrap());
    let mut app = App::new(project(&root)).unwrap();
    app.checkpoint = CheckpointSession::open(&app.project);
    app.apply_checkpoint_action(CheckpointAction::Restart)
        .unwrap();
    app.step_authoritative(RoomInput {
        move_x: -1,
        ..Default::default()
    });
    let checksum = app.sim.frame().checksum();
    app.apply_checkpoint_action(CheckpointAction::NewGame)
        .unwrap();
    assert_eq!(app.sim.frame().checksum(), checksum);
    assert!(app
        .checkpoint
        .as_ref()
        .unwrap()
        .status()
        .contains("not saved"));
    println!(
        "blocked reset preserved live frame: {}",
        app.checkpoint.as_ref().unwrap().status()
    );
}
