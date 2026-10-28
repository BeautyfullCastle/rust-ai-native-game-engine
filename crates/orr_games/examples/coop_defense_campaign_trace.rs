//! Emit a complete native consumed-input oracle for the actual WASM ABI checker.
//! No runtime state edits: authored scene -> three waves -> terminal -> restart.
use orr_games::{coop_defense_game::*, coop_defense_scene::parse_config};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::fmt::Write;

fn main() {
    let source =
        include_str!("../../../examples/static-game-starter/scenes/stormglass-short.scene.yaml");
    let config = parse_config(source).unwrap();
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
        assert_ne!(state.phase, LOST);
        assert!(!events
            .iter()
            .any(|e| e.payload.kind == EVENT_WAVE && e.payload.value > 3));
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
        comma = ",";
    }
    assert_eq!(victories, 1);
    assert_eq!(terminal_ticks, 6);
    assert_eq!(run(sim.frame()).phase, WON);
    assert_eq!(
        *sim.frame().singleton::<Campaign>(),
        Campaign { last_wave: 3 }
    );
    output.push_str("]}");
    println!("{output}");
}
