//! CPU acceptance of the real ERP/editor lifecycle. No native GUI claim.
use orr_editor::game::EditorGame;
use orr_editor::static_adapter::StaticGameRegistry;
use orr_editor::{Editor, Owner};
use orr_fp::{FPVec2, FP};
use orr_session::{ControlOp, PlaySession};
use orr_sim::PlayerSlot;
use orr_static_starter::{delivery::*, integration::*, switchboard::*};
use serde_json::{json, Value as J};
fn fixture<G: Starter>() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("game.scene.yaml");
    std::fs::write(&path, G::SCENE).unwrap();
    (dir, path)
}
fn input(ed: &mut Editor, value: J, ticks: u32) {
    ed.host_call("sim.input_value", json!({"player":0,"value":value}))
        .unwrap();
    ed.step(ticks);
    ed.sync();
}
#[test]
fn delivery_editor_source_replay_win_restart_and_save_as() {
    let (dir, path) = fixture::<DeliveryGame>();
    let handle = registry().handle("delivery").unwrap();
    let mut ed = Editor::open_registered(&path, handle).unwrap();
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Registered(handle));
    assert!(ed.select_named("courier"));
    assert!(ed.set_field(
        &Owner::Component("Courier".into()),
        "position",
        orr_reflect::Value::Vec2(FPVec2::new(FP::ONE, FP::ZERO))
    ));
    ed.sync();
    let moved = ed.checksum();
    assert!(ed.undo());
    ed.sync();
    assert_ne!(ed.checksum(), moved);
    assert!(ed.redo());
    ed.sync();
    assert_eq!(ed.checksum(), moved);
    let saved = dir.path().join("saved.scene.yaml");
    assert!(ed.save_as(&saved));
    assert!(ed.restart());
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Registered(handle));
    let second = dir.path().join("second.scene.yaml");
    std::fs::write(
        &second,
        DeliveryGame::SCENE
            .replace("[0, 0]", "[100, 20]")
            .replace("[4, 0]", "[104, 20]"),
    )
    .unwrap();
    assert!(ed.open_path(&second));
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Registered(handle));
    // Camera coordinates are view-only; ranges avoid float arithmetic in this test.
    assert!((101.99..=102.01).contains(&ed.camera.center[0]));
    assert!((19.99..=20.01).contains(&ed.camera.center[1]));
    assert!(ed.open_path(&saved));
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Registered(handle));
    let text = std::fs::read_to_string(&saved).unwrap();
    let mut source = player::<DeliveryGame>(&text).unwrap();
    assert!(ed.start_play());
    for (dx, deliver, ticks) in [(1, 0, 3), (0, 1, 1), (0, 0, 1), (0, 1, 1)] {
        input(&mut ed, json!({"dx":dx,"dy":0,"deliver":deliver}), ticks);
        source.set_input(PlayerSlot(0), DeliveryInput { dx, dy: 0, deliver });
        source.control(ControlOp::Step(ticks));
        assert_eq!(ed.checksum(), source.frame().checksum());
        if dx == 0 && deliver == 1 && source.frame().tick() == 4 {
            assert_eq!(source.frame().singleton::<Delivery>().won, 1);
        }
    }
    assert_eq!(source.frame().singleton::<Delivery>().won, 0);
    assert_eq!(
        source.frame().dense::<Courier>().1[0].position,
        FPVec2::ZERO
    );
    let end = ed.checksum();
    ed.seek(1);
    ed.sync();
    ed.seek(6);
    ed.sync();
    assert_eq!(ed.checksum(), end);
    let bytes = source.save_replay();
    let mut replay =
        PlaySession::<DeliveryGame>::open_replay(&bytes, (), DeliveryGame::BUILD).unwrap();
    replay.control(ControlOp::Step(6));
    assert_eq!(replay.frame().checksum(), end);
    ed.stop();
    ed.sync();
    std::fs::write(&saved, "not a scene").unwrap();
    assert!(!ed.restart());
    assert_eq!(ed.game(), EditorGame::Registered(handle));
}
#[test]
fn second_schema_edit_play_reconnect_is_not_an_arena_alias() {
    let (_dir, path) = fixture::<SwitchGame>();
    let handle = registry().handle("switchboard").unwrap();
    assert_ne!(
        handle.expected_identity().schema,
        registry()
            .handle("delivery")
            .unwrap()
            .expected_identity()
            .schema
    );
    assert_ne!(
        handle.expected_input(),
        registry().handle("delivery").unwrap().expected_input()
    );
    let mut ed = Editor::open_registered(&path, handle).unwrap();
    ed.sync();
    assert!(ed.set_field(
        &Owner::Singleton("Board".into()),
        "target",
        orr_reflect::Value::Int(3)
    ));
    assert!(ed.save());
    assert!(ed.restart());
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Registered(handle));
    let mut source = player::<SwitchGame>(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(ed.start_play());
    for (selection, toggle) in [(0, 1), (1, 0), (0, 1)] {
        input(&mut ed, json!({"selection":selection,"toggle":toggle}), 1);
        source.set_input(
            PlayerSlot(0),
            SwitchInput {
                selection: FP::from_raw(selection * (1 << 16)),
                toggle,
                pad: 0,
            },
        );
        source.control(ControlOp::Step(1));
        assert_eq!(ed.checksum(), source.frame().checksum());
    }
    assert_eq!(source.frame().singleton::<Board>().solved, 1);
    let expected = source.frame().checksum();
    let mut replay =
        PlaySession::<SwitchGame>::open_replay(&source.save_replay(), (), SwitchGame::BUILD)
            .unwrap();
    replay.control(ControlOp::Step(3));
    assert_eq!(replay.frame().checksum(), expected);
    ed.stop();
    ed.sync();
    assert!(ed.restart());
    ed.sync();
    assert!(ed.types().get("Board").is_some());
    assert!(ed.types().get("Courier").is_none());
}
#[test]
fn changed_build_is_rejected_before_editor_use() {
    let (_dir, path) = fixture::<DeliveryGame>();
    let mut descriptors = descriptors();
    descriptors[0].build_id ^= 1;
    let registry = Box::leak(Box::new(StaticGameRegistry::new(descriptors).unwrap()));
    assert!(Editor::open_registered(&path, registry.handle("delivery").unwrap()).is_err());
}
#[test]
fn changed_reflection_schema_is_rejected_before_editor_use() {
    let (_dir, path) = fixture::<DeliveryGame>();
    let mut descriptors = descriptors();
    descriptors[0].types = || {
        let mut t = orr_static_starter::delivery::types();
        t.register_component::<Cursor>("Cursor");
        t
    };
    let registry = Box::leak(Box::new(StaticGameRegistry::new(descriptors).unwrap()));
    assert!(Editor::open_registered(&path, registry.handle("delivery").unwrap()).is_err());
}
#[test]
fn changed_input_schema_and_noninitial_scene_are_rejected() {
    let (_dir, path) = fixture::<DeliveryGame>();
    let mut descriptors = descriptors();
    descriptors[0].input.types = || {
        let mut t = orr_reflect::TypeRegistry::new();
        t.register_component::<SwitchInput>("DeliveryInput");
        t
    };
    descriptors[0].input.held = |k| json!({"selection":k.x,"toggle":u32::from(k.action)});
    let registry = Box::leak(Box::new(StaticGameRegistry::new(descriptors).unwrap()));
    assert!(Editor::open_registered(&path, registry.handle("delivery").unwrap()).is_err());
    assert!(
        document::<SwitchGame>(&SwitchGame::SCENE.replace("selected: 0", "selected: 99")).is_err()
    );
    assert!(document::<DeliveryGame>(&DeliveryGame::SCENE.replace("won: 0", "won: 1")).is_err());
}

// Exercise the actual editor grant -> cached held mapping -> typed host path.
// Realtime tick count is intentionally not used as deterministic test evidence.
fn pump_until(ed: &mut Editor, mut predicate: impl FnMut(&mut Editor) -> bool) {
    for _ in 0..2_500 {
        ed.pump();
        if predicate(ed) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!(
        "editor input handshake did not complete: {}",
        ed.input_hint()
    );
}
#[test]
fn both_games_use_managed_keyboard_mapping_and_focus_loss_neutral() {
    use orr_editor::editor::input::{Keys, Phase};
    for (alias, scene, actor, component) in [
        ("delivery", DeliveryGame::SCENE, "courier", "Courier"),
        ("switchboard", SwitchGame::SCENE, "cursor", "Cursor"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("managed.scene.yaml");
        std::fs::write(&path, scene).unwrap();
        let handle = registry().handle(alias).unwrap();
        let mut ed = Editor::open_registered(&path, handle).unwrap();
        ed.sync();
        assert!(ed.select_named(actor));
        ed.sync();
        assert!(!ed.take_control(0, true), "edit mode never claims");
        ed.play();
        ed.sync();
        assert!(
            !ed.take_control(0, false),
            "unfocused viewport never claims"
        );
        assert!(ed.take_control(0, true));
        pump_until(&mut ed, |e| e.input_phase() == Phase::Active);
        ed.arena_keys(
            true,
            Keys {
                x: 1,
                y: 0,
                fire: true,
            },
        );
        if alias == "delivery" {
            pump_until(
                &mut ed,
                |e| matches!(e.read_field(&Owner::Component(component.into()), "position"), Ok(orr_reflect::Value::Vec2(p)) if p.x > FP::ZERO),
            );
        } else {
            pump_until(
                &mut ed,
                |e| matches!(e.read_field(&Owner::Singleton("Board".into()), "moves"), Ok(orr_reflect::Value::Int(n)) if n > 0),
            );
        }
        ed.arena_keys(
            false,
            Keys {
                x: 1,
                y: 0,
                fire: true,
            },
        );
        pump_until(&mut ed, |e| {
            e.input_phase() == Phase::Off && !e.input_cleanup_pending()
        });
        ed.pause();
        ed.sync();
        let before = ed
            .read_field(&Owner::Component(component.into()), "position")
            .unwrap();
        let moves_before = if alias == "switchboard" {
            Some(
                ed.read_field(&Owner::Singleton("Board".into()), "moves")
                    .unwrap(),
            )
        } else {
            None
        };
        // One tick cannot hide a failed neutralization behind the four-switch wrap.
        ed.step(1);
        ed.sync();
        assert_eq!(
            ed.read_field(&Owner::Component(component.into()), "position")
                .unwrap(),
            before,
            "focus loss must neutralize held direction"
        );
        if let Some(moves) = moves_before {
            assert_eq!(
                ed.read_field(&Owner::Singleton("Board".into()), "moves")
                    .unwrap(),
                moves
            );
        }
        let held_owner = if alias == "delivery" {
            Owner::Singleton("Delivery".into())
        } else {
            Owner::Component("Cursor".into())
        };
        let held_field = if alias == "delivery" {
            "action_held"
        } else {
            "held"
        };
        assert_eq!(
            ed.read_field(&held_owner, held_field).unwrap(),
            orr_reflect::Value::Int(0),
            "focus loss must neutralize action as well as direction"
        );
        ed.stop();
        ed.sync();
        assert!(ed.restart());
        ed.sync();
        ed.play();
        ed.sync();
        assert_eq!(ed.game(), EditorGame::Registered(handle));
        assert_eq!(
            ed.input_phase(),
            Phase::Off,
            "restart must not reclaim input"
        );
        ed.pause();
    }
}

#[test]
fn relocated_source_player_executable_matches_admitted_idle_trace() {
    fn check<G: Starter>() {
        let (dir, _) = fixture::<G>();
        let mut expected = player::<G>(G::SCENE).unwrap();
        expected.control(ControlOp::Step(60));
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_player"))
            .current_dir(dir.path())
            .args([G::ALIAS, "game.scene.yaml"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains(&format!(
                "{} build={:016x} seed=42 tick_rate=60 tick=60 checksum={:016x}",
                G::ID,
                G::BUILD,
                expected.frame().checksum()
            )),
            "{text}"
        );
    }
    check::<DeliveryGame>();
    check::<SwitchGame>();
}

#[test]
fn player_subprocess_nonzero_traces_win_for_both_schemas() {
    fn check<G: Starter>(steps: J)
    where
        G::Input: orr_reflect::Reflect,
    {
        let (dir, _) = fixture::<G>();
        let bytes = serde_json::to_vec(&steps).unwrap();
        std::fs::write(dir.path().join("trace.json"), &bytes).unwrap();
        let mut expected = player::<G>(G::SCENE).unwrap();
        for (ticks, input) in trace::<G>(&bytes).unwrap() {
            expected.set_input(PlayerSlot(0), input);
            expected.control(ControlOp::Step(ticks));
        }
        assert!(G::won(expected.frame()));
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_player"))
            .current_dir(dir.path())
            .args([G::ALIAS, "game.scene.yaml", "trace.json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains(&format!(
                "tick={} checksum={:016x} outcome=won",
                expected.frame().tick(),
                expected.frame().checksum()
            )),
            "{text}"
        );
    }
    check::<DeliveryGame>(json!([
        {"ticks":4,"input":{"dx":1,"dy":0,"deliver":0}},
        {"ticks":1,"input":{"dx":0,"dy":0,"deliver":1}}
    ]));
    check::<SwitchGame>(json!([
        {"ticks":1,"input":{"selection":0,"toggle":1}},
        {"ticks":2,"input":{"selection":1,"toggle":0}},
        {"ticks":1,"input":{"selection":0,"toggle":1}}
    ]));
}
#[test]
fn trace_rejects_malformed_unknown_and_unbounded_data_before_running() {
    for bad in [
        json!([]),
        json!({}),
        json!([{"ticks":1,"input":{"dx":0,"dy":0,"deliver":0},"script":"anything"}]),
        json!([{"ticks":0,"input":{"dx":0,"dy":0,"deliver":0}}]),
        json!([{"ticks":601,"input":{"dx":0,"dy":0,"deliver":0}}]),
        json!([{"ticks":1,"input":{"dx":2,"dy":0,"deliver":0}}]),
        json!([{"ticks":1,"input":{"dx":0,"dy":0,"deliver":0,"unknown":1}}]),
        json!([{"ticks":1,"input":{"dx":0,"dy":0}}]),
        json!([{"ticks":400,"input":{"dx":0,"dy":0,"deliver":0}},{"ticks":400,"input":{"dx":0,"dy":0,"deliver":0}}]),
    ] {
        assert!(trace::<DeliveryGame>(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    assert!(trace::<DeliveryGame>(&vec![b' '; 65_537]).is_err());
    assert!(trace::<DeliveryGame>(b"[").is_err());
    assert!(trace::<SwitchGame>(br#"[{"ticks":1,"input":{"selection":0.5,"toggle":0}}]"#).is_err());
    let (dir, _) = fixture::<DeliveryGame>();
    std::fs::write(dir.path().join("bad.json"), b"[").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_player"))
        .current_dir(dir.path())
        .args(["delivery", "game.scene.yaml", "bad.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
