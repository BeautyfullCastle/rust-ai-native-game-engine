//! Emit a complete native consumed-input oracle for the actual WASM ABI checker.
//! No runtime state edits: authored scene -> three waves -> terminal -> restart.
use orr_games::{coop_defense_game::*, coop_defense_scene::parse_config};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::fmt::Write;

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let source = match args.as_slice() {
        [] => {
            include_str!("../../../examples/static-game-starter/scenes/stormglass-short.scene.yaml")
        }
        [arg] if arg == "--training" => include_str!(
            "../../../examples/static-game-starter/scenes/stormglass-training.scene.yaml"
        ),
        [arg] if arg == "--counts" => include_str!(
            "../../../examples/static-game-starter/scenes/stormglass-counts.scene.yaml"
        ),
        [arg] if arg == "--health" => include_str!(
            "../../../examples/static-game-starter/scenes/stormglass-health.scene.yaml"
        ),
        [arg] if arg == "--composition" => include_str!(
            "../../../examples/static-game-starter/scenes/stormglass-composition.scene.yaml"
        ),
        _ => panic!("usage: coop_defense_campaign_trace [--training|--counts|--health|--composition]"),
    };
    let composition_trace = args.first().is_some_and(|arg| arg == "--composition");
    let health_trace = args.first().is_some_and(|arg| arg == "--health");
    let mut event_rows = String::new();
    let config = parse_config(source).unwrap();
    let interval = config.wave_pacing.spawn_interval_ticks;
    let make = || {
        Simulation::<DefenseGame>::with_build_id(
            config,
            TICK_RATE,
            u64::from(config.seed),
            SCENARIO_BUILD_ID,
        )
    };
    let mut sim = make();
    let mut replay = make();
    let mut output = format!(
        "{{\"buildId\":\"{:016x}\",\"initialChecksum\":\"{:016x}\",\"lastWave\":3,\"rows\":[",
        SCENARIO_BUILD_ID,
        sim.checksum()
    );
    let mut comma = "";
    let mut victories = 0;
    let mut terminal_ticks = 0;
    let mut spawned = [[0_u32; 3]; 2];
    for tick in 0..20000 {
        let state = run(sim.frame());
        let input = if state.phase == WON {
            terminal_ticks += 1;
            if terminal_ticks == 6 && victories == 0 {
                victories = 1;
                terminal_ticks = 0;
                DefenseInput {
                    action: RESTART,
                    sequence: tick + 1,
                    ..Default::default()
                }
            } else if terminal_ticks == 6 {
                break;
            } else {
                DefenseInput::default()
            }
        } else if tick % 15 == 0 {
            strategy_input(&state, &units(sim.frame()), 0, tick + 1)
        } else {
            DefenseInput::default()
        };
        let mut inputs = TickInputs::new(sim.tick(), 2);
        inputs.set_input(PlayerSlot(0), input);
        let events = sim.step(&inputs);
        let replay_events = replay.step(&inputs);
        assert_eq!(
            sim.checksum(),
            replay.checksum(),
            "native consumed-input replay"
        );
        assert_eq!(
            events
                .iter()
                .map(|e| (e.key, e.payload))
                .collect::<Vec<_>>(),
            replay_events
                .iter()
                .map(|e| (e.key, e.payload))
                .collect::<Vec<_>>()
        );
        let state = run(sim.frame());
        assert!(state.wave <= 3);
        for event in &events {
            if event.payload.kind == EVENT_ENEMY_SPAWN {
                assert!(state.wave_tick >= 180);
                assert_eq!(state.wave_tick % interval, 0);
                let wave = (state.wave - 1) as usize;
                let ordinal = spawned[victories][wave] / 2;
                let expected_kind = (ordinal + state.wave) % 4;
                let base_hp = 24 + state.wave * 15;
                let expected_hp = match expected_kind {
                    ARMORED => base_hp * 2,
                    HORDE => base_hp * 2 / 3,
                    _ => base_hp,
                };
                assert_eq!(event.payload.board, spawned[victories][wave] % 2);
                let (expected_kind, expected_hp) = if composition_trace {
                    // Literal oracle, not calculated from the authored config or spawn code.
                    const KINDS: [&[u32]; 3] = [&[4, 0, 2, 1, 3], &[2, 3, 1, 0, 2, 3], &[4, 2, 3, 1, 0, 2, 3]];
                    const HP: [&[u32]; 3] = [&[1092, 78, 156, 78, 52], &[216, 72, 108, 108, 216, 72], &[1932, 276, 92, 138, 138, 276, 92]];
                    (KINDS[wave][ordinal as usize], HP[wave][ordinal as usize])
                } else {
                    (expected_kind, expected_hp * config.enemy_health.multiplier)
                };
                assert_eq!(event.payload.value, expected_kind);
                assert_eq!(event.payload.hp_after, expected_hp);
                spawned[victories][wave] += 1;
            }
        }
        assert_eq!(*sim.frame().singleton::<WavePacing>(), config.wave_pacing);
        assert_eq!(*sim.frame().singleton::<WaveCounts>(), config.wave_counts);
        assert_eq!(*sim.frame().singleton::<EnemyHealth>(), config.enemy_health);
        assert_eq!(*sim.frame().singleton::<WaveComposition>(), config.wave_composition);
        assert_ne!(state.phase, LOST);
        assert!(
            !events
                .iter()
                .any(|e| e.payload.kind == EVENT_WAVE && e.payload.value > 3)
        );
        write!(
            output,
            "{comma}[{},{},{},{},\"{:016x}\",{},{},{},{}]",
            input.action,
            input.source_cell,
            input.target_cell,
            input.sequence,
            sim.checksum(),
            state.wave,
            state.phase,
            state.tick,
            state.generation
        )
        .unwrap();
        if health_trace || composition_trace {
            event_rows.push_str(comma);
            event_rows.push('[');
            for (index, event) in events.iter().enumerate() {
                if index != 0 { event_rows.push(','); }
                let e = event.payload;
                write!(event_rows, "[\"{}\",{},{},\"{}\",{},{},{},{},{},{},{},{},{},{},{}]",
                    event.key.tick, event.key.system_index, event.key.seq,
                    e.event_kind().unwrap().name(), e.generation, e.unit_id,
                    e.other_unit_id, e.enemy_id, e.projectile_id, e.board, e.role,
                    e.value, e.hp_after, e.x, e.y).unwrap();
            }
            event_rows.push(']');
        }
        comma = ",";
    }
    assert_eq!(victories, 1);
    assert_eq!(terminal_ticks, 6);
    assert_eq!(run(sim.frame()).phase, WON);
    assert_eq!(
        *sim.frame().singleton::<Campaign>(),
        Campaign { last_wave: 3 }
    );
    for run_counts in spawned {
        for (wave, count) in run_counts.into_iter().enumerate() {
            assert_eq!(count, config.wave_counts.pair_counts[wave] * 2);
        }
    }
    output.push(']');
    if composition_trace {
        output.push_str(",\"compositionOracle\":true");
    }
    if health_trace || composition_trace {
        write!(output, ",\"eventRows\":[{event_rows}]").unwrap();
    }
    output.push('}');
    println!("{output}");
}
