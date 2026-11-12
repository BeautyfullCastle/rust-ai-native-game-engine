//! Bounded, stdin-only authored-scene inspection. No filesystem or runtime launch.
use orr_games::{
    coop_defense_game::{
        run, strategy_input, units, DefenseConfig, DefenseGame, DefenseInput, PLAYING,
        SCENARIO_BUILD_ID, TICK_RATE, WON,
    },
    coop_defense_scene::{parse_config, MAX_SOURCE_BYTES},
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::ExitCode,
};

const HELP: &str = "coop_defense_scene_report --stdin [--simulate TICKS]\nRead at most 64 KiB of UTF-8 Stormglass scene YAML from stdin.\nPrint normalized admitted settings and the initial CPU Frame identity as JSON.\nOptional --simulate runs the ordinary strategy for 1..=36000 ticks with exact same-input CPU replay.\nNo file writes, window, Node/WASM, network or external runtime.\nExit 0: admitted/help; 2: usage; 1: input, admission or output failure.\nExit 0 does not require victory. This is not rendering, runtime trust or export verification.";

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Usage,
    InputRead,
    TooLarge,
    InvalidUtf8,
    InvalidScene,
    OutputWrite,
    ReplayMismatch,
}
impl Failure {
    fn code(&self) -> u8 {
        if *self == Self::Usage {
            2
        } else {
            1
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::InputRead => "input_read",
            Self::TooLarge => "source_too_large",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidScene => "invalid_scene",
            Self::OutputWrite => "output_write",
            Self::ReplayMismatch => "replay_mismatch",
        }
    }
    fn json(&self) -> String {
        format!(
            "{{\"error\":{{\"kind\":\"{}\",\"exit_code\":{}}}}}",
            self.kind(),
            self.code()
        )
    }
}

fn report(args: &[OsString], input: impl Read) -> Result<String, Failure> {
    if args == [OsString::from("--help")] {
        return Ok(HELP.to_owned());
    }
    let ticks = match args {
        [flag] if flag == "--stdin" => None,
        [flag, option, value] if flag == "--stdin" && option == "--simulate" => {
            let text = value.to_str().ok_or(Failure::Usage)?;
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Failure::Usage);
            }
            let n = text.parse::<u32>().map_err(|_| Failure::Usage)?;
            if !(1..=36000).contains(&n) {
                return Err(Failure::Usage);
            }
            Some(n)
        }
        _ => return Err(Failure::Usage),
    };
    let mut bytes = Vec::new();
    input
        .take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::InputRead)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(Failure::TooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Failure::InvalidUtf8)?;
    let c = parse_config(text).map_err(|_| Failure::InvalidScene)?;
    let simulation = Simulation::<DefenseGame>::with_build_id(
        c,
        TICK_RATE,
        u64::from(c.seed),
        SCENARIO_BUILD_ID,
    );
    // Every interpolated value is an admitted integer or a fixed-size array of
    // integers. No source text, paths, YAML diagnostics or unescaped strings enter JSON.
    let mut result = format!(
        concat!(
            "{{\"schema\":\"orr.coop.scene-report/1\",\"admitted\":true,",
            "\"source_bytes\":{},\"tick_rate\":{},\"initial_tick\":0,",
            "\"build_id\":\"0x{:016x}\",\"initial_checksum\":\"0x{:016x}\",",
            "\"config\":{{\"seed\":{},\"starting_gold\":{},\"reactor_hp\":{},",
            "\"last_wave\":{},\"spawn_interval_ticks\":{},\"pair_counts\":{:?},",
            "\"enemy_health_multiplier\":{},\"regular_kinds\":{:?},\"opening_boss\":{:?}}}}}"
        ),
        bytes.len(),
        TICK_RATE,
        SCENARIO_BUILD_ID,
        simulation.checksum(),
        c.seed,
        c.scenario.starting_gold,
        c.scenario.reactor_hp,
        c.campaign.last_wave,
        c.wave_pacing.spawn_interval_ticks,
        c.wave_counts.pair_counts,
        c.enemy_health.multiplier,
        c.wave_composition.regular_kinds,
        c.wave_composition.opening_boss
    );
    if let Some(limit) = ticks {
        let smoke = simulate(c, limit)?;
        result.pop(); // Append to the known, closed JSON object above.
        result.push_str(",\"simulation\":");
        result.push_str(&smoke);
        result.push('}');
    }
    Ok(result)
}

/// Ordinary deterministic strategy, no privileged edits, terminal restart or recording growth.
fn simulate(config: DefenseConfig, limit: u32) -> Result<String, Failure> {
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
    let mut executed = 0;
    for tick in 1..=limit {
        let state = run(sim.frame());
        if state.phase != PLAYING {
            break;
        }
        let input = if tick == 1 || tick % 90 == 0 {
            strategy_input(&state, &units(sim.frame()), 0, tick)
        } else {
            DefenseInput::default()
        };
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
        replay.step(&inputs);
        if sim.checksum() != replay.checksum() {
            return Err(Failure::ReplayMismatch);
        }
        executed += 1;
    }
    let state = run(sim.frame());
    let outcome = match state.phase {
        PLAYING => "playing",
        WON => "victory",
        _ => "defeat",
    };
    Ok(format!(
        concat!(
            "{{\"policy\":\"ordinary-90\",\"tick_budget\":{},\"ticks_executed\":{},",
            "\"outcome\":\"{}\",\"budget_exhausted\":{},\"wave\":{},\"reactor_hp\":{},",
            "\"kills\":{},\"leaks\":{},\"summons\":{},\"merges\":{},",
            "\"final_checksum\":\"0x{:016x}\",\"same_input_replay\":\"exact\"}}"
        ),
        limit,
        executed,
        outcome,
        state.phase == PLAYING,
        state.wave,
        state.reactor_hp,
        state.kills,
        state.leaks,
        state.summons,
        state.merges,
        sim.checksum()
    ))
}

fn emit(mut output: impl Write, text: &str) -> Result<(), Failure> {
    writeln!(output, "{text}")
        .and_then(|_| output.flush())
        .map_err(|_| Failure::OutputWrite)
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(4).collect();
    let result =
        report(&args, io::stdin().lock()).and_then(|text| emit(io::stdout().lock(), &text));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Best effort only: a closed stderr must not cause a panic.
            let _ = writeln!(io::stderr().lock(), "{}", error.json());
            ExitCode::from(error.code())
        }
    }
}

#[cfg(test)]
#[path = "coop_defense_scene_report/tests.rs"]
mod tests;
