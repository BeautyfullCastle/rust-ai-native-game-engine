//! Local Arena selection, scene authoring, and managed-control regressions.
#![allow(clippy::disallowed_types)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use common::{arena, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use orr_editor::editor::default_scene_path;
use orr_editor::editor::input::{Keys, Phase};
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp, HostSpec, Owner};
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

const BLANK_ARENA: &str = include_str!("../../../scenes/arena_blank.scene.yaml");
static NEXT_SCENE: AtomicU64 = AtomicU64::new(0);

struct TempScene(PathBuf);

impl TempScene {
    fn new(label: &str, text: &str) -> Self {
        let serial = NEXT_SCENE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orr_editor_local_arena_{}_{}_{}.scene.yaml",
            std::process::id(), label, serial,
        ));
        std::fs::write(&path, text).expect("write temporary Arena scene");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempScene {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn local_arena(scene: &TempScene) -> Editor {
    let mut editor = Editor::open_game(scene.path(), EditorGame::Arena).expect("start local Arena");
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    editor
}

fn current_state(editor: &mut Editor) -> J {
    editor.host_call("sim.state", J::Null).expect("read authoritative state")
}

fn player_value(editor: &mut Editor, name: &str, component: &str, path: &str) -> J {
    let target = editor.rows().iter()
        .find(|row| row.name.as_deref() == Some(name))
        .map(|row| row.target())
        .unwrap_or_else(|| panic!("no local Arena entity named {name}"));
    let value = editor.host_call("world.get", json!({
        "entity": target.param(), "component": component, "path": path,
    })).expect("read player component");
    value["value"].clone()
}

fn wait_until(label: &str, editor: &mut Editor, mut ready: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready(editor) {
        assert!(Instant::now() < deadline, "timed out waiting for {label}: {:?}", editor.status());
        editor.pump();
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn old_local_entry_point_stays_physics_and_explicit_local_arena_creates_players() {
    let mut physics = Editor::open(&default_scene_path()).expect("legacy Local opens PhysGame");
    physics.sync();
    assert_eq!(physics.game(), EditorGame::PhysGame);
    let physics_before = physics.checksum();
    assert!(!physics.spawn_arena_player(0, [0.0, 0.0]));
    assert_eq!(physics.checksum(), physics_before);
    assert!(matches!(physics.spec(), HostSpec::Local { .. } | HostSpec::LocalGame { .. }));

    let blank = TempScene::new("blank", BLANK_ARENA);
    let mut editor = local_arena(&blank);
    assert_eq!(editor.sim().player_count, 2);
    assert!(editor.types().get("Position").is_some());
    assert!(editor.types().get("PlayerTag").is_some());
    assert!(editor.singletons().iter().any(|(name, _)| name == "Score"));
    assert!(editor.rows().is_empty());

    let empty_history = editor.history().entries.len();
    assert!(editor.spawn_arena_player(0, [-300.0, 0.0]));
    editor.sync();
    let player_zero = editor.rows().iter().find(|row| row.name.as_deref() == Some("player_0")).expect("created player row").target();
    assert_eq!(editor.selection(), Some(&player_zero), "successful creation selects the new player");
    let mut player_components = editor.row_of(&player_zero).expect("selected player row").components.clone();
    player_components.sort();
    assert_eq!(player_components, vec!["PlayerTag".to_string(), "Position".to_string()], "player creation adds only the required components");
    assert_eq!(player_value(&mut editor, "player_0", "Position", "pos"), json!([-300, 0]));
    assert_eq!(player_value(&mut editor, "player_0", "PlayerTag", "slot"), json!(0));
    assert_eq!(editor.history().entries.len(), empty_history + 1, "player creation is one undo entry");

    let checksum = editor.checksum();
    let history_len = editor.history().entries.len();
    assert!(!editor.spawn_arena_player(0, [10.0, 20.0]), "duplicate slot is refused");
    assert!(!editor.spawn_arena_player(2, [10.0, 20.0]), "slot outside player_count is refused");
    assert!(!editor.spawn_arena_player(1, [f32::NAN, 0.0]), "non-finite positions are refused");
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history_len);

    assert!(editor.spawn_arena_player(1, [300.0, 0.0]));
    editor.sync();
    assert_eq!(player_value(&mut editor, "player_1", "Position", "pos"), json!([300, 0]));
    assert_eq!(player_value(&mut editor, "player_1", "PlayerTag", "slot"), json!(1));
    assert_eq!(editor.history().entries.len(), history_len + 1);
}

#[test]
fn arena_creation_uses_fresh_authoritative_slot_query_and_fails_closed_on_bad_layouts() {
    let blank = TempScene::new("stale_rows", BLANK_ARENA);
    let mut editor = local_arena(&blank);
    assert!(editor.rows().is_empty());
    let external = editor.host_call("world.spawn", json!({
        "name": "external_player",
        "components": {"Position": {"pos": [12, 34]}, "PlayerTag": {"slot": 0}},
    })).expect("spawn through ERP without refreshing editor rows");
    assert!(external.get("guid").and_then(J::as_str).is_some());
    assert!(editor.rows().is_empty(), "the editor cache must still be stale here");
    let authoritative_before = current_state(&mut editor);
    let history_before = editor.host_call("history.list", J::Null).expect("read host history");
    assert!(!editor.spawn_arena_player(0, [100.0, 100.0]), "fresh slot query must observe the uncached occupant");
    let authoritative_after = current_state(&mut editor);
    let history_after = editor.host_call("history.list", J::Null).expect("read host history after refusal");
    assert_eq!(authoritative_after["checksum"], authoritative_before["checksum"]);
    assert_eq!(history_after["entries"], history_before["entries"], "a refused spawn adds no history");
    editor.sync();
    assert!(editor.rows().iter().any(|row| row.name.as_deref() == Some("external_player")));

    let duplicate_text = arena::SCENE.replace("PlayerTag: { slot: 1 }", "PlayerTag: { slot: 0 }");
    let duplicate_scene = TempScene::new("duplicate_slot", &duplicate_text);
    let mut duplicate = local_arena(&duplicate_scene);
    let before = current_state(&mut duplicate);
    let history = duplicate.host_call("history.list", J::Null).expect("history before malformed-layout refusal");
    assert!(!duplicate.spawn_arena_player(1, [0.0, 0.0]), "duplicate imported slot must fail closed for UI creation");
    assert_eq!(current_state(&mut duplicate)["checksum"], before["checksum"]);
    assert_eq!(duplicate.host_call("history.list", J::Null).unwrap()["entries"], history["entries"]);

    let out_of_range_text = arena::SCENE.replace("PlayerTag: { slot: 1 }", "PlayerTag: { slot: 2 }");
    let out_of_range_scene = TempScene::new("out_of_range_slot", &out_of_range_text);
    let mut out_of_range = local_arena(&out_of_range_scene);
    let before = current_state(&mut out_of_range);
    assert!(!out_of_range.spawn_arena_player(1, [0.0, 0.0]), "out-of-range imported slot must fail closed for UI creation");
    assert_eq!(current_state(&mut out_of_range)["checksum"], before["checksum"]);
}

#[test]
fn local_arena_edit_history_save_reopen_and_restart_preserve_game_and_scene() {
    let source = TempScene::new("edit_source", arena::SCENE);
    let saved = TempScene::new("saved_as", "");
    let mut editor = local_arena(&source);
    assert!(editor.select_named("hero"));
    editor.sync();
    let initial_checksum = editor.checksum();
    let history_before = editor.history().entries.len();

    editor.begin_edit("move local Arena player");
    assert!(editor.set_field(&Owner::Component("Position".into()), "pos", vec2(-240, 40)));
    editor.end_edit();
    editor.sync();
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-240, 40]));
    assert_eq!(editor.history().entries.len(), history_before + 1, "one gesture is one history entry");

    assert!(editor.undo());
    editor.sync();
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-300, 0]));
    assert_eq!(editor.checksum(), initial_checksum);
    assert!(editor.redo());
    editor.sync();
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-240, 40]));

    let expected = editor.checksum();
    assert!(editor.save_as(saved.path()));
    let saved_text = std::fs::read_to_string(saved.path()).expect("saved Arena yaml");
    assert!(saved_text.contains("PlayerTag:") && saved_text.contains("[-240, 40]"));
    assert_eq!(editor.game(), EditorGame::Arena);
    assert_eq!(editor.path().as_deref(), Some(saved.path()), "Save As updates the local host restart path");
    assert!(editor.restart(), "the same editor restarts from its saved-as path");
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    assert_eq!(editor.checksum(), expected);
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-240, 40]));

    let alternate_text = arena::SCENE.replacen("[-300,0]", "[-120,70]", 1);
    let alternate = TempScene::new("alternate_open", &alternate_text);
    assert!(editor.open_path(alternate.path()), "open_path replaces the current Arena scene");
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-120, 70]));
    let alternate_checksum = editor.checksum();
    assert_eq!(editor.path().as_deref(), Some(alternate.path()), "open_path updates the local host restart path");
    assert!(editor.restart(), "same-instance restart reopens the scene most recently opened by open_path");
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    assert_eq!(editor.checksum(), alternate_checksum);
    assert_eq!(player_value(&mut editor, "hero", "Position", "pos"), json!([-120, 70]));

    drop(editor);
    let mut reopened = Editor::open_game(saved.path(), EditorGame::Arena).expect("reopen saved Arena scene");
    reopened.sync();
    assert_eq!(reopened.game(), EditorGame::Arena);
    assert_eq!(reopened.checksum(), expected);
    assert!(reopened.select_named("hero"));
    assert_eq!(reopened.movement_owner(), Some(Owner::Component("Position".into())));
    assert_eq!(reopened.path().as_deref(), Some(saved.path()));
    assert!(reopened.restart(), "the explicitly reopened Arena remains restartable");
    reopened.sync();
    assert_eq!(reopened.game(), EditorGame::Arena);
    assert_eq!(reopened.checksum(), expected);
    assert_eq!(player_value(&mut reopened, "hero", "Position", "pos"), json!([-240, 40]));
}

#[test]
fn foreign_transaction_and_viewer_preview_play_refusals_do_not_mutate_the_scene() {
    let scene = TempScene::new("foreign_transaction", BLANK_ARENA);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve a loopback port");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let mut listen = ServerConfig::new(Auth::DevNoAuth);
    listen.bind = addr;
    let spec = HostSpec::LocalGame {
        game: EditorGame::Arena,
        scene: scene.path().to_path_buf(),
        listen: Some(listen),
        debug_hooks: false,
    };
    let mut local = Editor::start(&spec).expect("start listened local Arena");
    local.sync();
    let mut other = ErpClient::connect(&format!("ws://{addr}/?client=foreign"), None).expect("connect another ERP client");
    let before_foreign_tx = current_state(&mut local)["checksum"].clone();
    other.call("tx.begin", json!({"label":"foreign edit remains owned"})).expect("begin foreign transaction");
    other.call("world.spawn", json!({
        "name": "foreign_player",
        "components": {"Position": {"pos": [50, 60]}, "PlayerTag": {"slot": 0}},
    })).expect("make the foreign transaction observable");
    assert!(!local.spawn_arena_player(0, [0.0, 0.0]), "second editor client cannot take over another client's transaction");
    other.call("tx.commit", J::Null).expect("failed player creation must not roll back the foreign transaction");
    let after_foreign_tx = current_state(&mut local);
    assert_ne!(after_foreign_tx["checksum"], before_foreign_tx, "the foreign transaction still commits its edit");
    let players = local.host_call("world.query", json!({
        "components": ["PlayerTag"], "values": true, "limit": 9,
    })).expect("query the foreign transaction result");
    assert!(players["entities"].as_array().unwrap().iter().any(|entity| entity["name"] == "foreign_player"));

    let phys = TempScene::new("physics_refusal", include_str!("../../../scenes/physics_demo.scene.yaml"));
    let mut physics = Editor::open(phys.path()).unwrap();
    physics.sync();
    let phys_checksum = physics.checksum();
    assert!(!physics.spawn_arena_player(0, [0.0, 0.0]));
    assert_eq!(physics.checksum(), phys_checksum);

    let viewer_host = arena::ArenaHost::start(true);
    let mut viewer = Editor::attach(&viewer_host.url, None).unwrap();
    viewer.sync();
    let viewer_checksum = viewer.checksum();
    assert!(!viewer.spawn_arena_player(0, [0.0, 0.0]));
    assert_eq!(viewer.checksum(), viewer_checksum);

    let preview_host = arena::ArenaHost::start(false);
    let mut agent = preview_host.client();
    let mut preview = Editor::attach(&preview_host.url, None).unwrap();
    let proposal = arena::proposal(&mut agent, -250, 10);
    preview.sync();
    assert!(preview.set_preview(Some(proposal)));
    preview.sync();
    let preview_checksum = preview.checksum();
    assert!(!preview.spawn_arena_player(0, [0.0, 0.0]));
    assert_eq!(preview.checksum(), preview_checksum);
    assert!(preview.start_play());
    preview.sync();
    assert!(!preview.spawn_arena_player(0, [0.0, 0.0]), "scene creation is unavailable in Play");
    preview.stop();
    preview.sync();
}

#[test]
fn local_arena_player_button_creates_a_player_in_the_selected_slot() {
    let blank = TempScene::new("player_button", BLANK_ARENA);
    let editor = local_arena(&blank);
    let mut harness = Harness::builder()
        .with_size([1400.0, 850.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    harness.get_by_label("+ Player").click();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !harness.state().editor.rows().iter().any(|row| row.name.as_deref() == Some("player_0")) {
        if Instant::now() >= deadline {
            let editor = &harness.state().editor;
            let row_names = editor.rows().iter().map(|row| row.name.as_deref()).collect::<Vec<_>>();
            panic!(
                "timed out waiting for the authoritative player_0 row after one + Player click: status={:?}, history_entries={}, selection={:?}, rows={row_names:?}",
                editor.status(), editor.history().entries.len(), editor.selection(),
            );
        }
        harness.run_steps(1);
        thread::sleep(Duration::from_millis(2));
    }
    let editor = &harness.state().editor;
    assert_eq!(editor.game(), EditorGame::Arena);
    assert!(editor.rows().iter().any(|row| row.name.as_deref() == Some("player_0")));
    assert!(harness.state_mut().editor.select_named("player_0"));
    let editor = &harness.state().editor;
    assert_eq!(editor.movement_owner(), Some(Owner::Component("Position".into())));
}

#[test]
fn local_arena_managed_input_requires_take_control_and_records_move_fire_release() {
    let scene = TempScene::new("managed_input", arena::SCENE);
    let mut editor = local_arena(&scene);
    let hero_entity = editor.rows().iter()
        .find(|row| row.name.as_deref() == Some("hero"))
        .expect("fixture hero")
        .entity;
    editor.play();
    wait_until("realtime play head", &mut editor, |ed| ed.can_take_control());
    let before = editor.bodies().iter().find(|body| body.entity == hero_entity).expect("hero drawable").pos;
    assert!(!editor.take_control(0, false), "focus alone never grants managed control");
    assert!(editor.take_control(0, true), "explicit focused action claims the selected slot");
    wait_until("managed input claim", &mut editor, |ed| ed.input_phase() == Phase::Active);
    editor.arena_keys(true, Keys { x: 1, y: 0, fire: true });
    wait_until("movement and fire command", &mut editor, |ed| {
        ed.bodies().len() > 2 && ed.bodies().iter().find(|body| body.entity == hero_entity).is_some_and(|body| body.pos[0] > before[0])
    });
    editor.release_control();
    wait_until("managed lease release", &mut editor, |ed| ed.input_phase() == Phase::Off);
    editor.arena_keys(true, Keys { x: 1, y: 0, fire: true });
    editor.pump();
    assert_eq!(editor.input_phase(), Phase::Off, "ordinary focused key updates cannot reacquire a released claim");
    editor.pause();
    editor.sync();
    assert!(!editor.take_control(0, true), "paused play cannot accept managed input");
    editor.step(2);
    editor.sync();
    let stopped = editor.stop().expect("stop retains the play recording").clone();
    assert!(!stopped.replay.is_empty());
    let verify = editor.host_call("verify.self", json!({
        "inputs":{"kind":"last_play"}, "checks":["recording_matches"],
    })).expect("verify recorded local Arena input");
    assert_eq!(verify["passed"], true, "recorded play must match the deterministic replay");
    editor.sync();
    assert_eq!(editor.mode(), orr_editor::Mode::Edit);
}
