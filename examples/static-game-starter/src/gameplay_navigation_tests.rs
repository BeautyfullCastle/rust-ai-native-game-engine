//! CPU-only semantic codec and egui presenter coverage. No host, socket or device.
use super::*;
use crate::{delivery::DeliveryInput, switchboard::SwitchInput};
use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};
use orr_sim::{Game, PlayerSlot, Simulation, TickInputs};

fn context() -> CommandContext {
    CommandContext {
        sequence: 1,
        scope: CommandScope {
            scene_epoch: 3,
            play_epoch: 4,
        },
        player: 0,
    }
}
fn make_frame<G: Game<Config = ()>>() -> Frame {
    let mut frame = Frame::new(Simulation::<G>::build_registry());
    G::setup(&mut frame, &());
    frame
}
fn courier(frame: &Frame) -> Entity {
    frame.dense::<Courier>().0[0]
}
fn cursor(frame: &Frame) -> Entity {
    frame.dense::<Cursor>().0[0]
}
fn position(x: i64, y: i64) -> FPVec2 {
    FPVec2::new(FP::from_raw(x), FP::from_raw(y))
}
fn tick<G: Game>(sim: &mut Simulation<G>, input: G::Input) {
    let mut inputs = TickInputs::new(sim.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}
fn delivery_input(output: &CodecOutput) -> DeliveryInput {
    DeliveryInput {
        dx: i32::try_from(output.input["dx"].as_i64().unwrap()).unwrap(),
        dy: i32::try_from(output.input["dy"].as_i64().unwrap()).unwrap(),
        deliver: u32::try_from(output.input["deliver"].as_u64().unwrap()).unwrap(),
    }
}
fn switch_input(output: &CodecOutput) -> SwitchInput {
    SwitchInput {
        selection: FP::from_int(
            i32::try_from(output.input["selection"].as_i64().unwrap()).unwrap(),
        ),
        toggle: u32::try_from(output.input["toggle"].as_u64().unwrap()).unwrap(),
        pad: 0,
    }
}

#[test]
fn navigation_catalog_requires_exact_integer_payloads() {
    for (alias, name, fields, valid) in [
        (
            "delivery",
            "move",
            vec!["dx", "dy"],
            json!({"dx":1,"dy":-1}),
        ),
        ("switchboard", "select", vec!["delta"], json!({"delta":-1})),
    ] {
        let contract = registry().get(alias).unwrap();
        let frame = if alias == "delivery" {
            make_frame::<DeliveryGame>()
        } else {
            make_frame::<SwitchGame>()
        };
        let admit = |name: &str, value: J| {
            let intent = contract.validate_intent(IntentRequest::new(name, value).unwrap())?;
            contract.encode(&intent, context(), FrameView::of(&frame))
        };
        let catalog: Vec<_> = contract.intent_catalog().collect();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog[0].id, "act");
        assert_eq!(catalog[1].id, name);
        for field in &fields {
            let schema = &catalog[1].payload_schema["properties"][*field];
            assert_eq!(schema["type"], "integer");
            assert_eq!(schema["minimum"], -1);
            assert_eq!(schema["maximum"], 1);
        }
        let canonical = admit(name, valid.clone()).unwrap();
        // JSON Schema integers describe values, not lexical spellings. The
        // reflected parser accepts exact integral decimals and exponent forms.
        for literal in ["-1.0", "-1e0"] {
            let mut equivalent = valid.clone();
            let field = fields[fields.len() - 1];
            equivalent[field] = serde_json::from_str(literal).unwrap();
            assert_eq!(admit(name, equivalent).unwrap().input(), canonical.input());
        }
        let act = admit("act", json!({"press":1})).unwrap();
        assert_eq!(
            admit("act", json!({"press":1.0})).unwrap().input(),
            act.input()
        );
        let mut invalid = vec![J::Null, json!([]), json!({}), json!(1)];
        for field in &fields {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(*field);
            invalid.push(missing);
            for bad in [
                json!(-2),
                json!(2),
                json!(1.5),
                json!("1"),
                json!("NaN"),
                json!("Infinity"),
                json!(true),
                J::Null,
            ] {
                let mut value = valid.clone();
                value[*field] = bad;
                invalid.push(value);
            }
        }
        let mut extra = valid.clone();
        extra["extra"] = json!(0);
        invalid.push(extra);
        for value in invalid {
            assert!(
                admit(name, value.clone()).is_err(),
                "accepted {name} {value}"
            );
        }
        let other = if alias == "delivery" {
            "select"
        } else {
            "move"
        };
        assert!(contract
            .validate_intent(IntentRequest::new(other, valid).unwrap())
            .is_err());
        for bad in [
            json!({"press":0}),
            json!({"press":1.5}),
            json!({"press":"1"}),
            json!({"press":1,"extra":0}),
        ] {
            assert!(admit("act", bad).is_err());
        }
    }
}

#[test]
fn encoders_reject_zero_unknown_payloads_wrong_players_and_actor_counts() {
    let mut delivery = make_frame::<DeliveryGame>();
    let mut switch = make_frame::<SwitchGame>();
    for (name, payload) in [
        ("move", json!({"dx":0,"dy":0})),
        ("move", json!({"dx":1})),
        ("move", json!({"dx":1,"dy":0,"extra":0})),
        ("move", json!({"dx":"1","dy":0})),
        ("move", json!({"dx":2,"dy":0})),
        ("select", json!({"delta":1})),
        ("act", json!({"press":0})),
    ] {
        assert!(delivery_encode(name, &payload, context(), FrameView::of(&delivery)).is_err());
    }
    for (name, payload) in [
        ("select", json!({"delta":0})),
        ("select", json!({})),
        ("select", json!({"delta":1,"extra":0})),
        ("select", json!({"delta":true})),
        ("select", json!({"delta":-2})),
        ("move", json!({"dx":1,"dy":0})),
        ("act", json!({"press":1.5})),
    ] {
        assert!(switch_encode(name, &payload, context(), FrameView::of(&switch)).is_err());
    }
    let wrong = CommandContext {
        player: 1,
        ..context()
    };
    assert!(delivery_encode("act", &json!({"press":1}), wrong, FrameView::of(&delivery)).is_err());
    assert!(switch_encode("act", &json!({"press":1}), wrong, FrameView::of(&switch)).is_err());
    delivery.singleton_mut::<Delivery>().action_held = 1;
    let id = cursor(&switch);
    switch.get_mut::<Cursor>(id).unwrap().held = 1;
    assert!(delivery_encode(
        "move",
        &json!({"dx":1,"dy":0}),
        context(),
        FrameView::of(&delivery)
    )
    .is_err());
    assert!(switch_encode(
        "select",
        &json!({"delta":1}),
        context(),
        FrameView::of(&switch)
    )
    .is_err());
    delivery.singleton_mut::<Delivery>().action_held = 0;
    switch.get_mut::<Cursor>(id).unwrap().held = 0;
    let delivery_receipt = delivery_encode(
        "act",
        &json!({"press":1}),
        context(),
        FrameView::of(&delivery),
    )
    .unwrap()
    .receipt;
    let switch_receipt = switch_encode(
        "act",
        &json!({"press":1}),
        context(),
        FrameView::of(&switch),
    )
    .unwrap()
    .receipt;
    for count in [0, 2] {
        let mut d = delivery.clone();
        let mut s = switch.clone();
        if count == 0 {
            d.despawn(courier(&d));
            s.despawn(cursor(&s));
        } else {
            let e = d.spawn();
            d.add(e, Courier::default());
            let e = s.spawn();
            s.add(e, Cursor::default());
        }
        assert!(delivery_encode("act", &json!({"press":1}), context(), FrameView::of(&d)).is_err());
        assert!(
            switch_encode("select", &json!({"delta":1}), context(), FrameView::of(&s)).is_err()
        );
        assert_eq!(
            delivery_observe(FrameView::of(&d), &delivery_receipt, context()).unwrap(),
            Consumption::Invalid
        );
        assert_eq!(
            switch_observe(FrameView::of(&s), &switch_receipt, context()).unwrap(),
            Consumption::Invalid
        );
    }
}

#[test]
fn delivery_move_receipt_proves_exact_raw_state_after_delayed_observation() {
    let mut frame = make_frame::<DeliveryGame>();
    let id = courier(&frame);
    frame.get_mut::<Courier>(id).unwrap().position = position(32769, -98305);
    frame.singleton_mut::<Delivery>().goal = position(98305, -163841);
    let before = frame.to_bytes();
    let output = delivery_encode(
        "move",
        &json!({"dx":1,"dy":-1}),
        context(),
        FrameView::of(&frame),
    )
    .unwrap();
    assert_eq!(frame.to_bytes(), before, "encoding must be read-only");
    assert_eq!(output.input, json!({"dx":1,"dy":-1,"deliver":2}));
    assert_eq!(output.receipt["kind"], "move");
    assert_eq!(
        output.receipt["expected"],
        json!({"position":[98305,-163841],"goal":[98305,-163841],"won":0})
    );
    assert_eq!(
        delivery_observe(FrameView::of(&frame), &output.receipt, context()).unwrap(),
        Consumption::Pending
    );
    let mut sim =
        Simulation::<DeliveryGame>::from_frame(&frame, crate::TICK_RATE, crate::delivery::BUILD)
            .unwrap();
    for _ in 0..40 {
        tick(&mut sim, delivery_input(&output));
    }
    assert_eq!(
        delivery_observe(FrameView::of(sim.frame()), &output.receipt, context()).unwrap(),
        Consumption::Consumed
    );
    assert_eq!(
        sim.frame().singleton::<Delivery>().won,
        0,
        "navigation never delivers at goal"
    );
    assert_eq!(
        sim.frame().get::<Courier>(id).unwrap().position,
        position(98305, -163841)
    );
    tick(&mut sim, DeliveryInput::default());
    assert_eq!(
        delivery_observe(FrameView::of(sim.frame()), &output.receipt, context()).unwrap(),
        Consumption::Invalid,
        "neutral after lost observation is not consumption proof"
    );
}

#[test]
fn delivery_navigation_rejects_atomic_raw_overflow_and_victory() {
    for (x, y, dx, dy) in [
        (i64::MAX, 5, 1, 1),
        (i64::MIN, 5, -1, 1),
        (5, i64::MAX, 1, 1),
        (5, i64::MIN, 1, -1),
    ] {
        let mut frame = make_frame::<DeliveryGame>();
        let id = courier(&frame);
        frame.get_mut::<Courier>(id).unwrap().position = position(x, y);
        let bytes = frame.to_bytes();
        assert!(delivery_encode(
            "move",
            &json!({"dx":dx,"dy":dy}),
            context(),
            FrameView::of(&frame)
        )
        .is_err());
        assert_eq!(frame.to_bytes(), bytes);
    }
    let mut frame = make_frame::<DeliveryGame>();
    let id = courier(&frame);
    frame.get_mut::<Courier>(id).unwrap().position =
        position(i64::MAX - (1 << 16), i64::MIN + (1 << 16));
    let output = delivery_encode(
        "move",
        &json!({"dx":1,"dy":-1}),
        context(),
        FrameView::of(&frame),
    )
    .unwrap();
    assert_eq!(
        output.receipt["expected"]["position"],
        json!([i64::MAX, i64::MIN])
    );
    frame.singleton_mut::<Delivery>().won = 1;
    assert!(delivery_encode(
        "move",
        &json!({"dx":1,"dy":0}),
        context(),
        FrameView::of(&frame)
    )
    .is_err());
    assert!(delivery_encode("act", &json!({"press":1}), context(), FrameView::of(&frame)).is_ok());
}

#[test]
fn switch_selection_wraps_custom_coordinates_and_preserves_every_board_field() {
    for (selected, delta, expected) in [(0, -1, 3), (3, 1, 0), (2, -1, 1), (u32::MAX, 1, 0)] {
        let mut frame = make_frame::<SwitchGame>();
        let id = cursor(&frame);
        *frame.get_mut::<Cursor>(id).unwrap() = Cursor {
            position: position(-70001, 32769),
            selected,
            held: 0,
        };
        frame.set_singleton(Board {
            bits: 11,
            target: 7,
            solved: 1,
            moves: 29,
        });
        let before = frame.to_bytes();
        let output = switch_encode(
            "select",
            &json!({"delta":delta}),
            context(),
            FrameView::of(&frame),
        )
        .unwrap();
        assert_eq!(frame.to_bytes(), before);
        assert_eq!(output.input, json!({"selection":delta,"toggle":2}));
        assert_eq!(output.receipt["kind"], "select");
        assert_eq!(
            output.receipt["expected"],
            json!({"position":[i64::from(expected)*(2<<16),32769],
            "selected":expected,"bits":11,"target":7,"solved":1,"moves":29})
        );
        assert_eq!(
            switch_observe(FrameView::of(&frame), &output.receipt, context()).unwrap(),
            Consumption::Pending
        );
        let mut sim = Simulation::<SwitchGame>::from_frame(
            &frame,
            crate::TICK_RATE,
            crate::switchboard::BUILD,
        )
        .unwrap();
        for _ in 0..40 {
            tick(&mut sim, switch_input(&output));
        }
        assert_eq!(
            switch_observe(FrameView::of(sim.frame()), &output.receipt, context()).unwrap(),
            Consumption::Consumed
        );
        assert_eq!(sim.frame().get::<Cursor>(id).unwrap().selected, expected);
        assert_eq!(
            sim.frame().get::<Cursor>(id).unwrap().position,
            position(i64::from(expected) * (2 << 16), 32769)
        );
        for field in ["bits", "target", "solved", "moves"] {
            assert_eq!(
                output.receipt["expected"][field],
                output.receipt["before"][field]
            );
        }
    }
}

#[test]
fn receipts_invalidate_malformed_identity_or_any_contradictory_held_state() {
    let d = make_frame::<DeliveryGame>();
    let s = make_frame::<SwitchGame>();
    let d_output = delivery_encode(
        "move",
        &json!({"dx":1,"dy":0}),
        context(),
        FrameView::of(&d),
    )
    .unwrap();
    let s_output =
        switch_encode("select", &json!({"delta":1}), context(), FrameView::of(&s)).unwrap();
    let mut ds =
        Simulation::<DeliveryGame>::from_frame(&d, crate::TICK_RATE, crate::delivery::BUILD)
            .unwrap();
    let mut ss =
        Simulation::<SwitchGame>::from_frame(&s, crate::TICK_RATE, crate::switchboard::BUILD)
            .unwrap();
    tick(&mut ds, delivery_input(&d_output));
    tick(&mut ss, switch_input(&s_output));
    let wrong_player = CommandContext {
        player: 1,
        ..context()
    };
    assert_eq!(
        delivery_observe(FrameView::of(ds.frame()), &d_output.receipt, wrong_player).unwrap(),
        Consumption::Invalid
    );
    assert_eq!(
        switch_observe(FrameView::of(ss.frame()), &s_output.receipt, wrong_player).unwrap(),
        Consumption::Invalid
    );
    for (key, bad) in [
        ("kind", json!("other")),
        ("player", json!(1)),
        ("player", json!("0")),
        ("actor", json!([999, 0])),
        ("before", J::Null),
        ("expected", json!({})),
    ] {
        let mut dr = d_output.receipt.clone();
        dr[key] = bad.clone();
        let mut sr = s_output.receipt.clone();
        sr[key] = bad;
        assert_eq!(
            delivery_observe(FrameView::of(ds.frame()), &dr, context()).unwrap(),
            Consumption::Invalid
        );
        assert_eq!(
            switch_observe(FrameView::of(ss.frame()), &sr, context()).unwrap(),
            Consumption::Invalid
        );
    }
    for key in ["position", "goal", "won"] {
        let mut receipt = d_output.receipt.clone();
        receipt["expected"][key] = json!("bad");
        assert_eq!(
            delivery_observe(FrameView::of(ds.frame()), &receipt, context()).unwrap(),
            Consumption::Invalid
        );
    }
    for key in ["position", "selected", "bits", "target", "solved", "moves"] {
        let mut receipt = s_output.receipt.clone();
        receipt["expected"][key] = json!("bad");
        assert_eq!(
            switch_observe(FrameView::of(ss.frame()), &receipt, context()).unwrap(),
            Consumption::Invalid
        );
    }
    // Every proof field is material, even one raw fractional bit or a latch > 1.
    for change in 0..6 {
        let mut f = ds.frame().clone();
        let id = courier(&f);
        match change {
            0 => f.get_mut::<Courier>(id).unwrap().position.x += FP::from_raw(1),
            1 => f.get_mut::<Courier>(id).unwrap().position.y += FP::from_raw(1),
            2 => f.singleton_mut::<Delivery>().goal.x += FP::from_raw(1),
            3 => f.singleton_mut::<Delivery>().goal.y += FP::from_raw(1),
            4 => f.singleton_mut::<Delivery>().won = 1,
            _ => f.singleton_mut::<Delivery>().action_held = 2,
        }
        assert_eq!(
            delivery_observe(FrameView::of(&f), &d_output.receipt, context()).unwrap(),
            Consumption::Invalid
        );
    }
    for change in 0..8 {
        let mut f = ss.frame().clone();
        let id = cursor(&f);
        match change {
            0 => f.get_mut::<Cursor>(id).unwrap().position.x += FP::from_raw(1),
            1 => f.get_mut::<Cursor>(id).unwrap().position.y += FP::from_raw(1),
            2 => f.get_mut::<Cursor>(id).unwrap().selected = 2,
            3 => f.singleton_mut::<Board>().bits ^= 1,
            4 => f.singleton_mut::<Board>().target ^= 1,
            5 => f.singleton_mut::<Board>().solved = 1,
            6 => f.singleton_mut::<Board>().moves = 1,
            _ => f.get_mut::<Cursor>(id).unwrap().held = 2,
        }
        assert_eq!(
            switch_observe(FrameView::of(&f), &s_output.receipt, context()).unwrap(),
            Consumption::Invalid
        );
    }
    // Actor replacement is invalid even if the replacement reproduces all values.
    let mut f = ds.frame().clone();
    let old = courier(&f);
    let value = *f.get::<Courier>(old).unwrap();
    f.despawn(old);
    let new = f.spawn();
    f.add(new, value);
    assert_eq!(
        delivery_observe(FrameView::of(&f), &d_output.receipt, context()).unwrap(),
        Consumption::Invalid
    );
    let mut f = ss.frame().clone();
    let old = cursor(&f);
    let value = *f.get::<Cursor>(old).unwrap();
    f.despawn(old);
    let new = f.spawn();
    f.add(new, value);
    assert_eq!(
        switch_observe(FrameView::of(&f), &s_output.receipt, context()).unwrap(),
        Consumption::Invalid
    );
}

#[test]
fn act_receipts_prove_fractional_delivery_win_origin_restart_and_switch_reset() {
    let mut frame = make_frame::<DeliveryGame>();
    let id = courier(&frame);
    frame.get_mut::<Courier>(id).unwrap().position = position(32769, -17);
    frame.singleton_mut::<Delivery>().goal = position(32769, -17);
    let mut sim =
        Simulation::<DeliveryGame>::from_frame(&frame, crate::TICK_RATE, crate::delivery::BUILD)
            .unwrap();
    for won in [1, 0] {
        let output = delivery_encode(
            "act",
            &json!({"press":1}),
            context(),
            FrameView::of(sim.frame()),
        )
        .unwrap();
        for _ in 0..10 {
            tick(&mut sim, delivery_input(&output));
        }
        assert_eq!(
            delivery_observe(FrameView::of(sim.frame()), &output.receipt, context()).unwrap(),
            Consumption::Consumed
        );
        assert_eq!(sim.frame().singleton::<Delivery>().won, won);
        assert_eq!(
            sim.frame().get::<Courier>(id).unwrap().position,
            if won == 1 {
                position(32769, -17)
            } else {
                FPVec2::ZERO
            }
        );
        tick(&mut sim, DeliveryInput::default());
    }
    let mut frame = make_frame::<SwitchGame>();
    let id = cursor(&frame);
    *frame.get_mut::<Cursor>(id).unwrap() = Cursor {
        position: position(-5, 12345),
        selected: 2,
        held: 0,
    };
    frame.set_singleton(Board {
        bits: 1,
        target: 5,
        moves: u32::MAX,
        solved: 0,
    });
    let mut sim =
        Simulation::<SwitchGame>::from_frame(&frame, crate::TICK_RATE, crate::switchboard::BUILD)
            .unwrap();
    for solved in [1, 0] {
        let output = switch_encode(
            "act",
            &json!({"press":1}),
            context(),
            FrameView::of(sim.frame()),
        )
        .unwrap();
        for _ in 0..10 {
            tick(&mut sim, switch_input(&output));
        }
        assert_eq!(
            switch_observe(FrameView::of(sim.frame()), &output.receipt, context()).unwrap(),
            Consumption::Consumed
        );
        let state = sim.frame().singleton::<Board>();
        assert_eq!(state.solved, solved);
        assert_eq!(state.bits, if solved == 1 { 5 } else { 0 });
        assert_eq!(state.moves, if solved == 1 { u32::MAX } else { 0 });
        assert_eq!(state.target, 5);
        let c = sim.frame().get::<Cursor>(id).unwrap();
        assert_eq!(c.selected, 2);
        assert_eq!(c.position, position(4 << 16, 12345));
        tick(&mut sim, SwitchInput::default());
    }
}

fn ui_frame(
    ctx: &egui::Context,
    view: &mut dyn EmbeddedGameplayView,
    frame: &Frame,
    context: GameplayViewContext,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<IntentRequest>) {
    let bytes = frame.to_bytes();
    let mut intents = Vec::new();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            if let Some(intent) = view.show(ui, FrameView::of(frame), context).unwrap() {
                intents.push(intent);
            }
        },
    );
    assert!(intents.len() <= 1, "at most one intent per rendered frame");
    assert_eq!(
        frame.to_bytes(),
        bytes,
        "presenter and extractor are read-only"
    );
    (output, intents.pop())
}
fn label_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.visual_bounding_rect().center())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("missing {label} button"))
}
fn click(
    alias: &str,
    frame: &Frame,
    context: GameplayViewContext,
    label: &str,
) -> Option<IntentRequest> {
    let ctx = egui::Context::default();
    let mut view = registry().get(alias).unwrap().create_view().unwrap();
    let (output, initial) = ui_frame(&ctx, view.as_mut(), frame, context, vec![]);
    assert!(initial.is_none());
    let pos = label_center(&output, label);
    let (_, press) = ui_frame(
        &ctx,
        view.as_mut(),
        frame,
        context,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    assert!(press.is_none());
    let (_, release) = ui_frame(
        &ctx,
        view.as_mut(),
        frame,
        context,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    let (_, idle) = ui_frame(&ctx, view.as_mut(), frame, context, vec![]);
    assert!(idle.is_none(), "holding no click must not resubmit");
    release
}
fn live() -> GameplayViewContext {
    GameplayViewContext {
        phase: GameplayViewPhase::Live,
        input_ready: true,
    }
}

#[test]
fn presenter_emits_all_registered_buttons_without_mutating_the_frame() {
    for (alias, frame, buttons) in [
        (
            "delivery",
            make_frame::<DeliveryGame>(),
            vec![
                ("Act", json!({"dx":0,"dy":0,"deliver":1})),
                ("Left", json!({"dx":-1,"dy":0,"deliver":2})),
                ("Right", json!({"dx":1,"dy":0,"deliver":2})),
                ("Up", json!({"dx":0,"dy":1,"deliver":2})),
                ("Down", json!({"dx":0,"dy":-1,"deliver":2})),
            ],
        ),
        (
            "switchboard",
            make_frame::<SwitchGame>(),
            vec![
                ("Act", json!({"selection":0,"toggle":1})),
                ("Previous", json!({"selection":-1,"toggle":2})),
                ("Next", json!({"selection":1,"toggle":2})),
            ],
        ),
    ] {
        let contract = registry().get(alias).unwrap();
        for (label, expected) in buttons {
            let request = click(alias, &frame, live(), label)
                .unwrap_or_else(|| panic!("{label} must emit an intent"));
            let intent = contract.validate_intent(request).unwrap();
            let command = contract
                .encode(&intent, context(), FrameView::of(&frame))
                .unwrap();
            assert_eq!(command.input(), &expected);
        }
    }
}

#[test]
fn presenter_disables_unavailable_actions_and_does_not_bind_keyboard_navigation() {
    for (alias, initial, navigation) in [
        ("delivery", make_frame::<DeliveryGame>(), "Right"),
        ("switchboard", make_frame::<SwitchGame>(), "Next"),
    ] {
        for phase in [
            GameplayViewPhase::Edit,
            GameplayViewPhase::Paused,
            GameplayViewPhase::Viewer,
        ] {
            for label in ["Act", navigation] {
                assert!(click(
                    alias,
                    &initial,
                    GameplayViewContext {
                        phase,
                        input_ready: true
                    },
                    label
                )
                .is_none());
            }
        }
        for label in ["Act", navigation] {
            assert!(click(
                alias,
                &initial,
                GameplayViewContext {
                    input_ready: false,
                    ..live()
                },
                label
            )
            .is_none());
            let mut held = initial.clone();
            if alias == "delivery" {
                held.singleton_mut::<Delivery>().action_held = 1;
            } else {
                let id = cursor(&held);
                held.get_mut::<Cursor>(id).unwrap().held = 1;
            }
            assert!(click(alias, &held, live(), label).is_none());
            let mut missing = initial.clone();
            let id = if alias == "delivery" {
                courier(&missing)
            } else {
                cursor(&missing)
            };
            missing.despawn(id);
            assert!(click(alias, &missing, live(), label).is_none());
        }
        let ctx = egui::Context::default();
        let mut view = registry().get(alias).unwrap().create_view().unwrap();
        let (_, intent) = ui_frame(
            &ctx,
            view.as_mut(),
            &initial,
            live(),
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(intent.is_none());
    }
    let mut won = make_frame::<DeliveryGame>();
    won.singleton_mut::<Delivery>().won = 1;
    assert!(click("delivery", &won, live(), "Right").is_none());
    assert!(click("delivery", &won, live(), "Act").is_some());
    let mut edge = make_frame::<DeliveryGame>();
    let id = courier(&edge);
    edge.get_mut::<Courier>(id).unwrap().position.x = FP::from_raw(i64::MAX);
    assert!(click("delivery", &edge, live(), "Right").is_none());
    assert!(click("delivery", &edge, live(), "Left").is_some());
    let mut solved = make_frame::<SwitchGame>();
    solved.singleton_mut::<Board>().solved = 1;
    assert!(click("switchboard", &solved, live(), "Next").is_some());
    assert!(click("switchboard", &solved, live(), "Act").is_some());
}
