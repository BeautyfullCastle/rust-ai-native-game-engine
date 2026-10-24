//! Real-input headless run and exact replay. This is a gameplay oracle, not a UI test.
use orr_games::coop_defense_game::*;
use orr_sim::{PlayerSlot, Simulation, TickInputs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str, default: u32| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].parse::<u32>().expect("unsigned option"))
            .unwrap_or(default)
    };
    let afk = args.iter().any(|arg| arg == "--afk");
    let parity = args.iter().any(|arg| arg == "--parity");
    let seed = value("--seed", DefenseConfig::default().seed);
    let cadence = value("--cadence", 90).max(1);
    let limit = value("--ticks", 60 * 600);
    let mut sim =
        Simulation::<DefenseGame>::new(DefenseConfig { seed }, TICK_RATE, u64::from(seed));
    let mut recorded = Vec::new();
    let mut sequence = 0;
    let mut previous_wave = 0;
    for tick in 1..=limit {
        let state = run(sim.frame());
        let input = if parity {
            if (tick - 1) % 30 == 0 {
                sequence += 1;
                DefenseInput {
                    action: SUMMON,
                    sequence,
                    ..Default::default()
                }
            } else {
                DefenseInput::default()
            }
        } else if !afk && (tick == 1 || tick % cadence == 0) {
            sequence += 1;
            strategy_input(&state, &units(sim.frame()), 0, sequence)
        } else {
            DefenseInput::default()
        };
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
        recorded.push(inputs);
        let state = run(sim.frame());
        if state.wave != previous_wave {
            println!(
                "wave={} tick={} reactor={} gold={} kills={} leaks={}",
                state.wave, state.tick, state.reactor_hp, state.gold[0], state.kills, state.leaks
            );
            previous_wave = state.wave;
        }
        if state.phase != PLAYING {
            break;
        }
    }
    let state = run(sim.frame());
    if !parity {
        assert_ne!(
            state.phase, PLAYING,
            "run exceeded bounded completion window"
        );
    }
    let mut replay =
        Simulation::<DefenseGame>::new(DefenseConfig { seed }, TICK_RATE, u64::from(seed));
    for input in &recorded {
        replay.step(input);
    }
    assert_eq!(
        sim.checksum(),
        replay.checksum(),
        "same seed/input replay diverged"
    );
    println!("result={} seed={seed} cadence={cadence} ticks={} reactor={} kills={} leaks={} summons={} merges={} checksum={:016x} replay=exact",
        match state.phase { WON => "victory", LOST => "defeat", _ => "playing" }, state.tick, state.reactor_hp,
        state.kills, state.leaks, state.summons, state.merges, sim.checksum());
    if !parity {
        assert_eq!(
            state.phase,
            if afk { LOST } else { WON },
            "ordinary policy/AFK acceptance failed"
        );
    }
}
