//! Read-only, bounded gameplay acceptance for an authored plain Room project.
use std::process::ExitCode;
const HELP: &str = "orr_playtest_room --project PATH --script FILE.json [--json]\nLinux CPU playtest; schema 1, at most 256 steps and 6000 total ticks.\nEach step replaces logical input and checks key/win state. No project files are written.";

// Scan arguments without retaining an unbounded list. Any exact --json requests
// JSON, including malformed invocations and flags beyond the retained limit.
fn main() -> ExitCode {
    let mut args = Vec::new();
    let mut json = false;
    let mut overflow = false;
    for arg in std::env::args_os().skip(1) {
        json |= arg == "--json";
        if args.len() < 6 {
            args.push(arg);
        } else {
            overflow = true;
        }
    }
    if !overflow && args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let valid = !overflow
        && (args.len() == 4 || (args.len() == 5 && args[4] == "--json"))
        && args[0] == "--project"
        && args[2] == "--script"
        && args[1] != "--json"
        && args[3] != "--json";
    if !valid {
        if json {
            println!(
                "{}",
                serde_json::json!({"schema":1,"status":"error","error":{"kind":"usage","message":HELP},"steps":[]})
            );
        } else {
            eprintln!("{HELP}");
        }
        return ExitCode::from(2);
    }
    let mut report = serde_json::json!({
        "schema":1, "status":"error", "engine_version":env!("CARGO_PKG_VERSION"), "steps":[]
    });
    #[cfg(target_os = "linux")]
    let result = linux::run(
        std::path::Path::new(&args[1]),
        std::path::Path::new(&args[3]),
        json,
        &mut report,
    );
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), (&str, String)> = Err((
        "unsupported_target",
        "Room playtests currently require Linux".into(),
    ));
    match result {
        Ok(()) => {
            if json {
                report["status"] = serde_json::json!("passed");
                println!("{report}");
            }
            ExitCode::SUCCESS
        }
        Err((kind, error)) => {
            if json {
                // Admission diagnostics may contain user-controlled strings. Cap
                // the diagnostic at a UTF-8 boundary; state fields stay exact.
                let mut message = error.clone();
                if message.len() > 2048 {
                    let mut end = 2048;
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    message.truncate(end);
                }
                report["error"] = serde_json::json!({"kind":kind,"message":message,"message_truncated":error.len()>2048});
                println!("{report}");
            } else {
                eprintln!("orr_playtest_room: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use orr_sample::{
        room_game::{RoomInput, RoomRun, INTERACT},
        room_project::PreparedProject,
    };
    use orr_sim::{PlayerSlot, TickInputs};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::{fs::File, io::Read, path::Path};
    const MAX_BYTES: u64 = 65536;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Script {
        schema: u32,
        steps: Vec<Step>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Step {
        ticks: u32,
        move_x: i8,
        move_z: i8,
        interact: bool,
        expect_key: bool,
        expect_won: bool,
    }
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    fn read_script(path: &Path) -> Result<(Script, String), String> {
        use rustix::fs::{open, Mode, OFlags};
        let fd = open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|error| format!("cannot open script: {error}"))?;
        let file = File::from(fd);
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err("script must be a regular file of at most 65536 bytes".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BYTES as usize {
            return Err("script exceeds 65536 bytes".into());
        }
        let script: Script =
            serde_json::from_slice(&bytes).map_err(|error| format!("invalid script: {error}"))?;
        if script.schema != 1 || script.steps.is_empty() || script.steps.len() > 256 {
            return Err("script requires schema 1 and 1..=256 steps".into());
        }
        let mut ticks = 0u32;
        for step in &script.steps {
            if step.ticks > 6000
                || !(-1..=1).contains(&step.move_x)
                || !(-1..=1).contains(&step.move_z)
            {
                return Err("step requires ticks 0..=6000 and axes -1..=1".into());
            }
            ticks += step.ticks;
            if ticks > 6000 {
                return Err("script exceeds 6000 total ticks".into());
            }
        }
        Ok((script, hash(&bytes)))
    }
    pub(super) fn run(
        project: &Path,
        script: &Path,
        json: bool,
        report: &mut serde_json::Value,
    ) -> Result<(), (&'static str, String)> {
        let (script, script_hash) = read_script(script).map_err(|e| ("script", e))?;
        report["script_sha256"] = serde_json::json!(script_hash);
        // This consumer deliberately admits plain Room only, independent of unified features.
        let project = PreparedProject::open(project).map_err(|e| ("project", e))?;
        #[cfg(feature = "room-input-actions")]
        if project.controls().is_some() {
            return Err((
                "unsupported_project",
                "Room playtests do not consume authored physical controls".into(),
            ));
        }
        if project
            .models()
            .document
            .bindings
            .values()
            .any(|binding| binding.kind != orr_model_bindings::model_bindings::ModelKind::Static)
        {
            return Err((
                "unsupported_project",
                "Room playtests require static model bindings".into(),
            ));
        }
        let mut sim = project
            .scene()
            .simulation()
            .map_err(|e| ("simulation", e))?;
        report["scene_sha256"] = serde_json::json!(hash(project.scene().text().as_bytes()));
        report["initial_checksum"] =
            serde_json::json!(format!("0x{:016x}", sim.frame().checksum()));
        if !json {
            println!(
                "script_sha256={script_hash} scene_sha256={} engine_version={}",
                hash(project.scene().text().as_bytes()),
                env!("CARGO_PKG_VERSION")
            );
            println!(
                "room playtest schema=1 initial_checksum=0x{:016x}",
                sim.frame().checksum()
            );
        }
        for (index, step) in script.steps.iter().enumerate() {
            let input = RoomInput {
                move_x: step.move_x,
                move_z: step.move_z,
                reserved: 0,
                buttons: if step.interact { INTERACT } else { 0 },
            };
            for _ in 0..step.ticks {
                let mut inputs = TickInputs::new(sim.tick() + 1, 1);
                inputs.set_input(PlayerSlot(0), input);
                sim.step(&inputs);
            }
            let state = sim.frame().singleton::<RoomRun>();
            let key = state.key_collected != 0;
            let won = state.won != 0;
            let passed = key == step.expect_key && won == step.expect_won;
            report["steps"].as_array_mut().expect("report steps array").push(serde_json::json!({
                "step": index + 1, "tick": sim.tick(), "expected":{"key":step.expect_key,"won":step.expect_won},
                "observed":{"key":key,"won":won}, "checksum":format!("0x{:016x}",sim.frame().checksum()), "passed":passed
            }));
            if key != step.expect_key || won != step.expect_won {
                return Err((
                    "assertion",
                    format!(
                        "step {} tick {}: expected key={} won={}, observed key={} won={}",
                        index + 1,
                        sim.tick(),
                        step.expect_key,
                        step.expect_won,
                        key,
                        won
                    ),
                ));
            }
            if !json {
                println!(
                    "step={} tick={} key={} won={} checksum=0x{:016x}",
                    index + 1,
                    sim.tick(),
                    key,
                    won,
                    sim.frame().checksum()
                );
            }
        }
        report["ticks"] = serde_json::json!(sim.tick());
        if !json {
            println!(
                "room playtest passed steps={} ticks={}",
                script.steps.len(),
                sim.tick()
            );
        }
        Ok(())
    }
}
