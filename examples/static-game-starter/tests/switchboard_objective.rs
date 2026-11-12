//! CPU-only objective visibility contracts for the real standalone presenter.
#![cfg(feature = "standalone")]

use orr_input::Key;
use orr_static_starter::{
    game_contract::{admit, Starter},
    presentation::StandaloneGame,
    standalone::Controller,
    switchboard::{SwitchGame, SwitchInput},
};
use orr_view::{RenderItem, Shape, Style, Transform2};
use serde_json::{json, Value};
use std::time::Duration;

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);

type Fingerprint = Vec<(Shape, [u32; 9])>;

fn state(controller: &Controller<SwitchGame>) -> Value {
    SwitchGame::state_value(controller.snapshot().predicted())
}

fn fresh(target: u32) -> Controller<SwitchGame> {
    assert_eq!(SwitchGame::SCENE.matches("target: 5").count(), 1);
    let scene = SwitchGame::SCENE.replace("target: 5", &format!("target: {target}"));
    let admitted = admit::<SwitchGame>(&scene).expect("valid authored Switchboard scene");
    let controller = Controller::new(&admitted, None).expect("real standalone controller");
    assert_eq!(controller.snapshot().tick(), 0);
    assert!(controller.paused());
    assert!(!controller.focused());
    assert!(!controller.neutral_pending());
    assert!(controller.held() == SwitchInput::default());
    assert_eq!(
        state(&controller),
        json!({"position":[0,0],"selected":0,"held":0,"bits":0,
               "target":target,"moves":0,"won":0})
    );
    assert_eq!(
        admitted.identity()["initial_checksum"],
        format!("0x{:016x}", controller.snapshot().predicted().checksum())
    );
    controller
}

fn fingerprint(items: &[RenderItem]) -> Fingerprint {
    items
        .iter()
        .map(|item| {
            // Include every rendered property and preserve draw order. Entity
            // handles are not visible and must not masquerade as an objective.
            let RenderItem {
                entity: _,
                transform: Transform2 { pos, rot },
                style:
                    Style {
                        shape,
                        size,
                        half_y,
                        color,
                    },
            } = *item;
            let values = [
                pos.x, pos.y, rot, size, half_y, color[0], color[1], color[2], color[3],
            ];
            assert!(values.iter().all(|value| value.is_finite()));
            (shape, values.map(f32::to_bits))
        })
        .collect()
}

fn rendered(controller: &Controller<SwitchGame>) -> Fingerprint {
    let snapshot = controller.snapshot();
    let before_state = state(controller);
    let before_checksum = snapshot.predicted().checksum();
    let before_controls = (
        controller.paused(),
        controller.focused(),
        controller.neutral_pending(),
    );
    let before_input = controller.held();
    let mut items = Vec::new();
    SwitchGame::extract(snapshot.predicted(), &mut items);
    assert!(
        !items.is_empty(),
        "the actual presenter must produce a board"
    );
    let first = fingerprint(&items);
    // Reuse the populated output to exercise the real extraction contract.
    SwitchGame::extract(snapshot.predicted(), &mut items);
    assert_eq!(fingerprint(&items), first, "extraction must be repeatable");
    assert_eq!(snapshot.predicted().checksum(), before_checksum);
    assert_eq!(
        controller.snapshot().predicted().checksum(),
        before_checksum
    );
    assert_eq!(controller.snapshot().tick(), snapshot.tick());
    assert_eq!(state(controller), before_state);
    assert_eq!(
        (
            controller.paused(),
            controller.focused(),
            controller.neutral_pending()
        ),
        before_controls
    );
    assert!(controller.held() == before_input);
    first
}

fn same_except_target(left: &Controller<SwitchGame>, right: &Controller<SwitchGame>) {
    let mut left_state = state(left);
    let mut right_state = state(right);
    assert_eq!(left_state["target"], 3);
    assert_eq!(right_state["target"], 5);
    assert_ne!(
        left.snapshot().predicted().checksum(),
        right.snapshot().predicted().checksum()
    );
    left_state.as_object_mut().unwrap().remove("target");
    right_state.as_object_mut().unwrap().remove("target");
    assert_eq!(
        left_state, right_state,
        "only the authored objective may differ"
    );
    assert_eq!(left.snapshot().tick(), right.snapshot().tick());
    assert_eq!(left.paused(), right.paused());
    assert_eq!(left.focused(), right.focused());
    assert_eq!(left.neutral_pending(), right.neutral_pending());
    assert!(left.held() == right.held());
}

fn consume_one(controller: &mut Controller<SwitchGame>, input: Value) {
    let before = controller.snapshot().tick();
    controller.update(ONE_TICK).unwrap();
    assert_eq!(controller.snapshot().tick(), before + 1);
    let consumed = controller.take_consumed();
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0]["tick"], before + 1);
    assert_eq!(consumed[0]["input"], input);
    assert_eq!(consumed[0]["state"], state(controller));
    assert_eq!(
        consumed[0]["checksum"],
        format!("0x{:016x}", controller.snapshot().predicted().checksum())
    );
    assert!(controller.take_consumed().is_empty());
}

#[test]
fn authored_objective_is_visible_before_starting() {
    let left = fresh(3);
    let right = fresh(5);
    same_except_target(&left, &right);
    assert_ne!(
        rendered(&left),
        rendered(&right),
        "different admitted targets must be distinguishable in the paused player-facing board"
    );
}

#[test]
fn authored_objective_stays_visible_after_a_shared_nonwinning_action() {
    let mut left = fresh(3);
    let mut right = fresh(5);
    for controller in [&mut left, &mut right] {
        let initial = controller.snapshot();
        let initial_state = state(controller);
        controller.focus(true).unwrap();
        controller.key(Key::Enter, true, false, false).unwrap();
        assert!(!controller.paused());
        assert!(controller.neutral_pending());
        assert_eq!(controller.snapshot().tick(), 0);
        consume_one(controller, json!({"selection":0,"toggle":0}));
        assert!(!controller.neutral_pending());
        assert_eq!(state(controller), initial_state);
        controller.key(Key::Space, true, false, false).unwrap();
        consume_one(controller, json!({"selection":0,"toggle":1}));
        assert_eq!(controller.snapshot().tick(), 2);
        assert_eq!(
            state(controller),
            json!({"position":[0,0],"selected":0,"held":1,
            "bits":1,"target":initial_state["target"],"moves":1,"won":0})
        );
        assert!(controller.focused());
        assert!(!controller.paused());
        assert!(!controller.neutral_pending());
        assert_eq!(SwitchGame::state_value(initial.predicted()), initial_state);
    }
    same_except_target(&left, &right);
    assert_ne!(
        rendered(&left), rendered(&right),
        "different admitted targets must remain distinguishable after a naturally consumed nonwinning action"
    );
}

fn press(controller: &mut Controller<SwitchGame>, key: Key) {
    controller.key(key, true, false, false).unwrap();
}

fn release(controller: &mut Controller<SwitchGame>, key: Key) {
    controller.key(key, false, false, false).unwrap();
}

fn start(controller: &mut Controller<SwitchGame>) {
    let initial = state(controller);
    controller.focus(true).unwrap();
    press(controller, Key::Enter);
    assert!(controller.neutral_pending());
    consume_one(controller, json!({"selection":0,"toggle":0}));
    assert!(!controller.neutral_pending());
    assert_eq!(state(controller), initial);
}

// Find a path through actual one-switch actions, never solving before the
// requested current pattern. This covers every reachable four-bit pattern,
// including zero and the target itself, without injecting state or using Step.
fn path_to_bits(target: u32, desired: u32) -> Vec<u32> {
    let mut paths = vec![None; 16];
    paths[0] = Some(Vec::new());
    let mut queue = std::collections::VecDeque::from([0_u32]);
    while let Some(bits) = queue.pop_front() {
        if bits == desired {
            return paths[bits as usize].clone().unwrap();
        }
        for index in 0..4 {
            let next = bits ^ (1 << index);
            if paths[next as usize].is_some() || (next == target && next != desired) {
                continue;
            }
            let mut path = paths[bits as usize].clone().unwrap();
            path.push(index);
            paths[next as usize] = Some(path);
            queue.push_back(next);
        }
    }
    panic!("every four-bit pattern is reachable without an earlier win");
}

fn toggle_at(controller: &mut Controller<SwitchGame>, selected: u32) {
    release(controller, Key::Space);
    consume_one(controller, json!({"selection":0,"toggle":0}));
    while state(controller)["selected"] != selected {
        press(controller, Key::D);
        consume_one(controller, json!({"selection":1,"toggle":0}));
        release(controller, Key::D);
    }
    press(controller, Key::Space);
    consume_one(controller, json!({"selection":0,"toggle":1}));
}

fn expected_marker(shape: Shape, x: f32, y: f32, color: [f32; 4]) -> (Shape, [u32; 9]) {
    (
        shape,
        [x, y, 0.0, 0.4, 0.0, color[0], color[1], color[2], color[3]].map(f32::to_bits),
    )
}

fn bit_color(bits: u32, index: u32) -> [f32; 4] {
    if bits & (1 << index) != 0 {
        [0.2, 1.0, 0.2, 1.0]
    } else {
        [0.3, 0.3, 0.3, 1.0]
    }
}

// The checked row coordinates and radii lie in [0.25, 16). Their
// binary32 values are exact multiples of 2^-25, so integer arithmetic can
// compare complete glyph extents without rounding or floating arithmetic.
fn exact_row_units(bits: u32) -> i64 {
    assert_eq!(bits >> 31, 0, "row geometry must be nonnegative");
    let exponent = (bits >> 23) & 0xff;
    assert!((125..=130).contains(&exponent));
    let significand = (bits & 0x7f_ffff) | (1 << 23);
    i64::from(significand) << (exponent - 125)
}

fn assert_board(controller: &Controller<SwitchGame>, target: u32, bits: u32) -> Fingerprint {
    let current = state(controller);
    assert_eq!(current["target"], target);
    assert_eq!(current["bits"], bits);
    let items = rendered(controller);
    assert_eq!(
        items.len(),
        9,
        "one cursor, four current circles, four objective squares"
    );
    let selected = current["selected"].as_u64().unwrap();
    assert!(selected < 4);
    assert_eq!(current["position"], json!([selected * (2 << 16), 0]));
    let columns = [0.0, 2.0, 4.0, 6.0];
    let x = columns[selected as usize];
    let y = 0.0;
    assert_eq!(
        items[0],
        expected_marker(Shape::Circle, x, y, [0.3, 0.6, 1.0, 1.0])
    );
    for index in 0..4 {
        let x = columns[index as usize];
        assert_eq!(
            items[1 + index as usize],
            expected_marker(Shape::Circle, x, 2.0, bit_color(bits, index)),
            "the existing current-bit prefix must not change"
        );
        assert_eq!(
            items[5 + index as usize],
            expected_marker(Shape::Quad, x, 4.0, bit_color(target, index)),
            "objective squares must show authored bits in left-to-right bit order"
        );
        // Check complete glyph extents, not just unequal center coordinates.
        let current_top = exact_row_units(items[1 + index as usize].1[1])
            + exact_row_units(items[1 + index as usize].1[3]);
        let objective_bottom = exact_row_units(items[5 + index as usize].1[1])
            - exact_row_units(items[5 + index as usize].1[3]);
        assert!(
            objective_bottom > current_top,
            "objective and current rows must not overlap"
        );
    }
    items
}

#[test]
fn every_authored_target_is_distinct_and_independent_of_reachable_current_bits() {
    let mut objectives = Vec::new();
    for target in 1..16 {
        let scene = SwitchGame::SCENE.replace("target: 5", &format!("target: {target}"));
        let admitted = admit::<SwitchGame>(&scene).unwrap();
        let identity = admitted.identity().clone();
        let initial = fresh(target);
        let expected_objective = assert_board(&initial, target, 0)[5..].to_vec();
        assert!(
            !objectives.contains(&expected_objective),
            "all 15 authored targets must be visually distinct"
        );
        objectives.push(expected_objective.clone());
        for desired in 0..16 {
            let mut controller = Controller::new(&admitted, None).unwrap();
            assert_eq!(rendered(&controller), rendered(&initial));
            start(&mut controller);
            let path = path_to_bits(target, desired);
            for &index in &path {
                assert_eq!(
                    state(&controller)["won"],
                    0,
                    "no premature win or hidden reset"
                );
                toggle_at(&mut controller, index);
            }
            assert_eq!(state(&controller)["moves"], path.len());
            assert_eq!(state(&controller)["won"], u32::from(desired == target));
            let items = assert_board(&controller, target, desired);
            assert_eq!(
                items[5..],
                expected_objective,
                "changing current bits cannot rewrite the objective"
            );
            assert_eq!(
                admitted.identity(),
                &identity,
                "presentation and play cannot mutate scene identity"
            );
            assert_eq!(
                Controller::new(&admitted, None)
                    .unwrap()
                    .snapshot()
                    .predicted()
                    .checksum(),
                initial.snapshot().predicted().checksum(),
                "new sessions retain the admitted initial state"
            );
        }
    }
}

#[test]
fn objective_survives_win_release_restart_and_focus_pause_without_hidden_ticks() {
    for target in 1..16 {
        let mut controller = fresh(target);
        let objective = assert_board(&controller, target, 0)[5..].to_vec();
        start(&mut controller);
        for index in path_to_bits(target, target) {
            toggle_at(&mut controller, index);
        }
        assert_eq!(state(&controller)["won"], 1);
        assert_eq!(assert_board(&controller, target, target)[5..], objective);
        let won = state(&controller);
        consume_one(&mut controller, json!({"selection":0,"toggle":1}));
        assert_eq!(
            state(&controller),
            won,
            "held action cannot restart a solved board"
        );
        release(&mut controller, Key::Space);
        consume_one(&mut controller, json!({"selection":0,"toggle":0}));
        assert_eq!(state(&controller)["held"], 0);
        assert_eq!(state(&controller)["won"], 1);
        assert_eq!(assert_board(&controller, target, target)[5..], objective);
        press(&mut controller, Key::Space);
        consume_one(&mut controller, json!({"selection":0,"toggle":1}));
        assert_eq!(state(&controller)["won"], 0);
        assert_eq!(state(&controller)["moves"], 0);
        assert_eq!(assert_board(&controller, target, 0)[5..], objective);

        let before = controller.snapshot();
        let before_state = state(&controller);
        controller.focus(false).unwrap();
        assert!(controller.paused());
        assert!(!controller.focused());
        assert!(!controller.neutral_pending());
        assert!(controller.held() == SwitchInput::default());
        press(&mut controller, Key::Space);
        press(&mut controller, Key::Enter);
        controller.update(Duration::from_secs(5)).unwrap();
        assert_eq!(controller.snapshot().tick(), before.tick());
        assert_eq!(
            controller.snapshot().predicted().checksum(),
            before.predicted().checksum()
        );
        assert_eq!(state(&controller), before_state);
        assert!(controller.take_consumed().is_empty());
        assert_eq!(assert_board(&controller, target, 0)[5..], objective);
        controller.focus(true).unwrap();
        assert!(controller.paused(), "focus gain alone must not resume");
        press(&mut controller, Key::Enter);
        assert!(controller.neutral_pending());
        consume_one(&mut controller, json!({"selection":0,"toggle":0}));
        assert_eq!(state(&controller)["held"], 0);
        assert_eq!(state(&controller)["moves"], 0);
        assert_eq!(assert_board(&controller, target, 0)[5..], objective);
        consume_one(&mut controller, json!({"selection":0,"toggle":0}));
        assert_eq!(
            state(&controller)["moves"],
            0,
            "discarded focus-lost press must stay discarded"
        );
    }
}

#[test]
fn objective_hint_explains_rows_without_changing_delivery_fallback() {
    use orr_static_starter::delivery::DeliveryGame;

    assert_eq!(
        SwitchGame::presentation_hint(),
        "Upper target squares; lower current circles; green on, gray off; match each column"
    );
    assert_eq!(DeliveryGame::presentation_hint(), "");
    let admitted = admit::<DeliveryGame>(DeliveryGame::SCENE).unwrap();
    let identity = admitted.identity().clone();
    let controller = Controller::new(&admitted, None).unwrap();
    let before = controller.snapshot();
    let checksum = before.predicted().checksum();
    let initial_state = DeliveryGame::state_value(before.predicted());
    let mut items = Vec::new();
    DeliveryGame::extract(before.predicted(), &mut items);
    assert_eq!(
        fingerprint(&items),
        vec![
            expected_marker(Shape::Circle, 0.0, 0.0, [0.3, 0.6, 1.0, 1.0]),
            expected_marker(Shape::Circle, 4.0, 0.0, [1.0, 0.8, 0.2, 1.0]),
        ]
    );
    assert_eq!(controller.snapshot().predicted().checksum(), checksum);
    assert_eq!(controller.snapshot().tick(), 0);
    assert_eq!(
        DeliveryGame::state_value(controller.snapshot().predicted()),
        initial_state
    );
    assert_eq!(admitted.identity(), &identity);
    assert!(controller.paused());
    assert!(!controller.focused());
    assert!(!controller.neutral_pending());
}

#[cfg(feature = "static-game-actions")]
#[test]
fn objective_presentation_does_not_opt_legacy_games_into_static_action_profiles() {
    use orr_static_starter::{delivery::DeliveryGame, game_contract::HeldKeys};

    assert!(SwitchGame::action_profile().is_none());
    assert!(DeliveryGame::action_profile().is_none());
    for x in -1..=1 {
        for action in [false, true] {
            let keys = HeldKeys::new(x, 0, action).unwrap();
            assert_eq!(
                SwitchGame::input_value(&SwitchGame::held_input(keys)),
                json!({"selection":x,"toggle":u32::from(action)})
            );
            assert_eq!(
                DeliveryGame::input_value(&DeliveryGame::held_input(keys)),
                json!({"dx":x,"dy":0,"deliver":u32::from(action)})
            );
        }
    }
}
