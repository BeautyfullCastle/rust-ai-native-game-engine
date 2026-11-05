//! CPU-only authored Delivery diagnostics through the shared document and editor UI.
//! No native window, device, socket, or external process is involved.
use orr_edit::{Op, Origin};
use orr_fp::{FPVec2, FP};
use orr_reflect::Value;
use orr_session::{ControlOp, PlaySession};
use orr_sim::PlayerSlot;
use orr_static_starter::{
    delivery::{
        Courier, Delivery, DeliveryGame, DeliveryInput, MODE_NAVIGATION_PULSE, MODE_PRIMARY,
        MODE_RELEASED,
    },
    game_contract::{admit, document, Starter},
};

const UNIT: i64 = 1 << 16;

fn point(x: i64, y: i64) -> FPVec2 {
    FPVec2::new(FP::from_raw(x), FP::from_raw(y))
}

fn position(session: &PlaySession<DeliveryGame>) -> FPVec2 {
    session.frame().dense::<Courier>().1[0].position
}

fn residues(p: FPVec2) -> [i64; 2] {
    [p.x.raw().rem_euclid(UNIT), p.y.raw().rem_euclid(UNIT)]
}

fn authored(start: FPVec2, goal: FPVec2) -> PlaySession<DeliveryGame> {
    let mut doc = document::<DeliveryGame>(DeliveryGame::SCENE).unwrap();
    let courier = doc.frame().dense::<Courier>().0[0];
    let guid = doc.index().guid(courier).unwrap().clone();
    doc.apply_atomic_batch(
        "author Delivery start and goal",
        vec![
            Op::SetField {
                guid,
                component: "Courier".into(),
                path: "position".into(),
                value: Value::Vec2(start),
            },
            Op::SetSingletonField {
                singleton: "Delivery".into(),
                path: "goal".into(),
                value: Value::Vec2(goal),
            },
        ],
        Origin::User,
    )
    .expect("ordinary coordinate edits pass the shared authoring admission");
    assert!(DeliveryGame::validate(doc.frame()));
    let checksum = doc.frame().checksum();
    let saved = doc.save_yaml();
    assert!(!doc.is_dirty());
    let reopened = document::<DeliveryGame>(&saved).expect("saved document reopens");
    assert_eq!(reopened.frame().checksum(), checksum);
    let admitted = admit::<DeliveryGame>(&saved).expect("saved scene passes player admission");
    let session = admitted.session().unwrap();
    assert_eq!(session.frame().checksum(), checksum);
    assert_eq!(session.frame().tick(), 0);
    assert_eq!(session.frame().dense::<Courier>().0.len(), 1);
    assert_eq!(position(&session), start);
    let state = session.frame().singleton::<Delivery>();
    assert_eq!(state.goal, goal);
    assert_eq!(state.won, 0);
    assert_eq!(state.action_held, 0);
    assert!(DeliveryGame::validate(session.frame()));
    session
}

fn tick(session: &mut PlaySession<DeliveryGame>, dx: i32, dy: i32, mode: u32) {
    let input = DeliveryInput {
        dx,
        dy,
        deliver: mode,
    };
    assert!(DeliveryGame::valid_input(&input));
    let before = session.frame().tick();
    session.set_input(PlayerSlot(0), input);
    session.control(ControlOp::Step(1));
    assert_eq!(session.frame().tick(), before + 1);
}

fn restart(session: &mut PlaySession<DeliveryGame>) {
    assert!(DeliveryGame::won(session.frame()));
    let goal = session.frame().singleton::<Delivery>().goal;
    tick(session, 0, 0, MODE_RELEASED);
    assert!(DeliveryGame::won(session.frame()));
    tick(session, 0, 0, MODE_PRIMARY);
    assert!(!DeliveryGame::won(session.frame()));
    assert_eq!(position(session), FPVec2::ZERO, "documented origin restart");
    assert_eq!(session.frame().singleton::<Delivery>().goal, goal);
    assert_eq!(session.frame().singleton::<Delivery>().action_held, 1);
}

// A bounded, explicit legal-input attempt, not an exhaustive path search. Eight
// moves suffice for the integer control and each compatible fractional first
// win used here. Each action gets a fresh edge, including after restart.
fn attempt_delivery(session: &mut PlaySession<DeliveryGame>) {
    let before = session.frame().tick();
    for _ in 0..8 {
        if DeliveryGame::won(session.frame()) {
            break;
        }
        tick(session, 0, 0, MODE_RELEASED);
        let p = position(session);
        let goal = session.frame().singleton::<Delivery>().goal;
        let direction = |from: FP, to: FP| match from.cmp(&to) {
            std::cmp::Ordering::Less => 1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => -1,
        };
        tick(
            session,
            direction(p.x, goal.x),
            direction(p.y, goal.y),
            MODE_PRIMARY,
        );
    }
    assert!(session.frame().tick() - before <= 16);
}

#[test]
fn authored_integer_scene_wins_and_wins_again_after_restart() {
    let mut session = authored(FPVec2::ZERO, point(4 * UNIT, 0));
    attempt_delivery(&mut session);
    assert!(DeliveryGame::won(session.frame()));
    assert_eq!(position(&session), point(4 * UNIT, 0));
    restart(&mut session);
    attempt_delivery(&mut session);
    assert!(DeliveryGame::won(session.frame()));
    assert_eq!(position(&session), point(4 * UNIT, 0));
}

#[test]
fn every_legal_direction_mode_and_latch_preserves_position_residues_before_win() {
    // This exhausts the 27 legal input combinations and both latch states at
    // integer and fractional starts. The all-path argument is algebraic: before
    // winning, each coordinate changes by -UNIT, 0, or UNIT; a suppressed or
    // overflowing navigation pulse changes neither. Every completed transition
    // therefore preserves raw-coordinate residues modulo UNIT. By induction no
    // sequence can reach a goal with different residues. Exact equality is
    // required to win, and origin reset is available only after a prior win.
    // The finite loop below checks those transition branches, not all paths.
    for start in [FPVec2::ZERO, point(UNIT + UNIT / 2, -UNIT / 4)] {
        for held in [false, true] {
            for mode in [MODE_RELEASED, MODE_PRIMARY, MODE_NAVIGATION_PULSE] {
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let mut session = authored(start, point(20 * UNIT, 20 * UNIT));
                        if held {
                            tick(&mut session, 0, 0, MODE_PRIMARY);
                        }
                        assert_eq!(position(&session), start);
                        assert_eq!(
                            session.frame().singleton::<Delivery>().action_held,
                            u32::from(held)
                        );
                        tick(&mut session, dx, dy, mode);
                        let moves = mode != MODE_NAVIGATION_PULSE || !held;
                        let expected = if moves {
                            point(
                                start.x.raw() + i64::from(dx) * UNIT,
                                start.y.raw() + i64::from(dy) * UNIT,
                            )
                        } else {
                            start
                        };
                        assert_eq!(position(&session), expected);
                        assert_eq!(residues(position(&session)), residues(start));
                        assert_eq!(
                            session.frame().singleton::<Delivery>().action_held,
                            u32::from(mode != MODE_RELEASED)
                        );
                        assert!(!DeliveryGame::won(session.frame()));
                    }
                }
            }
        }
    }
}

#[cfg(feature = "gameplay")]
mod diagnostics {
    use super::*;
    use orr_bridge::FrameView;
    use orr_editor::static_adapter::{
        GameplayViewContext, GameplayViewPhase, PresentationCapability,
    };

    const CURRENT_WARNING: &str = "Goal unreachable from the current position";
    const RESTART_WARNING: &str = "Goal unreachable after restart at the origin";

    fn rendered_text(session: &PlaySession<DeliveryGame>, phase: GameplayViewPhase) -> String {
        let descriptor = orr_static_starter::gameplay::descriptors()
            .into_iter()
            .find(|descriptor| descriptor.local_alias == DeliveryGame::ALIAS)
            .expect("the actual project registers Delivery");
        let PresentationCapability::Embedded { create } = descriptor.presentation else {
            panic!("Delivery must use its actual embedded presenter");
        };
        let mut view = create();
        let frame = session.frame();
        let bytes = frame.to_bytes();
        let checksum = frame.checksum();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let intent = view
                    .show(
                        ui,
                        FrameView::of(frame),
                        GameplayViewContext {
                            phase,
                            input_ready: phase == GameplayViewPhase::Live,
                        },
                    )
                    .expect("actual presenter renders admitted scene");
                assert!(intent.is_none(), "rendering must not submit a command");
            },
        );
        assert_eq!(frame.to_bytes(), bytes, "presenter must be read-only");
        assert_eq!(frame.checksum(), checksum);
        fn collect(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(label) => {
                    text.push_str(label.galley.text());
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in output.shapes {
            collect(&shape.shape, &mut text);
        }
        assert!(text.contains(DeliveryGame::ID), "actual heading is visible");
        assert!(text.contains("Act"), "actual game controls are visible");
        text
    }

    #[test]
    fn integer_scene_has_no_unreachable_warning_before_or_after_restart() {
        let mut session = authored(FPVec2::ZERO, point(4 * UNIT, 0));
        let initial = rendered_text(&session, GameplayViewPhase::Edit);
        attempt_delivery(&mut session);
        assert!(DeliveryGame::won(session.frame()));
        let won = rendered_text(&session, GameplayViewPhase::Live);
        restart(&mut session);
        let restarted = rendered_text(&session, GameplayViewPhase::Live);
        for text in [initial, won, restarted] {
            assert!(!text.contains(CURRENT_WARNING));
            assert!(!text.contains(RESTART_WARNING));
        }
    }

    #[test]
    fn authored_incompatible_goal_shows_current_position_warning() {
        let session = authored(FPVec2::ZERO, point(4 * UNIT + UNIT / 2, 0));
        assert_ne!(
            residues(position(&session)),
            residues(session.frame().singleton::<Delivery>().goal)
        );
        let text = rendered_text(&session, GameplayViewPhase::Edit);
        assert!(
            text.contains(CURRENT_WARNING),
            "author-visible presenter must explain unreachable current goal; got: {text}"
        );
        assert!(text.contains(RESTART_WARNING));
    }

    #[test]
    fn compatible_fractional_scene_shows_restart_warning_through_lifecycle() {
        let start = point(UNIT + UNIT / 2, -UNIT / 4);
        let goal = point(3 * UNIT + UNIT / 2, UNIT + 3 * UNIT / 4);
        let mut session = authored(start, goal);
        assert_eq!(residues(start), residues(goal));
        let initial = rendered_text(&session, GameplayViewPhase::Edit);
        attempt_delivery(&mut session);
        assert!(DeliveryGame::won(session.frame()), "first delivery works");
        assert_eq!(position(&session), goal);
        let won = rendered_text(&session, GameplayViewPhase::Live);
        restart(&mut session);
        assert_ne!(residues(position(&session)), residues(goal));
        let restarted = rendered_text(&session, GameplayViewPhase::Live);

        // Collect every lifecycle surface and verify all read-only controls
        // before any missing warning can fail this desired diagnostic contract.
        assert!(!initial.contains(CURRENT_WARNING));
        assert!(!won.contains(CURRENT_WARNING));
        assert!(
            initial.contains(RESTART_WARNING),
            "author-visible presenter must explain the restart limit; got: {initial}"
        );
        assert!(won.contains(RESTART_WARNING), "won presenter: {won}");
        assert!(
            restarted.contains(CURRENT_WARNING),
            "restart presenter: {restarted}"
        );
        assert!(restarted.contains(RESTART_WARNING));
    }
}
