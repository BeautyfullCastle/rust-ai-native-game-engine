use super::*;
use orr_sim::{Simulation, TickInputs};

#[test]
fn reflected_input_preserves_pod_layout_and_neutral_default() {
    use orr_reflect::{Kind, Reflect};

    let desc = DefenseInput::describe();
    assert_eq!(std::mem::size_of::<DefenseInput>(), 16);
    desc.check_layout(std::mem::size_of::<DefenseInput>()).unwrap();
    let Kind::Struct { size, fields } = &desc.kind else {
        panic!("DefenseInput must expose its existing struct layout");
    };
    assert_eq!(*size, 16);
    assert_eq!(
        fields.iter().map(|field| (field.name.as_str(), field.offset)).collect::<Vec<_>>(),
        [("action", 0), ("source_cell", 4), ("target_cell", 8), ("sequence", 12)]
    );
    let neutral = DefenseInput::default_value();
    assert_eq!(neutral, DefenseInput::default());
    assert_eq!(bytemuck::bytes_of(&neutral), &[0; 16]);
    desc.check(&desc.read(bytemuck::bytes_of(&neutral)), true).unwrap();
}

#[test]
fn reflected_input_round_trip_covers_all_actions_cells_and_full_sequence() {
    use orr_reflect::Reflect;

    let desc = DefenseInput::describe();
    for action in NOOP..=RESTART {
        for source_cell in 0..CELLS_PER_BOARD {
            for target_cell in 0..CELLS_PER_BOARD {
                let input = DefenseInput { action, source_cell, target_cell, sequence: u32::MAX };
                let value = desc.read(bytemuck::bytes_of(&input));
                let mut decoded = DefenseInput::default();
                desc.write(bytemuck::bytes_of_mut(&mut decoded), &value).unwrap();
                assert_eq!(decoded, input);
            }
        }
    }
}

#[test]
fn reflected_input_rejects_invalid_action_and_cells_without_mutating_bytes() {
    use orr_reflect::Reflect;

    let desc = DefenseInput::describe();
    for invalid in [
        DefenseInput { action: RESTART + 1, ..Default::default() },
        DefenseInput { source_cell: CELLS_PER_BOARD, ..Default::default() },
        DefenseInput { target_cell: CELLS_PER_BOARD, ..Default::default() },
    ] {
        let value = desc.read(bytemuck::bytes_of(&invalid));
        let mut decoded = DefenseInput { sequence: 123, ..Default::default() };
        let before = decoded;
        assert!(desc.write(bytemuck::bytes_of_mut(&mut decoded), &value).is_err());
        assert_eq!(decoded, before);
    }
}

fn new(seed: u32) -> Simulation<DefenseGame> {
    Simulation::new(DefenseConfig { seed }, TICK_RATE, seed as u64)
}
fn step(s: &mut Simulation<DefenseGame>, input: DefenseInput) {
    let mut inputs = TickInputs::new(s.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    s.step(&inputs);
}
fn input(action: u32, sequence: u32) -> DefenseInput {
    DefenseInput {
        action,
        sequence,
        ..Default::default()
    }
}
#[test]
fn duplicate_malformed_and_insufficient_actions_never_double_spend() {
    let mut s = new(12);
    let command = input(SUMMON, 1);
    step(&mut s, command);
    assert_eq!(run(s.frame()).gold[0], 125);
    for _ in 0..20 {
        step(&mut s, command);
    }
    assert_eq!(run(s.frame()).summons, 1);
    step(
        &mut s,
        DefenseInput {
            target_cell: u32::MAX,
            ..input(SUMMON, 2)
        },
    );
    step(&mut s, input(99, 3));
    assert_eq!(run(s.frame()).gold[0], 125);
    for seq in 4..10 {
        step(&mut s, input(SUMMON, seq));
    }
    assert_eq!(run(s.frame()).gold[0], 0);
    assert_eq!(run(s.frame()).summons, 6);
}
#[test]
fn merge_requires_two_matching_units_and_movement_preserves_identity() {
    let mut r = initial_run(1);
    let mut rng = FrameRng::new(1);
    let mut u = units(new(1).frame());
    let mut ev = Vec::new();
    u[0] = DefenseUnit {
        active: 1,
        role: RIVET,
        tier: 1,
        ..u[0]
    };
    u[1] = DefenseUnit {
        active: 1,
        role: RIVET,
        tier: 1,
        ..u[1]
    };
    action(
        &mut r,
        &mut u,
        0,
        DefenseInput {
            source_cell: 0,
            target_cell: 1,
            ..input(MOVE_OR_MERGE, 1)
        },
        &mut ev,
        &mut rng,
    );
    assert_eq!((u[0].active, u[1].tier, r.merges), (0, 2, 1));
    action(
        &mut r,
        &mut u,
        0,
        DefenseInput {
            source_cell: 1,
            target_cell: 2,
            ..input(MOVE_OR_MERGE, 2)
        },
        &mut ev,
        &mut rng,
    );
    assert_eq!((u[1].active, u[2].tier, u[2].cell), (0, 2, 2));
}
#[test]
fn projectile_damage_only_on_arrival_and_lanes_share_reactor() {
    let mut r = initial_run(1);
    let mut ev = Vec::new();
    let mut e = vec![DefenseEnemy {
        active: 1,
        id: 1,
        lane: 0,
        hp: 100,
        max_hp: 100,
        ..Default::default()
    }];
    let mut p = vec![DefenseProjectile {
        active: 1,
        target_id: 1,
        x: 400,
        y: 400,
        damage: 20,
        speed: 10,
        ..Default::default()
    }];
    advance_projectiles(&mut r, &mut e, &mut p, &mut ev);
    assert_eq!(e[0].hp, 100);
    for _ in 0..70 {
        advance_projectiles(&mut r, &mut e, &mut p, &mut ev);
    }
    assert_eq!(e[0].hp, 80);
    for lane in 0..2 {
        e[0].active = 1;
        e[0].progress = 1999;
        e[0].lane = lane;
        move_enemies(&mut r, &mut e, &mut ev);
    }
    assert_eq!(r.reactor_hp, 26);
    assert_eq!(r.leaks, 2);
}
#[test]
fn deterministic_inputs_and_snapshot_replay_match_exactly() {
    let mut a = new(2026);
    let mut b = new(2026);
    for tick in 0..1500 {
        let i = if tick % 20 == 0 {
            strategy_input(&run(a.frame()), &units(a.frame()), 0, tick + 1)
        } else {
            DefenseInput::default()
        };
        step(&mut a, i);
        step(&mut b, i);
        assert_eq!(a.checksum(), b.checksum());
    }
    let snapshot = a.frame().clone();
    let mut inputs = Vec::new();
    for tick in 1500..2500 {
        let i = if tick % 20 == 0 {
            strategy_input(&run(a.frame()), &units(a.frame()), 0, tick + 1)
        } else {
            DefenseInput::default()
        };
        inputs.push(i);
        step(&mut a, i);
    }
    let checksum = a.checksum();
    a.restore(&snapshot);
    for i in inputs {
        step(&mut a, i);
    }
    assert_eq!(checksum, a.checksum());
}
#[test]
fn strategy_wins_but_afk_loses_with_bounded_pools() {
    for seed in [1, 42, 2026] {
        let mut s = new(seed);
        for tick in 0..30000 {
            let r = run(s.frame());
            if r.phase != PLAYING {
                break;
            }
            let i = if tick % 15 == 0 {
                strategy_input(&r, &units(s.frame()), 0, tick + 1)
            } else {
                DefenseInput::default()
            };
            step(&mut s, i);
        }
        let r = run(s.frame());
        assert_eq!(r.phase, WON, "seed={seed} state={r:?}");
        assert_eq!(units(s.frame()).len(), 24);
        assert_eq!(enemies(s.frame()).len(), MAX_ENEMIES);
        assert_eq!(projectiles(s.frame()).len(), MAX_PROJECTILES);
    }
    let mut s = new(42);
    for _ in 0..10000 {
        if run(s.frame()).phase != PLAYING {
            break;
        }
        step(&mut s, DefenseInput::default());
    }
    assert_eq!(run(s.frame()).phase, LOST);
    assert!(units(s.frame())
        .iter()
        .any(|u| u.board == 1 && u.active != 0));
}
#[test]
fn restart_resets_gameplay_and_rng_without_replaying_held_action() {
    let mut a = new(7);
    step(&mut a, input(SUMMON, 1));
    let role = units(a.frame())[0].role;
    step(&mut a, input(RESTART, 2));
    assert_eq!(run(a.frame()).tick, 0);
    step(&mut a, input(RESTART, 2));
    assert_eq!(run(a.frame()).tick, 1);
    step(&mut a, input(SUMMON, 3));
    assert_eq!(units(a.frame())[0].role, role);
    assert_eq!(run(a.frame()).gold[0], 125);
}

#[test]
fn bounded_economy_caps_and_full_board_do_not_spend() {
    let mut r = initial_run(77);
    let mut rng = FrameRng::new(77);
    let mut u = units(new(77).frame());
    let mut events = Vec::new();
    r.gold[0] = 99999;
    for _ in 0..CELLS_PER_BOARD {
        action(&mut r, &mut u, 0, input(SUMMON, 1), &mut events, &mut rng);
    }
    let gold = r.gold[0];
    let draws = r.rng_draws;
    action(&mut r, &mut u, 0, input(SUMMON, 2), &mut events, &mut rng);
    assert_eq!((r.gold[0], r.rng_draws), (gold, draws));
    assert!(u.iter().filter(|x| x.board == 1).all(|x| x.active == 0));
    for _ in 0..MAX_UPGRADE {
        let cost = upgrade_cost(r.upgrades[0]);
        let gold = r.gold[0];
        action(&mut r, &mut u, 0, input(UPGRADE, 1), &mut events, &mut rng);
        assert_eq!(r.gold[0], gold - cost);
    }
    let gold = r.gold[0];
    action(&mut r, &mut u, 0, input(UPGRADE, 1), &mut events, &mut rng);
    assert_eq!((r.upgrades[0], r.gold[0]), (MAX_UPGRADE, gold));
    for _ in 0..MAX_INVESTMENT {
        let cost = investment_cost(r.investment[0]);
        let gold = r.gold[0];
        action(&mut r, &mut u, 0, input(INVEST, 1), &mut events, &mut rng);
        assert_eq!(r.gold[0], gold - cost);
    }
    let gold = r.gold[0];
    action(&mut r, &mut u, 0, input(INVEST, 1), &mut events, &mut rng);
    assert_eq!((r.investment[0], r.gold[0]), (MAX_INVESTMENT, gold));
}

#[test]
fn boss_waves_and_role_mechanics_are_real_simulation() {
    for wave in [5, 10] {
        let mut r = initial_run(1);
        r.wave = wave;
        r.wave_tick = 210;
        let mut e = vec![DefenseEnemy::default(); MAX_ENEMIES];
        spawn_wave(&mut r, &mut e, &mut Vec::new());
        assert_eq!(
            e.iter().filter(|e| e.active != 0 && e.kind == BOSS).count(),
            2
        );
        assert!(e[0].hp > (24 + wave * 15) * 10);
    }
    // Armor reduces rivets; prism ignores it; condenser applies timed slow.
    for (role, expected_hp, expected_slow) in [(RIVET, 87, 0), (PRISM, 80, 0), (CONDENSER, 87, 100)]
    {
        let mut r = initial_run(1);
        let mut e = vec![DefenseEnemy {
            active: 1,
            id: 1,
            kind: ARMORED,
            hp: 100,
            max_hp: 100,
            ..Default::default()
        }];
        let (x, y) = enemy_position(0, 0);
        let mut p = vec![DefenseProjectile {
            active: 1,
            target_id: 1,
            role,
            x,
            y,
            damage: 20,
            speed: 18,
            ..Default::default()
        }];
        advance_projectiles(&mut r, &mut e, &mut p, &mut Vec::new());
        assert_eq!((e[0].hp, e[0].slow_ticks), (expected_hp, expected_slow));
    }
}

#[test]
fn allied_mortar_splash_follows_target_lane_not_shooter_board() {
    let mut r = initial_run(1);
    let mut e = vec![
        DefenseEnemy {
            active: 1,
            id: 1,
            lane: 0,
            progress: 1800,
            hp: 100,
            max_hp: 100,
            ..Default::default()
        },
        DefenseEnemy {
            active: 1,
            id: 2,
            lane: 0,
            progress: 1820,
            hp: 100,
            max_hp: 100,
            ..Default::default()
        },
        DefenseEnemy {
            active: 1,
            id: 3,
            lane: 0,
            progress: 1400,
            hp: 100,
            max_hp: 100,
            ..Default::default()
        },
    ];
    let (x, y) = enemy_position(0, 1800);
    let mut p = vec![DefenseProjectile {
        active: 1,
        board: 1,
        role: MORTAR,
        target_id: 1,
        x,
        y,
        damage: 20,
        speed: 10,
        ..Default::default()
    }];
    advance_projectiles(&mut r, &mut e, &mut p, &mut Vec::new());
    assert_eq!((e[0].hp, e[1].hp, e[2].hp), (80, 80, 100));
}

fn step_events(
    s: &mut Simulation<DefenseGame>,
    input: DefenseInput,
) -> Vec<orr_sim::SimEvent<DefenseEvent>> {
    let mut inputs = TickInputs::new(s.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    s.step(&inputs)
}

#[test]
fn logical_unit_identity_survives_move_and_merge_consumes_only_source() {
    let mut r = initial_run(1);
    let mut rng = FrameRng::new(1);
    let mut u = units(new(1).frame());
    let mut events = Vec::new();
    action(&mut r, &mut u, 0, input(SUMMON, 1), &mut events, &mut rng);
    let first = u[0].id;
    action(
        &mut r,
        &mut u,
        0,
        DefenseInput {
            source_cell: 0,
            target_cell: 2,
            ..input(MOVE_OR_MERGE, 2)
        },
        &mut events,
        &mut rng,
    );
    assert_eq!(u[2].id, first);
    assert_eq!(
        events.last().unwrap().event_kind(),
        Some(DefenseEventKind::Move)
    );
    assert_eq!(events.last().unwrap().unit_id, first);
    action(&mut r, &mut u, 0, input(SUMMON, 3), &mut events, &mut rng);
    let second = u[0].id;
    assert_ne!(first, second, "reused cell must not reuse actor lifetime");
    u[0].role = u[2].role;
    action(
        &mut r,
        &mut u,
        0,
        DefenseInput {
            source_cell: 0,
            target_cell: 2,
            ..input(MOVE_OR_MERGE, 4)
        },
        &mut events,
        &mut rng,
    );
    let merged = events.last().unwrap();
    assert_eq!(merged.event_kind(), Some(DefenseEventKind::Merge));
    assert_eq!((merged.unit_id, merged.other_unit_id), (first, second));
    assert_eq!((u[2].id, u[2].tier, u[0].active), (first, 2, 0));
}

#[test]
fn projectile_identity_and_source_survive_motion_and_pool_reuse() {
    let mut r = initial_run(1);
    let mut u = vec![DefenseUnit {
        active: 1,
        id: 9,
        role: RIVET,
        tier: 1,
        ..Default::default()
    }];
    let mut e = vec![DefenseEnemy {
        active: 1,
        id: 11,
        hp: 100,
        max_hp: 100,
        ..Default::default()
    }];
    let mut p = vec![DefenseProjectile::default()];
    let mut events = Vec::new();
    fire(&mut r, &mut u, &e, &mut p, &mut events);
    let first = p[0].id;
    let release = events[0];
    assert_eq!(release.event_kind(), Some(DefenseEventKind::Release));
    assert_eq!(
        (release.unit_id, release.enemy_id, release.projectile_id),
        (9, 11, first)
    );
    advance_projectiles(&mut r, &mut e, &mut p, &mut events);
    assert_eq!((p[0].id, p[0].source_unit_id, p[0].active), (first, 9, 1));
    // Consuming/moving the shooter does not change a projectile's origin identity.
    u[0].id = 20;
    u[0].cooldown = 0;
    e[0].active = 0;
    advance_projectiles(&mut r, &mut e, &mut p, &mut events);
    let retired = events.last().unwrap();
    assert_eq!(
        retired.event_kind(),
        Some(DefenseEventKind::ProjectileExpired)
    );
    assert_eq!(
        (retired.unit_id, retired.enemy_id, retired.projectile_id),
        (9, 11, first)
    );
    assert!(!events.iter().any(|e| matches!(
        e.event_kind(),
        Some(DefenseEventKind::Hit | DefenseEventKind::Death)
    )));
    e[0].active = 1;
    fire(&mut r, &mut u, &e, &mut p, &mut events);
    assert_ne!(p[0].id, first);
    assert_eq!(p[0].source_unit_id, 20);
}

#[test]
fn simultaneous_splash_hits_have_exact_source_projectile_and_victim_ids() {
    let mut r = initial_run(1);
    let mut e = vec![
        DefenseEnemy {
            active: 1,
            id: 41,
            hp: 5,
            max_hp: 5,
            ..Default::default()
        },
        DefenseEnemy {
            active: 1,
            id: 42,
            hp: 100,
            max_hp: 100,
            progress: 20,
            ..Default::default()
        },
    ];
    let (x, y) = enemy_position(0, 0);
    let mut p = vec![DefenseProjectile {
        active: 1,
        id: 71,
        source_unit_id: 8,
        target_id: 41,
        role: MORTAR,
        board: 1,
        x,
        y,
        damage: 10,
        speed: 10,
    }];
    let mut events = Vec::new();
    advance_projectiles(&mut r, &mut e, &mut p, &mut events);
    let hits: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind() == Some(DefenseEventKind::Hit))
        .collect();
    assert_eq!(hits.len(), 2);
    assert_eq!(
        (hits[0].enemy_id, hits[0].value, hits[0].hp_after),
        (41, 5, 0)
    );
    assert_eq!(
        (hits[1].enemy_id, hits[1].value, hits[1].hp_after),
        (42, 10, 90)
    );
    assert!(hits
        .iter()
        .all(|e| (e.unit_id, e.projectile_id, e.role) == (8, 71, MORTAR)));
    let deaths: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind() == Some(DefenseEventKind::Death))
        .collect();
    assert_eq!(deaths.len(), 1);
    assert_eq!(
        (
            deaths[0].unit_id,
            deaths[0].projectile_id,
            deaths[0].enemy_id
        ),
        (8, 71, 41)
    );
}

#[test]
fn leak_and_kill_same_tick_are_not_ambiguous_retirements() {
    let mut r = initial_run(1);
    let mut e = vec![
        DefenseEnemy {
            active: 1,
            id: 41,
            hp: 5,
            max_hp: 5,
            ..Default::default()
        },
        DefenseEnemy {
            active: 1,
            id: 42,
            hp: 100,
            max_hp: 100,
            progress: 1999,
            ..Default::default()
        },
    ];
    let mut events = Vec::new();
    move_enemies(&mut r, &mut e, &mut events);
    let (x, y) = enemy_position(0, e[0].progress);
    let mut p = vec![DefenseProjectile {
        active: 1,
        id: 71,
        source_unit_id: 8,
        target_id: 41,
        x,
        y,
        damage: 10,
        speed: 18,
        ..Default::default()
    }];
    advance_projectiles(&mut r, &mut e, &mut p, &mut events);
    assert_eq!((r.kills, r.leaks), (1, 1));
    assert!(events
        .iter()
        .any(|e| e.event_kind() == Some(DefenseEventKind::Leak) && e.enemy_id == 42));
    assert!(events
        .iter()
        .any(|e| e.event_kind() == Some(DefenseEventKind::Death) && e.enemy_id == 41));
    assert!(!events
        .iter()
        .any(|e| e.event_kind() == Some(DefenseEventKind::Death) && e.enemy_id == 42));
}

#[test]
fn event_keys_and_payloads_replay_exactly_without_duplicate_input_effects() {
    let mut s = new(2026);
    let first = step_events(&mut s, input(SUMMON, 1));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].key, orr_sim::EventKey::new(1, 0, 0));
    let duplicate = step_events(&mut s, input(SUMMON, 1));
    assert!(duplicate
        .iter()
        .all(|e| e.payload.event_kind() != Some(DefenseEventKind::Summon)));
    let snapshot = s.frame().clone();
    let mut expected = Vec::new();
    for tick in 0..1200 {
        let command = if tick % 20 == 0 {
            input(SUMMON, tick + 2)
        } else {
            DefenseInput::default()
        };
        let events = step_events(&mut s, command);
        assert!(events.len() <= MAX_EVENTS_PER_TICK);
        for (seq, event) in events.iter().enumerate() {
            assert_eq!(event.key, orr_sim::EventKey::new(s.tick(), 0, seq as u32));
            assert!(event.payload.event_kind().is_some());
        }
        expected.push((command, events));
    }
    let checksum = s.checksum();
    s.restore(&snapshot);
    for (command, expected) in expected {
        let actual = step_events(&mut s, command);
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            assert_eq!((a.key, a.payload), (b.key, b.payload));
        }
    }
    assert_eq!(s.checksum(), checksum);
}

#[test]
fn restart_reused_ids_are_namespaced_by_rollback_safe_generation() {
    let mut s = new(7);
    let first = step_events(&mut s, input(SUMMON, 1))[0];
    let before_restart = s.frame().clone();
    let restarted = step_events(&mut s, input(RESTART, 2))[0];
    assert_eq!(restarted.key.tick, 2);
    assert_eq!(restarted.payload.generation, 1);
    assert_eq!((run(s.frame()).tick, s.tick()), (0, 2));
    let new_actor = step_events(&mut s, input(SUMMON, 3))[0];
    assert_eq!(first.payload.unit_id, new_actor.payload.unit_id);
    assert_ne!(first.payload.generation, new_actor.payload.generation);
    let replay_checksum = s.checksum();
    s.restore(&before_restart);
    let replay_restart = step_events(&mut s, input(RESTART, 2))[0];
    assert_eq!(
        (restarted.key, restarted.payload),
        (replay_restart.key, replay_restart.payload)
    );
    step_events(&mut s, input(SUMMON, 3));
    assert_eq!(replay_checksum, s.checksum());
    assert_eq!(DefenseEventKind::from_code(u32::MAX), None);
}
