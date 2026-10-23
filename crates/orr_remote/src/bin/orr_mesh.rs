#![allow(clippy::disallowed_types)]

use orr_fp::FP;
use orr_navigation_runtime::mesh::MeshRuntime;
use orr_remote::{
    mesh_navigation::{
        authoring::create_scene_from_asset, mesh_doc_from_path, spawn_mesh_navigation_scene_host,
        AgentSpec, MeshNavigation, MeshScenePin, YARD_PLAYERS, YARD_TICK_RATE,
    },
    Auth, ServerConfig,
};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

const HELP: &str = "orr_mesh asset --asset ABS.orrmesh\norr_mesh new --asset ABS.orrmesh --output ABS.scene.yaml --start X,Y,Z --goal X,Y,Z [--vertical N] [--distance N]\norr_mesh inspect --scene ABS.scene.yaml\norr_mesh run --scene ABS.scene.yaml [--ticks 300]\norr_mesh serve --scene ABS.scene.yaml --dev-no-auth [--bind 127.0.0.1:7777] [--stop-after-ms N]\nPoint-agent/open-sky navigation only. No physics controller or editor/export completion is implied.";

fn main() -> ExitCode {
    let args = match std::env::args_os()
        .skip(1)
        .take(26)
        .map(|argument| argument.into_string())
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(arguments) => arguments,
        Err(_) => {
            eprintln!("error: arguments must be valid UTF-8");
            return ExitCode::from(2);
        }
    };
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let (command, values) = match parse(&args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match execute(command, &values) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<(&str, BTreeMap<String, String>), String> {
    if args.len() > 24 || args.iter().any(|argument| argument.len() > 4_096) {
        return Err("argument limit exceeded".into());
    }
    let command = args.first().ok_or("expected a command")?.as_str();
    let allowed: &[&str] = match command {
        "asset" => &["--asset"],
        "new" => &[
            "--asset",
            "--output",
            "--start",
            "--goal",
            "--vertical",
            "--distance",
        ],
        "inspect" => &["--scene"],
        "run" => &["--scene", "--ticks"],
        "serve" => &["--scene", "--bind", "--dev-no-auth", "--stop-after-ms"],
        _ => return Err(format!("unknown command '{command}'")),
    };
    let mut values = BTreeMap::new();
    let mut arguments = args[1..].iter();
    while let Some(flag) = arguments.next() {
        if !allowed.contains(&flag.as_str()) || values.contains_key(flag) {
            return Err(format!("unknown or duplicate option '{flag}'"));
        }
        let value = if flag == "--dev-no-auth" {
            "true".to_string()
        } else {
            let value = arguments
                .next()
                .ok_or_else(|| format!("{flag} needs a value"))?;
            if value.starts_with("--") {
                return Err(format!("{flag} needs a value"));
            }
            value.clone()
        };
        values.insert(flag.clone(), value);
    }
    for flag in if command == "new" {
        &["--asset", "--output"][..]
    } else if command == "asset" {
        &["--asset"][..]
    } else {
        &["--scene"][..]
    } {
        absolute_path(&values, flag)?;
    }
    if command == "new" {
        point(required(&values, "--start")?)?;
        point(required(&values, "--goal")?)?;
        fixed(&values, "--vertical", "2")?;
        fixed(&values, "--distance", "0.03125")?;
    }
    if command == "run" {
        ticks(&values)?;
    }
    if command == "serve" {
        if !values.contains_key("--dev-no-auth") {
            return Err(
                "serve requires explicit --dev-no-auth; this tool is local development only".into(),
            );
        }
        bind(&values)?;
        stop_after(&values)?;
    }
    Ok((command, values))
}

fn execute(command: &str, values: &BTreeMap<String, String>) -> Result<(), String> {
    if command == "asset" {
        let graph =
            orr_remote::mesh_navigation::authoring::read_graph(&absolute_path(values, "--asset")?)?;
        let first = graph
            .triangles()
            .first()
            .ok_or("mesh has no walkable triangles")?;
        let face = first.vertices.map(|index| graph.vertices()[index as usize]);
        println!(
            "{}",
            json!({
                "graph_revision": hex(&graph.revision()),
                "vertices": graph.vertices().len(), "triangles": graph.triangles().len(),
                "first_face_points": face.map(|point| point.map(orr_reflect::decimal::fp_to_decimal)),
                "first_face_points_raw": face.map(|point| point.map(|value| value.raw())),
                "max_slope": orr_reflect::decimal::fp_to_decimal(graph.profile().max_slope),
            })
        );
        return Ok(());
    }
    if command == "new" {
        let receipt = create_scene_from_asset(
            &absolute_path(values, "--asset")?,
            &absolute_path(values, "--output")?,
            AgentSpec {
                start: point(required(values, "--start")?)?,
                goal: point(required(values, "--goal")?)?,
                max_vertical: fixed(values, "--vertical", "2")?,
                distance_per_tick: fixed(values, "--distance", "0.03125")?,
            },
        )?;
        println!(
            "{}",
            json!({
                "graph_revision": hex(&receipt.graph_revision),
                "checksum": format!("0x{:016x}", receipt.checksum),
                "graph_bytes": receipt.graph_bytes,
                "scene_bytes": receipt.scene_bytes,
            })
        );
        return Ok(());
    }
    let scene = absolute_path(values, "--scene")?;
    if command == "serve" {
        let mut config = ServerConfig::new(Auth::DevNoAuth);
        config.bind = bind(values)?;
        let host = spawn_mesh_navigation_scene_host(scene, config)?;
        println!(
            "{}",
            json!({"game": "MeshNavigation", "url": host.url(), "auth": "explicit loopback development only"})
        );
        let deadline = stop_after(values)?.map(|duration| Instant::now() + duration);
        while deadline.is_none_or(|deadline| Instant::now() < deadline) {
            if !host.is_running() {
                return Err("mesh host stopped unexpectedly".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(host);
        return Ok(());
    }
    let document = mesh_doc_from_path(&scene)?;
    let pin = *document.frame().singleton::<MeshScenePin>();
    if command == "inspect" {
        println!("{}", readback(document.frame(), pin)?);
        return Ok(());
    }
    let mut simulation = Simulation::<MeshNavigation>::from_frame(
        document.frame(),
        YARD_TICK_RATE,
        orr_remote::mesh_navigation::MESH_NAVIGATION_BUILD_ID,
    )
    .map_err(|error| error.to_string())?;
    orr_remote_game_validate(simulation.frame())?;
    for _ in 0..ticks(values)? {
        simulation.step(&TickInputs::new(simulation.tick() + 1, YARD_PLAYERS));
        orr_remote_game_validate(simulation.frame())?;
    }
    println!("{}", readback(simulation.frame(), pin)?);
    Ok(())
}

fn orr_remote_game_validate(frame: &orr_ecs::Frame) -> Result<(), String> {
    orr_games::mesh_navigation_game::validate_scene_frame(frame)
}

fn readback(frame: &orr_ecs::Frame, pin: MeshScenePin) -> Result<Value, String> {
    orr_remote_game_validate(frame)?;
    let agent = MeshRuntime::agent(frame, pin.binding()).map_err(|error| error.to_string())?;
    Ok(json!({
        "game": "MeshNavigation", "tick": frame.tick(),
        "checksum": format!("0x{:016x}", frame.checksum()),
        "graph_revision": hex(&pin.graph_revision),
        "position_raw": agent.position.map(|value| value.raw()),
        "goal_raw": agent.spec.goal.map(|value| value.raw()),
        "status": format!("{:?}", agent.status),
    }))
}

fn required<'a>(values: &'a BTreeMap<String, String>, flag: &str) -> Result<&'a str, String> {
    values
        .get(flag)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {flag}"))
}

fn absolute_path(values: &BTreeMap<String, String>, flag: &str) -> Result<PathBuf, String> {
    let value = required(values, flag)?;
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(format!("{flag} must be absolute"));
    }
    Ok(path.to_path_buf())
}

fn point(value: &str) -> Result<[FP; 3], String> {
    let coordinates: Vec<_> = value.split(',').collect();
    if coordinates.len() != 3 {
        return Err("point must contain exactly X,Y,Z".into());
    }
    Ok([
        FP::parse(coordinates[0]).map_err(|error| format!("invalid point: {error:?}"))?,
        FP::parse(coordinates[1]).map_err(|error| format!("invalid point: {error:?}"))?,
        FP::parse(coordinates[2]).map_err(|error| format!("invalid point: {error:?}"))?,
    ])
}

fn fixed(values: &BTreeMap<String, String>, flag: &str, default: &str) -> Result<FP, String> {
    FP::parse(values.get(flag).map_or(default, String::as_str))
        .map_err(|error| format!("{flag}: {error:?}"))
}

fn ticks(values: &BTreeMap<String, String>) -> Result<u32, String> {
    let count = values
        .get("--ticks")
        .map_or("300", String::as_str)
        .parse::<u32>()
        .map_err(|_| "--ticks must be a whole number")?;
    if count > 6_000 {
        return Err("--ticks must be at most6000".into());
    }
    Ok(count)
}

fn bind(values: &BTreeMap<String, String>) -> Result<SocketAddr, String> {
    let address = values
        .get("--bind")
        .map_or("127.0.0.1:7777", String::as_str)
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    if !address.ip().is_loopback() {
        return Err("serve only supports a loopback address".into());
    }
    Ok(address)
}

fn stop_after(values: &BTreeMap<String, String>) -> Result<Option<Duration>, String> {
    values
        .get("--stop-after-ms")
        .map(|value| {
            let milliseconds = value
                .parse::<u64>()
                .map_err(|_| "--stop-after-ms must be a whole number")?;
            if !(1..=600_000).contains(&milliseconds) {
                return Err("--stop-after-ms must be1..=600000".into());
            }
            Ok(Duration::from_millis(milliseconds))
        })
        .transpose()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
