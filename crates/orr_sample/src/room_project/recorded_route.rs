//! Convert an existing stopped recording, never sampled presentation frames.
use super::{game, PreparedScene, GAME, SEED};
use orr_session::ReplayReader;
use orr_sim::{PlayerSlot, TickInputs};
use serde::Serialize;

pub const MAX_REPLAY_BYTES: usize = 256 * 1024;
const MAX_DECODED_BYTES: u32 = 16 * 1024 * 1024;

#[derive(Serialize)]
struct Script {
    schema: u32,
    steps: Vec<Step>,
}
#[derive(Serialize)]
struct Step {
    ticks: u32,
    move_x: i8,
    move_z: i8,
    interact: bool,
    expect_key: bool,
    expect_won: bool,
}

/// Project eligibility matches the schema-1 playtest consumer.
pub fn from_project(
    project: &super::PreparedProject,
    bytes: &[u8],
    tick: u64,
    checksum: u64,
) -> Result<String, String> {
    #[cfg(feature = "room-input-actions")]
    if project.controls().is_some() {
        return Err("Recorded schema-1 routes require a project without authored controls".into());
    }
    if project
        .models()
        .document
        .bindings
        .values()
        .any(|b| b.kind != orr_model_bindings::model_bindings::ModelKind::Static)
    {
        return Err("Recorded routes require static Room model bindings".into());
    }
    convert(project.scene(), bytes, tick, checksum)
}

/// Validate the whole input-only recording against the admitted initial scene
/// and the actual stop receipt. Returns bounded schema-1 logical-input JSON.
/// This proves simulation identity, not original YAML or physical-key identity.
pub fn convert(
    scene: &PreparedScene,
    bytes: &[u8],
    stopped_tick: u64,
    stopped_checksum: u64,
) -> Result<String, String> {
    // Check the small fixed header and declared decompressed size BEFORE parsing.
    // This supplements, rather than changes, the shared decoder's ratio bound.
    if bytes.len() > MAX_REPLAY_BYTES || bytes.len() < 12 || &bytes[..4] != b"ORRP" {
        return Err("Room recording must be an ORRP file of at most 256 KiB".into());
    }
    let name_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if name_len != GAME.len() {
        return Err("Room recording game identity mismatch".into());
    }
    let compressed_offset = 12 + name_len + 8 + 8 + 1 + 4 + 4;
    let sizes = bytes
        .get(compressed_offset..compressed_offset + 8)
        .ok_or("truncated Room recording header")?;
    let compressed = u32::from_le_bytes(sizes[..4].try_into().unwrap()) as usize;
    let decoded = u32::from_le_bytes(sizes[4..8].try_into().unwrap());
    if compressed < 4
        || compressed != bytes.len().saturating_sub(compressed_offset + 4)
        || decoded > MAX_DECODED_BYTES
    {
        return Err("Room recording size or decompression budget is invalid".into());
    }
    if !(1..=6000).contains(&stopped_tick) {
        return Err("Room route requires 1..=6000 stopped ticks".into());
    }
    let reader = ReplayReader::<game::RoomEscapeV1>::parse(bytes).map_err(|e| e.to_string())?;
    let mut sim = scene.simulation()?;
    let header = &reader.header;
    if header.game_id != GAME
        || header.build_hash != sim.build_hash()
        || header.seed != SEED
        || header.player_count != 1
        || header.tick_rate != game::TICK_RATE
        || header.input_size != std::mem::size_of::<game::RoomInput>() as u32
    {
        return Err("Room recording header does not match this game/build/scene profile".into());
    }
    if reader.first_tick() != 1
        || reader.last_tick() != stopped_tick
        || reader.tick_count() as u64 != stopped_tick
        || reader.checksums.len() as u64 != stopped_tick + 1
        || reader.has_debug_commands()
    {
        return Err(
            "Room recording must be complete, contiguous and free of debug commands".into(),
        );
    }
    if reader.nearest_keyframe(0) != Some(0)
        || reader
            .seek(game::RoomConfig, 0)
            .map_err(|e| e.to_string())?
            .frame()
            .to_bytes()
            != sim.frame().to_bytes()
    {
        return Err("Room recording initial Frame does not match the saved scene".into());
    }
    let mut script = Script {
        schema: 1,
        steps: Vec::new(),
    };
    let mut previous = None;
    for tick in 0..=stopped_tick {
        if tick > 0 {
            let (inputs, commands) = reader
                .tick(tick)
                .ok_or("Room recording has a missing tick")?;
            if inputs.len() != 1 || !commands.is_empty() {
                return Err(
                    "Room route requires one input and no simulation commands per tick".into(),
                );
            }
            let input = inputs[0];
            if input != input.sanitized() {
                return Err("Room recording contains malformed input".into());
            }
            let mut sample = TickInputs::new(tick, 1);
            sample.set_input(PlayerSlot(0), input);
            sim.step(&sample);
            let state = sim.frame().singleton::<game::RoomRun>();
            if previous == Some(input) {
                let last = script.steps.last_mut().expect("previous step");
                last.ticks += 1;
                last.expect_key = state.key_collected != 0;
                last.expect_won = state.won != 0;
            } else {
                if script.steps.len() == 256 {
                    return Err("Room route exceeds 256 input runs; record a shorter route".into());
                }
                script.steps.push(Step {
                    ticks: 1,
                    move_x: input.move_x,
                    move_z: input.move_z,
                    interact: input.buttons != 0,
                    expect_key: state.key_collected != 0,
                    expect_won: state.won != 0,
                });
                previous = Some(input);
            }
        }
        if reader.checksums[tick as usize] != (tick, sim.frame().checksum()) {
            return Err(format!("Room recording checksum mismatch at tick {tick}"));
        }
    }
    if sim.frame().checksum() != stopped_checksum {
        return Err("Room stop receipt checksum mismatch".into());
    }
    let json = serde_json::to_string_pretty(&script).map_err(|e| e.to_string())?;
    if json.len() > 65536 {
        return Err("Room route exceeds script byte limit".into());
    }
    Ok(json)
}

#[cfg(test)]
mod tests;
