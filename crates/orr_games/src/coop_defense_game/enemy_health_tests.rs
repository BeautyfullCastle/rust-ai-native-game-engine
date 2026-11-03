use super::*;
use orr_reflect::{Kind, Reflect};
use orr_sim::{Simulation, TickInputs};

#[test]
fn enemy_health_layout_defaults_bounds_and_checked_multiplication() {
    assert_eq!(std::mem::size_of::<EnemyHealth>(), 4);
    assert_eq!(EnemyHealth::default().multiplier, 1);
    assert_eq!(EnemyHealth::default_value(), EnemyHealth::default());
    let desc = EnemyHealth::describe();
    desc.check_layout(4).unwrap();
    let Kind::Struct { fields, .. } = desc.kind else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "multiplier");
    let Kind::Int { int, range } = &fields[0].ty.kind else {
        panic!()
    };
    assert_eq!(*int, orr_reflect::IntKind::U32);
    assert_eq!((range.min, range.max), (Some(1), Some(4)));
    for multiplier in [0, 5, u32::MAX] {
        let health = EnemyHealth { multiplier };
        assert_eq!(scaled_enemy_hp(39, health), None);
        assert!(std::panic::catch_unwind(|| Simulation::<DefenseGame>::new(
            DefenseConfig {
                enemy_health: health,
                ..Default::default()
            },
            TICK_RATE,
            1
        ))
        .is_err());
    }
    assert_eq!(
        scaled_enemy_hp(u32::MAX, EnemyHealth { multiplier: 4 }),
        None
    );
    assert_eq!(
        scaled_enemy_hp(2436, EnemyHealth { multiplier: 4 }),
        Some(9744)
    );
    let registry = Simulation::<DefenseGame>::build_registry();
    let id = registry.singleton_id::<EnemyHealth>().unwrap();
    assert_eq!(registry.singleton_name(id), ENEMY_HEALTH_NAME);
    assert_eq!(registry.singleton_size(id), 4);
}

#[test]
fn all_wave_families_lanes_and_multipliers_preserve_spawn_order_and_cadence() {
    // Independent literal base-health oracle: normal, fast, armored, horde, boss.
    const HP: [[u32; 5]; 10] = [
        [39, 39, 78, 26, 546],
        [54, 54, 108, 36, 756],
        [69, 69, 138, 46, 966],
        [84, 84, 168, 56, 1176],
        [99, 99, 198, 66, 1386],
        [114, 114, 228, 76, 1596],
        [129, 129, 258, 86, 1806],
        [144, 144, 288, 96, 2016],
        [159, 159, 318, 106, 2226],
        [174, 174, 348, 116, 2436],
    ];
    const ORDER: [[u32; 4]; 4] = [
        [FAST, ARMORED, HORDE, NORMAL],
        [ARMORED, HORDE, NORMAL, FAST],
        [HORDE, NORMAL, FAST, ARMORED],
        [NORMAL, FAST, ARMORED, HORDE],
    ];
    for multiplier in 1..=4 {
        for wave in 1..=10 {
            let mut r = initial_run(1, Scenario::default());
            r.wave = wave;
            let mut pool = [DefenseEnemy::default(); MAX_ENEMIES];
            let mut events = Vec::new();
            for tick in 0..=560 {
                r.wave_tick = tick;
                let before = r.spawned;
                spawn_wave(
                    &mut r,
                    &mut pool,
                    &mut events,
                    WavePacing {
                        spawn_interval_ticks: 70,
                    },
                    WaveCounts {
                        pair_counts: [6; 10],
                    },
                    EnemyHealth { multiplier },
                    WaveComposition::default(),
                );
                assert_eq!(
                    r.spawned - before,
                    if tick >= 210 && tick % 70 == 0 { 2 } else { 0 }
                );
            }
            assert_eq!(r.spawned, 12);
            assert_eq!(events.len(), 12);
            for (ordinal, pair) in events.as_chunks::<2>().0.iter().enumerate() {
                let kind = if (wave == 5 || wave == 10) && ordinal == 0 {
                    BOSS
                } else {
                    ORDER[((wave - 1) % 4) as usize][ordinal % 4]
                };
                for (lane, event) in pair.iter().enumerate() {
                    let enemy = &pool[ordinal * 2 + lane];
                    let hp = HP[(wave - 1) as usize][kind as usize] * multiplier;
                    assert_eq!(
                        (enemy.kind, enemy.lane, enemy.hp, enemy.max_hp),
                        (kind, lane as u32, hp, hp)
                    );
                    assert_eq!(
                        (event.kind, event.value, event.hp_after),
                        (EVENT_ENEMY_SPAWN, kind, hp)
                    );
                    assert!(hp <= 9744);
                }
            }
        }
    }
}

#[test]
fn scaled_health_changes_actual_hits_and_deaths_and_restart_retains_it() {
    for multiplier in [1, 4] {
        let mut r = initial_run(1, Scenario::default());
        r.wave = 1;
        r.wave_tick = 210;
        let mut e = [DefenseEnemy::default(); 2];
        spawn_wave(
            &mut r,
            &mut e,
            &mut Vec::new(),
            WavePacing::default(),
            WaveCounts::default(),
            EnemyHealth { multiplier },
            WaveComposition::default(),
        );
        let (x, y) = enemy_position(0, 0);
        let mut shots = [DefenseProjectile {
            active: 1,
            target_id: e[0].id,
            role: PRISM,
            damage: 39,
            speed: 18,
            x,
            y,
            ..Default::default()
        }];
        let mut events = Vec::new();
        advance_projectiles(&mut r, &mut e, &mut shots, &mut events);
        assert_eq!(e[0].hp, if multiplier == 1 { 0 } else { 117 });
        assert_eq!(r.kills, if multiplier == 1 { 1 } else { 0 });
        assert_eq!(
            events.iter().filter(|e| e.kind == EVENT_KILL).count(),
            if multiplier == 1 { 1 } else { 0 }
        );
        assert_eq!(events.iter().filter(|e| e.kind == EVENT_HIT).count(), 1);
        let config = DefenseConfig {
            enemy_health: EnemyHealth { multiplier },
            ..Default::default()
        };
        let mut sim = Simulation::<DefenseGame>::new(config, TICK_RATE, 1);
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(
            PlayerSlot(0),
            DefenseInput {
                action: RESTART,
                sequence: 1,
                ..Default::default()
            },
        );
        sim.step(&inputs);
        assert_eq!(*sim.frame().singleton::<EnemyHealth>(), config.enemy_health);
        assert_eq!(run(sim.frame()).generation, 1);
    }
}

#[test]
fn old_v4_frame_is_rejected_before_simulation() {
    let mut old = ComponentRegistryBuilder::new();
    old.register_singleton::<FrameRng>("orr_sim::FrameRng");
    old.register_singleton::<DefenseRun>("CoopDefense::Run");
    old.register_component::<DefenseUnit>("CoopDefense::Unit");
    old.register_component::<DefenseEnemy>("CoopDefense::Enemy");
    old.register_component::<DefenseProjectile>("CoopDefense::Projectile");
    old.register_singleton::<Scenario>(SCENARIO_NAME);
    old.register_singleton::<Campaign>(CAMPAIGN_NAME);
    old.register_singleton::<WavePacing>(WAVE_PACING_NAME);
    old.register_singleton::<WaveCounts>(WAVE_COUNTS_NAME);
    let old = Frame::new(old.build());
    assert!(matches!(
        Frame::from_bytes(Simulation::<DefenseGame>::build_registry(), &old.to_bytes()),
        Err(orr_ecs::FrameDecodeError::SchemaCount {
            kind: "singleton",
            expected: 8,
            found: 6
        })
    ));
    assert!(Simulation::<DefenseGame>::from_frame(&old, TICK_RATE, SCENARIO_BUILD_ID).is_err());
    assert_ne!(
        SCENARIO_BUILD_ID,
        orr_sim::frame_build_id(0x5354_4f52_4d00_0004)
    );
}

#[cfg(feature = "coop-defense-scene")]
#[test]
fn source_health_is_independent_strict_and_defaults_to_one() {
    use crate::coop_defense_scene::parse_config;
    const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
    assert_eq!(
        parse_config(EMPTY).unwrap().enemy_health,
        EnemyHealth::default()
    );
    for singleton in [
        "CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}",
        "CoopDefense::Campaign: {last_wave: 3}",
        "CoopDefense::WavePacing: {spawn_interval_ticks: 70}",
        "CoopDefense::WaveCounts: {pair_counts: [1,1,1,1,1,1,1,1,1,1]}",
    ] {
        assert_eq!(
            parse_config(&format!("{EMPTY}singletons: {{{singleton}}}\n"))
                .unwrap()
                .enemy_health,
            EnemyHealth::default()
        );
    }
    for multiplier in 1..=4 {
        let config = parse_config(&format!(
            "{EMPTY}singletons: {{CoopDefense::EnemyHealth: {{multiplier: {multiplier}}}}}\n"
        ))
        .unwrap();
        assert_eq!(
            config,
            DefenseConfig {
                enemy_health: EnemyHealth { multiplier },
                ..Default::default()
            }
        );
    }
    for fields in [
        "{}",
        "null",
        "{multiplier: null}",
        "{multiplier: 0}",
        "{multiplier: 5}",
        "{multiplier: -1}",
        "{multiplier: 1.5}",
        "{multiplier: 4294967296}",
        "{multiplier: bad}",
        "{multiplier: 2, extra: 1}",
        "{multiplier: 1, multiplier: 2}",
    ] {
        assert!(
            parse_config(&format!(
                "{EMPTY}singletons: {{CoopDefense::EnemyHealth: {fields}}}\n"
            ))
            .is_err(),
            "{fields}"
        );
    }
    assert!(parse_config(&format!("{EMPTY}singletons: {{CoopDefense::EnemyHealth: {{multiplier: 1}}, CoopDefense::EnemyHealth: {{multiplier: 2}}}}\n")).is_err());
}
