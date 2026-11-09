#![allow(
    clippy::disallowed_types,
    reason = "The CPU integration test uses bounded host waits"
)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use orr_ecs::Frame;
use orr_editor::{
    Editor, editor::input::CommandStatus, registered_project::PreparedRegisteredProject,
    static_adapter::IntentRequest,
};
use orr_fp::{FP, FPVec2};
use orr_session::{PlayConfig, PlaySession, PlayerSlot, replay_seek_checked};
use orr_sim::{Simulation, TickInputs};
use orr_static_starter::{
    TICK_RATE,
    game_contract::{HeldKeys, Starter},
    presentation::StandaloneGame,
};
use serde_json::{Value, json};

use orr_collect_delivery_game::game::{
    CollectDeliveryGame, CollectDeliveryState, CollectInput, Courier, MODE_NAVIGATION_PULSE,
    MODE_PRIMARY, MODE_RELEASED,
};

fn game_state(editor: &Editor) -> (FPVec2, CollectDeliveryState) {
    let frame = editor
        .gameplay_frame_evidence()
        .expect("verified live gameplay frame")
        .snapshot()
        .predicted();
    (
        frame.dense::<Courier>().1[0].position,
        *frame.singleton::<CollectDeliveryState>(),
    )
}

fn wait_editor(editor: &mut Editor, mut ready: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.pump();
        assert!(editor.down().is_none(), "editor host disconnected");
        assert!(
            editor.gameplay_error().is_none(),
            "{:?}",
            editor.gameplay_error()
        );
        if ready(editor) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "editor journey timed out at tick {}",
            editor.sim().head_tick
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_command(editor: &mut Editor, id: orr_editor::editor::input::CommandId) {
    wait_editor(editor, |editor| {
        matches!(
            editor.gameplay_command_status(id),
            Some(CommandStatus::Completed { .. })
        )
    });
    wait_editor(editor, |editor| {
        game_state(editor).1.action_held == 0 && editor.can_submit_gameplay()
    });
}

fn claim_control(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match editor.claim_gameplay(0) {
            Ok(()) => return,
            Err(error) if error == "control_not_ready" => {
                editor.pump();
                assert!(
                    Instant::now() < deadline,
                    "gameplay control never became ready"
                );
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("unexpected gameplay claim error: {error}"),
        }
    }
}

fn act(editor: &mut Editor) {
    let id = editor
        .submit_gameplay_intent(IntentRequest::new("act", json!({"press":1})).unwrap())
        .unwrap();
    wait_command(editor, id);
}

fn move_one(editor: &mut Editor, dx: i32, dy: i32) {
    let id = editor
        .submit_gameplay_intent(IntentRequest::new("move", json!({"dx":dx,"dy":dy})).unwrap())
        .unwrap();
    wait_command(editor, id);
}

fn direct_tick(session: &mut PlaySession<CollectDeliveryGame>, input: CollectInput) {
    session.set_input_without_commands(PlayerSlot(0), input);
    session
        .step_now()
        .expect("recording session accepts a tick");
}

fn simulation_tick(simulation: &mut Simulation<CollectDeliveryGame>, input: CollectInput) {
    let mut inputs = TickInputs::new(simulation.tick() + 1, 1);
    inputs.set_input(PlayerSlot(0), input);
    simulation.step(&inputs);
}

fn direct_move(session: &mut PlaySession<CollectDeliveryGame>, dx: i32, dy: i32) {
    direct_tick(
        session,
        CollectInput {
            dx,
            dy,
            action: MODE_NAVIGATION_PULSE,
        },
    );
    direct_tick(session, CollectInput::default());
}

fn pair(position: FPVec2) -> [i64; 2] {
    [position.x.raw(), position.y.raw()]
}

fn pulse(inputs: &mut Vec<Value>, dx: i32, dy: i32) {
    inputs.push(json!({"dx":dx,"dy":dy,"action":MODE_NAVIGATION_PULSE}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
}

fn trace_inputs() -> Vec<Value> {
    let mut inputs = Vec::new();
    // Authored destination [5,0], wrong-order attempt, hazard failure, restart,
    // then pickup and successful delivery.
    for _ in 0..5 {
        pulse(&mut inputs, 1, 0);
    }
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_PRIMARY}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
    for _ in 0..3 {
        pulse(&mut inputs, -1, 0);
    }
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_PRIMARY}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
    pulse(&mut inputs, 0, 1); // hazard from pickup, carrying the item
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_PRIMARY}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
    for _ in 0..2 {
        pulse(&mut inputs, 1, 0);
    }
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_PRIMARY}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
    for _ in 0..3 {
        pulse(&mut inputs, 1, 0);
    }
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_PRIMARY}));
    inputs.push(json!({"dx":0,"dy":0,"action":MODE_RELEASED}));
    inputs
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path).unwrap();
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn snapshot_restore_and_replay(scene: &str) {
    let mut session =
        orr_static_starter::game_contract::player::<CollectDeliveryGame>(scene).unwrap();
    direct_move(&mut session, 1, 0);
    direct_move(&mut session, 1, 0);
    direct_tick(
        &mut session,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    direct_tick(&mut session, CollectInput::default());

    let saved_frame = session.frame().to_bytes();
    let restored = Frame::from_bytes(
        Simulation::<CollectDeliveryGame>::build_registry(),
        &saved_frame,
    )
    .expect("full frame snapshot decodes");
    let cargo = restored.singleton::<CollectDeliveryState>();
    assert_eq!(cargo.item_available, 0);
    assert_eq!(cargo.carrying, 1);
    assert_eq!(cargo.won, 0);
    assert_eq!(cargo.lost, 0);
    let sim = Simulation::<CollectDeliveryGame>::from_frame(
        &restored,
        TICK_RATE,
        CollectDeliveryGame::BUILD,
    )
    .expect("simulation restores from the checked snapshot");
    assert_eq!(sim.checksum(), restored.checksum());

    let mut config = PlayConfig::new(1, orr_static_starter::SEED, TICK_RATE);
    config.game_id = CollectDeliveryGame::ID.into();
    config.build_id = CollectDeliveryGame::BUILD;
    config.start_paused = true;
    let mut resumed = PlaySession::<CollectDeliveryGame>::from_frame(config, &restored).unwrap();
    for _ in 0..3 {
        direct_move(&mut resumed, 1, 0);
    }
    direct_tick(
        &mut resumed,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    let final_input = resumed.frame().checksum();
    let replay = resumed.save_replay();
    let seeked = replay_seek_checked::<CollectDeliveryGame>(
        &replay,
        (),
        CollectDeliveryGame::BUILD,
        resumed.head_tick(),
    )
    .expect("authored-scene replay restores and resimulates");
    assert_eq!(seeked.checksum(), final_input);
    assert_eq!(seeked.frame().singleton::<CollectDeliveryState>().won, 1);
}

#[test]
fn registered_project_play_save_reopen_and_source_free_bundle() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    let manifest = project.join("collect-delivery.project.json");
    let scene = project.join("collect-delivery.scene.yaml");
    fs::write(
        &manifest,
        include_str!("../scenes/collect-delivery.project.json"),
    )
    .unwrap();
    fs::write(&scene, CollectDeliveryGame::SCENE).unwrap();

    let prepared = PreparedRegisteredProject::open(
        &manifest,
        orr_collect_delivery_game::registration::registry(),
    )
    .unwrap();
    let mut editor = Editor::open_registered_project(prepared).unwrap();
    editor.sync();

    // Author a non-default destination with the editor's normal singleton edit,
    // Save, close and reopen the registered project from disk.
    editor
        .host_call(
            "world.singleton.patch",
            json!({"name":"CollectDeliveryState","path":"destination","value":[5,0]}),
        )
        .unwrap();
    assert!(editor.save(), "editor save failed");
    let authored_scene = fs::read_to_string(&scene).unwrap();
    assert!(
        authored_scene.contains("destination: [5, 0]"),
        "{authored_scene}"
    );
    assert_ne!(authored_scene, CollectDeliveryGame::SCENE);
    snapshot_restore_and_replay(&authored_scene);
    drop(editor);

    let reopened = PreparedRegisteredProject::open(
        &manifest,
        orr_collect_delivery_game::registration::registry(),
    )
    .unwrap();
    assert_eq!(reopened.scene_text(), authored_scene);
    let mut editor = Editor::open_registered_project(reopened).unwrap();
    editor.sync();
    editor.play();
    wait_editor(&mut editor, |editor| {
        editor.sim().playing
            && editor.gameplay_frame_evidence().is_some_and(|evidence| {
                let snapshot = evidence.snapshot();
                snapshot.timeline().is_some_and(|timeline| {
                    timeline.playing
                        && timeline.tick == snapshot.tick()
                        && timeline.last_tick == snapshot.tick()
                })
            })
    });
    claim_control(&mut editor);
    wait_editor(&mut editor, Editor::can_submit_gameplay);

    // Wrong order: the destination is reachable, but without cargo the action
    // must not set the win flag.
    for _ in 0..5 {
        move_one(&mut editor, 1, 0);
    }
    act(&mut editor);
    let (position, without_cargo) = game_state(&editor);
    assert_eq!(pair(position), [5 << 16, 0]);
    assert_eq!(without_cargo.won, 0);
    assert_eq!(without_cargo.carrying, 0);

    // Navigate back to the item, collect it, then hit the authored hazard.
    for _ in 0..3 {
        move_one(&mut editor, -1, 0);
    }
    act(&mut editor);
    let (position, collected) = game_state(&editor);
    assert_eq!(pair(position), [2 << 16, 0]);
    assert_eq!(collected.item_available, 0);
    assert_eq!(collected.carrying, 1);
    assert_eq!(collected.won, 0);
    let snapshot_checksum = editor
        .gameplay_frame_evidence()
        .unwrap()
        .snapshot()
        .predicted()
        .checksum();
    assert_ne!(snapshot_checksum, 0);

    move_one(&mut editor, 0, 1);
    let (hazard_position, failed) = game_state(&editor);
    assert_eq!(pair(hazard_position), [2 << 16, 1 << 16]);
    assert_eq!(failed.lost, 1);
    assert_eq!(failed.won, 0);
    assert_eq!(failed.carrying, 1);

    // Restart is an edge-triggered Space press after the loss. The next neutral
    // tick releases it and leaves every gameplay field at the authored initial state.
    act(&mut editor);
    let (reset_position, pressed_reset) = game_state(&editor);
    assert_eq!(pair(reset_position), [0, 0]);
    assert_eq!(pressed_reset.item_available, 1);
    assert_eq!(pressed_reset.carrying, 0);
    assert_eq!(pressed_reset.won, 0);
    assert_eq!(pressed_reset.lost, 0);
    wait_editor(&mut editor, |editor| {
        game_state(editor).1.action_held == MODE_RELEASED
    });

    // Successful order after restart, with a saved-scene destination at x=5.
    for _ in 0..2 {
        move_one(&mut editor, 1, 0);
    }
    act(&mut editor);
    let (_, collected_again) = game_state(&editor);
    assert_eq!(collected_again.item_available, 0);
    assert_eq!(collected_again.carrying, 1);
    for _ in 0..3 {
        move_one(&mut editor, 1, 0);
    }
    act(&mut editor);
    let (won_position, won) = game_state(&editor);
    assert_eq!(pair(won_position), [5 << 16, 0]);
    assert_eq!(won.won, 1);
    assert_eq!(won.lost, 0);
    assert_eq!(won.carrying, 1);

    editor.stop();
    wait_editor(&mut editor, |editor| {
        editor.mode() == orr_editor::Mode::Edit && editor.last_stopped().is_some()
    });
    let stopped = editor
        .last_stopped()
        .expect("completed stop retains the play recording")
        .clone();
    let replayed = replay_seek_checked::<CollectDeliveryGame>(
        &stopped.replay,
        (),
        CollectDeliveryGame::BUILD,
        stopped.tick,
    )
    .unwrap();
    assert_eq!(replayed.checksum(), stopped.checksum);
    assert_eq!(replayed.frame().singleton::<CollectDeliveryState>().won, 1);
    assert_eq!(fs::read_to_string(&scene).unwrap(), authored_scene);
    drop(editor);

    // Reopen the exact authored scene, then prove it still resolves through the
    // ordinary project path after play and replay verification.
    let reopened_after_play = PreparedRegisteredProject::open(
        &manifest,
        orr_collect_delivery_game::registration::registry(),
    )
    .unwrap();
    assert_eq!(reopened_after_play.scene_text(), authored_scene);

    // The production bundle path checks the concrete executable's identity,
    // admitted scene and compile-bound provenance before publishing files.
    let built_runtime = Path::new(env!("CARGO_BIN_EXE_collect-delivery-player"));
    let runtime = root.path().join("trusted-runtime");
    fs::copy(built_runtime, &runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        sha256(built_runtime),
        sha256(&runtime),
        "trusted runtime copy must preserve the exact compiled bytes"
    );
    let output = root.path().join("distribution");
    let source_sha = orr_static_starter::standalone::compiled_source_sha().unwrap();
    let bundle_tool = Path::new(env!("CARGO_BIN_EXE_collect-delivery-bundle"));
    let bundled = Command::new(bundle_tool)
        .arg("--runtime")
        .arg(&runtime)
        .arg("--runtime-sha256")
        .arg(sha256(&runtime))
        .arg("--scene")
        .arg(&scene)
        .arg("--output")
        .arg(&output)
        .arg("--source-sha")
        .arg(source_sha)
        .arg("--trusted-runtime")
        .output()
        .unwrap();
    assert!(
        bundled.status.success(),
        "bundle CLI failed: {}",
        String::from_utf8_lossy(&bundled.stderr)
    );
    let receipt: Value = serde_json::from_slice(&bundled.stdout).unwrap();
    assert_eq!(receipt["source_sha"], source_sha);
    assert_eq!(
        receipt["runtime_sha256"],
        sha256(&output.join("bin/standalone"))
    );
    assert_eq!(
        fs::read_to_string(output.join("scene/game.scene.yaml")).unwrap(),
        authored_scene
    );

    // No project manifest or authored source remains for the consumer. The
    // packaged binary runs its headless CPU trace against the bundled scene.
    fs::remove_dir_all(&project).unwrap();
    fs::remove_file(&runtime).unwrap();
    assert!(!runtime.exists(), "trusted runtime copy remains available");
    assert!(!manifest.exists() && !scene.exists());
    let consumer = root.path().join("consumer");
    fs::create_dir(&consumer).unwrap();
    let trace = consumer.join("journey.trace.json");
    fs::write(&trace, serde_json::to_vec(&trace_inputs()).unwrap()).unwrap();
    let packaged_runtime = output.join("bin/standalone");
    let output_run = Command::new(&packaged_runtime)
        .current_dir(&consumer)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .arg("--scene")
        .arg(output.join("scene/game.scene.yaml"))
        .arg("--trace")
        .arg(&trace)
        .output()
        .unwrap();
    assert!(
        output_run.status.success(),
        "bundled runtime failed: {}",
        String::from_utf8_lossy(&output_run.stderr)
    );
    let final_state: Value = serde_json::from_slice(&output_run.stdout).unwrap();
    assert_eq!(final_state["game"], CollectDeliveryGame::ID);
    assert_eq!(final_state["state"]["destination"], json!([5 << 16, 0]));
    assert_eq!(final_state["state"]["position"], json!([5 << 16, 0]));
    assert_eq!(final_state["state"]["item_available"], 0);
    assert_eq!(final_state["state"]["carrying"], 1);
    assert_eq!(final_state["state"]["won"], 1);
    assert_eq!(final_state["state"]["lost"], 0);
    assert!(!project.exists(), "consumer still has the project source");
}

#[test]
fn held_keys_are_discrete_and_neutral_release_gates_repeat_and_restart() {
    let mut repeat = orr_static_starter::game_contract::player::<CollectDeliveryGame>(
        CollectDeliveryGame::SCENE,
    )
    .unwrap();
    let direction = CollectDeliveryGame::held_input(HeldKeys {
        x: 1,
        y: 0,
        action: false,
    });
    assert_eq!(direction.action, MODE_NAVIGATION_PULSE);
    direct_tick(&mut repeat, direction);
    assert_eq!(
        pair(repeat.frame().dense::<Courier>().1[0].position),
        [1 << 16, 0]
    );
    direct_tick(&mut repeat, direction);
    assert_eq!(
        pair(repeat.frame().dense::<Courier>().1[0].position),
        [1 << 16, 0]
    );
    direct_tick(&mut repeat, CollectInput::default());
    direct_tick(&mut repeat, direction);
    assert_eq!(
        pair(repeat.frame().dense::<Courier>().1[0].position),
        [2 << 16, 0]
    );

    // Simultaneous movement+Space is still only a movement pulse; releasing
    // the direction while Space stays held cannot manufacture a press edge.
    let mut session = orr_static_starter::game_contract::player::<CollectDeliveryGame>(
        CollectDeliveryGame::SCENE,
    )
    .unwrap();
    direct_tick(&mut session, direction);
    direct_tick(&mut session, CollectInput::default());
    let direction_and_space = CollectDeliveryGame::held_input(HeldKeys {
        x: 1,
        y: 0,
        action: true,
    });
    assert_eq!(direction_and_space.action, MODE_NAVIGATION_PULSE);
    direct_tick(&mut session, direction_and_space);
    let space_only = CollectDeliveryGame::held_input(HeldKeys {
        x: 0,
        y: 0,
        action: true,
    });
    direct_tick(&mut session, space_only);
    assert_eq!(
        session.frame().singleton::<CollectDeliveryState>().carrying,
        0
    );
    assert_eq!(
        session
            .frame()
            .singleton::<CollectDeliveryState>()
            .item_available,
        1
    );
    direct_tick(&mut session, CollectInput::default());
    direct_tick(&mut session, space_only);
    assert_eq!(
        session.frame().singleton::<CollectDeliveryState>().carrying,
        1
    );

    // Restore at the authored state, then reach the hazard and verify terminal
    // restart does not trigger until neutral release has cleared the latch.
    let mut failure = orr_static_starter::game_contract::player::<CollectDeliveryGame>(
        CollectDeliveryGame::SCENE,
    )
    .unwrap();
    direct_move(&mut failure, 1, 0);
    direct_move(&mut failure, 1, 0);
    direct_tick(
        &mut failure,
        CollectInput {
            dx: 0,
            dy: 1,
            action: MODE_NAVIGATION_PULSE,
        },
    );
    let lost = failure.frame().singleton::<CollectDeliveryState>();
    assert_eq!(lost.lost, 1);
    let space = CollectDeliveryGame::held_input(HeldKeys {
        x: 0,
        y: 0,
        action: true,
    });
    direct_tick(&mut failure, space);
    assert_eq!(failure.frame().singleton::<CollectDeliveryState>().lost, 1);
    direct_tick(&mut failure, CollectInput::default());
    direct_tick(&mut failure, space);
    let reset = failure.frame().singleton::<CollectDeliveryState>();
    let reset_position = failure.frame().dense::<Courier>().1[0].position;
    assert_eq!(pair(reset_position), [0, 0]);
    assert_eq!(reset.item_available, 1);
    assert_eq!(reset.carrying, 0);
    assert_eq!(reset.won, 0);
    assert_eq!(reset.lost, 0);
    assert_eq!(reset.action_held, MODE_PRIMARY);
    direct_tick(&mut failure, CollectInput::default());
    assert_eq!(
        failure
            .frame()
            .singleton::<CollectDeliveryState>()
            .action_held,
        MODE_RELEASED
    );
}

#[test]
fn admission_rejects_invalid_authored_level_data() {
    let valid = CollectDeliveryGame::SCENE;
    let invalid = [
        valid.replace("pickup: [2, 0]", "pickup: [4, 0]"),
        valid.replace("pickup: [2, 0]", "pickup: [2.5, 0]"),
        valid.replace("hazard: [2, 1]", "hazard: [17, 0]"),
        valid.replace("item_available: 1", "item_available: 0"),
        valid.replace("carrying: 0", "carrying: 1"),
        valid.replace("won: 0", "won: 1"),
        valid.replace("lost: 0", "lost: 1"),
        valid.replace("reserved: 0", "reserved: 1"),
        valid.replace("action_held: 0", "action_held: 1"),
        valid.replace(
            "Courier: { position: [0, 0] }",
            "Courier: { position: [1, 0] }",
        ),
        format!("{valid}\n  e_00000002:\n    name: extra\n    Courier: {{ position: [1, 0] }}\n"),
    ];
    for scene in invalid {
        assert_ne!(scene, valid, "invalid fixture replacement did nothing");
        assert!(
            orr_static_starter::game_contract::document::<CollectDeliveryGame>(&scene).is_err(),
            "admitted invalid authored level:\n{scene}"
        );
    }
}

#[test]
fn trace_cli_enforces_byte_file_and_tick_limits() {
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("scene.yaml");
    fs::write(&scene, CollectDeliveryGame::SCENE).unwrap();

    let oversized = root.path().join("oversized.json");
    let file = fs::File::create(&oversized).unwrap();
    file.set_len((4 << 20) + 1).unwrap();
    drop(file);
    let nonregular = root.path().join("directory-trace");
    fs::create_dir(&nonregular).unwrap();
    let excessive_ticks = root.path().join("too-many-ticks.json");
    let trace_bytes = serde_json::to_vec(
        &(0..36_001)
            .map(|_| json!({"dx":0,"dy":0,"action":MODE_RELEASED}))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(trace_bytes.len() < (4 << 20));
    fs::write(&excessive_ticks, trace_bytes).unwrap();
    let runtime = Path::new(env!("CARGO_BIN_EXE_collect-delivery-player"));

    for (trace, expected) in [
        (&oversized, "input exceeds 4194304 byte limit"),
        (&nonregular, "input must be a regular file"),
        (&excessive_ticks, "trace exceeds 36000 ticks"),
    ] {
        let result = Command::new(runtime)
            .arg("--scene")
            .arg(&scene)
            .arg("--trace")
            .arg(trace)
            .output()
            .unwrap();
        assert!(!result.status.success(), "accepted trace path {trace:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(expected),
            "unexpected error for {trace:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn raw_input_schema_advertises_each_axis_bound() {
    let schema: Value = serde_json::from_str(
        &orr_static_starter::game_contract::inputs::<CollectDeliveryGame>()
            .type_json_schema(CollectDeliveryGame::INPUT)
            .expect("registered input schema"),
    )
    .expect("valid JSON schema");
    for axis in ["dx", "dy"] {
        assert_eq!(schema["properties"][axis]["minimum"], -1);
        assert_eq!(schema["properties"][axis]["maximum"], 1);
    }
}

#[test]
fn invalid_inputs_and_overflow_preserve_position_flags_and_latch_atomically() {
    let session = orr_static_starter::game_contract::player::<CollectDeliveryGame>(
        CollectDeliveryGame::SCENE,
    )
    .unwrap();
    let mut frame = Frame::from_bytes(
        Simulation::<CollectDeliveryGame>::build_registry(),
        &session.frame().to_bytes(),
    )
    .unwrap();
    let actor = frame.dense::<Courier>().0[0];
    let mut authored = *frame.singleton::<CollectDeliveryState>();
    authored.item_available = 0;
    authored.carrying = 1;
    authored.action_held = MODE_NAVIGATION_PULSE;
    frame.set_singleton(authored);
    let mut simulation = Simulation::<CollectDeliveryGame>::from_frame(
        &frame,
        TICK_RATE,
        CollectDeliveryGame::BUILD,
    )
    .unwrap();

    for invalid in [
        CollectInput {
            dx: 2,
            dy: 0,
            action: MODE_RELEASED,
        },
        CollectInput {
            dx: 0,
            dy: -2,
            action: MODE_RELEASED,
        },
        CollectInput {
            dx: 1,
            dy: 0,
            action: 3,
        },
        CollectInput {
            dx: 1,
            dy: 0,
            action: MODE_PRIMARY,
        },
        CollectInput {
            dx: 1,
            dy: 0,
            action: MODE_RELEASED,
        },
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_NAVIGATION_PULSE,
        },
    ] {
        simulation_tick(&mut simulation, invalid);
        let state = simulation.frame().singleton::<CollectDeliveryState>();
        assert_eq!(
            simulation.frame().get::<Courier>(actor).unwrap().position,
            FPVec2::ZERO
        );
        assert_eq!(state.item_available, 0);
        assert_eq!(state.carrying, 1);
        assert_eq!(state.won, 0);
        assert_eq!(state.lost, 0);
        assert_eq!(state.action_held, MODE_NAVIGATION_PULSE);
    }

    simulation
        .frame_mut()
        .get_mut::<Courier>(actor)
        .unwrap()
        .position
        .x = FP::from_raw(i64::MAX);
    let mut overflow_state = *simulation.frame().singleton::<CollectDeliveryState>();
    overflow_state.action_held = MODE_RELEASED;
    simulation.frame_mut().set_singleton(overflow_state);
    simulation_tick(
        &mut simulation,
        CollectInput {
            dx: 1,
            dy: 0,
            action: MODE_NAVIGATION_PULSE,
        },
    );
    let state = simulation.frame().singleton::<CollectDeliveryState>();
    assert_eq!(
        simulation
            .frame()
            .get::<Courier>(actor)
            .unwrap()
            .position
            .x
            .raw(),
        i64::MAX
    );
    assert_eq!(
        simulation.frame().get::<Courier>(actor).unwrap().position.y,
        FP::ZERO
    );
    assert_eq!(state.item_available, 0);
    assert_eq!(state.carrying, 1);
    assert_eq!(state.won, 0);
    assert_eq!(state.lost, 0);
    assert_eq!(state.action_held, MODE_RELEASED);
}

#[test]
fn diagonal_navigation_moves_one_integer_unit_on_both_axes() {
    let diagonal = CollectInput {
        dx: 1,
        dy: -1,
        action: MODE_NAVIGATION_PULSE,
    };
    assert!(CollectDeliveryGame::valid_input(&diagonal));
    let mut session = orr_static_starter::game_contract::player::<CollectDeliveryGame>(
        CollectDeliveryGame::SCENE,
    )
    .unwrap();
    direct_tick(&mut session, diagonal);
    assert_eq!(
        pair(session.frame().dense::<Courier>().1[0].position),
        [1 << 16, -(1 << 16)]
    );
    direct_tick(&mut session, diagonal);
    assert_eq!(
        pair(session.frame().dense::<Courier>().1[0].position),
        [1 << 16, -(1 << 16)]
    );
    direct_tick(&mut session, CollectInput::default());
    direct_tick(&mut session, diagonal);
    assert_eq!(
        pair(session.frame().dense::<Courier>().1[0].position),
        [2 << 16, -(2 << 16)]
    );
}

#[test]
fn nonzero_authored_start_is_restored_after_loss_and_win() {
    let scene = CollectDeliveryGame::SCENE
        .replace("start: [0, 0]", "start: [1, 0]")
        .replace(
            "Courier: { position: [0, 0] }",
            "Courier: { position: [1, 0] }",
        );
    assert_ne!(scene, CollectDeliveryGame::SCENE);
    let mut session =
        orr_static_starter::game_contract::player::<CollectDeliveryGame>(&scene).unwrap();

    direct_move(&mut session, 1, 0);
    direct_move(&mut session, 0, 1);
    assert_eq!(session.frame().singleton::<CollectDeliveryState>().lost, 1);
    direct_tick(
        &mut session,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    direct_tick(&mut session, CollectInput::default());
    let reset_after_loss = session.frame().singleton::<CollectDeliveryState>();
    assert_eq!(
        pair(session.frame().dense::<Courier>().1[0].position),
        [1 << 16, 0]
    );
    assert_eq!(reset_after_loss.item_available, 1);
    assert_eq!(reset_after_loss.carrying, 0);
    assert_eq!(reset_after_loss.won, 0);
    assert_eq!(reset_after_loss.lost, 0);
    assert_eq!(reset_after_loss.action_held, MODE_RELEASED);

    direct_move(&mut session, 1, 0);
    direct_tick(
        &mut session,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    direct_tick(&mut session, CollectInput::default());
    direct_move(&mut session, 1, 0);
    direct_move(&mut session, 1, 0);
    direct_tick(
        &mut session,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    assert_eq!(session.frame().singleton::<CollectDeliveryState>().won, 1);
    direct_tick(&mut session, CollectInput::default());
    direct_tick(
        &mut session,
        CollectInput {
            dx: 0,
            dy: 0,
            action: MODE_PRIMARY,
        },
    );
    direct_tick(&mut session, CollectInput::default());
    let reset_after_win = session.frame().singleton::<CollectDeliveryState>();
    assert_eq!(
        pair(session.frame().dense::<Courier>().1[0].position),
        [1 << 16, 0]
    );
    assert_eq!(reset_after_win.item_available, 1);
    assert_eq!(reset_after_win.carrying, 0);
    assert_eq!(reset_after_win.won, 0);
    assert_eq!(reset_after_win.lost, 0);
    assert_eq!(reset_after_win.action_held, MODE_RELEASED);
}
