//! The sim side of the physics sample: a two-peer loopback session that
//! measures its own cost, and the metrics store shared with the window loop.
//!
//! Timing uses `Instant` on the sim thread, around the sim calls, so no
//! sim crate has to know about the clock.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use orr_bridge::{BridgeConfig, SimHost, StepTiming};
use orr_ecs::Frame;
use orr_fp::FrameRng;
use orr_session::{AdvanceResult, LoopbackClock, LoopbackEnd, LoopbackNetwork, RollbackInfo, Session, SessionConfig};
use orr_sim::{Game, PlayerSlot};

use crate::arena_view::{Loopback, LOCAL_SLOT};
use crate::physics_game::{PhysConfig, PhysGame, PhysInput, TICK_RATE};

/// Count, total and worst of a set of durations, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeStat {
    pub count: u64,
    pub total_ms: f64,
    pub max_ms: f64,
}

impl TimeStat {
    pub fn add(&mut self, d: Duration) {
        let ms = d.as_secs_f64() * 1000.0;
        self.count += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    pub fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ms / self.count as f64
        }
    }
}

/// What the sim thread measured. Read with [`SimMetrics::report`].
#[derive(Clone, Debug, Default)]
pub struct SimReport {
    /// Peer A (the one on screen): a tick with no rollback.
    pub tick: TimeStat,
    /// 99th percentile of `tick`, in milliseconds.
    pub tick_p99_ms: f64,
    /// Peer A: a call that rolled back (resimulation burst plus, usually, the new tick).
    pub rollback: TimeStat,
    /// Ticks resimulated by all bursts together, and in the longest burst.
    pub resim_ticks: u64,
    pub max_resim_ticks: u32,
    /// Peer A: calls that did not simulate because the prediction limit was reached.
    pub stalled: TimeStat,
    /// The bot peer B: every call.
    pub bot: TimeStat,
    /// Snapshot copy and publish, after each step.
    pub publish: TimeStat,
    /// A whole step: peer A, the bot peer and publishing.
    pub step: TimeStat,
    /// Wall time from the first to the last step, in seconds.
    pub span_secs: f64,
    /// Steps that took longer than one tick period.
    pub over_budget: u64,
}

impl SimReport {
    /// Average cost of one resimulated tick inside a rollback burst.
    pub fn resim_tick_avg_ms(&self) -> f64 {
        let ticks = self.resim_ticks + self.rollback.count;
        if ticks == 0 {
            0.0
        } else {
            self.rollback.total_ms / ticks as f64
        }
    }
}

const MAX_SAMPLES: usize = 2_000_000;

#[derive(Default)]
struct Inner {
    report: SimReport,
    tick_samples_ms: Vec<f32>,
    window: TimeStat,
    first_step: Option<Instant>,
    last_step: Option<Instant>,
}

/// Shared between the sim thread (writer) and the window loop (reader).
#[derive(Default)]
pub struct SimMetrics {
    inner: Mutex<Inner>,
}

/// Period of one tick, the budget of a step.
pub fn tick_budget() -> Duration {
    Duration::from_nanos(1_000_000_000 / u64::from(TICK_RATE))
}

impl SimMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record_a(&self, d: Duration, outcome: Outcome) {
        let mut g = self.lock();
        match outcome {
            Outcome::Normal => {
                g.report.tick.add(d);
                g.window.add(d);
                if g.tick_samples_ms.len() < MAX_SAMPLES {
                    g.tick_samples_ms.push((d.as_secs_f64() * 1000.0) as f32);
                }
            }
            Outcome::Rollback(info) => {
                g.report.rollback.add(d);
                g.report.resim_ticks += u64::from(info.resim_count);
                g.report.max_resim_ticks = g.report.max_resim_ticks.max(info.resim_count);
            }
            Outcome::Stalled => g.report.stalled.add(d),
        }
    }

    fn record_bot(&self, d: Duration) {
        self.lock().report.bot.add(d);
    }

    /// Feeds the bridge's step observer (see [`BridgeConfig::with_step_observer`]).
    pub fn record_step(&self, t: StepTiming) {
        let now = Instant::now();
        let mut g = self.lock();
        g.first_step.get_or_insert(now);
        g.last_step = Some(now);
        g.report.publish.add(t.publish);
        let total = t.host_advance + t.publish;
        g.report.step.add(total);
        if total > tick_budget() {
            g.report.over_budget += 1;
        }
    }

    /// The numbers so far.
    pub fn report(&self) -> SimReport {
        let g = self.lock();
        let mut report = g.report.clone();
        let mut sorted = g.tick_samples_ms.clone();
        sorted.sort_by(f32::total_cmp);
        report.tick_p99_ms = percentile(&sorted, 0.99);
        if let (Some(first), Some(last)) = (g.first_step, g.last_step) {
            report.span_secs = (last - first).as_secs_f64();
        }
        report
    }

    /// Average and worst normal tick since the last call (for the window title).
    pub fn take_window(&self) -> (f64, f64) {
        let mut g = self.lock();
        let w = std::mem::take(&mut g.window);
        (w.avg_ms(), w.max_ms)
    }
}

fn percentile(sorted: &[f32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    f64::from(sorted[i])
}

enum Outcome {
    Normal,
    Rollback(RollbackInfo),
    Stalled,
}

type Bot<G> = Box<dyn FnMut(u64) -> (<G as Game>::Input, Vec<<G as Game>::Command>)>;

/// Two peers of one session in one process, like `orr_bridge::LoopbackPair`,
/// but each `advance` is timed per peer. Peer A is the one the view watches.
/// Not `Send` (the loopback network holds `Rc`s): build it on the sim thread.
pub struct TimedPair<G: Game> {
    a: Session<G, LoopbackEnd<G>>,
    b: Session<G, LoopbackEnd<G>>,
    clock: LoopbackClock,
    bot: Bot<G>,
    metrics: Arc<SimMetrics>,
}

impl<G: Game> TimedPair<G> {
    pub fn new(
        make_config: impl Fn() -> G::Config,
        cfg_a: SessionConfig,
        cfg_b: SessionConfig,
        net: Loopback,
        net_seed: u64,
        bot: impl FnMut(u64) -> (G::Input, Vec<G::Command>) + 'static,
        metrics: Arc<SimMetrics>,
    ) -> Self {
        let (end_a, end_b, clock) = LoopbackNetwork::new::<G>(net.latency_ticks, net.jitter_ticks, net_seed);
        let a = Session::<G, _>::new(make_config(), cfg_a, end_a);
        let b = Session::<G, _>::new(make_config(), cfg_b, end_b);
        Self { a, b, clock, bot: Box::new(bot), metrics }
    }

    pub fn peer_a(&self) -> &Session<G, LoopbackEnd<G>> {
        &self.a
    }

    pub fn peer_b(&self) -> &Session<G, LoopbackEnd<G>> {
        &self.b
    }
}

impl<G: Game> SimHost<G> for TimedPair<G> {
    fn tick_rate(&self) -> u32 {
        self.a.config().tick_rate
    }
    fn local_slot(&self) -> PlayerSlot {
        self.a.config().local_slot
    }
    fn player_count(&self) -> u8 {
        self.a.config().player_count
    }
    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G> {
        self.clock.tick();
        let rollbacks_before = self.a.rollback_count();
        let t = Instant::now();
        let result = self.a.advance(input, commands);
        let cost = t.elapsed();
        let outcome = match (self.a.rollback_count() > rollbacks_before, self.a.last_rollback()) {
            (true, Some(info)) => Outcome::Rollback(info),
            _ if matches!(result, AdvanceResult::Stalled { .. }) => Outcome::Stalled,
            _ => Outcome::Normal,
        };
        self.metrics.record_a(cost, outcome);

        let (bot_input, bot_commands) = (self.bot)(self.b.next_send_tick());
        let t = Instant::now();
        let _ = self.b.advance(bot_input, bot_commands);
        self.metrics.record_bot(t.elapsed());
        result
    }
    fn head_tick(&self) -> u64 {
        self.a.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.a.verified_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.a.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        self.a.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.a.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.a.rollback_count()
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        self.a.last_rollback()
    }
}

/// A random-walking bot: picks a new stick direction and spin every 20 to 60
/// ticks and holds the shoot button in bursts. Changes come often enough that
/// the local peer keeps mispredicting it.
pub fn phys_bot(seed: u64) -> impl FnMut(u64) -> (PhysInput, Vec<crate::physics_game::NoCommand>) {
    let mut rng = FrameRng::new(seed);
    let (mut ax, mut ay, mut spin) = (1, 0, 0);
    let mut left = 0u32;
    move |tick| {
        if left == 0 {
            ax = rng.range_i32(-1, 2);
            ay = rng.range_i32(-1, 2);
            spin = rng.range_i32(-1, 2);
            left = 20 + rng.next_u32() % 40;
        }
        left -= 1;
        (PhysInput::new(ax, ay, spin, tick % 120 < 20), Vec::new())
    }
}

/// The local player's input in headless runs: a slow square walk with a turn, and shoots in bursts.
pub fn scripted_local(tick: u64) -> PhysInput {
    let leg = tick / 90;
    let (ax, ay) = match leg % 4 {
        0 => (1, 0),
        1 => (0, -1),
        2 => (-1, 0),
        _ => (0, 1),
    };
    let spin = if (tick / 45) % 2 == 0 { 1 } else { 0 };
    PhysInput::new(ax, ay, spin, tick % 200 < 15)
}

const NET_SEED: u64 = 777;
const SESSION_SEED: u64 = 42;

/// Session settings of the local peer (slot 0) and the bot peer (slot 1).
pub fn session_configs() -> (SessionConfig, SessionConfig) {
    (
        SessionConfig::new(2, PlayerSlot(LOCAL_SLOT), SESSION_SEED, TICK_RATE),
        SessionConfig::new(2, PlayerSlot(1), SESSION_SEED, TICK_RATE),
    )
}

/// The physics scene as a two-peer loopback session with simulated latency.
pub fn physics_pair(scene: PhysConfig, net: Loopback, metrics: Arc<SimMetrics>) -> TimedPair<PhysGame> {
    let (cfg_a, cfg_b) = session_configs();
    TimedPair::new(move || scene, cfg_a, cfg_b, net, NET_SEED, phys_bot(1234), metrics)
}

/// Bridge settings of the physics sample: timing goes to `metrics`.
pub fn physics_bridge_config(metrics: Arc<SimMetrics>) -> BridgeConfig<PhysGame> {
    BridgeConfig::default().with_step_observer(move |t| metrics.record_step(t))
}

