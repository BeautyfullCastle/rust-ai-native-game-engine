//! The window loop: keyboard to bridge input, bridge snapshots to the view
//! world, view items to the renderer. Works with any [`Bridge`] adapter.

use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeEvent, EventStatus, RelayMetrics};
use orr_testgame::Arena;
use orr_view::{InterpMode, RenderItem, ViewConfig, ViewWorld};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::arena_view::{arena_floor, ArenaExtractor, Keys};
use crate::net_client::{log_lifecycle, relay_title};
use orr_render::orr_rhi::Wgpu;
use orr_render::{extract_items, Camera, RenderList, WindowRenderer};

#[derive(Clone, Debug)]
pub struct Options {
    /// Which adapter runs the sim, for the window title and the log.
    pub label: String,
    pub vsync: bool,
    /// Close the window after this many seconds and print a summary.
    pub seconds: Option<f32>,
    pub remote_mode: InterpMode,
    pub view: ViewConfig,
    /// Relay play: numbers for the window title (round trip, delay, ...).
    pub relay: Option<Arc<RelayMetrics>>,
}

/// What a run measured.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub adapter: String,
    pub frames: u64,
    pub seconds: f32,
    pub fps: f32,
    pub worst_frame_ms: f32,
    pub sim_tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
    pub max_rollback_depth: u32,
    pub stalls: u64,
    pub predicted_hits: u64,
    pub verified_hits: u64,
    pub canceled_events: u64,
}

struct Gfx {
    window: Arc<Window>,
    renderer: WindowRenderer<Wgpu>,
}

struct App<B: Bridge<Arena>> {
    bridge: B,
    view: ViewWorld<ArenaExtractor>,
    opts: Options,
    gfx: Option<Gfx>,
    keys: Keys,
    sent_keys: Option<Keys>,
    items: Vec<RenderItem>,
    list: RenderList,
    started: Instant,
    last_frame: Instant,
    window_frames: u32,
    window_start: Instant,
    frames: u64,
    worst_frame: Duration,
    summary: Summary,
    error: Option<String>,
}

impl<B: Bridge<Arena>> App<B> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        if self.frames > 0 {
            self.worst_frame = self.worst_frame.max(dt);
        }

        if self.sent_keys != Some(self.keys) {
            self.sent_keys = Some(self.keys);
            let _ = self.bridge.set_input(self.bridge.local_slot(), self.keys.to_input());
        }
        self.bridge.update(dt);
        let update = self.bridge.poll_view();
        let snapshot = update.snapshot.clone();
        if let Some(reset) = &update.resync {
            crate::net_client::log_view_resync(reset);
        }
        self.view.update_from_bridge(dt.as_secs_f32().min(0.1), &update);
        for event in update.events {
            match event {
                BridgeEvent::Sim { status: EventStatus::Predicted(_), .. } => self.summary.predicted_hits += 1,
                BridgeEvent::Sim { status: EventStatus::Verified(_), .. } => self.summary.verified_hits += 1,
                BridgeEvent::Sim { status: EventStatus::Canceled, .. } => self.summary.canceled_events += 1,
                BridgeEvent::Lifecycle(note) => log_lifecycle(&note),
                BridgeEvent::ViewResynced(reset) => crate::net_client::log_view_resync(&reset),
            }
        }

        self.items.clear();
        self.items.push(arena_floor());
        self.view.render_items(&mut self.items);
        self.list.clear();
        extract_items(&self.items, &mut self.list);
        if let Some(gfx) = &mut self.gfx {
            let camera = Camera::new([0.0, 0.0], 2100.0);
            gfx.renderer.render(&self.list, &camera);
        }

        self.frames += 1;
        self.window_frames += 1;
        let window_secs = self.window_start.elapsed().as_secs_f32();
        if window_secs >= 0.5 {
            let fps = self.window_frames as f32 / window_secs;
            let stats = snapshot.as_ref().map(|s| (s.tick(), s.verified_tick(), s.stats()));
            if let (Some(gfx), Some((tick, verified, stats))) = (&self.gfx, stats) {
                let net = self.opts.relay.as_ref().map(|m| relay_title(&m.status())).unwrap_or_default();
                gfx.window.set_title(&format!(
                    "Orrery arena [{}] fps {:.0} | tick {} (verified {}) | rollbacks {} (deepest {}){net}",
                    self.opts.label, fps, tick, verified, stats.rollbacks, stats.max_rollback_depth
                ));
            }
            self.window_frames = 0;
            self.window_start = Instant::now();
        }

        if let Some(limit) = self.opts.seconds {
            if self.started.elapsed().as_secs_f32() >= limit {
                if let Some(s) = &snapshot {
                    self.summary.sim_tick = s.tick();
                    self.summary.verified_tick = s.verified_tick();
                    let stats = s.stats();
                    self.summary.rollbacks = stats.rollbacks;
                    self.summary.max_rollback_depth = stats.max_rollback_depth;
                    self.summary.stalls = stats.stalls;
                }
                event_loop.exit();
            }
        }
    }
}

impl<B: Bridge<Arena>> ApplicationHandler for App<B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Orrery arena")
            .with_inner_size(LogicalSize::new(900.0, 900.0));
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
                        KeyCode::Space => self.keys.fire = down,
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
pub fn run<B: Bridge<Arena>>(bridge: B, opts: Options) -> Result<Summary, String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let local_slot = bridge.local_slot().0;
    let extractor = ArenaExtractor { remote_mode: opts.remote_mode, local_slot };
    let mut app = App {
        bridge,
        view: ViewWorld::new(extractor, opts.view),
        opts,
        gfx: None,
        keys: Keys::default(),
        sent_keys: None,
        items: Vec::new(),
        list: RenderList::new(),
        started: now,
        last_frame: now,
        window_frames: 0,
        window_start: now,
        frames: 0,
        worst_frame: Duration::ZERO,
        summary: Summary::default(),
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| format!("run: {e}"))?;
    if let Some(e) = app.error {
        return Err(e);
    }
    let seconds = app.started.elapsed().as_secs_f32();
    app.summary.frames = app.frames;
    app.summary.seconds = seconds;
    app.summary.fps = app.frames as f32 / seconds.max(0.001);
    app.summary.worst_frame_ms = app.worst_frame.as_secs_f32() * 1000.0;
    Ok(app.summary)
}
