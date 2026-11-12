//! Point-in-time compatibility check, not trust authentication or publication.
use super::UnvalidatedStage;
use orr_games::coop_defense_game as game;
use orr_sim::{Simulation, TickInputs};
use std::{path::Path, time::Duration};

#[path = "process.rs"]
mod process;
const TICKS: u32 = 60;

impl UnvalidatedStage<'_> {
    /// Execute the copied, already trusted runtime for 60 idle CPU/headless ticks.
    /// `trusted_runtime` must explicitly acknowledge code whose origin the caller
    /// has verified independently; a digest/PE header is not authentication.
    ///
    /// Holds all existing source/stage reservations, compares the exact engine
    /// summary/checksum, and rechecks even after execution failure. The execution
    /// budget is 15 seconds plus up to 5 seconds of owned process/job cleanup;
    /// synchronous Windows setup calls are outside that polling budget. Output
    /// is capped at 64 KiB combined; nonempty stderr and nonzero exit are refused.
    /// Success returns the engine-derived summary only, leaves this value an
    /// UnvalidatedStage, and is not a distribution/readiness receipt. No files are
    /// renamed, deleted, or written by this method. The trusted runtime is not
    /// sandboxed; cooperating writers and host dependencies remain assumptions.
    pub fn check_trusted_runtime(&mut self, trusted_runtime: bool) -> Result<String, String> {
        self.check_with(trusted_runtime, |path| {
            process::run(path, Duration::from_secs(15), 65536)
        })
    }

    fn check_with(
        &mut self,
        trusted_runtime: bool,
        execute: impl FnOnce(&Path) -> Result<process::Output, String>,
    ) -> Result<String, String> {
        let check = (|| {
            if !trusted_runtime {
                return Err(
                    "explicit trusted-runtime acknowledgement required; digest is not trust".into(),
                );
            }
            self.recheck()?;
            let config = *self.inputs.admit(&self.pin)?.config();
            let expected = expected(config);
            let execution = execute(&self.path);
            // Never let process failure skip the post-run source/stage check.
            let recheck = self.recheck();
            match (execution, recheck) {
                (Ok(output), Ok(())) => {
                    verify_output(&output, &expected)?;
                    Ok(expected)
                }
                (Err(run), Ok(())) => Err(run),
                (Ok(_), Err(stage)) => Err(format!("post-smoke stage recheck failed: {stage}")),
                (Err(run), Err(stage)) => {
                    Err(format!("{run}; post-smoke stage recheck failed: {stage}"))
                }
            }
        })();
        check.map_err(|e| format!("{e}; retained stage: {}", self.path.display()))
    }
}

fn expected(config: game::DefenseConfig) -> String {
    let mut simulation = Simulation::<game::DefenseGame>::with_build_id(
        config,
        game::TICK_RATE,
        u64::from(config.seed),
        game::SCENARIO_BUILD_ID,
    );
    for tick in 1..=TICKS {
        simulation.step(&TickInputs::new(u64::from(tick), 1));
    }
    let frame = simulation.frame();
    let run = game::run(frame);
    format!(
        "Stormglass tick={} wave={} phase={} reactor={} kills={} merges={} checksum=0x{:016x}",
        run.tick,
        run.wave,
        run.phase,
        run.reactor_hp,
        run.kills,
        run.merges,
        frame.checksum()
    )
}

fn verify_output(output: &process::Output, expected: &str) -> Result<(), String> {
    if output.exit_code != 0 {
        return Err(format!("trusted runtime exited with {}", output.exit_code));
    }
    if !output.stderr.is_empty() {
        return Err("trusted runtime wrote stderr".into());
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "runtime stdout is not UTF-8")?;
    let mut lines = text.lines();
    let first = lines.next().ok_or("missing native summary")?;
    let actual = if let Some(extra) = first.strip_prefix("Stormglass pixelfx ") {
        let fields: Vec<_> = extra.split(' ').collect();
        if fields.len() != 4
            || fields
                .iter()
                .zip(["active=", "started=", "dropped=", "canceled="])
                .any(|(field, prefix)| {
                    field.strip_prefix(prefix).is_none_or(|v| {
                        v.is_empty()
                            || !v.bytes().all(|b| b.is_ascii_digit())
                            || v.parse::<u32>().is_err()
                    })
                })
        {
            return Err("invalid optional pixelfx summary".into());
        }
        lines.next().ok_or("missing native summary after pixelfx")?
    } else {
        first
    };
    if actual != expected || lines.next().is_some() {
        return Err("copied runtime summary/checksum differs from admitted scene".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
