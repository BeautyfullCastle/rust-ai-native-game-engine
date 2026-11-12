use super::*;
use orr_reflect::{Kind, Reflect};
use orr_sim::{Simulation, TickInputs};

#[test]
fn composition_literal_defaults_layout_and_nested_schema() {
    let expected = WaveComposition {
        regular_kinds: [
            [1, 2, 3, 0],
            [2, 3, 0, 1],
            [3, 0, 1, 2],
            [0, 1, 2, 3],
            [1, 2, 3, 0],
            [2, 3, 0, 1],
            [3, 0, 1, 2],
            [0, 1, 2, 3],
            [1, 2, 3, 0],
            [2, 3, 0, 1],
        ],
        opening_boss: [0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    };
    assert_eq!(WaveComposition::default(), expected);
    assert_eq!(WaveComposition::default_value(), expected);
    assert_eq!(std::mem::size_of::<WaveComposition>(), 200);
    let desc = WaveComposition::describe();
    desc.check_layout(200).unwrap();
    let Kind::Struct { fields, .. } = desc.kind else {
        panic!()
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "regular_kinds");
    let Kind::Array { elem: row, len: 10 } = &fields[0].ty.kind else {
        panic!()
    };
    let Kind::Array { elem: kind, len: 4 } = &row.kind else {
        panic!()
    };
    let Kind::Int { int, range } = &kind.kind else {
        panic!()
    };
    assert_eq!(*int, orr_reflect::IntKind::U32);
    assert_eq!((range.min, range.max), (Some(0), Some(3)));
    assert_eq!(fields[1].name, "opening_boss");
    let Kind::Array {
        elem: flag,
        len: 10,
    } = &fields[1].ty.kind
    else {
        panic!()
    };
    let Kind::Int { int, range } = &flag.kind else {
        panic!()
    };
    assert_eq!(*int, orr_reflect::IntKind::U32);
    assert_eq!((range.min, range.max), (Some(0), Some(1)));
    let registry = Simulation::<DefenseGame>::build_registry();
    let id = registry.singleton_id::<WaveComposition>().unwrap();
    assert_eq!(registry.singleton_name(id), WAVE_COMPOSITION_NAME);
    assert_eq!(registry.singleton_size(id), 200);
}

#[test]
fn composition_rejects_every_invalid_cell_before_ticking() {
    for wave in 0..10 {
        for slot in 0..4 {
            for invalid in [4, u32::MAX] {
                let mut composition = WaveComposition::default();
                composition.regular_kinds[wave][slot] = invalid;
                assert!(!composition.is_valid());
                assert!(std::panic::catch_unwind(|| Simulation::<DefenseGame>::new(
                    DefenseConfig {
                        campaign: Campaign { last_wave: 1 },
                        wave_composition: composition,
                        ..Default::default()
                    },
                    TICK_RATE,
                    1
                ))
                .is_err());
            }
        }
        for invalid in [2, u32::MAX] {
            let mut composition = WaveComposition::default();
            composition.opening_boss[wave] = invalid;
            assert!(!composition.is_valid());
            assert!(std::panic::catch_unwind(|| Simulation::<DefenseGame>::new(
                DefenseConfig {
                    wave_composition: composition,
                    ..Default::default()
                },
                TICK_RATE,
                1
            ))
            .is_err());
        }
    }
}

#[test]
fn authored_rows_wrap_by_pair_and_boss_replaces_only_first_pair() {
    // Distinct per-wave rows expose transposed indexing and lane-based indexing.
    let rows = [
        [3, 3, 1, 0],
        [0, 2, 0, 1],
        [2, 1, 2, 3],
        [1, 0, 3, 1],
        [0, 0, 0, 0],
        [1, 1, 1, 1],
        [2, 2, 2, 2],
        [3, 3, 3, 3],
        [0, 3, 2, 1],
        [2, 0, 1, 3],
    ];
    for flags in [[0; 10], [1; 10]] {
        let composition = WaveComposition {
            regular_kinds: rows,
            opening_boss: flags,
        };
        for wave in 1..=10 {
            for interval in [35, 70, 350] {
                let mut state = initial_run(1, Scenario::default());
                state.wave = wave;
                let mut pool = [DefenseEnemy::default(); MAX_ENEMIES];
                let mut events = Vec::new();
                for tick in 0..=3500 {
                    state.wave_tick = tick;
                    let before = state.spawned;
                    spawn_wave(
                        &mut state,
                        &mut pool,
                        &mut events,
                        WavePacing {
                            spawn_interval_ticks: interval,
                        },
                        WaveCounts {
                            pair_counts: [9; 10],
                        },
                        EnemyHealth { multiplier: 4 },
                        composition,
                    );
                    assert_eq!(
                        state.spawned - before,
                        if tick >= 180 && tick % interval == 0 && before < 18 {
                            2
                        } else {
                            0
                        }
                    );
                }
                assert_eq!(events.len(), 18);
                for (ordinal, pair) in events.chunks_exact(2).enumerate() {
                    let expected = if ordinal == 0 && flags[(wave - 1) as usize] == 1 {
                        BOSS
                    } else {
                        rows[(wave - 1) as usize][ordinal % 4]
                    };
                    let base = 24 + wave * 15;
                    let hp = match expected {
                        BOSS => base * 14,
                        ARMORED => base * 2,
                        HORDE => base * 2 / 3,
                        _ => base,
                    } * 4;
                    for (lane, event) in pair.iter().enumerate() {
                        assert_eq!(
                            (event.board, event.value, event.hp_after),
                            (lane as u32, expected, hp)
                        );
                        assert_eq!(event.enemy_id, (ordinal * 2 + lane + 1) as u32);
                        assert_eq!(pool[ordinal * 2 + lane].kind, expected);
                        assert_eq!(pool[ordinal * 2 + lane].max_hp, hp);
                    }
                }
            }
        }
    }
}

#[test]
fn composition_frame_restore_replay_and_restart_are_exact() {
    let composition = WaveComposition {
        regular_kinds: [[3, 0, 2, 1]; 10],
        opening_boss: [1; 10],
    };
    let config = DefenseConfig {
        wave_composition: composition,
        ..Default::default()
    };
    let mut sim = Simulation::<DefenseGame>::with_build_id(
        config,
        TICK_RATE,
        config.seed.into(),
        SCENARIO_BUILD_ID,
    );
    let initial = sim.frame().clone();
    let mut recorded = Vec::new();
    for tick in 0..500 {
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(
            PlayerSlot(0),
            if tick == 400 {
                DefenseInput {
                    action: RESTART,
                    sequence: 1,
                    ..Default::default()
                }
            } else {
                DefenseInput::default()
            },
        );
        let events: Vec<_> = sim
            .step(&inputs)
            .into_iter()
            .map(|event| (event.key, event.payload))
            .collect();
        recorded.push((inputs, events, sim.frame().checksum()));
        assert_eq!(*sim.frame().singleton::<WaveComposition>(), composition);
    }
    assert_eq!(run(sim.frame()).generation, 1);
    let restored = Frame::from_bytes(
        Simulation::<DefenseGame>::build_registry(),
        &initial.to_bytes(),
    )
    .unwrap();
    let mut replay =
        Simulation::<DefenseGame>::from_frame(&restored, TICK_RATE, SCENARIO_BUILD_ID).unwrap();
    for (inputs, events, checksum) in recorded {
        assert_eq!(
            replay
                .step(&inputs)
                .into_iter()
                .map(|event| (event.key, event.payload))
                .collect::<Vec<_>>(),
            events
        );
        assert_eq!(replay.frame().checksum(), checksum);
    }
}

#[test]
fn previous_v5_frame_is_rejected() {
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
    old.register_singleton::<EnemyHealth>(ENEMY_HEALTH_NAME);
    let old = Frame::new(old.build());
    assert!(matches!(
        Frame::from_bytes(Simulation::<DefenseGame>::build_registry(), &old.to_bytes()),
        Err(orr_ecs::FrameDecodeError::SchemaCount {
            kind: "singleton",
            expected: 8,
            found: 7
        })
    ));
    assert!(Simulation::<DefenseGame>::from_frame(&old, TICK_RATE, SCENARIO_BUILD_ID).is_err());
    assert_ne!(
        SCENARIO_BUILD_ID,
        orr_sim::frame_build_id(0x5354_4f52_4d00_0005)
    );
}

#[cfg(feature = "coop-defense-scene")]
#[test]
fn composition_source_is_nested_strict_and_independently_defaulted() {
    use crate::coop_defense_scene::parse_config;
    const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
    let rows = "[[3,0,2,1],[0,0,0,0],[1,1,1,1],[2,2,2,2],[3,3,3,3],[0,1,2,3],[3,2,1,0],[1,0,3,2],[2,3,0,1],[1,2,0,3]]";
    let flags = "[1,0,1,0,0,1,0,1,0,0]";
    let fields = format!("{{regular_kinds: {rows}, opening_boss: {flags}}}");
    let source =
        |fields: &str| format!("{EMPTY}singletons: {{CoopDefense::WaveComposition: {fields}}}\n");
    let config = parse_config(&source(&fields)).unwrap();
    assert_eq!(config.wave_composition.regular_kinds[9], [1, 2, 0, 3]);
    assert_eq!(
        config.wave_composition.opening_boss,
        [1, 0, 1, 0, 0, 1, 0, 1, 0, 0]
    );
    assert_eq!(
        DefenseConfig {
            wave_composition: WaveComposition::default(),
            ..config
        },
        DefenseConfig::default()
    );
    assert_eq!(
        parse_config(EMPTY).unwrap().wave_composition,
        WaveComposition::default()
    );
    for singleton in [
        "CoopDefense::Campaign: {last_wave: 1}",
        "CoopDefense::EnemyHealth: {multiplier: 4}",
    ] {
        assert_eq!(
            parse_config(&format!("{EMPTY}singletons: {{{singleton}}}"))
                .unwrap()
                .wave_composition,
            WaveComposition::default()
        );
    }
    let mut invalids = vec![
        "{}".to_owned(),
        "null".into(),
        format!("{{regular_kinds: {rows}}}"),
        format!("{{opening_boss: {flags}}}"),
        fields.replace("regular_kinds:", "extra: 1, regular_kinds:"),
        fields.replace(
            "regular_kinds:",
            &format!("regular_kinds: {rows}, regular_kinds:"),
        ),
    ];
    for invalid in ["[]", "[0,1,2,3]", "null", "[[0,1,2,3]]"] {
        invalids.push(format!(
            "{{regular_kinds: {invalid}, opening_boss: {flags}}}"
        ));
    }
    for invalid in ["4", "-1", "1.5", "4294967296", "null"] {
        invalids.push(fields.replace("[1,2,0,3]", &format!("[1,2,0,{invalid}]")));
    }
    invalids.push(fields.replace("[1,2,0,3]", "[1,2,0]"));
    invalids.push(fields.replace("[1,2,0,3]", "[1,2,0,3,0]"));
    for invalid in [
        "[]",
        "[0]",
        "[0,0,0,0,0,0,0,0,0,2]",
        "[0,0,0,0,0,0,0,0,0,-1]",
        "[0,0,0,0,0,0,0,0,0,0,0]",
        "null",
    ] {
        invalids.push(format!(
            "{{regular_kinds: {rows}, opening_boss: {invalid}}}"
        ));
    }
    for invalid in invalids {
        assert!(
            parse_config(&source(&invalid)).is_err(),
            "accepted {invalid}"
        );
    }
}
