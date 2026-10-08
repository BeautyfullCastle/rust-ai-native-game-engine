#![cfg(feature = "collect-dodge")]
use orr_edit::{Op, Origin};
use orr_fp::FP;
use orr_reflect::{Guid, Value};
use orr_remote::collect_dodge::document;
use orr_sample::{
    collect_game::*,
    collect_project::{PreparedScene, ACTOR, RUN},
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
const SOURCE: &str = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
fn input(tick: u64, buttons: u32) -> TickInputs<CollectInput, NoCommand> {
    let mut input = TickInputs::new(tick, 1);
    input.set_input(
        PlayerSlot(0),
        CollectInput {
            x: FP::ONE,
            y: FP::ZERO,
            buttons,
            reserved: 0,
        },
    );
    input
}
#[test]
fn authored_frame_editor_and_runtime_match_reopen_restart_and_ticks() {
    let prepared = PreparedScene::parse(SOURCE).unwrap();
    let mut doc = document(SOURCE).unwrap();
    assert_eq!(prepared.frame().checksum(), doc.frame().checksum());
    let before = doc.frame().checksum();
    doc.apply(
        Op::SetField {
            guid: Guid::parse("e_00000002").unwrap(),
            component: ACTOR.into(),
            path: "position.x".into(),
            value: Value::Fixed(FP::from_int(12)),
        },
        Origin::User,
    )
    .unwrap();
    assert_ne!(doc.frame().checksum(), before);
    let edited = doc.frame().checksum();
    doc.undo().unwrap();
    assert_eq!(doc.frame().checksum(), before);
    doc.redo().unwrap();
    assert_eq!(doc.frame().checksum(), edited);
    let saved = doc.save_yaml();
    let reopened = PreparedScene::parse(&saved).unwrap();
    assert_eq!(reopened.frame().checksum(), edited);
    let mut runtime = reopened.simulation().unwrap();
    let mut editor = Simulation::<CollectDodgeV1>::from_frame(
        doc.frame(),
        60,
        orr_sample::collect_project::build_id(),
    )
    .unwrap();
    for t in 1..=100 {
        let i = input(t, if (30..=32).contains(&t) { RESTART } else { 0 });
        let a = runtime.step(&i);
        let b = editor.step(&i);
        assert_eq!(runtime.checksum(), editor.checksum());
        assert_eq!(
            a.iter().map(|e| (e.key, e.payload)).collect::<Vec<_>>(),
            b.iter().map(|e| (e.key, e.payload)).collect::<Vec<_>>()
        );
    }
    assert_eq!(runtime.frame().singleton::<CollectRun>().phase, WON);
}
#[test]
fn bad_authoring_never_publishes_partial_document_or_frame() {
    let mut doc = document(SOURCE).unwrap();
    let before = doc.frame().checksum();
    let saved = doc.save_yaml();
    for (guid, path, value) in [
        ("e_00000001", "kind", Value::Int(1)),
        ("e_00000002", "ordinal", Value::Int(1)),
        ("e_00000001", "position.x", Value::Fixed(FP(i64::MAX))),
        ("e_00000002", "velocity.x", Value::Fixed(FP::ONE)),
    ] {
        assert!(doc
            .apply(
                Op::SetField {
                    guid: Guid::parse(guid).unwrap(),
                    component: ACTOR.into(),
                    path: path.into(),
                    value
                },
                Origin::User
            )
            .is_err());
        assert_eq!(doc.frame().checksum(), before);
        assert_eq!(doc.save_yaml(), saved);
    }
    assert!(doc
        .apply(
            Op::SetSingletonField {
                singleton: RUN.into(),
                path: "time_limit_ticks".into(),
                value: Value::Int(0)
            },
            Origin::User
        )
        .is_err());
    assert_eq!(doc.frame().checksum(), before);
    assert!(doc
        .apply(
            Op::DespawnEntity {
                guid: Guid::parse("e_00000001").unwrap()
            },
            Origin::User
        )
        .is_err());
    assert_eq!(doc.frame().checksum(), before);
}
#[test]
fn rejects_hidden_state_extra_fields_bad_kinds_and_nonfinite_positions() {
    for text in [
        SOURCE.replace("time_limit_ticks: 600", "time_limit_ticks: 600, score: 9"),
        SOURCE.replace("position: [0, 0]", "position: [NaN, 0]"),
        SOURCE.replace("kind: 2", "kind: 99"),
        SOURCE.replace("kind: 0, ordinal: 0", "kind: 0, ordinal: 1"),
        SOURCE.replace("kind: 0, ordinal: 0", "kind: 0, ordinal: 0, active: 1"),
        SOURCE.replace(
            "kind: 0, ordinal: 0",
            "kind: 0, ordinal: 0, initial_position: [0,0]",
        ),
        SOURCE.replace("time_limit_ticks: 600", "time_limit_ticks: 36001"),
    ] {
        assert!(PreparedScene::parse(&text).is_err(), "accepted {text}");
        assert!(document(&text).is_err());
    }
    assert!(PreparedScene::parse(&" ".repeat(65537)).is_err());
}
#[test]
fn play_time_debug_edits_are_explicitly_disabled() {
    use orr_edit::BakeAdmission;
    assert!(!orr_remote::collect_dodge::InitialAdmission.allow_play_edits());
}

#[test]
fn oversized_rename_and_raw_reload_leave_old_frame_source_and_undo_intact() {
    let mut doc = document(SOURCE).unwrap();
    let checksum = doc.frame().checksum();
    let before = doc.to_yaml();
    assert!(doc
        .apply(
            Op::Rename {
                guid: Guid::parse("e_00000001").unwrap(),
                name: Some("x".repeat(129))
            },
            Origin::User
        )
        .is_err());
    assert_eq!(doc.frame().checksum(), checksum);
    assert_eq!(doc.to_yaml(), before);
    let whitespace = format!("{SOURCE}{}", " ".repeat(65537));
    assert!(doc.load_yaml(&whitespace).is_err());
    assert_eq!(doc.frame().checksum(), checksum);
    assert_eq!(doc.to_yaml(), before);
    assert!(doc.undo().is_err());
    assert!(document(&whitespace).is_err());
}

#[test]
fn authored_play_session_serialized_replay_seek_and_restart_are_exact() {
    use orr_bridge::ControlOp;
    use orr_fp::FPVec2;
    let prepared = PreparedScene::parse(SOURCE).unwrap();
    let mut session = prepared.session().unwrap();
    let mut checksums = vec![session.frame().checksum()];
    for t in 1..=100 {
        session.set_input(
            PlayerSlot(0),
            CollectInput {
                x: FP::ONE,
                y: FP::ZERO,
                buttons: if (30..=32).contains(&t) { RESTART } else { 0 },
                reserved: 0,
            },
        );
        session.step_now().unwrap();
        checksums.push(session.frame().checksum());
    }
    let bytes = session.save_replay();
    let config = || CollectLevel::new(FPVec2::ZERO, vec![FPVec2::ZERO], vec![], 1).unwrap();
    // Generic replay_verify starts from Game::setup(config), so it is not the
    // verifier for an arbitrary authored initial Frame. Re-simulate the decoded
    // recording from the exact admitted scene and compare every recorded tick.
    let reader = orr_session::ReplayReader::<CollectDodgeV1>::parse(&bytes).unwrap();
    let mut verified = prepared.simulation().unwrap();
    for tick in 1..=100 {
        let (inputs, commands) = reader.tick(tick).unwrap();
        let mut sample = TickInputs::new(tick, 1);
        sample.set_input(PlayerSlot(0), inputs[0]);
        sample.set_commands(commands.clone());
        verified.step(&sample);
        assert_eq!(verified.checksum(), checksums[tick as usize]);
    }
    assert_eq!(reader.checksums.len(), 101);
    for tick in [0, 15, 30, 31, 33, 50, 100] {
        session.control(ControlOp::Seek(tick));
        assert_eq!(session.frame().checksum(), checksums[tick as usize]);
        let cold = orr_session::replay_seek_checked::<CollectDodgeV1>(
            &bytes,
            config(),
            orr_sample::collect_project::build_id(),
            tick,
        )
        .unwrap();
        assert_eq!(cold.checksum(), checksums[tick as usize]);
    }
    assert!(orr_session::replay_verify_checked::<CollectDodgeV1>(&bytes, config(), 12345).is_err());
}
