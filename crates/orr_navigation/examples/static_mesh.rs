use orr_fp::{fp, FP};
use orr_navigation::{
    mesh::{MeshPath, StaticMeshGraph},
    AgentProfile, SearchBudget, MAX_FILE_BYTES,
};
use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "static_mesh --output ABSENT_FILE";
const MAX_OUTPUT_PATH_BYTES: usize = 4096;
const MOVEMENT_TICKS: u32 = 160;
const MOVEMENT_STEP: FP = fp!(0.0625);
const MIN_MOVING_TICKS: u32 = 100;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).take(4).collect();
    if arguments.len() == 1 && arguments[0] == "--help" {
        println!("{USAGE}");
        println!(
            "Bake, save, reload and follow a fixed-point route over a 3D ramp and raised bridge."
        );
        return ExitCode::SUCCESS;
    }
    let output = match parse_output(arguments) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("static_mesh: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("static_mesh: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_output(arguments: Vec<OsString>) -> Result<PathBuf, String> {
    if arguments.len() != 2 || arguments[0] != "--output" {
        return Err(format!("expected --output ABSENT_FILE; {USAGE}"));
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
    let triangles = [
        [0, 2, 3],
        [0, 3, 1],
        [1, 3, 5],
        [1, 5, 4],
        [6, 7, 5],
        [6, 5, 4],
    ];
    let profile = AgentProfile {
        max_slope: fp!(4),
        radius: FP::ZERO,
        headroom: FP::ZERO,
        max_step: FP::ZERO,
    };
    let baked = StaticMeshGraph::bake(&vertices, &triangles, profile)
        .map_err(|error| format!("mesh bake failed: {error}"))?;
    let bytes = baked.to_bytes();
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "serialized mesh exceeds the {MAX_FILE_BYTES}-byte asset limit"
        ));
    }
    let start = [fp!(1), fp!(0), fp!(1)];
    let goal = [fp!(3), fp!(2), fp!(1)];
    let max_vertical = fp!(2);
    let budget = SearchBudget::default();
    let baked_path = baked
        .find_path(start, goal, max_vertical, budget)
        .map_err(|error| format!("baked graph route failed: {error}"))?;
    let route_length = baked
        .path_length(&baked_path)
        .map_err(|error| format!("baked path length failed: {error}"))?;
    let moving_distance = MOVEMENT_STEP * FP::from_int(MIN_MOVING_TICKS as i32);
    if route_length <= moving_distance {
        return Err("demo route is too short to exercise 100 moving ticks".into());
    }
    if !baked_path
        .points
        .windows(2)
        .any(|segment| segment[0][1] != segment[1][1])
    {
        return Err("planned route did not traverse a vertical ramp".into());
    }

    let (trace, moving_ticks) = follow_trace(&baked, &baked_path, route_length)?;
    if moving_ticks < MIN_MOVING_TICKS {
        return Err("movement trace did not exercise 100 moving ticks".into());
    }
    if trace.last().copied() != Some(goal) {
        return Err("movement trace did not reach the requested goal".into());
    }

    let memory_roundtrip = StaticMeshGraph::from_bytes(&bytes)
        .map_err(|error| format!("in-memory mesh reload failed: {error}"))?;
    let memory_path = memory_roundtrip
        .find_path(start, goal, max_vertical, budget)
        .map_err(|error| format!("in-memory graph route failed: {error}"))?;
    if memory_roundtrip.revision() != baked.revision()
        || memory_roundtrip.profile() != baked.profile()
        || memory_roundtrip.vertices() != baked.vertices()
        || memory_roundtrip.triangles() != baked.triangles()
        || memory_path.corridor != baked_path.corridor
        || memory_path.points != baked_path.points
        || memory_roundtrip.path_length(&memory_path) != Ok(route_length)
    {
        return Err("in-memory mesh reload differs from the baked graph".into());
    }
    if follow_trace(&memory_roundtrip, &memory_path, route_length)?.0 != trace {
        return Err("in-memory fixed-point movement trace diverged".into());
    }

    let saved_bytes = write_new_and_read_back(output, &bytes)?;
    let loaded = StaticMeshGraph::from_bytes(&saved_bytes)
        .map_err(|error| format!("saved mesh reload failed: {error}"))?;
    let loaded_path = loaded
        .find_path(start, goal, max_vertical, budget)
        .map_err(|error| format!("saved graph route failed: {error}"))?;
    if loaded.revision() != baked.revision()
        || loaded.profile() != baked.profile()
        || loaded.vertices() != baked.vertices()
        || loaded.triangles() != baked.triangles()
        || loaded_path.corridor != baked_path.corridor
        || loaded_path.points != baked_path.points
        || loaded.path_length(&loaded_path) != Ok(route_length)
    {
        return Err("saved mesh reload differs from the baked graph".into());
    }
    if follow_trace(&loaded, &loaded_path, route_length)?.0 != trace {
        return Err("saved fixed-point movement trace diverged".into());
    }

    println!("asset: {}", output.display());
    println!("graph_revision: {}", format_revision(loaded.revision()));
    println!("graph_triangles: {}", loaded.triangles().len());
    println!("path_triangles: {}", loaded_path.corridor.len());
    println!("path_length: {route_length}");
    println!("trace_ticks: {MOVEMENT_TICKS}");
    println!("moving_ticks: {moving_ticks}");
    println!("replay_identical: true");
    println!("reached_goal: true");
    println!("goal: {}", format_position(goal));
    println!("navigation-only demo; no GPU rendering or real-physics claim");
    Ok(())
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
            "could not finish writing {}; a partial new asset may remain: {error}",
            path.display()
        ));
    }
    if let Err(error) = file.sync_all() {
        return Err(format!(
            "could not sync {}; the new asset may remain: {error}",
            path.display()
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("could not rewind saved asset: {error}"))?;
    let mut saved = Vec::with_capacity(bytes.len());
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut saved)
        .map_err(|error| format!("could not read back saved asset: {error}"))?;
    if saved.len() > MAX_FILE_BYTES || saved != bytes {
        return Err(format!(
            "saved asset did not match the bounded serialized bytes: {}",
            path.display()
        ));
    }
    Ok(saved)
}

fn follow_trace(
    graph: &StaticMeshGraph,
    path: &MeshPath,
    path_length: FP,
) -> Result<(Vec<[FP; 3]>, u32), String> {
    let mut distance = FP::ZERO;
    let mut moving_ticks = 0;
    let mut trace = Vec::with_capacity(MOVEMENT_TICKS as usize);
    for _tick in 0..MOVEMENT_TICKS {
        if distance < path_length {
            distance = (distance + MOVEMENT_STEP).min(path_length);
            moving_ticks += 1;
        }
        trace.push(
            graph
                .point_at_distance(path, distance)
                .map_err(|error| format!("fixed-point movement failed: {error}"))?,
        );
    }
    Ok((trace, moving_ticks))
}

fn format_revision(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn format_position(position: [FP; 3]) -> String {
    format!("[{},{},{}]", position[0], position[1], position[2])
}
