use crate::game::{self, AssetFixtureGame, Input};
use crate::{
    sha256, CueEvent, Error, FixtureState, PreparedFixture, Result, BUILD_ID, GAME_ID,
    MAX_INPUT_BYTES, PLAYER_COUNT, SEED, TICKS, TICK_RATE,
};
use orr_session::{ReplayHeader, ReplayReader, ReplayWriter, VerifyReport};

/// Source-controlled whole compressed ORRP SHA. Updating it requires explicit
/// regeneration/review; no runtime caller can add a digest to this closed set.
pub const EXPECTED_REPLAY_SHA256: [u8; 32] = [
    62, 170, 130, 116, 69, 128, 232, 224, 128, 177, 187, 212, 139, 251, 192, 42, 41, 9, 110, 252,
    78, 148, 169, 159, 213, 61, 104, 112, 106, 68, 130, 11,
];
const ALLOWED_REPLAYS: &[[u8; 32]] = &[EXPECTED_REPLAY_SHA256];
const KEYS_DESCENDING: [u64; 5] = [300, 240, 180, 120, 60];

/// Scalar snapshots/events and a fixed generated replay; never mutable sim state.
#[derive(Debug)]
pub struct Recording {
    pub states: Vec<FixtureState>,
    pub events: Vec<CueEvent>,
    pub replay: Vec<u8>,
}

pub(crate) fn header() -> ReplayHeader {
    ReplayHeader {
        format_version: 3,
        game_id: GAME_ID.into(),
        build_hash: orr_sim::build_hash_of(BUILD_ID, 0),
        seed: SEED,
        player_count: PLAYER_COUNT,
        tick_rate: TICK_RATE,
        input_size: 4,
    }
}

impl PreparedFixture {
    /// Generate exactly the approved 300-tick schedule. There is deliberately no
    /// input/command/debug parameter and no automatic allowlist update.
    pub fn record(&self) -> Result<Recording> {
        let mut run = self.start();
        let mut writer = ReplayWriter::<AssetFixtureGame>::new(header()).with_keyframe_interval(60);
        let mut states = Vec::with_capacity(301);
        let mut events = Vec::with_capacity(2);
        states.push(run.state());
        for tick in 1..=TICKS {
            let step = run.advance()?;
            writer.record_tick(
                tick,
                &[Input {
                    axis: game::axis(tick),
                }],
                &[],
            );
            writer.record_checksum(tick, step.state.checksum);
            writer.maybe_record_keyframe(run.sim.frame());
            states.push(step.state);
            events.extend(step.events);
        }
        Ok(Recording {
            states,
            events,
            replay: writer.finish(),
        })
    }

    /// Admit only the checked-in trusted fixture, preflight every restore, then
    /// require complete replay verification. This is not arbitrary ORRP ingestion.
    pub fn verify(&self, bytes: &[u8]) -> Result<VerifyReport> {
        self.admit_replay(bytes)?;
        let report = orr_session::replay_verify_checked::<AssetFixtureGame>(bytes, (), BUILD_ID)?;
        validate_report(&report)?;
        Ok(report)
    }

    /// Read-only playback. The generic helper returns a zero-build-ID Simulation;
    /// that value never escapes this wrapper and cannot create a branch recording.
    pub fn seek(&self, bytes: &[u8], tick: u64) -> Result<FixtureState> {
        if tick > TICKS {
            return Err(Error::TickOutOfRange(tick));
        }
        self.verify(bytes)?;
        let reader = ReplayReader::<AssetFixtureGame>::parse(bytes)?;
        let sim = orr_session::replay_seek_checked::<AssetFixtureGame>(bytes, (), BUILD_ID, tick)?;
        let expected = if tick == 0 {
            self.initial_checksum
        } else {
            reader.checksums[(tick - 1) as usize].1
        };
        validate_seek_result(sim.frame(), tick, expected)
    }

    fn admit_replay(&self, bytes: &[u8]) -> Result<ReplayReader<AssetFixtureGame>> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(Error::BudgetExceeded);
        }
        if !ALLOWED_REPLAYS.contains(&sha256(bytes)) {
            return Err(Error::UntrackedReplay);
        }
        let reader = ReplayReader::parse(bytes)?;
        validate_reader(&reader, BUILD_ID)?;
        Ok(reader)
    }
}

fn validate_report(report: &VerifyReport) -> Result<()> {
    if report.ticks_simulated != TICKS || report.checksums_checked != TICKS as u32 || !report.ok() {
        return Err(Error::Verification);
    }
    Ok(())
}

fn validate_seek_result(
    frame: &orr_ecs::Frame,
    tick: u64,
    expected_checksum: u64,
) -> Result<FixtureState> {
    game::validate_frame(frame, tick)?;
    if frame.checksum() != expected_checksum {
        return Err(Error::Checksum(tick));
    }
    game::state(frame)
}

// Private validators are tested independently, without publishing an allowlist
// bypass. They intentionally rely on the closed digest gate for parser-normalized
// duplicates and encoded debug commands outside 0..=300.
pub(crate) fn validate_reader(
    reader: &ReplayReader<AssetFixtureGame>,
    expected_id: u64,
) -> Result<()> {
    validate_header(&reader.header, expected_id)?;
    validate_ticks(reader)?;
    validate_debug(reader)?;
    validate_checksums(reader)?;
    validate_keyframes(reader)?;
    Ok(())
}

fn validate_header(header: &ReplayHeader, expected_id: u64) -> Result<()> {
    let hash = orr_sim::build_hash_of(expected_id, 0);
    if expected_id == 0 || hash == 0 || header.build_hash == 0 || header.build_hash != hash {
        return Err(Error::Header("untracked or mismatched build"));
    }
    if header.format_version != 3 || header.game_id != GAME_ID {
        return Err(Error::Header("format or game"));
    }
    if header.seed != SEED
        || header.player_count != PLAYER_COUNT
        || header.tick_rate != TICK_RATE
        || header.input_size != 4
    {
        return Err(Error::Header("seed, players, tick rate or input size"));
    }
    Ok(())
}

fn validate_ticks(reader: &ReplayReader<AssetFixtureGame>) -> Result<()> {
    if reader.first_tick() != 1
        || reader.last_tick() != TICKS
        || reader.tick_count() != TICKS as usize
    {
        return Err(Error::Ticks);
    }
    for tick in 1..=TICKS {
        let (inputs, commands) = reader.tick(tick).ok_or(Error::Ticks)?;
        if inputs.as_slice()
            != [Input {
                axis: game::axis(tick),
            }]
            || !commands.is_empty()
        {
            return Err(Error::Input(tick));
        }
    }
    Ok(())
}

fn validate_debug(reader: &ReplayReader<AssetFixtureGame>) -> Result<()> {
    for tick in 0..=TICKS {
        if !reader.debug_commands(tick).is_empty() {
            return Err(Error::DebugCommand(tick));
        }
    }
    Ok(())
}

fn validate_checksums(reader: &ReplayReader<AssetFixtureGame>) -> Result<()> {
    if reader.checksums.len() != TICKS as usize
        || reader
            .checksums
            .iter()
            .enumerate()
            .any(|(i, &(tick, _))| tick != i as u64 + 1)
    {
        return Err(Error::Checksums);
    }
    Ok(())
}

fn validate_keyframes(reader: &ReplayReader<AssetFixtureGame>) -> Result<()> {
    let mut keys = Vec::new();
    let mut next = reader.nearest_keyframe(reader.last_tick());
    while let Some(key) = next {
        if keys.len() == 64 {
            return Err(Error::Keyframes);
        }
        keys.push(key);
        next = if key == 0 {
            None
        } else {
            reader.nearest_keyframe(key - 1)
        };
    }
    if keys.len() != reader.keyframe_count() || keys != KEYS_DESCENDING {
        return Err(Error::Keyframes);
    }
    for key in keys {
        // Exact-key lookup guarantees zero ticks of resimulation in the current
        // public helper. Validate every reference before any final verify/seek.
        let sim = reader.seek((), key)?;
        game::validate_frame(sim.frame(), key)?;
        if sim.tick() != key || sim.checksum() != reader.checksums[(key - 1) as usize].1 {
            return Err(Error::Checksum(key));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
