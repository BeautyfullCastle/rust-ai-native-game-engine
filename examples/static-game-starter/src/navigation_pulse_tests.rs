//! CPU-only project simulation/admission tests. No copied systems or native host.
use super::{admit, document, inputs, trace, Starter};
use crate::{
    delivery::{self, Courier, Delivery, DeliveryGame, DeliveryInput},
    switchboard::{self, Board, Cursor, SwitchGame, SwitchInput},
    SEED, TICK_RATE,
};
use orr_fp::{FPVec2, FP};
use orr_session::{ReplayHeader, ReplayReader, ReplayWriter};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use serde_json::json;

fn simulation<G: Starter>(scene: &str) -> Simulation<G> {
    let doc = document::<G>(scene).unwrap();
    Simulation::from_frame(doc.frame(), TICK_RATE, G::BUILD).unwrap()
}

fn tick<G: Starter>(sim: &mut Simulation<G>, input: G::Input) {
    let mut inputs = TickInputs::new(sim.frame().tick() + 1, 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}

fn delivery_input(dx: i32, dy: i32, mode: u32) -> DeliveryInput {
    DeliveryInput {
        dx,
        dy,
        deliver: mode,
    }
}

fn switch_input(selection: i32, mode: u32) -> SwitchInput {
    SwitchInput {
        selection: FP::from_int(selection),
        toggle: mode,
        pad: 0,
    }
}

fn position(sim: &Simulation<DeliveryGame>) -> FPVec2 {
    sim.frame().dense::<Courier>().1[0].position
}

fn cursor(sim: &Simulation<SwitchGame>) -> Cursor {
    sim.frame().dense::<Cursor>().1[0]
}

fn delivery_bytes(sim: &Simulation<DeliveryGame>) -> (Vec<u8>, Vec<u8>) {
    (
        bytemuck::bytes_of(&sim.frame().dense::<Courier>().1[0]).to_vec(),
        bytemuck::bytes_of(sim.frame().singleton::<Delivery>()).to_vec(),
    )
}

fn switch_bytes(sim: &Simulation<SwitchGame>) -> (Vec<u8>, Vec<u8>) {
    (
        bytemuck::bytes_of(&cursor(sim)).to_vec(),
        bytemuck::bytes_of(sim.frame().singleton::<Board>()).to_vec(),
    )
}

#[test]
fn navigation_modes_preserve_input_layouts_and_reflected_field_sets() {
    assert_eq!(
        (
            delivery::MODE_RELEASED,
            delivery::MODE_PRIMARY,
            delivery::MODE_NAVIGATION_PULSE
        ),
        (0, 1, 2)
    );
    assert_eq!(
        (
            switchboard::MODE_RELEASED,
            switchboard::MODE_PRIMARY,
            switchboard::MODE_NAVIGATION_PULSE
        ),
        (0, 1, 2)
    );
    assert_eq!(core::mem::size_of::<DeliveryInput>(), 12);
    assert_eq!(core::mem::offset_of!(DeliveryInput, dx), 0);
    assert_eq!(core::mem::offset_of!(DeliveryInput, dy), 4);
    assert_eq!(core::mem::offset_of!(DeliveryInput, deliver), 8);
    assert_eq!(core::mem::size_of::<SwitchInput>(), 16);
    assert_eq!(core::mem::offset_of!(SwitchInput, selection), 0);
    assert_eq!(core::mem::offset_of!(SwitchInput, toggle), 8);
    assert_eq!(core::mem::offset_of!(SwitchInput, pad), 12);
    assert_eq!(core::mem::size_of::<Delivery>(), 24);
    assert_eq!(core::mem::size_of::<Cursor>(), 24);
    for (schema, expected) in [
        (
            inputs::<DeliveryGame>()
                .type_json_schema(DeliveryGame::INPUT)
                .unwrap(),
            vec!["dx", "dy", "deliver"],
        ),
        (
            inputs::<SwitchGame>()
                .type_json_schema(SwitchGame::INPUT)
                .unwrap(),
            vec!["selection", "toggle"],
        ),
    ] {
        let schema: serde_json::Value = serde_json::from_str(&schema).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(properties.len(), expected.len());
        for field in expected {
            assert!(properties.contains_key(field));
        }
    }
}

fn reflected_input<G: Starter>(value: &serde_json::Value) -> Result<G::Input, String> {
    let registry = inputs::<G>();
    let info = registry.get(G::INPUT).unwrap();
    let value = orr_remote::json::json_to_value(info.desc(), value, false)?;
    let mut bytes = info.default_bytes();
    info.set(&mut bytes, "", value)
        .map_err(|error| error.to_string())?;
    bytemuck::try_pod_read_unaligned(&bytes).map_err(|error| error.to_string())
}

#[test]
fn navigation_mode_two_is_reflected_and_finite_but_three_plus_is_rejected() {
    for mode in 0..=2 {
        let delivery = json!({"dx":1,"dy":-1,"deliver":mode});
        let switch = json!({"selection":-1,"toggle":mode});
        assert!(DeliveryGame::valid_input(
            &reflected_input::<DeliveryGame>(&delivery).unwrap()
        ));
        assert!(SwitchGame::valid_input(
            &reflected_input::<SwitchGame>(&switch).unwrap()
        ));
        assert!(trace::<DeliveryGame>(
            &serde_json::to_vec(&json!([{"ticks":1,"input":delivery}])).unwrap()
        )
        .is_ok());
        assert!(trace::<SwitchGame>(
            &serde_json::to_vec(&json!([{"ticks":1,"input":switch}])).unwrap()
        )
        .is_ok());
    }
    for mode in [3, u32::MAX] {
        assert!(reflected_input::<DeliveryGame>(&json!({"dx":0,"dy":0,"deliver":mode})).is_err());
        assert!(reflected_input::<SwitchGame>(&json!({"selection":0,"toggle":mode})).is_err());
        assert!(!DeliveryGame::valid_input(&delivery_input(0, 0, mode)));
        assert!(!SwitchGame::valid_input(&switch_input(0, mode)));
    }
    // Reflected/raw directions retain their existing broader domain. Finite
    // trace/replay admission is intentionally narrower and padding stays zero.
    assert!(reflected_input::<DeliveryGame>(&json!({"dx":2,"dy":0,"deliver":2})).is_ok());
    assert!(!DeliveryGame::valid_input(&delivery_input(2, 0, 2)));
    let fractional = SwitchInput {
        selection: FP::from_raw(1),
        toggle: 2,
        pad: 0,
    };
    assert!(!SwitchGame::valid_input(&fractional));
    assert!(reflected_input::<SwitchGame>(&json!({"selection":0.5,"toggle":2})).is_ok());
    assert!(!SwitchGame::valid_input(&SwitchInput {
        pad: 1,
        ..switch_input(1, 2)
    }));
}

#[test]
fn delivery_navigation_pulse_moves_once_with_diagonals_and_shared_boolean_latch() {
    let scene = delivery::SCENE.replace("position: [0, 0]", "position: [0.5, -0.25]");
    let mut sim = simulation::<DeliveryGame>(&scene);
    tick(&mut sim, delivery_input(1, -1, 2));
    let expected = FPVec2::new(FP::from_raw(98_304), FP::from_raw(-81_920));
    assert_eq!(position(&sim), expected);
    assert_eq!(sim.frame().singleton::<Delivery>().action_held, 1);
    let held = delivery_bytes(&sim);
    for input in [
        delivery_input(1, -1, 2),
        delivery_input(-1, 1, 2),
        delivery_input(0, 0, 2),
    ] {
        for _ in 0..40 {
            tick(&mut sim, input);
            assert_eq!(delivery_bytes(&sim), held);
        }
    }
    // 2 -> 1 has no primary edge, but restores old per-tick axis movement.
    tick(&mut sim, delivery_input(-1, 1, 1));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(32_768), FP::from_raw(-16_384))
    );
    tick(&mut sim, delivery_input(-1, 1, 1));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(-32_768), FP::from_raw(49_152))
    );
    let held = delivery_bytes(&sim);
    tick(&mut sim, delivery_input(1, 0, 2)); // 1 -> 2 is not a new edge.
    assert_eq!(delivery_bytes(&sim), held);
    tick(&mut sim, DeliveryInput::default());
    assert_eq!(sim.frame().singleton::<Delivery>().action_held, 0);
    tick(&mut sim, delivery_input(1, 0, 2));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(32_768), FP::from_raw(49_152))
    );
}

#[test]
fn delivery_navigation_never_delivers_or_restarts_and_primary_retains_origin_reset() {
    let mut sim = simulation::<DeliveryGame>(delivery::SCENE);
    for x in 1..=4 {
        tick(&mut sim, delivery_input(1, 0, 2));
        for _ in 0..12 {
            tick(&mut sim, delivery_input(1, 0, 2));
        }
        assert_eq!(position(&sim), FPVec2::new(FP::from_int(x), FP::ZERO));
        assert_eq!(sim.frame().singleton::<Delivery>().won, 0);
        if x < 4 {
            tick(&mut sim, DeliveryInput::default());
        }
    }
    tick(&mut sim, delivery_input(0, 0, 1));
    assert_eq!(
        sim.frame().singleton::<Delivery>().won,
        0,
        "2 -> 1 is not an action edge"
    );
    tick(&mut sim, DeliveryInput::default());
    tick(&mut sim, delivery_input(0, 0, 1));
    assert_eq!(sim.frame().singleton::<Delivery>().won, 1);
    tick(&mut sim, DeliveryInput::default());
    tick(&mut sim, delivery_input(-1, 1, 2));
    let won = delivery_bytes(&sim);
    for _ in 0..40 {
        tick(&mut sim, delivery_input(-1, 1, 2));
        assert_eq!(delivery_bytes(&sim), won);
    }
    tick(&mut sim, delivery_input(1, 1, 1));
    assert_eq!(delivery_bytes(&sim), won, "pulse -> primary cannot restart");
    tick(&mut sim, DeliveryInput::default());
    tick(&mut sim, delivery_input(1, 1, 1));
    assert_eq!(position(&sim), FPVec2::ZERO);
    assert_eq!(sim.frame().singleton::<Delivery>().won, 0);
}

#[test]
fn delivery_legacy_modes_keep_per_tick_axes_action_edges_and_custom_scene_reset() {
    let scene = delivery::SCENE
        .replace("position: [0, 0]", "position: [1.5, -0.25]")
        .replace("goal: [4, 0]", "goal: [3.5, 1.75]");
    let mut sim = simulation::<DeliveryGame>(&scene);
    // Mode 0 with axes releases the latch but is not neutral. Raw axes clamp.
    tick(&mut sim, delivery_input(7, 9, 0));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(163_840), FP::from_raw(49_152))
    );
    assert_eq!(sim.frame().singleton::<Delivery>().action_held, 0);
    tick(&mut sim, delivery_input(1, 1, 1));
    assert_eq!(sim.frame().singleton::<Delivery>().won, 1);
    let won = delivery_bytes(&sim);
    tick(&mut sim, delivery_input(-1, -1, 1));
    assert_eq!(delivery_bytes(&sim), won);
    tick(&mut sim, delivery_input(1, 1, 0));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(229_376), FP::from_raw(114_688))
    );
    tick(&mut sim, delivery_input(1, 1, 1));
    assert_eq!(
        position(&sim),
        FPVec2::ZERO,
        "restart is origin, not authored start"
    );
    tick(&mut sim, delivery_input(-1, 1, 1));
    assert_eq!(position(&sim), FPVec2::new(-FP::ONE, FP::ONE));
    assert_eq!(sim.frame().singleton::<Delivery>().won, 0);
}

#[test]
fn delivery_navigation_overflow_is_atomic_and_consumes_the_latch() {
    for (x, y, dx, dy) in [
        (i64::MAX, 0, 1, 1),
        (0, i64::MAX, 1, 1),
        (i64::MIN, 0, -1, -1),
        (0, i64::MIN, -1, -1),
    ] {
        let mut sim = simulation::<DeliveryGame>(delivery::SCENE);
        let id = sim.frame().dense::<Courier>().0[0];
        let initial = FPVec2::new(FP::from_raw(x), FP::from_raw(y));
        sim.frame_mut().get_mut::<Courier>(id).unwrap().position = initial;
        tick(&mut sim, delivery_input(dx, dy, 2));
        assert_eq!(position(&sim), initial, "neither axis can partially move");
        assert_eq!(sim.frame().singleton::<Delivery>().action_held, 1);
        tick(&mut sim, delivery_input(-dx, -dy, 2));
        assert_eq!(position(&sim), initial, "overflow still consumed the pulse");
        tick(&mut sim, DeliveryInput::default());
        tick(&mut sim, delivery_input(-dx, -dy, 2));
        assert_eq!(
            position(&sim),
            FPVec2::new(
                FP::from_raw(x - (i64::from(dx) << 16)),
                FP::from_raw(y - (i64::from(dy) << 16))
            )
        );
    }
    // The representable boundary itself is allowed on both axes together.
    let mut sim = simulation::<DeliveryGame>(delivery::SCENE);
    let id = sim.frame().dense::<Courier>().0[0];
    sim.frame_mut().get_mut::<Courier>(id).unwrap().position = FPVec2::new(
        FP::from_raw(i64::MAX - 65_536),
        FP::from_raw(i64::MIN + 65_536),
    );
    tick(&mut sim, delivery_input(1, -1, 2));
    assert_eq!(
        position(&sim),
        FPVec2::new(FP::from_raw(i64::MAX), FP::from_raw(i64::MIN))
    );
}

#[test]
fn switch_navigation_wraps_once_canonicalizes_x_preserves_y_and_board_after_solved() {
    let scene = switchboard::SCENE
        .replace(
            "position: [0, 0], selected: 0",
            "position: [99.5, -0.25], selected: 3",
        )
        .replace("target: 5", "target: 9");
    let mut sim = simulation::<SwitchGame>(&scene);
    let board = bytemuck::bytes_of(sim.frame().singleton::<Board>()).to_vec();
    tick(&mut sim, switch_input(1, 2));
    assert_eq!(cursor(&sim).selected, 0);
    assert_eq!(
        cursor(&sim).position,
        FPVec2::new(FP::ZERO, FP::from_raw(-16_384))
    );
    assert_eq!(cursor(&sim).held, 1);
    let held = switch_bytes(&sim);
    for input in [switch_input(1, 2), switch_input(-1, 2), switch_input(0, 2)] {
        for _ in 0..40 {
            tick(&mut sim, input);
            assert_eq!(switch_bytes(&sim), held);
        }
    }
    tick(&mut sim, SwitchInput::default());
    tick(&mut sim, switch_input(-1, 2));
    assert_eq!(cursor(&sim).selected, 3);
    assert_eq!(
        cursor(&sim).position,
        FPVec2::new(FP::from_int(6), FP::from_raw(-16_384))
    );
    assert_eq!(bytemuck::bytes_of(sim.frame().singleton::<Board>()), board);
    tick(&mut sim, SwitchInput::default());
    *sim.frame_mut().singleton_mut::<Board>() = Board {
        bits: 9,
        target: 9,
        solved: 1,
        moves: 7,
    };
    let solved = bytemuck::bytes_of(sim.frame().singleton::<Board>()).to_vec();
    tick(&mut sim, switch_input(1, 2));
    assert_eq!(cursor(&sim).selected, 0);
    assert_eq!(bytemuck::bytes_of(sim.frame().singleton::<Board>()), solved);
    tick(&mut sim, switch_input(0, 1));
    assert_eq!(
        bytemuck::bytes_of(sim.frame().singleton::<Board>()),
        solved,
        "2 -> 1 cannot restart"
    );
    tick(&mut sim, SwitchInput::default());
    tick(&mut sim, switch_input(0, 1));
    let board = sim.frame().singleton::<Board>();
    assert_eq!(
        (board.bits, board.target, board.solved, board.moves),
        (0, 9, 0, 0)
    );
    assert_eq!(cursor(&sim).selected, 0);
    assert_eq!(cursor(&sim).position.y, FP::from_raw(-16_384));
}

#[test]
fn switch_navigation_normalizes_raw_selection_without_overflow() {
    for (selection, delta, expected) in [(u32::MAX, 1, 0), (u32::MAX, -1, 2), (u32::MAX, 0, 3)] {
        let mut sim = simulation::<SwitchGame>(switchboard::SCENE);
        let id = sim.frame().dense::<Cursor>().0[0];
        sim.frame_mut().get_mut::<Cursor>(id).unwrap().selected = selection;
        tick(&mut sim, switch_input(delta, 2));
        assert_eq!(cursor(&sim).selected, expected);
        assert_eq!(cursor(&sim).position.x, FP::from_int(expected as i32 * 2));
        assert_eq!(cursor(&sim).held, 1);
        assert_eq!(sim.frame().singleton::<Board>().bits, 0);
    }
}

#[test]
fn switch_legacy_axes_resume_without_new_edges_across_primary_navigation_transitions() {
    let mut sim = simulation::<SwitchGame>(switchboard::SCENE);
    tick(&mut sim, switch_input(1, 1));
    assert_eq!(
        (cursor(&sim).selected, sim.frame().singleton::<Board>().bits),
        (1, 2)
    );
    tick(&mut sim, switch_input(1, 1));
    assert_eq!(
        (cursor(&sim).selected, sim.frame().singleton::<Board>().bits),
        (2, 2)
    );
    let held = switch_bytes(&sim);
    tick(&mut sim, switch_input(-1, 2));
    assert_eq!(switch_bytes(&sim), held, "1 -> 2 does not navigate");
    tick(&mut sim, switch_input(-1, 1));
    assert_eq!(
        (cursor(&sim).selected, sim.frame().singleton::<Board>().bits),
        (1, 2)
    );
    tick(&mut sim, SwitchInput::default());
    tick(&mut sim, switch_input(-1, 2));
    assert_eq!(cursor(&sim).selected, 0);
    tick(&mut sim, switch_input(1, 1));
    assert_eq!(
        (cursor(&sim).selected, sim.frame().singleton::<Board>().bits),
        (1, 2)
    );
    tick(&mut sim, switch_input(1, 0));
    assert_eq!(
        (cursor(&sim).selected, cursor(&sim).held),
        (2, 0),
        "mode 0 axes are not neutral"
    );
    tick(&mut sim, switch_input(0, 1));
    assert_eq!(
        (
            sim.frame().singleton::<Board>().bits,
            sim.frame().singleton::<Board>().moves
        ),
        (6, 2)
    );
    // The old restart retains a nonzero selection and authored y.
    let id = sim.frame().dense::<Cursor>().0[0];
    sim.frame_mut().get_mut::<Cursor>(id).unwrap().position.y = FP::from_raw(12_345);
    sim.frame_mut().singleton_mut::<Board>().solved = 1;
    tick(&mut sim, SwitchInput::default());
    tick(&mut sim, switch_input(0, 1));
    assert_eq!(
        (cursor(&sim).selected, cursor(&sim).position.y.raw()),
        (2, 12_345)
    );
    assert_eq!(
        (
            sim.frame().singleton::<Board>().bits,
            sim.frame().singleton::<Board>().moves,
            sim.frame().singleton::<Board>().solved
        ),
        (0, 0, 0)
    );
}

#[test]
fn invalid_raw_modes_are_deterministic_noops_without_releasing_existing_latches() {
    for held in [0, 1] {
        for mode in [3, u32::MAX] {
            let mut delivery = simulation::<DeliveryGame>(delivery::SCENE);
            delivery.frame_mut().singleton_mut::<Delivery>().action_held = held;
            let before = delivery_bytes(&delivery);
            for _ in 0..3 {
                tick(&mut delivery, delivery_input(1, -1, mode));
                assert_eq!(delivery_bytes(&delivery), before);
            }
            let mut switch = simulation::<SwitchGame>(switchboard::SCENE);
            let id = switch.frame().dense::<Cursor>().0[0];
            let c = switch.frame_mut().get_mut::<Cursor>(id).unwrap();
            c.held = held;
            c.position.x = FP::from_raw(987_654); // Must not even canonicalize x.
            let before = switch_bytes(&switch);
            for _ in 0..3 {
                tick(&mut switch, switch_input(1, mode));
                assert_eq!(switch_bytes(&switch), before);
            }
        }
    }
}

fn header<G: Starter>(build: u64) -> ReplayHeader {
    ReplayHeader {
        format_version: 3,
        game_id: G::ID.into(),
        build_hash: orr_sim::build_hash_of(build, 0),
        seed: SEED,
        player_count: 1,
        tick_rate: TICK_RATE,
        input_size: core::mem::size_of::<G::Input>() as u32,
    }
}

fn mixed_replay<G: Starter>(sequence: &[G::Input]) {
    let admitted = admit::<G>(G::SCENE).unwrap();
    let mut session = admitted.session().unwrap();
    let mut writer = ReplayWriter::<G>::new(header::<G>(G::BUILD));
    writer.record_keyframe(session.frame());
    let mut frames = vec![session.frame().to_bytes()];
    for tick in 1..=130 {
        let input = sequence[(tick as usize - 1) % sequence.len()];
        session.set_input(PlayerSlot(0), input);
        session.step_now().unwrap();
        writer.record_tick(tick, &[input], &[]);
        writer.record_checksum(tick, session.frame().checksum());
        if tick % 60 == 0 {
            writer.record_keyframe(session.frame());
        }
        frames.push(session.frame().to_bytes());
    }
    let bytes = writer.finish();
    let reader = ReplayReader::<G>::parse(&bytes).unwrap();
    let mut replay = admitted.open_replay(&bytes).unwrap();
    for expected in frames.iter().skip(1) {
        replay.step_now().unwrap();
        assert_eq!(&replay.frame().to_bytes(), expected);
    }
    for tick in [0, 1, 2, 59, 60, 61, 119, 120, 130, 17, 0, 130] {
        assert_eq!(
            reader.seek((), tick).unwrap().frame().to_bytes(),
            frames[tick as usize]
        );
        replay.seek(tick).unwrap();
        assert_eq!(replay.frame().to_bytes(), frames[tick as usize]);
    }
}

#[test]
fn both_games_mixed_legacy_and_navigation_replays_and_seeks_are_byte_exact() {
    mixed_replay::<DeliveryGame>(&[
        delivery_input(1, 0, 2),
        delivery_input(-1, 1, 2),
        delivery_input(0, 0, 1),
        DeliveryInput::default(),
        delivery_input(0, 1, 2),
        DeliveryInput::default(),
        delivery_input(1, -1, 0),
        delivery_input(-1, 0, 1),
        delivery_input(0, -1, 1),
        delivery_input(1, 0, 2),
        DeliveryInput::default(),
        delivery_input(-1, 0, 2),
    ]);
    mixed_replay::<SwitchGame>(&[
        switch_input(1, 2),
        switch_input(-1, 2),
        switch_input(0, 1),
        SwitchInput::default(),
        switch_input(-1, 2),
        SwitchInput::default(),
        switch_input(1, 0),
        switch_input(-1, 1),
        switch_input(1, 1),
        switch_input(1, 2),
        SwitchInput::default(),
        switch_input(-1, 2),
    ]);
}

fn reject_old_replay<G: Starter>(old_build: u64) {
    assert_ne!(G::BUILD, old_build);
    let admitted = admit::<G>(G::SCENE).unwrap();
    let session = admitted.session().unwrap();
    let mut writer = ReplayWriter::<G>::new(header::<G>(old_build));
    writer.record_keyframe(session.frame());
    writer.record_tick(1, &[G::Input::default()], &[]);
    let error = match admitted.open_replay(&writer.finish()) {
        Ok(_) => panic!("accepted old build"),
        Err(error) => error,
    };
    assert!(error.contains("build hash"), "{error}");
}

#[test]
fn navigation_builds_reject_old_mode_zero_one_replay_headers() {
    reject_old_replay::<DeliveryGame>(0x5354415254455201);
    reject_old_replay::<SwitchGame>(0x5354415254455202);
}

#[cfg(feature = "gameplay")]
#[test]
fn navigation_builds_accept_current_manifests_and_reject_previous_build_manifests() {
    use orr_editor::registered_project::PreparedRegisteredProject;
    for (manifest, scene, old_build) in [
        (
            include_str!("../scenes/delivery.project.json"),
            delivery::SCENE,
            "0x5354415254455201",
        ),
        (
            include_str!("../scenes/switchboard.project.json"),
            switchboard::SCENE,
            "0x5354415254455202",
        ),
    ] {
        let mut manifest: serde_json::Value = serde_json::from_str(manifest).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        std::fs::write(dir.path().join(manifest["scene"].as_str().unwrap()), scene).unwrap();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(PreparedRegisteredProject::open(&path, crate::gameplay::registry()).is_ok());
        manifest["build_id"] = json!(old_build);
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error =
            PreparedRegisteredProject::open(&path, crate::gameplay::registry()).unwrap_err();
        assert!(error.contains("build identity mismatch"), "{error}");
    }
}
