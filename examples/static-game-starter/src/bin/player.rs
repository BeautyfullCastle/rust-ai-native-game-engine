//! Headless source player/replay example, NOT a native playable/export acceptance claim.
use orr_static_starter::{
    delivery::DeliveryGame,
    integration::{player, trace, Starter},
    switchboard::SwitchGame,
};
use std::io::Read;
fn run<G: Starter>(text: &str, trace_bytes: Option<&[u8]>) -> Result<(), String>
where
    G::Input: orr_reflect::Reflect,
{
    let steps = trace_bytes.map(trace::<G>).transpose()?;
    let mut session = player::<G>(text)?;
    if let Some(steps) = steps {
        for (ticks, input) in steps {
            session.set_input(orr_sim::PlayerSlot(0), input);
            session.control(orr_session::ControlOp::Step(ticks));
        }
    } else {
        session.control(orr_session::ControlOp::Step(60));
    }
    println!(
        "{} build={:016x} seed={} tick_rate={} tick={} checksum={:016x} outcome={}",
        G::ID,
        G::BUILD,
        orr_static_starter::SEED,
        orr_static_starter::TICK_RATE,
        session.frame().tick(),
        session.frame().checksum(),
        if G::won(session.frame()) {
            "won"
        } else {
            "running"
        }
    );
    Ok(())
}
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 && args.len() != 3 {
        return Err("usage: player <delivery|switchboard> <scene.yaml> [trace.json]".into());
    }
    let text = std::fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
    let trace_bytes = if let Some(path) = args.get(2) {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Some(bytes)
    } else {
        None
    };
    match args[0].as_str() {
        "delivery" => run::<DeliveryGame>(&text, trace_bytes.as_deref()),
        "switchboard" => run::<SwitchGame>(&text, trace_bytes.as_deref()),
        _ => Err("unknown game".into()),
    }
}
