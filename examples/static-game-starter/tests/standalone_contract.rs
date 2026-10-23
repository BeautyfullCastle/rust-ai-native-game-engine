//! CPU-only contracts for the actual standalone controller. No window, GPU,
//! subprocess, wall clock, paused Step, or private bridge access is used.
use orr_bridge::FrameView;
use orr_fp::FP;
use orr_input::Key;
use orr_session::{ReplayHeader, ReplayReader, ReplayWriter};
use orr_sim::PlayerSlot;
use orr_static_starter::{
    delivery::{DeliveryGame, DeliveryInput},
    game_contract::{admit, trace, Admitted, HeldKeys, Starter},
    presentation::StandaloneGame,
    standalone::{Controller, Options},
    switchboard::{SwitchGame, SwitchInput},
    SEED, TICK_RATE,
};
use serde_json::{json, Value};
use std::time::Duration;

// At 60 Hz this advances exactly one natural bridge tick, including when used
// repeatedly. Do not substitute a paused Step or a real-time sleep.
const ONE_TICK: Duration = Duration::from_nanos(16_666_667);

fn state<G: StandaloneGame>(controller: &Controller<G>) -> Value {
    G::state_value(controller.snapshot().predicted())
}

fn fresh<G: StandaloneGame>() -> (Admitted<G>, Controller<G>) {
    let admitted = admit::<G>(G::SCENE).unwrap();
    let controller = Controller::new(&admitted, None).unwrap();
    (admitted, controller)
}

fn press<G: StandaloneGame>(controller: &mut Controller<G>, key: Key) {
    controller.key(key, true, false, false).unwrap();
}

fn release<G: StandaloneGame>(controller: &mut Controller<G>, key: Key) {
    controller.key(key, false, false, false).unwrap();
}

fn consume_one<G: StandaloneGame>(controller: &mut Controller<G>) -> Value {
    let before = controller.snapshot().tick();
    controller.update(ONE_TICK).unwrap();
    assert_eq!(controller.snapshot().tick(), before + 1);
    let mut consumed = controller.take_consumed();
    assert_eq!(consumed.len(), 1);
    assert!(controller.take_consumed().is_empty());
    let item = consumed.remove(0);
    assert_eq!(item["tick"], before + 1);
    assert_eq!(item["state"], state(controller));
    assert_eq!(
        item["checksum"],
        format!("0x{:016x}", controller.snapshot().predicted().checksum())
    );
    item
}

fn start<G: StandaloneGame>(controller: &mut Controller<G>) -> Value {
    controller.focus(true).unwrap();
    press(controller, Key::Enter);
    assert!(!controller.paused());
    assert!(controller.neutral_pending());
    let consumed = consume_one(controller);
    assert_eq!(consumed["input"], G::input_value(&G::Input::default()));
    assert!(!controller.neutral_pending());
    consumed
}

fn set_keys<G: StandaloneGame>(controller: &mut Controller<G>, keys: HeldKeys) {
    for key in [
        Key::A,
        Key::D,
        Key::W,
        Key::S,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::Space,
    ] {
        release(controller, key);
    }
    match keys.x {
        -1 => press(controller, Key::A),
        1 => press(controller, Key::D),
        _ => {}
    }
    match keys.y {
        -1 => press(controller, Key::S),
        1 => press(controller, Key::W),
        _ => {}
    }
    if keys.action {
        press(controller, Key::Space);
    }
}

fn startup_contract<G: StandaloneGame>() {
    let (_, mut controller) = fresh::<G>();
    let initial = controller.snapshot();
    let initial_state = state(&controller);
    assert_eq!(initial.tick(), 0);
    assert!(controller.paused());
    assert!(!controller.focused());
    assert!(!controller.neutral_pending());
    assert!(controller.held() == G::Input::default());

    // Neither unfocused presses nor elapsed time may secretly advance a tick.
    for key in [Key::Enter, Key::D, Key::Space] {
        press(&mut controller, key);
    }
    controller.update(Duration::from_secs(5)).unwrap();
    assert_eq!(controller.snapshot().tick(), 0);
    assert_eq!(
        controller.snapshot().predicted().checksum(),
        initial.predicted().checksum()
    );
    assert!(controller.take_consumed().is_empty());

    controller.focus(true).unwrap();
    assert!(controller.focused());
    assert!(controller.paused(), "focus gain must not start the game");
    for (repeat, synthetic) in [(true, false), (false, true), (true, true)] {
        controller.key(Key::Enter, true, repeat, synthetic).unwrap();
        controller.update(ONE_TICK).unwrap();
        assert!(controller.paused());
        assert_eq!(controller.snapshot().tick(), 0);
        assert!(controller.take_consumed().is_empty());
    }

    press(&mut controller, Key::Enter);
    assert!(!controller.paused());
    assert!(controller.neutral_pending());
    assert_eq!(
        controller.snapshot().tick(),
        0,
        "Enter itself is not a tick"
    );
    press(&mut controller, Key::D);
    press(&mut controller, Key::Space);
    assert!(controller.held() == G::Input::default());
    controller.update(Duration::ZERO).unwrap();
    assert!(controller.neutral_pending());
    assert_eq!(controller.snapshot().tick(), 0);
    assert!(controller.take_consumed().is_empty());

    let neutral = consume_one(&mut controller);
    assert_eq!(neutral["input"], G::input_value(&G::Input::default()));
    assert_eq!(state(&controller), initial_state);
    assert!(!controller.neutral_pending());
    assert!(controller.held() == G::Input::default());
    // Presses discarded before the neutral boundary cannot spring back later.
    assert_eq!(consume_one(&mut controller)["input"], neutral["input"]);
    assert_eq!(state(&controller), initial_state);

    for (repeat, synthetic) in [(true, false), (false, true), (true, true)] {
        controller.key(Key::D, true, repeat, synthetic).unwrap();
        controller.key(Key::Space, true, repeat, synthetic).unwrap();
        assert!(controller.held() == G::Input::default());
    }
    press(&mut controller, Key::D);
    let moved = consume_one(&mut controller);
    assert_eq!(
        moved["input"],
        G::input_value(&G::held_input(HeldKeys::new(1, 0, false).unwrap()))
    );
    assert_ne!(moved["state"]["position"], initial_state["position"]);

    // A retained snapshot and read-only extraction stay unchanged after updates.
    assert_eq!(initial.tick(), 0);
    assert_eq!(G::state_value(initial.predicted()), initial_state);
    let checksum = controller.snapshot().predicted().checksum();
    let mut items = Vec::new();
    G::extract(controller.snapshot().predicted(), &mut items);
    assert!(!items.is_empty());
    assert_eq!(controller.snapshot().predicted().checksum(), checksum);
}

#[test]
fn both_games_start_paused_and_require_a_fresh_neutral_boundary() {
    startup_contract::<DeliveryGame>();
    startup_contract::<SwitchGame>();
}

fn cancellation_contract<G: StandaloneGame>() {
    let (_, mut controller) = fresh::<G>();
    start(&mut controller);
    let initial = state(&controller);
    for key in [Key::A, Key::D, Key::W, Key::S] {
        press(&mut controller, key);
    }
    assert!(controller.held() == G::Input::default());
    assert_eq!(consume_one(&mut controller)["state"], initial);

    // Aliases share one action: releasing D does not release ArrowRight.
    press(&mut controller, Key::ArrowRight);
    release(&mut controller, Key::A);
    release(&mut controller, Key::D);
    assert!(controller.held() == G::held_input(HeldKeys::new(1, 0, false).unwrap()));
    let moved = consume_one(&mut controller);
    assert_ne!(moved["state"]["position"], initial["position"]);
    press(&mut controller, Key::ArrowLeft);
    assert!(controller.held() == G::Input::default());
    assert_eq!(consume_one(&mut controller)["state"], moved["state"]);

    // Even a synthetic/repeated release must clear a previously held key.
    controller.key(Key::ArrowRight, false, true, true).unwrap();
    assert!(controller.held() == G::held_input(HeldKeys::new(-1, 0, false).unwrap()));
    controller.key(Key::ArrowLeft, false, true, true).unwrap();
    assert!(controller.held() == G::Input::default());
}

#[test]
fn both_games_cancel_opposing_keys_and_clear_releases() {
    cancellation_contract::<DeliveryGame>();
    cancellation_contract::<SwitchGame>();
}

fn interrupted_contract<G: StandaloneGame>(lose_focus: bool) {
    let (_, mut controller) = fresh::<G>();
    start(&mut controller);
    press(&mut controller, Key::D);
    press(&mut controller, Key::Space);
    let held_tick = consume_one(&mut controller);
    let before = controller.snapshot();
    let held_state = state(&controller);
    assert_eq!(held_state["held"], 1);
    assert_eq!(held_state["won"], 0, "the fixture must remain nonterminal");
    assert_eq!(
        held_tick["input"],
        G::input_value(&G::held_input(HeldKeys::new(1, 0, true).unwrap()))
    );

    if lose_focus {
        controller.focus(false).unwrap();
        assert!(!controller.focused());
    } else {
        for (repeat, synthetic) in [(true, false), (false, true)] {
            controller
                .key(Key::Escape, true, repeat, synthetic)
                .unwrap();
            assert!(!controller.paused());
            assert_eq!(controller.snapshot().tick(), before.tick());
        }
        press(&mut controller, Key::Escape);
        assert!(controller.focused());
    }
    assert!(controller.paused());
    assert!(!controller.neutral_pending());
    assert!(controller.held() == G::Input::default());
    assert_eq!(
        controller.snapshot().tick(),
        before.tick(),
        "pause must not hide a Step"
    );
    assert_eq!(
        state(&controller),
        held_state,
        "sim-held state clears on the next natural tick, not on pause"
    );
    controller.update(Duration::from_secs(5)).unwrap();
    assert_eq!(controller.snapshot().tick(), before.tick());
    assert_eq!(
        controller.snapshot().predicted().checksum(),
        before.predicted().checksum()
    );
    assert!(controller.take_consumed().is_empty());

    controller.focus(true).unwrap();
    assert!(
        controller.paused(),
        "regaining focus is not consent to resume"
    );
    for (repeat, synthetic) in [(true, false), (false, true)] {
        controller.key(Key::Enter, true, repeat, synthetic).unwrap();
        controller.update(ONE_TICK).unwrap();
        assert!(controller.paused());
        assert_eq!(controller.snapshot().tick(), before.tick());
        assert!(controller.take_consumed().is_empty());
    }
    press(&mut controller, Key::Enter);
    assert!(controller.neutral_pending());
    assert_eq!(controller.snapshot().tick(), before.tick());
    // A new press before the neutral tick is still too early to arm gameplay.
    press(&mut controller, Key::D);
    press(&mut controller, Key::Space);
    assert!(controller.held() == G::Input::default());
    let neutral = consume_one(&mut controller);
    let mut expected_neutral_state = held_state.clone();
    expected_neutral_state["held"] = json!(0);
    assert_eq!(neutral["input"], G::input_value(&G::Input::default()));
    assert_eq!(
        neutral["state"], expected_neutral_state,
        "resume must not move, toggle, win, or restart"
    );
    assert!(!controller.neutral_pending());
    assert!(controller.held() == G::Input::default());
    assert_eq!(
        consume_one(&mut controller)["state"],
        expected_neutral_state
    );

    press(&mut controller, Key::D);
    press(&mut controller, Key::Space);
    let fresh = consume_one(&mut controller);
    assert_eq!(fresh["input"], held_tick["input"]);
    assert_eq!(fresh["state"]["held"], 1);
    assert_ne!(fresh["state"]["position"], held_state["position"]);
    if G::ID == SwitchGame::ID {
        assert_eq!(fresh["state"]["moves"], 2);
        assert_ne!(fresh["state"]["bits"], held_state["bits"]);
    }
}

#[test]
fn both_games_focus_loss_pauses_without_a_hidden_tick_and_resumes_neutral() {
    interrupted_contract::<DeliveryGame>(true);
    interrupted_contract::<SwitchGame>(true);
}

#[test]
fn both_games_escape_pauses_without_a_hidden_tick_and_resumes_neutral() {
    interrupted_contract::<DeliveryGame>(false);
    interrupted_contract::<SwitchGame>(false);
}

fn finite_input_contract<G: StandaloneGame>() {
    let mut count = 0;
    for x in -1..=1 {
        for y in -1..=1 {
            for action in [false, true] {
                let keys = HeldKeys::new(x, y, action).unwrap();
                let typed = G::held_input(keys);
                assert!(G::valid_input(&typed));
                let value = G::input_value(&typed);
                let bytes = serde_json::to_vec(&json!([{"ticks":1,"input":value}])).unwrap();
                let decoded = trace::<G>(&bytes).unwrap();
                assert_eq!(decoded.len(), 1);
                assert_eq!(decoded[0].0, 1);
                assert!(decoded[0].1 == typed, "{} finite input {keys:?}", G::ID);
                // Byte equality also covers reflected-away typed padding.
                assert_eq!(
                    bytemuck::bytes_of(&decoded[0].1),
                    bytemuck::bytes_of(&typed)
                );

                let (_, mut controller) = fresh::<G>();
                start(&mut controller);
                set_keys(&mut controller, keys);
                assert!(controller.held() == typed);
                assert_eq!(consume_one(&mut controller)["input"], value);
                count += 1;
            }
        }
    }
    assert_eq!(count, 18);
}

#[test]
fn both_games_all_eighteen_held_values_round_trip_through_the_typed_trace_decoder() {
    finite_input_contract::<DeliveryGame>();
    finite_input_contract::<SwitchGame>();
    for (x, y) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
        assert!(HeldKeys::new(x, y, false).is_err());
    }
}

#[cfg(feature = "editor")]
#[test]
fn editor_public_descriptors_and_frozen_neutral_match_the_shared_contract() {
    use orr_editor::static_adapter::StaticHeldKeys;
    use orr_static_starter::integration::{descriptors, registry};

    fn check<G: StandaloneGame>() {
        let descriptor = descriptors()
            .into_iter()
            .find(|d| d.game_id == G::ID)
            .unwrap();
        let frozen = registry().handle(G::ALIAS).unwrap();
        assert_eq!(frozen.neutral_value(), G::input_value(&G::Input::default()));
        for x in -1..=1 {
            for y in -1..=1 {
                for action in [false, true] {
                    let keys = HeldKeys::new(x, y, action).unwrap();
                    assert_eq!(
                        (descriptor.input.held)(StaticHeldKeys { x, y, action }),
                        G::input_value(&G::held_input(keys))
                    );
                }
            }
        }
    }
    // The all-18 comparison above is of public descriptor callbacks; the frozen
    // handle exposes only neutral_value, not its private held-value cache.
    check::<DeliveryGame>();
    check::<SwitchGame>();
}

fn recorded_controller<G: StandaloneGame>() -> (Admitted<G>, Vec<u8>, Vec<Value>) {
    let (admitted, mut controller) = fresh::<G>();
    let mut consumed = vec![start(&mut controller)];
    for tick in 2..=125 {
        let keys = HeldKeys::new(
            if tick % 3 == 0 { -1 } else { 1 },
            if tick % 5 == 0 { 1 } else { 0 },
            tick % 4 == 0,
        )
        .unwrap();
        set_keys(&mut controller, keys);
        consumed.push(consume_one(&mut controller));
    }
    controller.key(Key::Escape, true, false, false).unwrap();
    assert_eq!(controller.snapshot().tick(), 125);
    let bytes = controller.recording().unwrap();
    (admitted, bytes, consumed)
}

fn replay_round_trip<G: StandaloneGame>() {
    let (admitted, bytes, consumed) = recorded_controller::<G>();
    assert_eq!(&bytes[..4], b"ORRP");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 3);
    let reader = ReplayReader::<G>::parse(&bytes).unwrap();
    assert_eq!(reader.first_tick(), 1);
    assert_eq!(reader.last_tick(), 125);
    assert_eq!(reader.tick_count(), 125);
    assert!(reader.keyframe_count() >= 3);
    for tick in [0, 60, 120] {
        assert_eq!(reader.nearest_keyframe(tick), Some(tick));
    }
    let mut admitted_replay = admitted.open_replay(&bytes).unwrap();
    assert_eq!(admitted_replay.frame().tick(), 0);
    let mut viewer = Controller::new(&admitted, Some(&bytes)).unwrap();
    assert!(viewer.paused());
    viewer.update(Duration::from_secs(5)).unwrap();
    assert_eq!(viewer.snapshot().tick(), 0);
    assert!(viewer.take_consumed().is_empty());
    viewer.focus(true).unwrap();
    press(&mut viewer, Key::Enter);
    assert!(
        !viewer.neutral_pending(),
        "replay must consume the recorded trace, not inject live input"
    );

    for expected in &consumed {
        // Deliberately conflicting live presses cannot rewrite a replay tick.
        press(&mut viewer, Key::A);
        press(&mut viewer, Key::Space);
        assert!(viewer.held() == G::Input::default());
        let actual = consume_one(&mut viewer);
        assert_eq!(&actual, expected);
        admitted_replay.step_now().unwrap();
        assert_eq!(
            actual["checksum"],
            format!("0x{:016x}", admitted_replay.frame().checksum())
        );
        assert_eq!(
            actual["state"],
            G::state_value(FrameView::of(admitted_replay.frame()))
        );
        let tick = expected["tick"].as_u64().unwrap();
        let (inputs, commands) = reader.tick(tick).unwrap();
        assert_eq!(inputs.len(), 1);
        assert!(commands.is_empty());
        assert_eq!(G::input_value(&inputs[0]), expected["input"]);
    }
    assert!(
        viewer.paused(),
        "the read-only viewer pauses at the last recorded tick"
    );
    let end = viewer.snapshot();
    viewer.update(ONE_TICK).unwrap();
    assert_eq!(viewer.snapshot().tick(), end.tick());
    assert_eq!(
        viewer.snapshot().predicted().checksum(),
        end.predicted().checksum()
    );
    assert!(viewer.take_consumed().is_empty());
    assert!(viewer.recording().is_err());
}

#[test]
fn both_games_record_actual_consumed_inputs_and_replay_every_tick_past_two_keyframes() {
    replay_round_trip::<DeliveryGame>();
    replay_round_trip::<SwitchGame>();
}

fn header<G: Starter>() -> ReplayHeader {
    ReplayHeader {
        format_version: 3,
        game_id: G::ID.into(),
        build_hash: orr_sim::build_hash_of(G::BUILD, 0),
        seed: SEED,
        player_count: 1,
        tick_rate: TICK_RATE,
        input_size: core::mem::size_of::<G::Input>() as u32,
    }
}

fn one_tick_replay<G: StandaloneGame>(
    admitted: &Admitted<G>,
    header: ReplayHeader,
    input: G::Input,
) -> Vec<u8> {
    let session = admitted.session().unwrap();
    let mut writer = ReplayWriter::<G>::new(header);
    writer.record_keyframe(session.frame());
    writer.record_tick(1, &[input], &[]);
    writer.finish()
}

fn rejection<G: Starter>(admitted: &Admitted<G>, bytes: &[u8], expected: &str) {
    let error = match admitted.open_replay(bytes) {
        Ok(_) => panic!("{} unexpectedly admitted invalid replay: {expected}", G::ID),
        Err(error) => error,
    };
    assert!(
        error.contains(expected),
        "{} expected {expected:?}, got {error:?}",
        G::ID
    );
}

type HeaderVariant = (&'static str, fn(&mut ReplayHeader));

fn replay_header_contract<G: StandaloneGame>() {
    let admitted = admit::<G>(G::SCENE).unwrap();
    let valid = one_tick_replay(&admitted, header::<G>(), G::Input::default());
    assert!(admitted.open_replay(&valid).is_ok());
    let variants: [HeaderVariant; 8] = [
        ("game identity", |h| h.game_id = "AnotherGame".into()),
        ("build hash", |h| h.build_hash = 0),
        ("build hash", |h| h.build_hash ^= 1),
        ("seed", |h| h.seed += 1),
        ("exactly one player", |h| h.player_count = 0),
        ("exactly one player", |h| h.player_count = 2),
        ("tick rate", |h| h.tick_rate += 1),
        ("input size", |h| h.input_size += 1),
    ];
    for (expected, change) in variants {
        let mut altered = header::<G>();
        change(&mut altered);
        let bytes = one_tick_replay(&admitted, altered, G::Input::default());
        rejection(&admitted, &bytes, expected);
    }
    for version in [0_u32, 1, 2, 4] {
        let mut bytes = valid.clone();
        bytes[4..8].copy_from_slice(&version.to_le_bytes());
        rejection(&admitted, &bytes, "canonical engine-writer v3");
    }
    let mut trailing = valid.clone();
    trailing.extend_from_slice(b"trailing bytes");
    rejection(&admitted, &trailing, "trailing bytes");

    // Outer-envelope offset, not a hand-written replay decoder: the engine
    // writer supplies the canonical payload; only its LZ4 size prefix is changed.
    let body_offset = 4 + 4 + 4 + G::ID.len() + 8 + 8 + 1 + 4 + 4 + 4;
    let mut huge_prefix = valid.clone();
    huge_prefix[body_offset..body_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    rejection(&admitted, &huge_prefix, "decompressed body exceeds 16 MiB");
    rejection(&admitted, &valid[..valid.len() - 1], "truncated");
    rejection(&admitted, &vec![0; (4 << 20) + 1], "exceeds 4 MiB");
}

#[test]
fn both_games_reject_wrong_replay_identity_and_unbounded_or_noncanonical_envelopes() {
    replay_header_contract::<DeliveryGame>();
    replay_header_contract::<SwitchGame>();
    let delivery = admit::<DeliveryGame>(DeliveryGame::SCENE).unwrap();
    let switchboard = admit::<SwitchGame>(SwitchGame::SCENE).unwrap();
    let delivery_bytes = one_tick_replay(
        &delivery,
        header::<DeliveryGame>(),
        DeliveryInput::default(),
    );
    let switch_bytes =
        one_tick_replay(&switchboard, header::<SwitchGame>(), SwitchInput::default());
    rejection(&delivery, &switch_bytes, "game identity");
    rejection(&switchboard, &delivery_bytes, "game identity");

    let moved_delivery =
        admit::<DeliveryGame>(&DeliveryGame::SCENE.replace("[4, 0]", "[5, 0]")).unwrap();
    let changed_switchboard =
        admit::<SwitchGame>(&SwitchGame::SCENE.replace("target: 5", "target: 3")).unwrap();
    assert_ne!(delivery.identity(), moved_delivery.identity());
    assert_ne!(switchboard.identity(), changed_switchboard.identity());
    rejection(&moved_delivery, &delivery_bytes, "tick 0 differs");
    rejection(&changed_switchboard, &switch_bytes, "tick 0 differs");
}

#[test]
fn both_games_reject_inputs_outside_the_finite_held_contract() {
    let delivery = admit::<DeliveryGame>(DeliveryGame::SCENE).unwrap();
    for input in [
        DeliveryInput {
            dx: 2,
            dy: 0,
            deliver: 0,
        },
        DeliveryInput {
            dx: 0,
            dy: -2,
            deliver: 0,
        },
        DeliveryInput {
            dx: 0,
            dy: 0,
            deliver: 2,
        },
    ] {
        assert!(!DeliveryGame::valid_input(&input));
        let bytes = one_tick_replay(&delivery, header::<DeliveryGame>(), input);
        rejection(&delivery, &bytes, "outside the finite input contract");
    }
    let switchboard = admit::<SwitchGame>(SwitchGame::SCENE).unwrap();
    for input in [
        SwitchInput {
            selection: FP::from_raw(1 << 15),
            toggle: 0,
            pad: 0,
        },
        SwitchInput {
            selection: FP::from_int(2),
            toggle: 0,
            pad: 0,
        },
        SwitchInput {
            selection: FP::ZERO,
            toggle: 2,
            pad: 0,
        },
        // Reserved bytes are not another input channel. Both the physical
        // mapping and reflected JSON decoder always canonicalize them to zero.
        SwitchInput {
            selection: FP::ZERO,
            toggle: 0,
            pad: 1,
        },
    ] {
        assert!(!SwitchGame::valid_input(&input));
        let bytes = one_tick_replay(&switchboard, header::<SwitchGame>(), input);
        rejection(&switchboard, &bytes, "outside the finite input contract");
    }
}

fn forged_keyframe_contract<G: StandaloneGame>() {
    let admitted = admit::<G>(G::SCENE).unwrap();
    let mut expected = admitted.session().unwrap();
    let mut divergent = admitted.session().unwrap();
    divergent.set_input(
        PlayerSlot(0),
        G::held_input(HeldKeys::new(1, 0, true).unwrap()),
    );
    let mut valid = ReplayWriter::<G>::new(header::<G>());
    let mut forged = ReplayWriter::<G>::new(header::<G>());
    let mut bad_checksum = ReplayWriter::<G>::new(header::<G>());
    for writer in [&mut valid, &mut forged, &mut bad_checksum] {
        writer.record_keyframe(expected.frame());
    }
    for tick in 1..=65 {
        expected.step_now().unwrap();
        divergent.step_now().unwrap();
        for writer in [&mut valid, &mut forged, &mut bad_checksum] {
            writer.record_tick(tick, &[G::Input::default()], &[]);
        }
        valid.record_checksum(tick, expected.frame().checksum());
        forged.record_checksum(tick, expected.frame().checksum());
        bad_checksum.record_checksum(tick, expected.frame().checksum() ^ u64::from(tick == 60));
        if tick == 60 {
            assert_ne!(expected.frame().checksum(), divergent.frame().checksum());
            valid.record_keyframe(expected.frame());
            forged.record_keyframe(divergent.frame());
            bad_checksum.record_keyframe(expected.frame());
        }
    }
    // Multiple valid keyframes must remain supported, not rejected wholesale as
    // a shortcut for refusing a forged future checkpoint.
    let valid_bytes = valid.to_bytes();
    assert!(admitted.open_replay(&valid_bytes).is_ok());
    rejection(&admitted, &forged.finish(), "keyframe at tick 60 differs");
    rejection(
        &admitted,
        &bad_checksum.finish(),
        "checksum at tick 60 differs",
    );

    expected.step_now().unwrap();
    valid.record_keyframe(expected.frame());
    rejection(
        &admitted,
        &valid.finish(),
        "keyframes outside its input timeline",
    );

    let mut missing_initial = ReplayWriter::<G>::new(header::<G>());
    missing_initial.record_tick(1, &[G::Input::default()], &[]);
    rejection(&admitted, &missing_initial.finish(), "tick-0 keyframe");
}

#[test]
fn both_games_verify_future_keyframes_against_independent_admitted_simulation() {
    forged_keyframe_contract::<DeliveryGame>();
    forged_keyframe_contract::<SwitchGame>();
}

fn options(args: &[&str]) -> Result<Options, String> {
    Options::parse(args.iter().map(|arg| (*arg).to_owned()))
}

#[test]
fn options_reject_duplicate_relative_and_conflicting_output_queries() {
    let ordinary = options(&["--scene", "relative.scene.yaml"]).unwrap();
    assert_eq!(
        ordinary.scene.unwrap(),
        std::path::Path::new("relative.scene.yaml")
    );
    assert!(options(&["--provenance"]).unwrap().provenance);
    assert!(
        options(&["--scene", "game.scene.yaml", "--identity"])
            .unwrap()
            .identity
    );
    assert!(options(&[
        "--scene",
        "game.scene.yaml",
        "--record",
        "/tmp/contract.orrp",
        "--observe",
        "/tmp/contract.jsonl",
    ])
    .is_ok());
    assert!(options(&[
        "--scene",
        "game.scene.yaml",
        "--replay",
        "/tmp/contract.orrp",
        "--observe",
        "/tmp/contract.jsonl",
    ])
    .is_ok());

    for args in [
        vec![],
        vec!["--scene"],
        vec!["--scene", "--identity"],
        vec!["--identity"],
        vec!["--scene", "game.scene.yaml", "--unknown"],
        vec!["--scene", "game.scene.yaml", "--identity", "--identity"],
        vec!["--provenance", "--provenance"],
        vec![
            "--scene",
            "game.scene.yaml",
            "--record",
            "/tmp/same",
            "--observe",
            "/tmp/same",
        ],
        vec![
            "--scene",
            "game.scene.yaml",
            "--record",
            "/tmp/record",
            "--replay",
            "/tmp/replay",
        ],
    ] {
        assert!(
            options(&args).is_err(),
            "accepted invalid options: {args:?}"
        );
    }
    for flag in ["--scene", "--record", "--replay", "--observe"] {
        let args = if flag == "--scene" {
            vec![flag, "game.scene.yaml", flag, "another.scene.yaml"]
        } else {
            vec!["--scene", "game.scene.yaml", flag, "/tmp/a", flag, "/tmp/b"]
        };
        assert!(options(&args).is_err(), "accepted duplicate {flag}");
    }
    for flag in ["--record", "--replay", "--observe"] {
        assert!(options(&["--scene", "game.scene.yaml", flag, "relative.out"]).is_err());
        assert!(options(&[
            "--scene",
            "game.scene.yaml",
            "--identity",
            flag,
            "/tmp/output"
        ])
        .is_err());
        assert!(options(&["--provenance", flag, "/tmp/output"]).is_err());
        assert!(options(&["--scene", "game.scene.yaml", flag]).is_err());
    }
    assert!(options(&["--provenance", "--identity"]).is_err());
    assert!(options(&["--provenance", "--scene", "game.scene.yaml"]).is_err());
}
