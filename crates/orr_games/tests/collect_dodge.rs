#![cfg(feature = "collect-dodge")]
use orr_ecs::Frame;
use orr_fp::{FPVec2, FP};
use orr_games::collect_dodge_game::*;
use orr_sim::{PlayerFlags, PlayerSlot, Simulation, TickInputs};

fn p(x: i64, y: i64) -> FPVec2 {
    FPVec2::new(FP(x << 16), FP(y << 16))
}
fn level(coins: Vec<FPVec2>, hazards: Vec<HazardSpec>, limit: u32) -> CollectLevel {
    CollectLevel::new(p(0, 0), coins, hazards, limit).unwrap()
}
fn sim(coins: Vec<FPVec2>, hazards: Vec<HazardSpec>, limit: u32) -> Simulation<CollectDodgeV1> {
    Simulation::new(level(coins, hazards, limit), 60, 42)
}
fn step(
    s: &mut Simulation<CollectDodgeV1>,
    x: FP,
    y: FP,
    buttons: u32,
) -> Vec<orr_sim::SimEvent<CollectEvent>> {
    let mut input = TickInputs::new(s.frame().tick() + 1, 1);
    input.set_input(
        PlayerSlot(0),
        CollectInput {
            x,
            y,
            buttons,
            reserved: 0,
        },
    );
    s.step(&input)
}
fn run(s: &Simulation<CollectDodgeV1>) -> CollectRun {
    *s.frame().singleton::<CollectRun>()
}
fn actors(s: &Simulation<CollectDodgeV1>) -> Vec<CollectActor> {
    let mut out: Vec<_> = s.frame().dense::<CollectActor>().1.to_vec();
    out.sort_by_key(|a| (a.kind, a.ordinal));
    out
}
#[test]
fn collects_once_and_wins_at_authored_goal() {
    let mut s = sim(vec![p(5, 0), p(10, 0)], vec![], 100);
    let events = step(&mut s, FP::ONE, FP::ZERO, 0);
    assert_eq!(run(&s).score, 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload.kind, EVENT_COLLECTED);
    assert!(step(&mut s, FP::ZERO, FP::ZERO, 0).is_empty());
    for _ in 0..5 {
        step(&mut s, FP::ONE, FP::ZERO, 0);
    }
    assert_eq!(run(&s).phase, WON);
    assert_eq!(run(&s).score, 2);
    let before = actors(&s);
    let elapsed = run(&s).elapsed_ticks;
    assert!(step(&mut s, FP::ONE, FP::ONE, 0).is_empty());
    assert_eq!(actors(&s), before);
    assert_eq!(run(&s).elapsed_ticks, elapsed);
}
#[test]
fn hazard_precedes_collection_and_timeout() {
    let mut s = sim(
        vec![p(1, 0)],
        vec![HazardSpec {
            position: p(1, 0),
            velocity: p(0, 0),
        }],
        1,
    );
    let events = step(&mut s, FP::ZERO, FP::ZERO, 0);
    assert_eq!(run(&s).phase, LOST_HAZARD);
    assert_eq!(run(&s).score, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload.value, LOST_HAZARD);
}
#[test]
fn winning_on_deadline_precedes_timeout() {
    let mut s = sim(vec![p(5, 0)], vec![], 1);
    let events = step(&mut s, FP::ONE, FP::ZERO, 0);
    assert_eq!(run(&s).phase, WON);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload.kind, EVENT_COLLECTED);
    assert_eq!(events[1].payload.kind, EVENT_FINISHED);
}
#[test]
fn timeout_emitted_once_and_stops_motion() {
    let mut s = sim(vec![p(200, 0)], vec![], 2);
    assert!(step(&mut s, FP::ONE, FP::ZERO, 0).is_empty());
    assert_eq!(
        step(&mut s, FP::ONE, FP::ZERO, 0)[0].payload.value,
        LOST_TIMEOUT
    );
    let before = actors(&s);
    assert!(step(&mut s, FP::ONE, FP::ZERO, 0).is_empty());
    assert_eq!(actors(&s), before);
}
#[test]
fn restart_is_fresh_edge_and_restores_every_actor_and_progress() {
    let mut s = sim(
        vec![p(5, 0), p(100, 0)],
        vec![HazardSpec {
            position: p(80, 80),
            velocity: p(1, -1),
        }],
        50,
    );
    let initial = actors(&s);
    step(&mut s, FP::ONE, FP::ZERO, 0);
    assert_eq!(run(&s).score, 1);
    assert_eq!(
        step(&mut s, FP::ONE, FP::ZERO, RESTART)[0].payload.kind,
        EVENT_RESTARTED
    );
    assert_eq!(actors(&s), initial);
    assert_eq!(run(&s).elapsed_ticks, 0);
    assert_eq!(run(&s).score, 0);
    let events = step(&mut s, FP::ONE, FP::ZERO, RESTART);
    assert!(events.iter().all(|e| e.payload.kind != EVENT_RESTARTED));
    assert_eq!(run(&s).elapsed_ticks, 1);
    step(&mut s, FP::ZERO, FP::ZERO, 0);
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    assert_eq!(actors(&s), initial);
}
#[test]
fn terminal_state_can_restart_but_held_restart_does_not_retrigger() {
    let mut s = sim(vec![p(0, 0)], vec![], 20);
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    assert_eq!(run(&s).phase, WON);
    assert!(step(&mut s, FP::ZERO, FP::ZERO, RESTART).is_empty());
    step(&mut s, FP::ZERO, FP::ZERO, 0);
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    assert_eq!(run(&s).phase, PLAYING);
}
#[test]
fn hostile_raw_input_clamps_without_overflow() {
    let mut s = sim(vec![p(200, 200)], vec![], 1000);
    for _ in 0..300 {
        step(&mut s, FP(i64::MAX), FP(i64::MIN), !RESTART);
    }
    assert_eq!(actors(&s)[0].position, p(254, -254));
}
#[test]
fn hazard_reflects_overshoot_and_restarts_velocity() {
    let mut s = sim(
        vec![p(200, 0)],
        vec![HazardSpec {
            position: p(254, 254),
            velocity: p(1, 1),
        }],
        100,
    );
    step(&mut s, FP::ZERO, FP::ZERO, 0);
    let h = actors(&s).into_iter().find(|a| a.kind == HAZARD).unwrap();
    assert_eq!(h.position, p(253, 253));
    assert_eq!(h.velocity, p(-1, -1));
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    let h = actors(&s).into_iter().find(|a| a.kind == HAZARD).unwrap();
    assert_eq!(h.position, p(254, 254));
    assert_eq!(h.velocity, p(1, 1));
}
#[test]
fn contacts_use_inclusive_aabb_and_stable_ordinal_events() {
    let mut s = sim(vec![p(4, 4), p(-4, -4), p(4, -4)], vec![], 100);
    let events = step(&mut s, FP::ZERO, FP::ZERO, 0);
    assert_eq!(
        events
            .iter()
            .take(3)
            .map(|e| e.payload.ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(run(&s).score, 3);
}
#[test]
fn empty_or_disconnected_input_is_neutral_and_other_slots_ignored() {
    let mut s = sim(vec![p(100, 100)], vec![], 100);
    s.step(&TickInputs::new(1, 0));
    let mut inputs = TickInputs::new(2, 2);
    inputs.set_input(
        PlayerSlot(0),
        CollectInput {
            x: FP::ONE,
            buttons: RESTART,
            ..Default::default()
        },
    );
    inputs.set_flags(
        PlayerSlot(0),
        PlayerFlags {
            disconnected: true,
            predicted: false,
        },
    );
    inputs.set_input(
        PlayerSlot(1),
        CollectInput {
            x: FP::ONE,
            buttons: RESTART,
            ..Default::default()
        },
    );
    s.step(&inputs);
    assert_eq!(actors(&s)[0].position, p(0, 0));
    assert_eq!(run(&s).elapsed_ticks, 2);
}
#[test]
fn rejects_invalid_levels_including_extreme_raw_values() {
    assert!(CollectLevel::new(p(0, 0), vec![], vec![], 10).is_err());
    assert!(CollectLevel::new(p(0, 0), vec![p(1, 1); 33], vec![], 10).is_err());
    for raw in [i64::MIN, i64::MAX, 255 << 16] {
        assert!(
            CollectLevel::new(FPVec2::new(FP(raw), FP::ZERO), vec![p(1, 1)], vec![], 10).is_err()
        );
        assert!(
            CollectLevel::new(p(0, 0), vec![FPVec2::new(FP(raw), FP::ZERO)], vec![], 10).is_err()
        );
    }
    for limit in [0, 36_001, u32::MAX] {
        assert!(CollectLevel::new(p(0, 0), vec![p(1, 1)], vec![], limit).is_err());
    }
    assert!(CollectLevel::new(
        p(0, 0),
        vec![p(1, 1)],
        vec![HazardSpec {
            position: p(1, 1),
            velocity: p(2, 0)
        }],
        10
    )
    .is_err());
    assert!(CollectLevel::new(
        p(0, 0),
        vec![p(1, 1)],
        vec![
            HazardSpec {
                position: p(1, 1),
                velocity: p(0, 0)
            };
            17
        ],
        10
    )
    .is_err());
}
#[test]
fn snapshot_restore_and_cold_serialized_frame_replay_match_each_tick_and_event() {
    let cfg = level(
        vec![p(12, 0), p(32, 0), p(60, 0)],
        vec![HazardSpec {
            position: p(30, 20),
            velocity: p(0, -1),
        }],
        300,
    );
    let mut original = Simulation::<CollectDodgeV1>::new(cfg, 60, 42);
    for _ in 0..8 {
        step(&mut original, FP::ONE, FP::ZERO, 0);
    }
    let snapshot = original.frame().clone();
    let bytes = snapshot.to_bytes();
    let restored =
        Frame::from_bytes(Simulation::<CollectDodgeV1>::build_registry(), &bytes).unwrap();
    let mut cold = Simulation::<CollectDodgeV1>::from_frame(&restored, 60, 0).unwrap();
    let mut expected = Vec::new();
    for n in 0..100 {
        let buttons = if (20..=23).contains(&n) || n == 60 {
            RESTART
        } else {
            0
        };
        let a = step(&mut original, FP::ONE, FP::ZERO, buttons);
        let b = step(&mut cold, FP::ONE, FP::ZERO, buttons);
        assert_eq!(event_values(&a), event_values(&b));
        assert_eq!(original.checksum(), cold.checksum());
        expected.push((buttons, a, original.checksum()));
    }
    original.restore(&snapshot);
    for (buttons, events, checksum) in expected {
        assert_eq!(
            event_values(&step(&mut original, FP::ONE, FP::ZERO, buttons)),
            event_values(&events)
        );
        assert_eq!(original.checksum(), checksum);
    }
}
#[test]
fn restart_edge_survives_cold_restore() {
    let mut s = sim(vec![p(100, 0)], vec![], 100);
    step(&mut s, FP::ZERO, FP::ZERO, RESTART);
    let mut cold = Simulation::<CollectDodgeV1>::from_frame(s.frame(), 60, 0).unwrap();
    assert!(step(&mut cold, FP::ZERO, FP::ZERO, RESTART).is_empty());
    assert_eq!(run(&cold).elapsed_ticks, 1);
}
#[test]
fn no_command_rejects_nonzero_or_wrong_length() {
    use orr_sim::SimCommand;
    assert!(NoCommand::decode(&[0, 0, 0, 0]).is_some());
    assert!(NoCommand::decode(&[1, 0, 0, 0]).is_none());
    assert!(NoCommand::decode(&[]).is_none());
}

fn event_values(
    events: &[orr_sim::SimEvent<CollectEvent>],
) -> Vec<(orr_sim::EventKey, CollectEvent)> {
    events.iter().map(|e| (e.key, e.payload)).collect()
}

// First version's new golden, recorded from the independently executable
// example. No existing Arena/PhysGame/Yard3D golden is modified.
#[test]
fn golden_win_restart_hazard_loss() {
    let mut s = sim(
        vec![p(10, 0), p(20, 0)],
        vec![HazardSpec {
            position: p(0, 12),
            velocity: p(0, 0),
        }],
        600,
    );
    for tick in 1..=25 {
        if tick <= 16 {
            step(&mut s, FP::ONE, FP::ZERO, 0);
        } else if tick == 17 {
            step(&mut s, FP::ZERO, FP::ZERO, RESTART);
        } else {
            step(&mut s, FP::ZERO, FP::ONE, 0);
        }
    }
    assert_eq!(run(&s).phase, LOST_HAZARD);
    assert_eq!(s.checksum(), 0xa7b6_1c44_ba9f_26bd);
}

#[test]
fn maximum_level_and_deadline_remain_bounded() {
    let mut s = sim(
        vec![p(200, 200); MAX_COLLECTIBLES],
        vec![
            HazardSpec {
                position: p(100, 100),
                velocity: p(0, 0)
            };
            MAX_HAZARDS
        ],
        36_000,
    );
    for _ in 0..35_999 {
        assert!(step(&mut s, FP::ZERO, FP::ZERO, 0).is_empty());
    }
    assert_eq!(run(&s).phase, PLAYING);
    let events = step(&mut s, FP::ZERO, FP::ZERO, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(run(&s).elapsed_ticks, 36_000);
    assert_eq!(run(&s).phase, LOST_TIMEOUT);
    assert_eq!(actors(&s).len(), 1 + MAX_COLLECTIBLES + MAX_HAZARDS);
}

#[test]
fn fractional_negative_boundary_reflection_preserves_overshoot() {
    let velocity = FPVec2::new(FP(-32_768), FP::ZERO);
    let start = FPVec2::new(FP((-254 << 16) + 16_384), FP::ZERO);
    let mut s = sim(
        vec![p(200, 200)],
        vec![HazardSpec {
            position: start,
            velocity,
        }],
        100,
    );
    step(&mut s, FP::ZERO, FP::ZERO, 0);
    let h = actors(&s).into_iter().find(|a| a.kind == HAZARD).unwrap();
    assert_eq!(h.position, start);
    assert_eq!(h.velocity.x, FP(32_768));
}
