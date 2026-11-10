//! The concrete user-game runtime. `--trace` is a bounded CPU-only test runner.
use std::path::PathBuf;

use orr_session::{PlaySession, PlayerSlot};
use orr_static_starter::game_contract::Starter;
use serde_json::{Value, json};

use orr_collect_delivery_game::game::{
    CollectDeliveryGame, CollectDeliveryState, CollectInput, Courier,
};

const MAX_TRACE_BYTES: usize = 4 << 20;
const MAX_TRACE_TICKS: usize = 36_000;

fn trace_args(args: &[String]) -> Result<(PathBuf, PathBuf), String> {
    let mut scene = None;
    let mut trace = None;
    let mut index = 0;
    while index < args.len() {
        let key = args[index].as_str();
        let value = args.get(index + 1).ok_or("missing trace option value")?;
        let slot = match key {
            "--scene" => &mut scene,
            "--trace" => &mut trace,
            _ => return Err("trace mode accepts only --scene and --trace".into()),
        };
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(format!("duplicate {key}"));
        }
        index += 2;
    }
    let scene = scene.ok_or("trace mode requires --scene PATH")?;
    let trace = trace.ok_or("trace mode requires --trace PATH")?;
    Ok((scene, trace))
}

fn parse_input(value: &Value) -> Result<CollectInput, String> {
    let object = value
        .as_object()
        .ok_or("each trace tick must be an object")?;
    if object.len() != 3
        || !["dx", "dy", "action"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return Err("trace input requires exactly dx, dy and action".into());
    }
    let dx = value["dx"]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or("trace dx must be an integer")?;
    let dy = value["dy"]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or("trace dy must be an integer")?;
    let action = value["action"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("trace action must be an unsigned integer")?;
    let input = CollectInput { dx, dy, action };
    if !<CollectDeliveryGame as orr_static_starter::game_contract::Starter>::valid_input(&input) {
        return Err("trace input is outside the compiled game contract".into());
    }
    Ok(input)
}

fn frame_state(session: &PlaySession<CollectDeliveryGame>) -> Value {
    let frame = session.frame();
    let state = frame.singleton::<CollectDeliveryState>();
    let courier = frame.dense::<Courier>().1[0];
    let pair = |point: orr_fp::FPVec2| [point.x.raw(), point.y.raw()];
    json!({
        "position": pair(courier.position),
        "start": pair(state.start),
        "pickup": pair(state.pickup),
        "destination": pair(state.destination),
        "hazard": pair(state.hazard),
        "item_available": state.item_available,
        "carrying": state.carrying,
        "won": state.won,
        "lost": state.lost,
        "reserved": state.reserved,
        "action_held": state.action_held
    })
}

fn run_trace(scene_path: PathBuf, trace_path: PathBuf) -> Result<(), String> {
    let scene = orr_static_starter::game_contract::read_scene(&scene_path)?;
    let bytes = orr_static_starter::game_contract::read_regular(&trace_path, MAX_TRACE_BYTES)?;
    let inputs: Vec<Value> = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if inputs.len() > MAX_TRACE_TICKS {
        return Err("trace exceeds 36000 ticks".into());
    }
    let mut session = orr_static_starter::game_contract::player::<CollectDeliveryGame>(&scene)?;
    for value in &inputs {
        let input = parse_input(value)?;
        session.set_input_without_commands(PlayerSlot(0), input);
        session
            .step_now()
            .ok_or("game session stopped before the trace ended")?;
    }
    println!(
        "{}",
        serde_json::to_string(&json!({
            "game": CollectDeliveryGame::ID,
            "tick": session.head_tick(),
            "checksum": session.frame().checksum(),
            "state": frame_state(&session)
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--trace") {
        let (scene, trace) = trace_args(&args)?;
        run_trace(scene, trace)
    } else {
        orr_static_starter::standalone::main_for::<CollectDeliveryGame>()
    }
}
