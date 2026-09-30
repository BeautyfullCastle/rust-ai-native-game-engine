//! A real-time loop for a [`RelayClient`] without a window: for bots,
//! automated multi-client runs and tests. Also the file sink for desync
//! dumps.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_proto::Link;
use orr_session::{ClientState, DumpSink, RelayClient};
use orr_sim::Game;

/// Writes each desync dump as a file in a directory (created on demand).
pub struct DirSink {
    dir: PathBuf,
}

impl DirSink {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl DumpSink for DirSink {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>) {
        let path = self.dir.join(name);
        let result = std::fs::create_dir_all(&self.dir).and_then(|()| std::fs::write(&path, &bytes));
        match result {
            Ok(()) => eprintln!("desync dump written: {}", path.display()),
            Err(e) => eprintln!("cannot write desync dump {}: {e}", path.display()),
        }
    }
}

/// Settings of [`drive`].
pub struct DriveOptions {
    /// Play this long after the game started, then stop. `None`: until `stop`.
    pub play_for: Option<Duration>,
    /// Give up when the game has not started after this long.
    pub connect_timeout: Duration,
    pub stop: Option<Arc<AtomicBool>>,
    /// Sleep between updates.
    pub poll: Duration,
    /// Print a status line to stderr this often.
    pub log_every: Option<Duration>,
    pub tag: String,
    /// Send `Leave` when the loop ends.
    pub leave_at_end: bool,
}

impl Default for DriveOptions {
    fn default() -> Self {
        Self {
            play_for: None,
            connect_timeout: Duration::from_secs(30),
            stop: None,
            poll: Duration::from_millis(1),
            log_every: None,
            tag: String::new(),
            leave_at_end: true,
        }
    }
}

/// What a client measured. All counters run from the start of the session.
#[derive(Clone, Debug)]
pub struct ClientReport {
    pub slot: Option<u8>,
    pub state: ClientState,
    /// Seconds from the start of the game to the end of the loop.
    pub play_secs: f64,
    pub rtt_ms: f64,
    pub delay: u32,
    pub delay_changes: u32,
    pub rate_ppm: u32,
    pub rollbacks: u64,
    pub rollbacks_per_s: f64,
    pub resim_ticks: u64,
    pub max_prediction_depth: u64,
    pub stall_episodes: u64,
    pub stalled_ms: u64,
    /// Longest single stall.
    pub max_stall_ms: u64,
    /// Ticks the server repeated this client's input for.
    pub repeats: u64,
    pub overridden: u64,
    pub hard_resyncs: u32,
    pub desyncs: u64,
    pub dumps: u64,
    pub decode_errors: u64,
    pub head_tick: u64,
    pub verified_tick: u64,
    /// Verified `(tick, checksum)` pairs, for comparing clients.
    pub checksums: Vec<(u64, u64)>,
    /// Token to rejoin the same slot.
    pub token: Option<u64>,
}

impl ClientReport {
    /// One line for logs and result tables.
    pub fn summary(&self) -> String {
        format!(
            "slot {} | rtt {:.0} ms | delay {} | rollbacks {} ({:.1}/s) | stall {} eps / {} ms (max {} ms) | repeats {} | rate {:+} ppm | verified {} (head {}) | desyncs {}",
            self.slot.map_or("-".to_string(), |s| s.to_string()),
            self.rtt_ms,
            self.delay,
            self.rollbacks,
            self.rollbacks_per_s,
            self.stall_episodes,
            self.stalled_ms,
            self.max_stall_ms,
            self.repeats,
            i64::from(self.rate_ppm) - 1_000_000,
            self.verified_tick,
            self.head_tick,
            self.desyncs,
        )
    }
}

/// Builds the report of `client` for a run of `play_secs`.
pub fn report<G: Game, L: Link>(client: &RelayClient<G, L>, play_secs: f64) -> ClientReport {
    let (cs, ss) = (client.stats(), client.source_stats());
    let session = client.session();
    let rollbacks = session.map_or(0, |s| s.rollback_count());
    ClientReport {
        slot: client.welcome().map(|w| w.slot),
        state: client.state().clone(),
        play_secs,
        rtt_ms: client.srtt_us() as f64 / 1000.0,
        delay: client.delay(),
        delay_changes: cs.delay_changes,
        rate_ppm: client.rate_ppm(),
        rollbacks,
        rollbacks_per_s: if play_secs > 0.0 { rollbacks as f64 / play_secs } else { 0.0 },
        resim_ticks: cs.resim_ticks,
        max_prediction_depth: cs.max_prediction_depth,
        stall_episodes: cs.stall_episodes,
        stalled_ms: cs.stalled_us / 1000,
        max_stall_ms: cs.max_stall_us / 1000,
        repeats: ss.own_repeated,
        overridden: ss.own_overridden,
        hard_resyncs: cs.hard_resyncs,
        desyncs: cs.desyncs,
        dumps: cs.dumps_written,
        decode_errors: ss.decode_errors,
        head_tick: session.map_or(0, |s| s.head_tick()),
        verified_tick: session.map_or(0, |s| s.verified_tick()),
        checksums: session.map_or_else(Vec::new, |s| s.checksums().to_vec()),
        token: client.token(),
    }
}

/// Runs `client` in real time until `opts.play_for` has passed since the
/// game started, `opts.stop` is set, or the client is disconnected or
/// rejected. `script(slot, tick)` gives the local input and commands of a
/// tick (`slot` is this client's slot, `0` before it is welcomed).
pub fn drive<G: Game, L: Link>(
    client: &mut RelayClient<G, L>,
    script: &mut dyn FnMut(u8, u64) -> (G::Input, Vec<G::Command>),
    opts: &DriveOptions,
) -> ClientReport {
    let start = Instant::now();
    let mut play_start: Option<Instant> = None;
    let mut next_log = opts.log_every.map(|d| start + d);
    loop {
        let slot = client.welcome().map_or(0, |w| w.slot);
        client.update(start.elapsed().as_micros() as u64, &mut |tick| script(slot, tick));
        let state = client.state();
        if play_start.is_none() && matches!(state, ClientState::Playing) {
            play_start = Some(Instant::now());
        }
        let over = match state {
            ClientState::Rejected(_) | ClientState::Disconnected | ClientState::Failed(_) => true,
            _ => {
                opts.stop.as_ref().is_some_and(|s| s.load(Relaxed))
                    || match (play_start, opts.play_for) {
                        (Some(p), Some(d)) => p.elapsed() >= d,
                        (None, _) => start.elapsed() >= opts.connect_timeout,
                        _ => false,
                    }
            }
        };
        if let Some(t) = next_log {
            if Instant::now() >= t {
                let r = report(client, play_start.map_or(0.0, |p| p.elapsed().as_secs_f64()));
                eprintln!("[{}] t={:.0}s {}", opts.tag, start.elapsed().as_secs_f64(), r.summary());
                next_log = opts.log_every.map(|d| t + d);
            }
        }
        if over {
            break;
        }
        std::thread::sleep(opts.poll);
    }
    let secs = play_start.map_or(0.0, |p| p.elapsed().as_secs_f64());
    let rep = report(client, secs);
    if opts.leave_at_end && matches!(client.state(), ClientState::Playing | ClientState::Syncing | ClientState::CatchingUp) {
        client.leave();
        let slot = client.welcome().map_or(0, |w| w.slot);
        // Let the reliable `Leave` go out before the caller drops the link.
        for _ in 0..20 {
            client.update(start.elapsed().as_micros() as u64, &mut |tick| script(slot, tick));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    rep
}
