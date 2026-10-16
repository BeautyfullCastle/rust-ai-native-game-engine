//! `--headless`: no terminal UI. Plays a scripted scenario through the stream, renders every
//! frame to text and prints a checksum line that is computed exactly like the C client of
//! `orr_ffi` (`tests/c/view_client.c`) does, so the same scenario can be compared across views.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use orr_viewstream::{ViewFrame, HEADER_LEN, RECORD_LEN};

use crate::input::{encode, scenario, Controls};
use crate::render::render;
use crate::schema::ViewSchema;
use crate::source::{Control, Incoming, NetState, Source};
use crate::state::ViewState;

pub struct HeadlessOpts {
    /// Ticks to play (one step and one frame each).
    pub frames: u32,
    /// Grid size of the dump.
    pub size: (usize, usize),
    pub dump: Option<PathBuf>,
}

/// The last line of a headless run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub entities: u32,
    pub frames: u32,
    pub fnv: u64,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RESULT entities={} frames={} fnv=0x{:016x}", self.entities, self.frames, self.fnv)
    }
}

const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

pub fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

const WAIT: Duration = Duration::from_secs(30);

/// Waits for a frame of at least `tick` (events that come meanwhile are counted).
fn wait_frame(src: &mut dyn Source, state: &mut ViewState, tick: u64) -> Result<Vec<u8>, String> {
    let end = Instant::now() + WAIT;
    while Instant::now() < end {
        match src.recv(Duration::from_millis(50))? {
            Some(Incoming::Error(e)) => return Err(e),
            Some(msg) => {
                state.ingest(&msg, Instant::now());
                if let Incoming::Frame(bytes) = msg {
                    if state.frame.as_ref().is_some_and(|f| f.tick >= tick) {
                        return Ok(bytes);
                    }
                }
            }
            None => {}
        }
    }
    Err(format!("timed out waiting for the frame of tick {tick}"))
}

/// Plays `opts.frames` ticks: every tick sets the input of every player from the schema layout
/// (the scenario of the C client), steps once, reads the frame, hashes its tick and entity
/// records, and appends the rendered grid to the dump.
pub fn run(src: &mut dyn Source, opts: &HeadlessOpts) -> Result<Summary, String> {
    let schema = ViewSchema::parse(src.schema_text())?;
    if schema.dimensions == 3 {
        return Err("headless scripted game input is unsupported for 3D; use the interactive XZ observer".into());
    }
    let mut state = ViewState::new(schema.clone(), Instant::now());
    let first = wait_frame(src, &mut state, 0)?;
    let base = ViewFrame::decode(&first).map_err(|e| e.to_string())?.tick;
    let mut dump = String::new();
    let (mut hash, mut entities) = (FNV_BASIS, 0u32);
    for t in 1..=opts.frames {
        for p in 0..schema.player_count {
            src.set_input(p, &encode(&schema, &scenario(u32::from(p), t)))?;
        }
        src.control(Control::Step(1))?;
        let bytes = wait_frame(src, &mut state, base + u64::from(t))?;
        let f = state.frame.as_ref().expect("a frame was ingested");
        entities = f.entities.len() as u32;
        // The same bytes the C client hashes: the tick field, then the entity records.
        hash = fnv(hash, &bytes[8..16]);
        hash = fnv(hash, &bytes[HEADER_LEN..HEADER_LEN + f.entities.len() * RECORD_LEN]);
        if opts.dump.is_some() {
            let cam = state.camera.expect("camera is fitted with the first frame");
            let grid = render(f, &schema, &cam, 1.0, opts.size.0, opts.size.1, false);
            let _ = writeln!(dump, "# frame {t} tick={} verified={} entities={}", f.tick, f.verified_tick, f.entities.len());
            dump.push_str(&grid.text());
            dump.push('\n');
        }
    }
    if let Some(path) = &opts.dump {
        std::fs::write(path, dump).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(Summary { entities, frames: opts.frames, fnv: hash })
}

// ---- client mode: playing on a server through the C ABI ----

/// Options of [`run_client`].
pub struct ClientOpts {
    /// Play at least this many ticks (every tick sets the scripted input of the joined slot).
    pub ticks: u64,
    /// The verified tick whose confirmed checksum is printed (a checkpoint tick: a multiple of
    /// the room's checksum interval, 30 by default).
    pub check_tick: u64,
}

impl Default for ClientOpts {
    fn default() -> Self {
        ClientOpts { ticks: 480, check_tick: 300 }
    }
}

/// The last line of a headless client run. The `checkpoint` and `checksum` parts are the
/// confirmed state at a fixed verified tick: every player of a room prints the same ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientSummary {
    pub slot: u32,
    pub players: u32,
    pub ticks: u64,
    pub rolled_back_frames: u64,
    pub max_depth: u64,
    pub rtt_ms: u32,
    pub predicted: u64,
    pub verified: u64,
    pub canceled: u64,
    pub checkpoint: u64,
    pub checksum: u64,
}

impl std::fmt::Display for ClientSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RESULT client slot={} players={} ticks={} rolled_back_frames={} max_depth={} rtt_ms={} predicted={} verified={} canceled={} {}",
            self.slot,
            self.players,
            self.ticks,
            self.rolled_back_frames,
            self.max_depth,
            self.rtt_ms,
            self.predicted,
            self.verified,
            self.canceled,
            self.agreed_part()
        )
    }
}

impl ClientSummary {
    /// The part every player of the room must agree on.
    pub fn agreed_part(&self) -> String {
        format!("checkpoint={} checksum=0x{:016x}", self.checkpoint, self.checksum)
    }
}

/// Plays a scripted scenario on a server. Every new frame sets the input of the joined slot from
/// the schema layout. Runs until `ticks` were played and the confirmed state at `check_tick` is
/// known (waits for progress, not for a fixed time: it fails when no frame arrives for 30 s).
pub fn run_client(src: &mut dyn Source, opts: &ClientOpts) -> Result<ClientSummary, String> {
    let schema = ViewSchema::parse(src.schema_text())?;
    if schema.dimensions == 3 {
        return Err("headless scripted game input is unsupported for 3D; use the interactive XZ observer".into());
    }
    let mut state = ViewState::new(schema.clone(), Instant::now());
    let mut status = src.net_status().ok_or("this source is not a network client")?;
    let mut last_input_tick = None;
    let mut last_progress = Instant::now();
    let mut last_status = Instant::now();
    loop {
        let now = Instant::now();
        match src.recv(Duration::from_millis(5))? {
            Some(Incoming::Error(e)) => return Err(e),
            Some(msg) => {
                let is_frame = matches!(msg, Incoming::Frame(_));
                state.ingest(&msg, now);
                if is_frame {
                    last_progress = now;
                    if let Some(f) = &state.frame {
                        if last_input_tick != Some(f.tick) {
                            last_input_tick = Some(f.tick);
                            src.set_input(status.slot as u8, &encode(&schema, &scenario_for(status.slot, f.tick)))?;
                        }
                    }
                }
            }
            None => {}
        }
        if last_status.elapsed() >= Duration::from_millis(20) {
            last_status = Instant::now();
            status = src.net_status().ok_or("lost the session status")?;
            state.set_net(status, now);
            if matches!(status.state, NetState::Disconnected | NetState::Failed) {
                return Err(format!("the session ended: {}", status.state.name()));
            }
        }
        if now.duration_since(last_progress) > Duration::from_secs(30) {
            return Err(format!("no frame for 30 s (head tick {}, verified {})", status.head_tick, status.verified_tick));
        }
        let played = state.frame.as_ref().map_or(0, |f| f.tick);
        if played >= opts.ticks && status.verified_tick >= opts.check_tick {
            if let Some(checksum) = src.confirmed_checksum(opts.check_tick) {
                // Events that are still on their way count too: drain what is queued.
                while let Some(msg) = src.recv(Duration::from_millis(1))? {
                    state.ingest(&msg, Instant::now());
                }
                let [predicted, verified, canceled] = state.event_counts;
                return Ok(ClientSummary {
                    slot: status.slot,
                    players: status.players,
                    ticks: played,
                    rolled_back_frames: state.rolled_back_frames,
                    max_depth: state.max_rollback_depth,
                    rtt_ms: status.rtt_ms,
                    predicted,
                    verified,
                    canceled,
                    checkpoint: opts.check_tick,
                    checksum,
                });
            }
        }
    }
}

/// The scripted player of the client run: the scenario of the local headless run, with the joined
/// slot as the player and the tick of the frame, so every client moves differently.
fn scenario_for(slot: u32, tick: u64) -> Controls {
    scenario(slot, u32::try_from(tick).unwrap_or(u32::MAX))
}
