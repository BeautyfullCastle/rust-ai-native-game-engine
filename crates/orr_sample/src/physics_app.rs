//! The window loop and the headless benchmark loop of the physics sample,
//! with the metrics both print at the end.

use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeEvent, InProc, PlayerSlot};
use orr_view::{RenderItem, ViewWorld};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::app::Options;
use crate::arena_view::{Loopback, LOCAL_SLOT};
use crate::net_client::{log_lifecycle, relay_title};
use crate::physics_game::{PhysConfig, PhysGame, TICK_RATE};
use crate::physics_host::{physics_bridge_config, physics_pair, scripted_local, SimMetrics, SimReport};
use crate::physics_view::{scene_camera, scene_floor, PhysExtractor, PhysKeys};
use orr_render::orr_rhi::Wgpu;
use orr_render::{extract_items, RenderList, WindowRenderer};

/// Frames at the start that are left out of the render statistics (shader
/// compile, window creation, first snapshots).
const WARMUP_FRAMES: usize = 30;

/// Render-side numbers of a window run.
#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub frames: u64,
    pub seconds: f32,
    pub fps_avg: f32,
    /// Frame rate of the slowest 1 % of frames (1000 / their mean frame time).
    pub fps_1pct_low: f32,
    pub worst_frame_ms: f32,
    /// Mean and worst CPU time of the view update (taking in a snapshot), building render items, and the render call (includes waiting for the display).
    pub view_update_ms: (f32, f32),
    pub items_ms: (f32, f32),
    pub render_ms: (f32, f32),
}

/// Everything a run measured.
#[derive(Clone, Debug, Default)]
pub struct PhysSummary {
    pub adapter: String,
    /// Bodies alive in the newest predicted frame (walls and paddles included).
    pub entities: u32,
    /// `None` for a headless run.
    pub render: Option<RenderStats>,
    /// Wall time the sim ran for.
    pub seconds: f32,
    pub sim_tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
    pub max_rollback_depth: u32,
    pub stalls: u64,
    pub sim: SimReport,
}

impl PhysSummary {
    /// Ticks the sim reached per second of sim wall time (60 = keeping up).
    pub fn tick_rate_achieved(&self) -> f32 {
        let span = if self.sim.span_secs > 0.0 { self.sim.span_secs as f32 } else { self.seconds };
        self.sim_tick as f32 / span.max(0.001)
    }
}

fn mean_max(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    (samples.iter().sum::<f32>() / samples.len() as f32, samples.iter().copied().fold(0.0, f32::max))
}

fn render_stats(frame_ms: &[f32], stages: &Stages, seconds: f32) -> RenderStats {
    let kept = frame_ms.get(WARMUP_FRAMES..).unwrap_or(&[]);
    let mut sorted = kept.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mean = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
    let slow = &sorted[sorted.len() - (sorted.len() / 100).max(1).min(sorted.len())..];
    let skip = |v: &[f32]| mean_max(v.get(WARMUP_FRAMES..).unwrap_or(&[]));
    RenderStats {
        frames: frame_ms.len() as u64,
        seconds,
        fps_avg: if mean(kept) > 0.0 { 1000.0 / mean(kept) } else { 0.0 },
        fps_1pct_low: if mean(slow) > 0.0 { 1000.0 / mean(slow) } else { 0.0 },
        worst_frame_ms: sorted.last().copied().unwrap_or(0.0),
        view_update_ms: skip(&stages.view_update),
        items_ms: skip(&stages.items),
        render_ms: skip(&stages.render),
    }
}

#[derive(Default)]
struct Stages {
    view_update: Vec<f32>,
    items: Vec<f32>,
    render: Vec<f32>,
}

struct Gfx {
    window: Arc<Window>,
    renderer: WindowRenderer<Wgpu>,
}

struct App<B: Bridge<PhysGame>> {
    bridge: B,
    view: ViewWorld<PhysExtractor>,
    opts: Options,
    scene: PhysConfig,
    metrics: Arc<SimMetrics>,
    gfx: Option<Gfx>,
    keys: PhysKeys,
    sent_keys: Option<PhysKeys>,
    items: Vec<RenderItem>,
    list: RenderList,
    started: Instant,
    last_frame: Instant,
    window_frames: u32,
    window_start: Instant,
    frame_ms: Vec<f32>,
    stages: Stages,
    summary: PhysSummary,
    error: Option<String>,
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

impl<B: Bridge<PhysGame>> App<B> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        if self.gfx.is_some() {
            self.frame_ms.push(ms(dt));
        }

        if self.sent_keys != Some(self.keys) {
            self.sent_keys = Some(self.keys);
            let _ = self.bridge.set_input(self.bridge.local_slot(), self.keys.to_input());
        }
        self.bridge.update(dt);
        let snapshot = self.bridge.snapshot();

        let t = Instant::now();
        self.view.update(dt.as_secs_f32().min(0.1), snapshot.as_ref());
        self.stages.view_update.push(ms(t.elapsed()));
        // Game events are not used by this scene, but the queue must not grow.
        for event in self.bridge.drain_events() {
            if let BridgeEvent::Lifecycle(note) = event {
                log_lifecycle(&note);
            }
        }

        let t = Instant::now();
        self.items.clear();
        self.items.push(scene_floor(self.scene.bodies));
        self.view.render_items(&mut self.items);
        self.list.clear();
        extract_items(&self.items, &mut self.list);
        self.stages.items.push(ms(t.elapsed()));

        let t = Instant::now();
        if let Some(gfx) = &mut self.gfx {
            gfx.renderer.render(&self.list, &scene_camera(self.scene.bodies));
        }
        self.stages.render.push(ms(t.elapsed()));

        self.window_frames += 1;
        let window_secs = self.window_start.elapsed().as_secs_f32();
        if window_secs >= 0.5 {
            let fps = self.window_frames as f32 / window_secs;
            let (sim_avg, sim_max) = self.metrics.take_window();
            if let (Some(gfx), Some(s)) = (&self.gfx, &snapshot) {
                let stats = s.stats();
                let net = self.opts.relay.as_ref().map(|m| relay_title(&m.status())).unwrap_or_default();
                gfx.window.set_title(&format!(
                    "Orrery physics [{}] {} bodies | fps {:.0} | sim {:.1}/{:.1} ms | tick {} (verified {}) | rollbacks {} (deepest {}) | stalls {}{net}",
                    self.opts.label,
                    s.predicted().alive_count(),
                    fps,
                    sim_avg,
                    sim_max,
                    s.tick(),
                    s.verified_tick(),
                    stats.rollbacks,
                    stats.max_rollback_depth,
                    stats.stalls
                ));
            }
            self.window_frames = 0;
            self.window_start = Instant::now();
        }

        if let Some(limit) = self.opts.seconds {
            if self.started.elapsed().as_secs_f32() >= limit {
                if let Some(s) = &snapshot {
                    fill_from_snapshot(&mut self.summary, s);
                }
                event_loop.exit();
            }
        }
    }
}

fn fill_from_snapshot(summary: &mut PhysSummary, s: &orr_bridge::Snapshot) {
    summary.sim_tick = s.tick();
    summary.verified_tick = s.verified_tick();
    let stats = s.stats();
    summary.rollbacks = stats.rollbacks;
    summary.max_rollback_depth = stats.max_rollback_depth;
    summary.stalls = stats.stalls;
    summary.entities = s.predicted().alive_count();
}

impl<B: Bridge<PhysGame>> ApplicationHandler for App<B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attributes =
            Window::default_attributes().with_title("Orrery physics").with_inner_size(LogicalSize::new(900.0, 900.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(format!("create_window: {e}"));
                event_loop.exit();
                return;
            }
        };
        match WindowRenderer::new(window.clone(), (window.inner_size().width, window.inner_size().height), self.opts.vsync) {
            Ok(renderer) => {
                self.summary.adapter = renderer.adapter_name();
                println!("adapter: {}", self.summary.adapter);
                self.gfx = Some(Gfx { window, renderer });
                self.started = Instant::now();
                self.last_frame = self.started;
                self.window_start = self.started;
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    match code {
                        KeyCode::KeyA | KeyCode::ArrowLeft => self.keys.left = down,
                        KeyCode::KeyD | KeyCode::ArrowRight => self.keys.right = down,
                        KeyCode::KeyW | KeyCode::ArrowUp => self.keys.up = down,
                        KeyCode::KeyS | KeyCode::ArrowDown => self.keys.down = down,
                        KeyCode::KeyQ => self.keys.spin_left = down,
                        KeyCode::KeyE => self.keys.spin_right = down,
                        KeyCode::Space => self.keys.shoot = down,
                        KeyCode::Escape if down => event_loop.exit(),
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}

/// Opens the window and runs until it closes (or `opts.seconds` pass).
/// `metrics` must be the one the bridge's sim side writes to.
pub fn run_window<B: Bridge<PhysGame>>(
    bridge: B,
    scene: PhysConfig,
    metrics: Arc<SimMetrics>,
    opts: Options,
) -> Result<PhysSummary, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let local_slot = bridge.local_slot().0;
    let mut app = App {
        bridge,
        view: ViewWorld::new(PhysExtractor { remote_mode: opts.remote_mode, local_slot }, opts.view),
        opts,
        scene,
        metrics: metrics.clone(),
        gfx: None,
        keys: PhysKeys::default(),
        sent_keys: None,
        items: Vec::new(),
        list: RenderList::new(),
        started: now,
        last_frame: now,
        window_frames: 0,
        window_start: now,
        frame_ms: Vec::new(),
        stages: Stages::default(),
        summary: PhysSummary::default(),
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| format!("run: {e}"))?;
    if let Some(e) = app.error {
        return Err(e);
    }
    let seconds = app.started.elapsed().as_secs_f32();
    let mut summary = app.summary;
    if summary.sim_tick == 0 {
        if let Some(s) = app.bridge.snapshot() {
            fill_from_snapshot(&mut summary, &s);
        }
    }
    summary.seconds = seconds;
    summary.render = Some(render_stats(&app.frame_ms, &app.stages, seconds));
    summary.sim = metrics.report();
    Ok(summary)
}

/// How long a headless run lasts.
#[derive(Clone, Copy, Debug)]
pub enum HeadlessLimit {
    /// Wall-clock seconds.
    Seconds(f32),
    /// Sim steps (a step that stalls counts too).
    Steps(u64),
}

/// Runs the same session as the window without a window, GPU or clock
/// pacing: one sim step after another as fast as the machine allows,
/// with a scripted local player. The view side is not run.
pub fn run_headless(scene: PhysConfig, net: Loopback, limit: HeadlessLimit) -> PhysSummary {
    let metrics = SimMetrics::new();
    let mut bridge = InProc::new(physics_pair(scene, net, metrics.clone()), physics_bridge_config(metrics.clone()));
    let started = Instant::now();
    let mut step: u64 = 0;
    loop {
        let done = match limit {
            HeadlessLimit::Seconds(s) => started.elapsed().as_secs_f32() >= s,
            HeadlessLimit::Steps(n) => step >= n,
        };
        if done {
            break;
        }
        let _ = bridge.set_input(PlayerSlot(LOCAL_SLOT), scripted_local(step));
        bridge.step(1);
        step += 1;
    }
    let mut summary = PhysSummary { adapter: "headless".to_string(), seconds: started.elapsed().as_secs_f32(), ..Default::default() };
    if let Some(s) = bridge.snapshot() {
        fill_from_snapshot(&mut summary, &s);
    }
    summary.sim = metrics.report();
    summary
}

/// Prints a summary in the format the sample documents.
pub fn print_summary(s: &PhysSummary) {
    println!("adapter        : {} ({} entities alive)", s.adapter, s.entities);
    if let Some(r) = &s.render {
        println!(
            "render         : {} frames in {:.2} s = {:.1} fps avg, {:.1} fps 1% low, worst frame {:.1} ms",
            r.frames, r.seconds, r.fps_avg, r.fps_1pct_low, r.worst_frame_ms
        );
        println!(
            "render cpu     : view update {:.2}/{:.2} ms, items {:.2}/{:.2} ms, render call {:.2}/{:.2} ms (mean/worst)",
            r.view_update_ms.0, r.view_update_ms.1, r.items_ms.0, r.items_ms.1, r.render_ms.0, r.render_ms.1
        );
    }
    println!(
        "sim ticks      : {} (verified {}) over {:.2} s = {:.1} ticks/s (real time needs {})",
        s.sim_tick,
        s.verified_tick,
        s.sim.span_secs,
        s.tick_rate_achieved(),
        TICK_RATE
    );
    println!("rollbacks      : {} (deepest {} ticks), stalls {}", s.rollbacks, s.max_rollback_depth, s.stalls);
    let sim = &s.sim;
    println!(
        "sim tick       : avg {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} normal ticks (peer A, no rollback)",
        sim.tick.avg_ms(),
        sim.tick_p99_ms,
        sim.tick.max_ms,
        sim.tick.count
    );
    println!(
        "rollback burst : avg {:.3} ms, max {:.3} ms over {} rollbacks; {} ticks resimulated (longest {}), {:.3} ms per resimulated tick",
        sim.rollback.avg_ms(),
        sim.rollback.max_ms,
        sim.rollback.count,
        sim.resim_ticks,
        sim.max_resim_ticks,
        sim.resim_tick_avg_ms()
    );
    println!(
        "bot peer B     : avg {:.3} ms, max {:.3} ms; publish snapshot avg {:.3} ms, max {:.3} ms",
        sim.bot.avg_ms(),
        sim.bot.max_ms,
        sim.publish.avg_ms(),
        sim.publish.max_ms
    );
    println!(
        "whole step     : avg {:.3} ms, max {:.3} ms (peer A + bot + publish); {} steps over the {:.2} ms budget; {} stalled calls",
        sim.step.avg_ms(),
        sim.step.max_ms,
        sim.over_budget,
        1000.0 / f64::from(TICK_RATE),
        sim.stalled.count
    );
}
