//! Bounded, stdin-only authored-scene inspection. No filesystem or runtime launch.
use orr_games::{
    coop_defense_game::{DefenseGame, SCENARIO_BUILD_ID, TICK_RATE},
    coop_defense_scene::{MAX_SOURCE_BYTES, parse_config},
};
use orr_sim::Simulation;
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::ExitCode,
};

const HELP: &str = "coop_defense_scene_report --stdin\nRead at most 64 KiB of UTF-8 Stormglass scene YAML from stdin.\nPrint normalized admitted settings and the initial CPU Frame identity as JSON.\nNo file writes, window, Node/WASM, network or external runtime.\nExit 0: admitted/help; 2: usage; 1: input, admission or output failure.\nThis is source admission, not gameplay, runtime trust or export verification.";

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Usage,
    InputRead,
    TooLarge,
    InvalidUtf8,
    InvalidScene,
    OutputWrite,
}
impl Failure {
    fn code(&self) -> u8 {
        if *self == Self::Usage { 2 } else { 1 }
    }
    fn kind(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::InputRead => "input_read",
            Self::TooLarge => "source_too_large",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidScene => "invalid_scene",
            Self::OutputWrite => "output_write",
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
    if args != [OsString::from("--stdin")] {
        return Err(Failure::Usage);
    }
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
    Ok(format!(
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
    ))
}

fn emit(mut output: impl Write, text: &str) -> Result<(), Failure> {
    writeln!(output, "{text}")
        .and_then(|_| output.flush())
        .map_err(|_| Failure::OutputWrite)
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(3).collect();
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
