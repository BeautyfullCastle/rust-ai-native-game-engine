//! The window loop, the headless loop and the screenshot path of the
//! Yard3D sample, with the numbers they print at the end.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeEvent, InProc, PlayerSlot};
use orr_render::text::{draw_text, fill_rect, text_height, text_width};
use orr_render::{OrbitCamera, RenderList, RenderList3D, Settings3D, WindowRenderer3D};
use orr_render::orr_rhi::Wgpu;
use orr_view::{RenderItem3, ViewConfig, ViewWorld3};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::arena_view::{Loopback, LOCAL_SLOT};
use crate::net_client::log_lifecycle;
use crate::physics_host::{SimMetrics, SimReport};
use crate::yard3d_game::{Yard3D, YardConfig, TICK_RATE};
use crate::yard3d_host::{scripted_local, yard_bridge_config, yard_pair};
use crate::yard3d_view::{debug_lines, fill_list, input_for, yard_camera, yard_lighting, YardExtractor, YardKeys};

/// Frames at the start that are left out of the render statistics (shader
/// compile, window creation, first snapshots).
const WARMUP_FRAMES: usize = 10;

/// What a window run does besides playing.
#[derive(Clone, Debug)]
pub struct YardOptions {
    /// Shown in the stats line: how the sim runs.
    pub label: String,
    pub vsync: bool,
    /// Close after this many wall-clock seconds.
    pub seconds: Option<f32>,
    /// Close after this many frames. Frames then advance the sim by exactly
    /// one tick (a fixed 1/60 s step, in process), so the picture at frame N
    /// is the same on every run.
    pub frames: Option<u64>,
    /// Write the presented frame of the last frame to this PNG file (needs `frames`).
    pub screenshot: Option<PathBuf>,
    /// Window inner size in logical pixels.
    pub size: (u32, u32),
    /// Requested MSAA samples (the renderer falls back to what the adapter has).
    pub msaa: u32,
    pub shadow_map: u32,
    /// Longitude segments of the sphere and capsule meshes.
    pub mesh_segments: u32,
    pub shadows: bool,
    /// Start with the debug lines (boxes and velocity arrows) on.
    pub debug: bool,
    pub view: ViewConfig,
}

impl Default for YardOptions {
    fn default() -> Self {
        Self {
            label: String::new(),
            vsync: true,
            seconds: None,
            frames: None,
            screenshot: None,
            size: (1280, 720),
            msaa: 4,
            shadow_map: 2048,
            mesh_segments: orr_render::DEFAULT_SEGMENTS,
            shadows: true,
            debug: false,
            view: crate::yard3d_view::yard_view_config(),
        }
    }
}

impl YardOptions {
    /// The mobile / weak-GPU preset (`--low`): see [`Settings3D::LOW`].
    pub fn low(&mut self) {
        let low = Settings3D::LOW;
        self.msaa = low.msaa;
        self.shadow_map = low.shadow_map_size;
        self.mesh_segments = low.mesh_segments;
    }
}

/// Mean and worst of a set of milliseconds.
pub type MeanMax = (f32, f32);

/// Render-side numbers of a window run (CPU times per frame, after warm-up).
#[derive(Clone, Debug, Default)]
pub struct YardRender {
    pub frames: u64,
    pub seconds: f32,
    pub fps_avg: f32,
    pub fps_1pct_low: f32,
    pub worst_frame_ms: f32,
    /// Taking in a snapshot and advancing the view clocks.
    pub view_update_ms: MeanMax,
    /// Interpolating the items and building the instance lists.
    pub list_ms: MeanMax,
    /// The render call: instance upload, command recording, submit, present (and so, on a
    /// software adapter, the whole rasterization; on a GPU the wait for the display).
    pub render_ms: MeanMax,
    /// Instances in the last frame.
    pub instances: usize,
    /// MSAA samples in use.
    pub msaa: u32,
}

/// Everything a run measured.
#[derive(Clone, Debug, Default)]
pub struct YardSummary {
    pub adapter: String,
    /// Entities alive in the newest predicted frame.
    pub entities: u32,
    pub render: Option<YardRender>,
    /// Wall time of the run.
    pub seconds: f32,
    pub sim_tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
    pub max_rollback_depth: u32,
    pub stalls: u64,
    pub sim: SimReport,
    /// Where the screenshot went, if one was taken.
    pub screenshot: Option<PathBuf>,
}

impl YardSummary {
    /// Ticks the sim reached per second of sim wall time (60 = keeping up).
    pub fn tick_rate_achieved(&self) -> f32 {
        let span = if self.sim.span_secs > 0.0 { self.sim.span_secs as f32 } else { self.seconds };
        self.sim_tick as f32 / span.max(0.001)
    }
}

fn mean_max(v: &[f32]) -> MeanMax {
    let v = v.get(WARMUP_FRAMES.min(v.len())..).unwrap_or(&[]);
    if v.is_empty() {
        return (0.0, 0.0);
    }
    (v.iter().sum::<f32>() / v.len() as f32, v.iter().copied().fold(0.0, f32::max))
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

#[derive(Default)]
struct Stages {
    frame: Vec<f32>,
    view_update: Vec<f32>,
    list: Vec<f32>,
    render: Vec<f32>,
}

impl Stages {
    fn finish(&self, seconds: f32, instances: usize, msaa: u32) -> YardRender {
        let kept = self.frame.get(WARMUP_FRAMES.min(self.frame.len())..).unwrap_or(&[]);
        let mut sorted = kept.to_vec();
        sorted.sort_by(f32::total_cmp);
        let mean = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
        let slow = &sorted[sorted.len() - (sorted.len() / 100).max(1).min(sorted.len())..];
        YardRender {
            frames: self.frame.len() as u64,
            seconds,
            fps_avg: if mean(kept) > 0.0 { 1000.0 / mean(kept) } else { 0.0 },
            fps_1pct_low: if mean(slow) > 0.0 { 1000.0 / mean(slow) } else { 0.0 },
            worst_frame_ms: sorted.last().copied().unwrap_or(0.0),
            view_update_ms: mean_max(&self.view_update),
            list_ms: mean_max(&self.list),
            render_ms: mean_max(&self.render),
            instances,
            msaa,
        }
    }
}

enum Drag {
    Orbit,
    Pan,
}

struct Gfx {
    window: Arc<Window>,
    renderer: WindowRenderer3D<Wgpu>,
}

struct App<B: Bridge<Yard3D>> {
    bridge: B,
    view: ViewWorld3<YardExtractor>,
    opts: YardOptions,
    metrics: Arc<SimMetrics>,
    gfx: Option<Gfx>,
    keys: YardKeys,
    sent_input: Option<crate::yard3d_game::YardInput>,
    orbit: OrbitCamera,
    cursor: [f32; 2],
    drag: Option<Drag>,
    shadows: bool,
    tonemap: bool,
    debug: bool,
    items: Vec<RenderItem3>,
    list: RenderList3D,
    overlay: RenderList,
    started: Instant,
    last_frame: Instant,
    frame_count: u64,
    fps_frames: u32,
    fps_start: Instant,
    fps: f32,
    sim_ms: (f64, f64),
    stages: Stages,
    last_instances: usize,
    msaa_used: u32,
    summary: YardSummary,
    error: Option<String>,
}

const HELP: [&str; 2] = [
    "DRAG ORBIT | RIGHT DRAG PAN | WHEEL ZOOM | R RESET | ESC QUIT",
    "SPACE SHOOT | HOLD B BOX / N BALL / C CAPSULE | G DEBUG | H SHADOWS | T TONEMAP",
];

impl<B: Bridge<Yard3D>> App<B> {
    fn fixed_step(&self) -> bool {
        self.opts.frames.is_some()
    }

    fn build_overlay(&mut self, size: (u32, u32), s: &orr_bridge::Snapshot) {
        let (tick, verified, bodies) = (s.tick(), s.verified_tick(), s.predicted().alive_count());
        let stats = s.stats();
        let (rollbacks, depth, stalls) = (stats.rollbacks, stats.max_rollback_depth, stats.stalls);
        let h = size.1 as f32;
        let scale = if size.0 >= 1100 { 2.0 } else { 1.0 };
        let pad = 8.0 * scale;
        let line_h = text_height(scale) + 5.0 * scale;
        let adapter = self.gfx.as_ref().map(|g| g.renderer.adapter_name()).unwrap_or_default();
        let lines = [
            format!("ORRERY 3D YARD [{}]  {} BODIES", self.opts.label, bodies),
            format!("FPS {:.0}  FRAME {:.1} MS  SIM {:.1}/{:.1} MS", self.fps, 1000.0 / self.fps.max(0.001), self.sim_ms.0, self.sim_ms.1),
            format!("TICK {tick} (VERIFIED {verified})  ROLLBACKS {rollbacks} (DEEPEST {depth})  STALLS {stalls}"),
            format!(
                "MSAA {}X  SHADOWS {}  TONEMAP {}  DEBUG {}",
                self.msaa_used,
                if self.shadows { "ON" } else { "OFF" },
                if self.tonemap { "ON" } else { "OFF" },
                if self.debug { "ON" } else { "OFF" }
            ),
            format!("GPU {adapter}"),
        ];
        let width = lines.iter().map(|l| text_width(l, scale)).fold(0.0, f32::max);
        fill_rect(&mut self.overlay, h, pad * 0.5, pad * 0.5, width + pad * 1.5, line_h * lines.len() as f32 + pad, [0.0, 0.0, 0.0, 0.55]);
        for (i, l) in lines.iter().enumerate() {
            let color = if i == 0 { [1.0, 0.9, 0.5, 1.0] } else { [0.9, 0.95, 1.0, 1.0] };
            draw_text(&mut self.overlay, h, pad, pad + i as f32 * line_h, scale, l, color);
        }
        let hw = HELP.iter().map(|l| text_width(l, scale)).fold(0.0, f32::max);
        let y = h - (text_height(scale) + 5.0 * scale) * HELP.len() as f32 - pad;
        fill_rect(&mut self.overlay, h, pad * 0.5, y - pad * 0.5, hw + pad * 1.5, line_h * HELP.len() as f32 + pad, [0.0, 0.0, 0.0, 0.55]);
        for (i, l) in HELP.iter().enumerate() {
            draw_text(&mut self.overlay, h, pad, y + i as f32 * line_h, scale, l, [0.8, 0.85, 0.95, 1.0]);
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let real_dt = now - self.last_frame;
        self.last_frame = now;
        let dt = if self.fixed_step() { Duration::from_secs_f64(1.0 / f64::from(TICK_RATE)) } else { real_dt };
        if self.gfx.is_some() {
            self.stages.frame.push(ms(real_dt));
        }
        let viewport = self.gfx.as_ref().map_or((1, 1), |g| g.renderer.size());
        let camera = self.orbit.camera();

        let input = input_for(self.keys, &camera, self.cursor, viewport);
        if self.sent_input != Some(input) {
            self.sent_input = Some(input);
            let _ = self.bridge.set_input(self.bridge.local_slot(), input);
        }
        self.bridge.update(dt);
        let update = self.bridge.poll_view();
        let snapshot = update.snapshot.clone();
        if let Some(reset) = &update.resync {
            crate::net_client::log_view_resync(reset);
        }

        let t = Instant::now();
        self.view.update_from_bridge(dt.as_secs_f32().min(0.1), &update);
        self.stages.view_update.push(ms(t.elapsed()));
        for event in update.events {
            if let BridgeEvent::Lifecycle(note) = event {
                log_lifecycle(&note);
            }
        }

        let t = Instant::now();
        self.items.clear();
        self.view.render_items(&mut self.items);
        self.list.clear();
        self.list.lighting = yard_lighting();
        self.list.lighting.shadows = self.shadows;
        self.list.lighting.tonemap = self.tonemap;
        fill_list(&self.items, &mut self.list);
        if self.debug {
            if let Some(s) = &snapshot {
                debug_lines(s.predicted(), &mut self.list);
            }
        }
        self.last_instances = self.list.instance_count();
        self.stages.list.push(ms(t.elapsed()));

        self.fps_frames += 1;
        let window_secs = self.fps_start.elapsed().as_secs_f32();
        if window_secs >= 0.5 {
            self.fps = self.fps_frames as f32 / window_secs;
            self.sim_ms = self.metrics.take_window();
            self.fps_frames = 0;
            self.fps_start = Instant::now();
        }
        self.overlay.clear();
        if let Some(s) = &snapshot {
            self.build_overlay(viewport, s);
        }

        self.frame_count += 1;
        let last = self.opts.frames.is_some_and(|n| self.frame_count >= n);
        let capture = last && self.opts.screenshot.is_some();
        let t = Instant::now();
        let mut shot = None;
        if let Some(gfx) = &mut self.gfx {
            shot = gfx.renderer.render_capture(&self.list, &camera, &self.overlay, capture).1;
        }
        self.stages.render.push(ms(t.elapsed()));

        if let Some(s) = &snapshot {
            if let (Some(path), Some((w, h, rgba))) = (&self.opts.screenshot, shot) {
                match write_png(path, w, h, &rgba) {
                    Ok(()) => {
                        println!("screenshot     : {} ({w}x{h}, the presented frame)", path.display());
                        self.summary.screenshot = Some(path.clone());
                    }
                    Err(e) => self.error = Some(format!("screenshot: {e}")),
                }
            } else if capture {
                self.error = Some("the surface cannot be read back (no COPY_SRC): no screenshot".to_string());
            }
            if last || self.opts.seconds.is_some_and(|l| self.started.elapsed().as_secs_f32() >= l) {
                fill_from_snapshot(&mut self.summary, s);
                event_loop.exit();
            }
        } else if last {
            event_loop.exit();
        }
    }

    fn key(&mut self, code: KeyCode, down: bool, event_loop: &ActiveEventLoop) {
        match code {
            KeyCode::Space => self.keys.shoot = down,
            KeyCode::KeyB => self.keys.spawn_box = down,
            KeyCode::KeyN => self.keys.spawn_ball = down,
            KeyCode::KeyC => self.keys.spawn_capsule = down,
            KeyCode::Escape if down => event_loop.exit(),
            KeyCode::KeyG if down => self.debug = !self.debug,
            KeyCode::KeyH if down => self.shadows = !self.shadows,
            KeyCode::KeyT if down => self.tonemap = !self.tonemap,
            KeyCode::KeyR if down => self.orbit = yard_camera(),
            _ => {}
        }
    }
}

fn fill_from_snapshot(summary: &mut YardSummary, s: &orr_bridge::Snapshot) {
    summary.sim_tick = s.tick();
    summary.verified_tick = s.verified_tick();
    let stats = s.stats();
    summary.rollbacks = stats.rollbacks;
    summary.max_rollback_depth = stats.max_rollback_depth;
    summary.stalls = stats.stalls;
    summary.entities = s.predicted().alive_count();
}

/// Writes straight RGBA8 pixels as an opaque RGB PNG.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}

impl<B: Bridge<Yard3D>> ApplicationHandler for App<B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let (w, h) = self.opts.size;
        let attributes = Window::default_attributes().with_title("Orrery 3D yard").with_inner_size(LogicalSize::new(f64::from(w), f64::from(h)));
        let window = match event_loop.create_window(attributes) {
            Ok(win) => Arc::new(win),
            Err(e) => {
                self.error = Some(format!("create_window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let settings = Settings3D { msaa: self.opts.msaa, shadow_map_size: self.opts.shadow_map, mesh_segments: self.opts.mesh_segments };
        match WindowRenderer3D::new(window.clone(), (size.width, size.height), self.opts.vsync, settings) {
            Ok(renderer) => {
                self.summary.adapter = renderer.adapter_name();
                self.msaa_used = renderer.renderer.samples();
                println!(
                    "adapter: {} (msaa {}x, shadow map {}, mesh segments {})",
                    self.summary.adapter, self.msaa_used, self.opts.shadow_map, self.opts.mesh_segments
                );
                self.gfx = Some(Gfx { window, renderer });
                self.started = Instant::now();
                self.last_frame = self.started;
                self.fps_start = self.started;
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
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(code, event.state == ElementState::Pressed, event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let now = [position.x as f32, position.y as f32];
                let delta = [now[0] - self.cursor[0], now[1] - self.cursor[1]];
                self.cursor = now;
                let viewport = self.gfx.as_ref().map_or((1, 1), |g| g.renderer.size());
                match self.drag {
                    Some(Drag::Orbit) => self.orbit.orbit(delta),
                    Some(Drag::Pan) => self.orbit.pan(delta, viewport),
                    None => {}
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                self.drag = match (button, down) {
                    (MouseButton::Left, true) => Some(Drag::Orbit),
                    (MouseButton::Right | MouseButton::Middle, true) => Some(Drag::Pan),
                    _ => None,
                };
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.orbit.zoom(steps);
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

/// Opens the window and runs until it closes (or `opts.seconds` / `opts.frames`
/// pass). `metrics` must be the one the bridge's sim side writes to.
pub fn run_window<B: Bridge<Yard3D>>(bridge: B, metrics: Arc<SimMetrics>, opts: YardOptions) -> Result<YardSummary, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let mut app = App {
        bridge,
        view: ViewWorld3::new(YardExtractor, opts.view),
        shadows: opts.shadows,
        tonemap: true,
        debug: opts.debug,
        opts,
        metrics: metrics.clone(),
        gfx: None,
        keys: YardKeys::default(),
        sent_input: None,
        orbit: yard_camera(),
        cursor: [0.0; 2],
        drag: None,
        items: Vec::new(),
        list: RenderList3D::new(),
        overlay: RenderList::new(),
        started: now,
        last_frame: now,
        frame_count: 0,
        fps_frames: 0,
        fps_start: now,
        fps: 0.0,
        sim_ms: (0.0, 0.0),
        stages: Stages::default(),
        last_instances: 0,
        msaa_used: 1,
        summary: YardSummary::default(),
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
    summary.render = Some(app.stages.finish(seconds, app.last_instances, app.msaa_used));
    summary.sim = metrics.report();
    Ok(summary)
}

/// How long a headless run lasts.
#[derive(Clone, Copy, Debug)]
pub enum HeadlessLimit {
    /// Wall-clock seconds.
    Seconds(f32),
    /// Sim steps.
    Steps(u64),
}

/// Runs the session without a window or GPU, one sim step after another as
/// fast as the machine allows, with a scripted local player. Unlike the 2D
/// sample's headless run it also runs the whole CPU side of the view each
/// step: taking in the snapshot (extractor, interpolation, rollback error
/// decay) and building the render list, which is what a real GPU frame
/// spends its CPU time on.
pub fn run_headless(scene: YardConfig, net: Loopback, limit: HeadlessLimit) -> YardSummary {
    let metrics = SimMetrics::new();
    let mut bridge = InProc::new(yard_pair(scene, net, metrics.clone()), yard_bridge_config(metrics.clone()));
    let mut view = ViewWorld3::new(YardExtractor, crate::yard3d_view::yard_view_config());
    let (mut items, mut list) = (Vec::<RenderItem3>::new(), RenderList3D::new());
    let (mut view_ms, mut list_ms) = (Vec::new(), Vec::new());
    let started = Instant::now();
    let mut step: u64 = 0;
    let mut instances = 0;
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
        let snapshot = bridge.snapshot();
        let t = Instant::now();
        view.update(1.0 / TICK_RATE as f32, snapshot.as_ref());
        view_ms.push(ms(t.elapsed()));
        let t = Instant::now();
        items.clear();
        view.render_items(&mut items);
        list.clear();
        list.lighting = yard_lighting();
        fill_list(&items, &mut list);
        instances = list.instance_count();
        list_ms.push(ms(t.elapsed()));
        let _ = bridge.drain_events();
    }
    let mut summary = YardSummary { adapter: "headless".to_string(), seconds: started.elapsed().as_secs_f32(), ..Default::default() };
    if let Some(s) = bridge.snapshot() {
        fill_from_snapshot(&mut summary, &s);
    }
    summary.sim = metrics.report();
    summary.render = Some(YardRender {
        frames: step,
        seconds: summary.seconds,
        view_update_ms: mean_max(&view_ms),
        list_ms: mean_max(&list_ms),
        instances,
        msaa: 0,
        ..Default::default()
    });
    summary
}

/// Prints a summary.
pub fn print_summary(s: &YardSummary, headless: bool) {
    println!("adapter        : {} ({} entities alive)", s.adapter, s.entities);
    if let Some(r) = &s.render {
        if !headless {
            println!(
                "render         : {} frames in {:.2} s = {:.1} fps avg, {:.1} fps 1% low, worst frame {:.1} ms ({}x MSAA)",
                r.frames, r.seconds, r.fps_avg, r.fps_1pct_low, r.worst_frame_ms, r.msaa
            );
        }
        println!(
            "view cpu       : snapshot update {:.3}/{:.3} ms, items + instance lists {:.3}/{:.3} ms for {} instances{} (mean/worst)",
            r.view_update_ms.0,
            r.view_update_ms.1,
            r.list_ms.0,
            r.list_ms.1,
            r.instances,
            if headless { String::new() } else { format!(", render call {:.2}/{:.2} ms", r.render_ms.0, r.render_ms.1) }
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
        "whole step     : avg {:.3} ms, max {:.3} ms (peer A + bot + publish); {} steps over the {:.2} ms budget; {} stalled calls",
        sim.step.avg_ms(),
        sim.step.max_ms,
        sim.over_budget,
        1000.0 / f64::from(TICK_RATE),
        sim.stalled.count
    );
}

/// The `PlayerSlot` of the local player (re-exported for the binary).
pub const LOCAL: PlayerSlot = PlayerSlot(LOCAL_SLOT);

/// Used by the binary to build the bridge config.
pub fn bridge_config(metrics: Arc<SimMetrics>) -> orr_bridge::BridgeConfig<Yard3D> {
    yard_bridge_config(metrics)
}
