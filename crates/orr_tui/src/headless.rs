//! `--headless`: no terminal UI. Plays a scripted scenario through the stream, renders every
//! frame to text and prints a checksum line that is computed exactly like the C client of
//! `orr_ffi` (`tests/c/view_client.c`) does, so the same scenario can be compared across views.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use orr_viewstream::{ViewFrame, HEADER_LEN, RECORD_LEN};

use crate::input::{encode, scenario};
use crate::render::render;
use crate::schema::ViewSchema;
use crate::source::{Control, Incoming, Source};
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
