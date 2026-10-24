use super::*;
use orr_sim::{Simulation, TickInputs};
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
        spawn_wave(&mut r, &mut e);
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
    }];
    advance_projectiles(&mut r, &mut e, &mut p, &mut Vec::new());
    assert_eq!((e[0].hp, e[1].hp, e[2].hp), (80, 80, 100));
}
