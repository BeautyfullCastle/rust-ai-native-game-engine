use super::*;
use orr_reflect::{Kind, Reflect, Value};
use orr_sim::{Simulation, TickInputs};

#[test]
fn wave_counts_are_a_forty_byte_reflected_singleton_with_literal_defaults() {
    const EXPECTED: [u32; 10] = [10, 12, 14, 16, 18, 20, 22, 24, 26, 28];
    assert_eq!(WaveCounts::default().pair_counts, EXPECTED);
    assert_eq!(WaveCounts::default_value().pair_counts, EXPECTED);
    assert_eq!(std::mem::size_of::<WaveCounts>(), 40);
    let desc = WaveCounts::describe();
    desc.check_layout(40).unwrap();
    let Kind::Struct { size, fields } = desc.kind else {
        panic!("WaveCounts must have a fixed struct layout");
    };
    assert_eq!(size, 40);
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].name.as_str(), fields[0].offset),
        ("pair_counts", 0)
    );
    let Kind::Array { elem, len } = &fields[0].ty.kind else {
        panic!("pair_counts must be an exact-size array");
    };
    assert_eq!(*len, 10);
    let Kind::Int { int, range } = &elem.kind else {
        panic!("every pair count must be an integer");
    };
    assert_eq!(*int, orr_reflect::IntKind::U32);
    assert_eq!((range.min, range.max), (Some(1), Some(28)));
    let registry = Simulation::<DefenseGame>::build_registry();
    let id = registry.singleton_id::<WaveCounts>().unwrap();
    assert_eq!(registry.singleton_name(id), WAVE_COUNTS_NAME);
    assert_eq!(registry.singleton_size(id), 40);
    let types = scenario_types();
    assert_eq!(
        types.get(WAVE_COUNTS_NAME).unwrap().default_bytes().len(),
        40
    );
}

#[test]
fn reflected_whole_array_and_indexed_edits_reject_invalid_counts_atomically() {
    let types = scenario_types();
    let mut sim = Simulation::<DefenseGame>::new(DefenseConfig::default(), TICK_RATE, 1);
    for index in 0..10 {
        for bad in [0, 29, u32::MAX] {
            let before = sim.frame().checksum();
            let path = format!("pair_counts[{index}]");
            assert!(
                types
                    .set_singleton_field(
                        sim.frame_mut(),
                        WAVE_COUNTS_NAME,
                        &path,
                        Value::Int(bad.into())
                    )
                    .is_err()
            );
            assert_eq!(sim.frame().checksum(), before);
            let mut counts = WaveCounts::default();
            counts.pair_counts[index] = bad;
            let value = WaveCounts::describe().read(bytemuck::bytes_of(&counts));
            assert!(
                types
                    .set_singleton_field(sim.frame_mut(), WAVE_COUNTS_NAME, "", value)
                    .is_err()
            );
            assert_eq!(sim.frame().checksum(), before);
            assert!(
                types
                    .set_singleton_field(
                        sim.frame_mut(),
                        WAVE_COUNTS_NAME,
                        "pair_counts",
                        Value::Array(
                            counts
                                .pair_counts
                                .into_iter()
                                .map(|v| Value::Int(v.into()))
                                .collect()
                        ),
                    )
                    .is_err()
            );
            assert_eq!(sim.frame().checksum(), before);
        }
    }
    for len in [0, 9, 11] {
        let before = sim.frame().checksum();
        assert!(
            types
                .set_singleton_field(
                    sim.frame_mut(),
                    WAVE_COUNTS_NAME,
                    "pair_counts",
                    Value::Array(vec![Value::Int(1); len]),
                )
                .is_err()
        );
        assert_eq!(sim.frame().checksum(), before);
    }
    for value in [1, 28] {
        types
            .set_singleton_field(
                sim.frame_mut(),
                WAVE_COUNTS_NAME,
                "pair_counts[9]",
                Value::Int(value),
            )
            .unwrap();
        assert_eq!(
            sim.frame().singleton::<WaveCounts>().pair_counts[9],
            value as u32
        );
    }
}

#[test]
fn typed_setup_rejects_every_invalid_entry_even_beyond_campaign_end() {
    for index in 0..10 {
        for bad in [0, 29, u32::MAX] {
            let mut config = DefenseConfig {
                campaign: Campaign { last_wave: 1 },
                ..Default::default()
            };
            config.wave_counts.pair_counts[index] = bad;
            assert!(!config.wave_counts.is_valid());
            let rejected =
                std::panic::catch_unwind(|| Simulation::<DefenseGame>::new(config, TICK_RATE, 1));
            assert!(rejected.is_err(), "accepted index {index} = {bad}");
        }
    }
}

#[test]
fn default_and_custom_counts_preserve_literal_kind_hp_order_lanes_and_pacing() {
    // Independent literal expectations, not calls back into the count/kind/HP implementation.
    const DEFAULT_PAIRS: [u32; 10] = [10, 12, 14, 16, 18, 20, 22, 24, 26, 28];
    const CUSTOM_PAIRS: [u32; 10] = [1, 28, 3, 2, 1, 4, 5, 6, 7, 1];
    const ORDERS: [[u32; 4]; 10] = [
        [FAST, ARMORED, HORDE, NORMAL],
        [ARMORED, HORDE, NORMAL, FAST],
        [HORDE, NORMAL, FAST, ARMORED],
        [NORMAL, FAST, ARMORED, HORDE],
        [FAST, ARMORED, HORDE, NORMAL],
        [ARMORED, HORDE, NORMAL, FAST],
        [HORDE, NORMAL, FAST, ARMORED],
        [NORMAL, FAST, ARMORED, HORDE],
        [FAST, ARMORED, HORDE, NORMAL],
        [ARMORED, HORDE, NORMAL, FAST],
    ];
    // Columns are NORMAL, FAST, ARMORED, HORDE, BOSS.
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
    for (counts, expected) in [
        (WaveCounts::default(), DEFAULT_PAIRS),
        (
            WaveCounts {
                pair_counts: CUSTOM_PAIRS,
            },
            CUSTOM_PAIRS,
        ),
    ] {
        for interval in [35, 70, 350] {
            for wave in 1..=10 {
                let index = (wave - 1) as usize;
                let mut state = initial_run(1, Scenario::default());
                state.wave = wave;
                let mut pool = [DefenseEnemy::default(); MAX_ENEMIES];
                let mut events = Vec::new();
                let first = 180_u32.div_ceil(interval) * interval;
                let pairs = expected[index];
                for tick in 0..=first + interval * (pairs + 1) {
                    state.wave_tick = tick;
                    let before = state.spawned;
                    spawn_wave(
                        &mut state,
                        &mut pool,
                        &mut events,
                        WavePacing {
                            spawn_interval_ticks: interval,
                        },
                        counts,
                        EnemyHealth::default(),
                        WaveComposition::default(),
                    );
                    let is_spawn =
                        tick >= first && tick < first + interval * pairs && tick % interval == 0;
                    assert_eq!(state.spawned - before, if is_spawn { 2 } else { 0 });
                }
                assert_eq!(state.spawned, pairs * 2);
                assert_eq!(events.len(), (pairs * 2) as usize);
                assert_eq!(
                    pool.iter().filter(|enemy| enemy.active != 0).count(),
                    events.len()
                );
                for (ordinal, pair) in events.as_chunks::<2>().0.iter().enumerate() {
                    let kind = if (wave == 5 || wave == 10) && ordinal == 0 {
                        BOSS
                    } else {
                        ORDERS[index][ordinal % 4]
                    };
                    for (lane, event) in pair.iter().enumerate() {
                        assert_eq!(
                            (event.kind, event.board, event.value),
                            (EVENT_ENEMY_SPAWN, lane as u32, kind)
                        );
                        assert_eq!(event.enemy_id, (ordinal * 2 + lane + 1) as u32);
                        assert_eq!(event.hp_after, HP[index][kind as usize]);
                        let enemy = pool
                            .iter()
                            .find(|enemy| enemy.id == event.enemy_id)
                            .unwrap();
                        assert_eq!(
                            (enemy.lane, enemy.kind, enemy.hp, enemy.max_hp),
                            (
                                lane as u32,
                                kind,
                                HP[index][kind as usize],
                                HP[index][kind as usize]
                            )
                        );
                    }
                }
            }
        }
    }
}

fn tick(sim: &mut Simulation<DefenseGame>, input: DefenseInput) -> Vec<DefenseEvent> {
    let mut inputs = TickInputs::new(sim.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs)
        .into_iter()
        .map(|event| event.payload)
        .collect()
}

#[test]
fn custom_counts_complete_all_waves_replay_exactly_and_survive_restart() {
    const PAIRS: [u32; 10] = [1, 2, 3, 4, 1, 2, 3, 4, 2, 1];
    const ENEMIES: [u32; 10] = [2, 4, 6, 8, 2, 4, 6, 8, 4, 2];
    let config = DefenseConfig {
        scenario: Scenario {
            starting_gold: 10000,
            reactor_hp: 1000,
        },
        wave_pacing: WavePacing {
            spawn_interval_ticks: 70,
        },
        wave_counts: WaveCounts { pair_counts: PAIRS },
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
    let mut total = [0; 10];
    let mut waves = Vec::new();
    for index in 0..20_000 {
        let state = run(sim.frame());
        if state.phase != PLAYING {
            break;
        }
        let input = if index % 15 == 0 {
            strategy_input(&state, &units(sim.frame()), 0, index + 1)
        } else {
            DefenseInput::default()
        };
        let events = tick(&mut sim, input);
        for event in &events {
            if event.kind == EVENT_ENEMY_SPAWN {
                total[(state.wave - 1) as usize] += 1;
            } else if event.kind == EVENT_WAVE {
                waves.push(event.value);
            }
        }
        recorded.push((input, sim.checksum(), events));
    }
    assert_eq!((run(sim.frame()).phase, run(sim.frame()).wave), (WON, 10));
    assert_eq!(total, ENEMIES);
    assert_eq!(waves, [2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert!(enemies(sim.frame()).iter().all(|enemy| enemy.active == 0));
    let sequence = recorded
        .iter()
        .map(|(input, _, _)| input.sequence)
        .max()
        .unwrap()
        + 1;
    sim.restore(&initial);
    for (input, checksum, events) in recorded {
        assert_eq!(tick(&mut sim, input), events);
        assert_eq!(sim.checksum(), checksum);
    }
    tick(
        &mut sim,
        DefenseInput {
            action: RESTART,
            sequence,
            ..Default::default()
        },
    );
    assert_eq!(*sim.frame().singleton::<WaveCounts>(), config.wave_counts);
    assert_eq!(*sim.frame().singleton::<WavePacing>(), config.wave_pacing);
    assert_eq!(*sim.frame().singleton::<Campaign>(), config.campaign);
    assert_eq!(*sim.frame().singleton::<Scenario>(), config.scenario);
    assert_eq!(
        (
            run(sim.frame()).wave,
            run(sim.frame()).tick,
            run(sim.frame()).phase
        ),
        (1, 0, PLAYING)
    );
    let mut restarted = [0; 10];
    for index in 0..20_000 {
        let state = run(sim.frame());
        if state.phase != PLAYING {
            break;
        }
        let input = if index % 15 == 0 {
            strategy_input(&state, &units(sim.frame()), 0, sequence + index + 1)
        } else {
            DefenseInput::default()
        };
        for event in tick(&mut sim, input) {
            if event.kind == EVENT_ENEMY_SPAWN {
                restarted[(state.wave - 1) as usize] += 1;
            }
        }
    }
    assert_eq!((run(sim.frame()).phase, run(sim.frame()).wave), (WON, 10));
    assert_eq!(restarted, ENEMIES);
}

#[test]
fn custom_final_count_waits_for_last_enemy_and_final_leak_loss_wins() {
    for hp in [1, 30] {
        let config = DefenseConfig {
            campaign: Campaign { last_wave: 3 },
            wave_counts: WaveCounts {
                pair_counts: [1; 10],
            },
            ..Default::default()
        };
        let mut sim = Simulation::<DefenseGame>::new(config, TICK_RATE, 1);
        // A narrow terminal-state fixture, separate from real gameplay above.
        let mut state = run(sim.frame());
        state.wave = 3;
        state.spawned = 2;
        state.reactor_hp = hp;
        sim.frame_mut().set_singleton(state);
        let id = sim.frame_mut().query::<(&DefenseEnemy,)>().next().unwrap().0;
        *sim.frame_mut().get_mut::<DefenseEnemy>(id).unwrap() = DefenseEnemy {
            active: 1,
            id: 1,
            hp: 100,
            max_hp: 100,
            ..Default::default()
        };
        let events = tick(&mut sim, DefenseInput::default());
        assert_eq!(run(sim.frame()).phase, PLAYING);
        assert!(!events.iter().any(|event| event.kind == EVENT_FINISH));
        sim.frame_mut()
            .get_mut::<DefenseEnemy>(id)
            .unwrap()
            .progress = 1999;
        let events = tick(&mut sim, DefenseInput::default());
        let expected = if hp == 1 { LOST } else { WON };
        assert_eq!(run(sim.frame()).phase, expected);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == EVENT_FINISH)
                .map(|event| event.value)
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(run(sim.frame()).wave, 3);
    }
}

#[test]
fn old_v3_frame_schema_is_rejected_and_nonzero_build_identity_changes() {
    // Exactly the pre-WaveCounts registry, not a corrupted current snapshot.
    let mut old = ComponentRegistryBuilder::new();
    old.register_singleton::<FrameRng>("orr_sim::FrameRng");
    old.register_singleton::<DefenseRun>("CoopDefense::Run");
    old.register_component::<DefenseUnit>("CoopDefense::Unit");
    old.register_component::<DefenseEnemy>("CoopDefense::Enemy");
    old.register_component::<DefenseProjectile>("CoopDefense::Projectile");
    old.register_singleton::<Scenario>(SCENARIO_NAME);
    old.register_singleton::<Campaign>(CAMPAIGN_NAME);
    old.register_singleton::<WavePacing>(WAVE_PACING_NAME);
    let old = Frame::new(old.build());
    let current_registry = Simulation::<DefenseGame>::build_registry();
    let result = Frame::from_bytes(current_registry, &old.to_bytes());
    assert!(matches!(
        result,
        Err(orr_ecs::FrameDecodeError::SchemaCount {
            kind: "singleton",
            expected: 8,
            found: 5,
        })
    ));
    assert!(Simulation::<DefenseGame>::from_frame(&old, TICK_RATE, SCENARIO_BUILD_ID).is_err());
    let v3 = orr_sim::frame_build_id(0x5354_4f52_4d00_0003);
    assert_ne!(v3, 0);
    assert_ne!(SCENARIO_BUILD_ID, 0);
    assert_ne!(
        orr_sim::build_hash_of(v3, 0),
        orr_sim::build_hash_of(SCENARIO_BUILD_ID, 0)
    );
}

#[cfg(feature = "coop-defense-scene")]
mod scene_admission {
    use super::*;
    use crate::coop_defense_scene::{config_from_baked, parse_config};
    const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
    const DEFAULTS: &str = "[10, 12, 14, 16, 18, 20, 22, 24, 26, 28]";

    #[test]
    fn legacy_sources_default_counts_independently_and_valid_arrays_round_trip() {
        for source in [
            EMPTY.to_string(),
            format!(
                "{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 250, reactor_hp: 75}}}}\n"
            ),
            format!("{EMPTY}singletons: {{CoopDefense::Campaign: {{last_wave: 3}}}}\n"),
            format!(
                "{EMPTY}singletons: {{CoopDefense::WavePacing: {{spawn_interval_ticks: 70}}}}\n"
            ),
        ] {
            assert_eq!(
                parse_config(&source).unwrap().wave_counts.pair_counts,
                [10, 12, 14, 16, 18, 20, 22, 24, 26, 28]
            );
        }
        for pairs in [[1; 10], [28; 10], [1, 28, 3, 2, 1, 4, 5, 6, 7, 1]] {
            let source = format!(
                "{EMPTY}singletons: {{CoopDefense::WaveCounts: {{pair_counts: {pairs:?}}}}}\n"
            );
            let config = parse_config(&source).unwrap();
            assert_eq!(config.wave_counts.pair_counts, pairs);
            assert_eq!(config.scenario, Scenario::default());
            assert_eq!(config.campaign, Campaign::default());
            assert_eq!(config.wave_pacing, WavePacing::default());
            let types = scenario_types();
            let scene = orr_reflect::Scene::parse(&source, &types).unwrap();
            let mut baked = Frame::new(Simulation::<DefenseGame>::build_registry());
            scene.bake(&types, &mut baked).unwrap();
            assert_eq!(config_from_baked(&scene, &baked).unwrap(), config);
        }
        let explicit = format!(
            "{EMPTY}singletons: {{CoopDefense::WaveCounts: {{pair_counts: {DEFAULTS}}}}}\n"
        );
        assert_eq!(
            parse_config(&explicit).unwrap(),
            parse_config(EMPTY).unwrap()
        );
    }

    #[test]
    fn source_requires_exact_field_array_shape_and_rejects_duplicates() {
        for fields in [
            "{}".to_string(),
            "null".into(),
            "{pair_counts: null}".into(),
            "{pair_counts: 10}".into(),
            "{pair_counts: {one: 10}}".into(),
            "{pair_counts: []}".into(),
            "{pair_counts: [1,1,1,1,1,1,1,1,1]}".into(),
            "{pair_counts: [1,1,1,1,1,1,1,1,1,1,1]}".into(),
            format!("{{pair_counts: {DEFAULTS}, hp: 10}}"),
            format!("{{pair_counts: {DEFAULTS}, kind: 1}}"),
            format!("{{pair_counts: {DEFAULTS}, extra: 1}}"),
            format!("{{pair_counts: {DEFAULTS}, pair_counts: {DEFAULTS}}}"),
        ] {
            let source = format!("{EMPTY}singletons: {{CoopDefense::WaveCounts: {fields}}}\n");
            assert!(parse_config(&source).is_err(), "accepted {source}");
        }
        let duplicate = format!(
            "{EMPTY}singletons: {{CoopDefense::WaveCounts: {{pair_counts: {DEFAULTS}}}, CoopDefense::WaveCounts: {{pair_counts: {DEFAULTS}}}}}\n"
        );
        assert!(parse_config(&duplicate).is_err());
    }

    #[test]
    fn source_checks_every_array_value_even_beyond_campaign_end() {
        for index in 0..10 {
            for bad in [
                "0",
                "29",
                "-1",
                "4294967296",
                "1.5",
                "true",
                "null",
                "\"1\"",
            ] {
                let mut entries = ["1"; 10];
                entries[index] = bad;
                let source = format!(
                    "{EMPTY}singletons: {{CoopDefense::Campaign: {{last_wave: 1}}, CoopDefense::WaveCounts: {{pair_counts: [{}]}}}}\n",
                    entries.join(",")
                );
                assert!(parse_config(&source).is_err(), "accepted {source}");
            }
        }
        // Shared bake admission also rejects a typed candidate whose bytes were
        // modified after parsing, before it can initialize runtime state.
        let source = format!(
            "{EMPTY}singletons: {{CoopDefense::WaveCounts: {{pair_counts: {DEFAULTS}}}}}\n"
        );
        let types = scenario_types();
        let scene = orr_reflect::Scene::parse(&source, &types).unwrap();
        let mut frame = Frame::new(Simulation::<DefenseGame>::build_registry());
        scene.bake(&types, &mut frame).unwrap();
        let mut counts = *frame.singleton::<WaveCounts>();
        counts.pair_counts[9] = 0;
        frame.set_singleton(counts);
        assert!(config_from_baked(&scene, &frame).is_err());
    }
}
