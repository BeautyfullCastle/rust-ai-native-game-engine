use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationStatus};
use orr_navigation_runtime::mesh::{AgentSpec, MeshRuntime, WorldBinding};
use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "frame_mesh --output ABSENT_ABSOLUTE_FILE";
const MAX_OUTPUT_PATH_BYTES: usize = 4096;
const PRE_SAVE_TICKS: u64 = 24;
const MAX_REPLAY_TICKS: u32 = 512;
const REPLAY_DISTANCE_PER_TICK: FP = fp!(0.03125);

#[derive(PartialEq, Eq)]
struct ReplayTick {
    checksum: u64,
    frame_bytes: Vec<u8>,
}

struct ReplayResult {
    ticks: Vec<ReplayTick>,
    moving_ticks: u32,
    reached_goal: bool,
}

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).take(4).collect();
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("{USAGE}");
        println!("Bake explicit world geometry, admit a Frame agent, save and replay it deterministically.");
        return ExitCode::SUCCESS;
    }
    let output = match parse_output(arguments) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("frame_mesh: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("frame_mesh: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_output(arguments: Vec<OsString>) -> Result<PathBuf, String> {
    if arguments.len() != 2 || arguments[0] != "--output" {
        return Err(format!("expected --output ABSENT_ABSOLUTE_FILE; {USAGE}"));
    }
    if arguments[1].is_empty() || arguments[1].as_encoded_bytes().len() > MAX_OUTPUT_PATH_BYTES {
        return Err("output path must contain 1..=4096 bytes".into());
    }
    let output = PathBuf::from(&arguments[1]);
    if !output.is_absolute() {
        return Err("output path must be absolute".into());
    }
    Ok(output)
}

fn run(output: &Path) -> Result<(), String> {
    ensure_absent(output)?;

    let vertices = [
        [fp!(0), fp!(0), fp!(0)],
        [fp!(4), fp!(0), fp!(0)],
        [fp!(0), fp!(0), fp!(2)],
        [fp!(4), fp!(0), fp!(2)],
        [fp!(6), fp!(2), fp!(0)],
        [fp!(6), fp!(2), fp!(2)],
        [fp!(2), fp!(2), fp!(0)],
        [fp!(2), fp!(2), fp!(2)],
    ];
    let faces = [
        [0, 2, 3],
        [0, 3, 1],
        [1, 3, 5],
        [1, 5, 4],
        [6, 7, 5],
        [6, 5, 4],
    ];
    let graph = StaticMeshGraph::bake(
        &vertices,
        &faces,
        AgentProfile {
            max_slope: fp!(4),
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .map_err(|error| format!("world-geometry bake failed: {error}"))?;
    let binding = WorldBinding {
        world_revision: graph.revision(),
        graph_revision: graph.revision(),
    };
    let goal = [fp!(3), fp!(2), fp!(1)];
    let spec = AgentSpec {
        start: [fp!(1), fp!(0), fp!(1)],
        goal,
        max_vertical: fp!(2),
        distance_per_tick: REPLAY_DISTANCE_PER_TICK,
    };

    let mut registry_builder = ComponentRegistryBuilder::new();
    MeshRuntime::register(&mut registry_builder);
    let registry = registry_builder.build();
    let mut frame = Frame::new(registry.clone());
    let agent_entity = MeshRuntime::admit(&mut frame, &graph, binding, spec)
        .map_err(|error| format!("Frame admission failed: {error}"))?;
    MeshRuntime::validate(&frame, binding)
        .map_err(|error| format!("admitted Frame validation failed: {error}"))?;
    let admitted = MeshRuntime::agent(&frame, binding)
        .map_err(|error| format!("admitted agent readback failed: {error}"))?;
    if admitted.entity != agent_entity
        || admitted.spec != spec
        || admitted.binding != binding
        || admitted.position != spec.start
        || admitted.status != NavigationStatus::Moving
    {
        return Err("admission readback differs from the requested world agent".into());
    }

    for tick in 1..=PRE_SAVE_TICKS {
        MeshRuntime::step(&mut frame, binding)
            .map_err(|error| format!("pre-save movement tick failed: {error}"))?;
        frame.set_tick(tick);
    }
    let before_stop = MeshRuntime::agent(&frame, binding)
        .map_err(|error| format!("pre-stop agent readback failed: {error}"))?;
    if before_stop.position == spec.start || before_stop.distance <= FP::ZERO {
        return Err("agent did not move before the stop checkpoint".into());
    }
    MeshRuntime::stop(&mut frame, binding)
        .map_err(|error| format!("agent stop failed: {error}"))?;
    let stopped = MeshRuntime::agent(&frame, binding)
        .map_err(|error| format!("stopped agent readback failed: {error}"))?;
    if stopped.status != NavigationStatus::Stopped
        || stopped.position != before_stop.position
        || stopped.distance != before_stop.distance
    {
        return Err("stop failed to retain the current agent position and route distance".into());
    }
    MeshRuntime::step(&mut frame, binding)
        .map_err(|error| format!("stopped agent validation failed: {error}"))?;
    let stopped_after_step = MeshRuntime::agent(&frame, binding)
        .map_err(|error| format!("post-stop agent readback failed: {error}"))?;
    if stopped_after_step.status != NavigationStatus::Stopped
        || stopped_after_step.position != stopped.position
        || stopped_after_step.distance != stopped.distance
    {
        return Err("stopped agent advanced without an explicit replan".into());
    }

    let saved_frame_bytes = frame.to_bytes();
    let in_memory_checkpoint = Frame::from_bytes(registry.clone(), &saved_frame_bytes)
        .map_err(|error| format!("in-memory Frame checkpoint decode failed: {error}"))?;
    MeshRuntime::validate(&in_memory_checkpoint, binding)
        .map_err(|error| format!("in-memory Frame checkpoint validation failed: {error}"))?;
    if in_memory_checkpoint.to_bytes() != saved_frame_bytes
        || in_memory_checkpoint.checksum() != frame.checksum()
    {
        return Err("in-memory Frame checkpoint did not round-trip exactly".into());
    }
    let checkpoint_agent = MeshRuntime::agent(&in_memory_checkpoint, binding)
        .map_err(|error| format!("checkpoint agent readback failed: {error}"))?;
    if checkpoint_agent.entity != agent_entity
        || checkpoint_agent.status != NavigationStatus::Stopped
        || checkpoint_agent.position != stopped.position
        || checkpoint_agent.distance != stopped.distance
    {
        return Err("checkpoint lost the stopped agent's identity or cursor".into());
    }

    let mut stale_binding = binding;
    stale_binding.graph_revision[0] ^= 1;
    let mut stale_probe = in_memory_checkpoint.clone();
    let stale_before = stale_probe.checksum();
    if MeshRuntime::step(&mut stale_probe, stale_binding).is_ok()
        || stale_probe.checksum() != stale_before
    {
        return Err("stale graph binding did not fail atomically".into());
    }

    let mut replay_frame = in_memory_checkpoint.clone();
    let expected_replay = replay_to_goal(&mut replay_frame, &graph, binding, goal)?;
    if !expected_replay.reached_goal || expected_replay.moving_ticks < 100 {
        return Err("saved checkpoint replay did not reach the goal after 100 moving ticks".into());
    }
    replay_frame.copy_from(&in_memory_checkpoint);
    let rollback_replay = replay_to_goal(&mut replay_frame, &graph, binding, goal)?;
    if rollback_replay.ticks != expected_replay.ticks {
        return Err("rollback replay from the saved Frame checkpoint diverged".into());
    }

    let readback_bytes = write_new_and_read_back(output, &saved_frame_bytes)?;
    let mut loaded_frame = Frame::from_bytes(registry.clone(), &readback_bytes)
        .map_err(|error| format!("saved Frame decode failed: {error}"))?;
    MeshRuntime::validate(&loaded_frame, binding)
        .map_err(|error| format!("saved Frame validation failed: {error}"))?;
    if loaded_frame.to_bytes() != saved_frame_bytes
        || loaded_frame.checksum() != in_memory_checkpoint.checksum()
    {
        return Err("saved Frame readback differs from the validated checkpoint".into());
    }
    let loaded_checkpoint_checksum = loaded_frame.checksum();
    MeshRuntime::step(&mut loaded_frame, binding)
        .map_err(|error| format!("saved stopped Frame step failed: {error}"))?;
    if loaded_frame.checksum() != loaded_checkpoint_checksum {
        return Err("saved stopped Frame advanced without an explicit replan".into());
    }
    let loaded_agent = MeshRuntime::agent(&loaded_frame, binding)
        .map_err(|error| format!("saved agent readback failed: {error}"))?;
    if loaded_agent.entity != agent_entity
        || loaded_agent.binding != binding
        || loaded_agent.status != NavigationStatus::Stopped
        || loaded_agent.position != stopped.position
        || loaded_agent.distance != stopped.distance
    {
        return Err("saved Frame changed the stopped agent or route cursor".into());
    }
    let mut loaded_replay = loaded_frame.clone();
    let disk_replay = replay_to_goal(&mut loaded_replay, &graph, binding, goal)?;
    if disk_replay.ticks != expected_replay.ticks
        || disk_replay.moving_ticks != expected_replay.moving_ticks
        || !disk_replay.reached_goal
    {
        return Err("disk-reloaded Frame replay diverged from rollback replay".into());
    }
    if loaded_frame.checksum() != in_memory_checkpoint.checksum() {
        return Err("replay unexpectedly mutated the saved checkpoint Frame".into());
    }

    let final_replay = replay_to_goal(&mut loaded_frame, &graph, binding, goal)?;
    let final_agent = MeshRuntime::agent(&loaded_frame, binding)
        .map_err(|error| format!("final agent readback failed: {error}"))?;
    let final_frame_bytes = loaded_frame.to_bytes();
    if !final_replay.reached_goal
        || final_agent.status != NavigationStatus::Arrived
        || expected_replay.ticks.last().map(|tick| tick.checksum) != Some(loaded_frame.checksum())
        || expected_replay
            .ticks
            .last()
            .map(|tick| tick.frame_bytes.as_slice())
            != Some(final_frame_bytes.as_slice())
    {
        return Err("final replay diverged or did not arrive at the requested goal".into());
    }
    println!("frame_asset: {}", output.display());
    println!(
        "world_revision: {}",
        format_revision(binding.world_revision)
    );
    println!(
        "graph_revision: {}",
        format_revision(binding.graph_revision)
    );
    println!("mesh_triangles: {}", graph.triangles().len());
    println!("pre_save_ticks: {PRE_SAVE_TICKS}");
    println!("replay_ticks: {}", expected_replay.ticks.len());
    println!("moving_ticks: {}", expected_replay.moving_ticks);
    println!("goal: {}", format_position(goal));
    println!("reached_goal: {}", final_agent.position == goal);
    println!("rollback_replay_identical: true");
    println!("frame_checksum: {:016x}", loaded_frame.checksum());
    println!("Frame checksum is a deterministic state checksum, not authentication");
    println!("world binding uses the supplied mesh revision only; it proves no collider or physics identity");
    println!("Frame-owned CPU navigation only; no collider, physics, editor or source-hidden export claim");
    Ok(())
}

fn replay_to_goal(
    frame: &mut Frame,
    graph: &StaticMeshGraph,
    binding: WorldBinding,
    goal: [FP; 3],
) -> Result<ReplayResult, String> {
    MeshRuntime::replan(
        frame,
        graph,
        binding,
        goal,
        fp!(2),
        REPLAY_DISTANCE_PER_TICK,
    )
    .map_err(|error| format!("explicit route replan failed: {error}"))?;
    let mut checksums = Vec::new();
    for completed_ticks in 0..MAX_REPLAY_TICKS {
        let before = MeshRuntime::agent(frame, binding)
            .map_err(|error| format!("replay agent readback failed: {error}"))?;
        if before.status == NavigationStatus::Arrived {
            return Ok(ReplayResult {
                ticks: checksums,
                moving_ticks: completed_ticks,
                reached_goal: before.position == goal,
            });
        }
        if before.status != NavigationStatus::Moving {
            return Err("explicit replan did not resume the stopped agent".into());
        }
        MeshRuntime::step(frame, binding)
            .map_err(|error| format!("replay movement tick failed: {error}"))?;
        frame.set_tick(frame.tick().saturating_add(1));
        let after = MeshRuntime::agent(frame, binding)
            .map_err(|error| format!("replay post-tick readback failed: {error}"))?;
        if after.entity != before.entity || after.spec.start != before.spec.start {
            return Err(
                "Frame replay changed agent identity or original start specification".into(),
            );
        }
        checksums.push(ReplayTick {
            checksum: frame.checksum(),
            frame_bytes: frame.to_bytes(),
        });
        if after.status == NavigationStatus::Arrived {
            return Ok(ReplayResult {
                ticks: checksums,
                moving_ticks: completed_ticks + 1,
                reached_goal: after.position == goal,
            });
        }
    }
    Err("route exceeded the bounded replay tick limit".into())
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("output already exists: {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect output path: {error}")),
    }
}

fn write_new_and_read_back(path: &Path, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create output without replacement: {error}"))?;
    if let Err(error) = file.write_all(bytes) {
        return Err(format!(
            "could not finish writing {}; a partial new Frame may remain: {error}",
            path.display()
        ));
    }
    if let Err(error) = file.sync_all() {
        return Err(format!(
            "could not sync {}; the new Frame may remain: {error}",
            path.display()
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("could not rewind saved Frame: {error}"))?;
    let mut saved = Vec::with_capacity(bytes.len());
    file.take(bytes.len() as u64 + 1)
        .read_to_end(&mut saved)
        .map_err(|error| format!("could not read back saved Frame: {error}"))?;
    if saved.len() != bytes.len() || saved != bytes {
        return Err(format!(
            "saved Frame did not match its validated bounded bytes: {}",
            path.display()
        ));
    }
    Ok(saved)
}

fn format_revision(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn format_position(position: [FP; 3]) -> String {
    format!("[{},{},{}]", position[0], position[1], position[2])
}
