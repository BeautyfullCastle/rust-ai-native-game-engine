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
#[cfg(not(feature = "input-actions"))]
use winit::keyboard::KeyCode;
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::arena_audio::{ArenaAudio, AudioMode};
use crate::arena_view::{arena_floor, ArenaExtractor, Keys};
use crate::net_client::{log_lifecycle, relay_title};
use orr_render::orr_rhi::Wgpu;
use orr_render::{extract_items, Camera, RenderList, WindowRenderer};

#[derive(Clone, Debug)]
pub struct Options {
    /// Which adapter runs the sim, for the window title and the log.
    pub label: String,
    #[cfg(feature = "input-actions")]
    pub input_map: Option<orr_input::ActionMap>,
    #[cfg(feature = "game-ui")]
    pub game_ui_project: Option<std::path::PathBuf>,
    pub vsync: bool,
    /// Arena-only audio policy; the physics sample ignores it.
    pub audio: AudioMode,
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
    pub audio_status: String,
    pub audio_started: u64,
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
    renderer: ArenaRenderer,
    #[cfg(feature = "game-ui")]
    ui: Option<WindowUi>,
}

#[cfg(feature = "game-ui")]
struct WindowUi {
    state: egui_winit::State,
    gpu: crate::game_ui_gpu::GpuOverlay,
    pending: crate::game_ui_gpu::PendingOverlay,
}

enum ArenaRenderer {
    Shapes(Box<WindowRenderer<Wgpu>>),
    #[cfg(feature = "sprites")]
    Sprites(Box<crate::sprite_scene::SpriteWindow>),
}
impl ArenaRenderer {
    #[cfg(feature = "game-ui")]
    fn ui_gpu(&self) -> crate::game_ui_gpu::GpuOverlay {
        match self {
            Self::Shapes(r) => {
                crate::game_ui_gpu::GpuOverlay::new(r.renderer.rhi(), r.renderer.format())
            }
            #[cfg(feature = "sprites")]
            Self::Sprites(r) => crate::game_ui_gpu::GpuOverlay::new(r.rhi(), r.format()),
        }
    }
    fn adapter_name(&self) -> String {
        match self {
            Self::Shapes(renderer) => renderer.adapter_name(),
            #[cfg(feature = "sprites")]
            Self::Sprites(renderer) => renderer.adapter_name(),
        }
    }
    fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::Shapes(renderer) => renderer.resize(width, height),
            #[cfg(feature = "sprites")]
            Self::Sprites(renderer) => renderer.resize(width, height),
        }
    }
}

struct App<B: Bridge<Arena>> {
    bridge: B,
    #[cfg(feature = "game-ui")]
    restart: Option<Box<dyn FnMut() -> Result<B, String>>>,
    #[cfg(feature = "game-ui")]
    ui: Option<crate::game_ui::GameUi>,
    #[cfg(feature = "sprites")]
    sprites: Option<crate::sprite_scene::SpriteScene>,
    audio: ArenaAudio,
    view: ViewWorld<ArenaExtractor>,
    opts: Options,
    gfx: Option<Gfx>,
    keys: Keys,
    #[cfg(feature = "input-actions")]
    controls: crate::arena_input::ArenaControls,
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
    fn publish_input(&mut self, event_loop: &ActiveEventLoop) -> bool {
        #[cfg(feature = "input-actions")]
        {
            self.keys = self.controls.keys();
        }
        if self.sent_keys != Some(self.keys) {
            #[cfg(feature = "input-actions")]
            let result = self.controls.publish(&mut self.bridge);
            #[cfg(not(feature = "input-actions"))]
            let result = self
                .bridge
                .set_input(self.bridge.local_slot(), self.keys.to_input());
            match result {
                Ok(()) => self.sent_keys = Some(self.keys),
                Err(error) => {
                    self.error = Some(format!("set input: {error:?}"));
                    event_loop.exit();
                    return false;
                }
            }
        }
        true
    }

    #[cfg(feature = "game-ui")]
    fn ui_action(
        &mut self,
        action: crate::game_ui::Action,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), String> {
        use crate::game_ui::Action;
        if !self.ui.as_mut().is_some_and(|ui| ui.apply(action)) {
            return Ok(());
        }
        if action == Action::Quit {
            event_loop.exit();
        }
        if action == Action::Restart {
            self.restart_local()?;
        }
        let blocked = self.ui.as_ref().is_some_and(|ui| ui.blocks_controls());
        self.controls.set_paused(blocked);
        // Clearing capture clears held presses, including the menu/resume click.
        self.controls.set_ui_capture(true);
        self.controls.set_ui_capture(blocked);
        self.publish_input(event_loop);
        Ok(())
    }

    #[cfg(feature = "game-ui")]
    fn restart_local(&mut self) -> Result<(), String> {
        let factory = self
            .restart
            .as_mut()
            .ok_or("restart is unavailable for this bridge")?;
        self.bridge = factory()?;
        // Close old native voices/device before reopening an exclusive device.
        self.audio = ArenaAudio::open(AudioMode::Off)?;
        self.audio = ArenaAudio::open(self.opts.audio)?;
        self.view = ViewWorld::new(
            ArenaExtractor {
                remote_mode: self.opts.remote_mode,
                local_slot: self.bridge.local_slot().0,
            },
            self.opts.view,
        );
        #[cfg(feature = "sprites")]
        if let Some(scene) = &mut self.sprites {
            scene.reset();
        }
        self.controls = crate::arena_input::ArenaControls::new(
            self.opts
                .input_map
                .clone()
                .unwrap_or_else(crate::arena_input::default_map),
        )?;
        self.keys = Keys::default();
        self.sent_keys = None;
        self.items.clear();
        self.list.clear();
        self.summary.predicted_hits = 0;
        self.summary.verified_hits = 0;
        self.summary.canceled_events = 0;
        self.last_frame = Instant::now();
        Ok(())
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        if self.frames > 0 {
            self.worst_frame = self.worst_frame.max(dt);
        }

        if !self.publish_input(event_loop) {
            return;
        }
        self.bridge.update(dt);
        let update = self.bridge.poll_view();
        let snapshot = update.snapshot.clone();
        if let Err(error) = self.audio.update(&update) {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        if let Some(reset) = &update.resync {
            crate::net_client::log_view_resync(reset);
        }
        self.view
            .update_from_bridge(dt.as_secs_f32().min(0.1), &update);
        #[cfg(feature = "sprites")]
        let sprite_reset = update.resync.is_some();
        for event in update.events {
            match event {
                BridgeEvent::Sim {
                    status: EventStatus::Predicted(_),
                    ..
                } => self.summary.predicted_hits += 1,
                BridgeEvent::Sim {
                    status: EventStatus::Verified(_),
                    ..
                } => self.summary.verified_hits += 1,
                BridgeEvent::Sim {
                    status: EventStatus::Canceled,
                    ..
                } => self.summary.canceled_events += 1,
                BridgeEvent::Lifecycle(note) => log_lifecycle(&note),
                BridgeEvent::ViewResynced(reset) => crate::net_client::log_view_resync(&reset),
            }
        }

        self.items.clear();
        self.items.push(arena_floor());
        self.view.render_items(&mut self.items);
        #[cfg(feature = "sprites")]
        if let Some(scene) = &mut self.sprites {
            scene.update(
                dt,
                snapshot.as_ref(),
                self.bridge.local_slot().0,
                sprite_reset,
                &mut self.items,
            );
        }
        self.list.clear();
        extract_items(&self.items, &mut self.list);
        #[cfg(feature = "game-ui")]
        if let (Some(ui), Some(gfx)) = (&mut self.ui, &mut self.gfx) {
            if let Some(window_ui) = &mut gfx.ui {
                let input = window_ui.state.take_egui_input(&gfx.window);
                let hud = snapshot
                    .as_ref()
                    .map(|s| crate::game_ui::Hud {
                        tick: s.tick(),
                        verified_tick: s.verified_tick(),
                        rollbacks: s.stats().rollbacks,
                    })
                    .unwrap_or_default();
                let (mut output, action) = ui.show(input, hud);
                window_ui.state.handle_platform_output(
                    &gfx.window,
                    std::mem::take(&mut output.platform_output),
                );
                if let Err(error) = window_ui.pending.push(output) {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
                let capture = ui.blocks_controls()
                    || ui.context.egui_wants_keyboard_input()
                    || ui.context.egui_wants_pointer_input();
                self.controls.set_ui_capture(capture);
                if let Some(action) = action {
                    if let Err(error) = self.ui_action(action, event_loop) {
                        self.error = Some(error);
                        event_loop.exit();
                        return;
                    }
                }
            }
        }
        // Capturing a held input must neutralize even a threaded bridge now.
        if !self.publish_input(event_loop) {
            return;
        }
        if let Some(gfx) = &mut self.gfx {
            let camera = Camera::new([0.0, 0.0], 2100.0);
            match &mut gfx.renderer {
                ArenaRenderer::Shapes(renderer) => {
                    #[cfg(feature = "game-ui")]
                    if let (Some(window_ui), Some(ui)) = (&mut gfx.ui, &self.ui) {
                        let mut error = None;
                        renderer.render_with_overlay(&self.list, &camera, |rhi, view, size| {
                            if let Err(e) = window_ui.pending.paint(
                                &mut window_ui.gpu,
                                rhi,
                                view,
                                size,
                                &ui.context,
                            ) {
                                error = Some(e);
                            }
                        });
                        if let Some(error) = error {
                            self.error = Some(error);
                            event_loop.exit();
                            return;
                        }
                    } else {
                        renderer.render(&self.list, &camera);
                    }
                    #[cfg(not(feature = "game-ui"))]
                    renderer.render(&self.list, &camera);
                }
                #[cfg(feature = "sprites")]
                ArenaRenderer::Sprites(renderer) => {
                    #[cfg(feature = "game-ui")]
                    let result = if let (Some(window_ui), Some(ui)) = (&mut gfx.ui, &self.ui) {
                        renderer.render_with_overlay(
                            &self.list,
                            self.sprites.as_ref().expect("sprite mode"),
                            |rhi, view, size| {
                                window_ui.pending.paint(
                                    &mut window_ui.gpu,
                                    rhi,
                                    view,
                                    size,
                                    &ui.context,
                                )
                            },
                        )
                    } else {
                        renderer.render(&self.list, self.sprites.as_ref().expect("sprite mode"))
                    };
                    #[cfg(not(feature = "game-ui"))]
                    let result =
                        renderer.render(&self.list, self.sprites.as_ref().expect("sprite mode"));
                    if let Err(error) = result {
                        self.error = Some(error);
                        event_loop.exit();
                        return;
                    }
                }
            }
        }

        self.frames += 1;
        self.window_frames += 1;
        let window_secs = self.window_start.elapsed().as_secs_f32();
        if window_secs >= 0.5 {
            let fps = self.window_frames as f32 / window_secs;
            let stats = snapshot
                .as_ref()
                .map(|s| (s.tick(), s.verified_tick(), s.stats()));
            if let (Some(gfx), Some((tick, verified, stats))) = (&self.gfx, stats) {
                #[cfg(feature = "input-actions")]
                let input_status = if self.controls.paused() {
                    " | LOCAL INPUT PAUSED (simulation continues)"
                } else {
                    ""
                };
                #[cfg(not(feature = "input-actions"))]
                let input_status = "";
                let net = self
                    .opts
                    .relay
                    .as_ref()
                    .map(|m| relay_title(&m.status()))
                    .unwrap_or_default();
                gfx.window.set_title(&format!(
                    "Orrery arena [{}] fps {:.0} | tick {} (verified {}) | rollbacks {} (deepest {}){net}{input_status}",
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
        if let Some(gfx) = &mut self.gfx {
            let focused = gfx.window.has_focus();
            #[cfg(feature = "input-actions")]
            self.controls.set_focused(focused);
            #[cfg(feature = "game-ui")]
            if let Some(ui) = &mut gfx.ui {
                let _ = ui
                    .state
                    .on_window_event(&gfx.window, &WindowEvent::Focused(focused));
            }
            let size = gfx.window.inner_size();
            gfx.renderer.resize(size.width, size.height);
            #[cfg(not(feature = "input-actions"))]
            let _ = focused;
            self.publish_input(event_loop);
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
        let create_renderer = || {
            #[cfg(feature = "sprites")]
            if let Some(scene) = &self.sprites {
                return crate::sprite_scene::SpriteWindow::new(
                    window.clone(),
                    self.opts.vsync,
                    scene,
                )
                .map(Box::new)
                .map(ArenaRenderer::Sprites);
            }
            WindowRenderer::new(
                window.clone(),
                (window.inner_size().width, window.inner_size().height),
                self.opts.vsync,
            )
            .map(Box::new)
            .map(ArenaRenderer::Shapes)
        };
        match create_renderer() {
            Ok(renderer) => {
                self.summary.adapter = renderer.adapter_name();
                println!("adapter: {}", self.summary.adapter);
                #[cfg(feature = "game-ui")]
                let ui = self.ui.as_ref().map(|ui| WindowUi {
                    state: egui_winit::State::new(
                        ui.context.clone(),
                        egui::ViewportId::ROOT,
                        window.as_ref(),
                        Some(window.scale_factor() as f32),
                        window.theme(),
                        None,
                    ),
                    gpu: renderer.ui_gpu(),
                    pending: crate::game_ui_gpu::PendingOverlay::default(),
                });
                self.gfx = Some(Gfx {
                    window,
                    renderer,
                    #[cfg(feature = "game-ui")]
                    ui,
                });
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
        #[cfg(feature = "game-ui")]
        let captured = if let (Some(ui), Some(gfx)) = (&mut self.ui, &mut self.gfx) {
            let response = gfx
                .ui
                .as_mut()
                .map(|window_ui| window_ui.state.on_window_event(&gfx.window, &event));
            let captured = ui.blocks_controls()
                || response.is_some_and(|response| response.consumed)
                || ui.context.egui_wants_keyboard_input()
                || ui.context.egui_wants_pointer_input();
            self.controls.set_ui_capture(captured);
            captured
        } else {
            false
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                #[cfg(not(feature = "input-actions"))]
                let _ = is_synthetic;
                let down = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    #[cfg(feature = "input-actions")]
                    if let Some(button) = crate::arena_input::keyboard(code) {
                        #[cfg(feature = "game-ui")]
                        let consumed = captured || (is_synthetic && down);
                        #[cfg(not(feature = "game-ui"))]
                        let consumed = is_synthetic && down;
                        let control = self.controls.button(button, down, event.repeat, consumed);
                        if control.quit {
                            event_loop.exit();
                        }
                        #[cfg(feature = "game-ui")]
                        if control.pause_changed && self.ui.is_some() {
                            let action = if self.controls.paused() {
                                crate::game_ui::Action::Menu
                            } else {
                                crate::game_ui::Action::Continue
                            };
                            if let Err(error) = self.ui_action(action, event_loop) {
                                self.error = Some(error);
                                event_loop.exit();
                            }
                        }
                    }
                    #[cfg(not(feature = "input-actions"))]
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
            WindowEvent::Focused(focused) => {
                #[cfg(feature = "input-actions")]
                self.controls.set_focused(focused);
                if !focused {
                    self.keys = Keys::default();
                }
            }
            #[cfg(feature = "input-actions")]
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button) = crate::arena_input::mouse(button) {
                    #[cfg(feature = "game-ui")]
                    let consumed = captured;
                    #[cfg(not(feature = "game-ui"))]
                    let consumed = false;
                    let control = self.controls.button(
                        button,
                        state == ElementState::Pressed,
                        false,
                        consumed,
                    );
                    if control.quit {
                        event_loop.exit();
                    }
                    #[cfg(feature = "game-ui")]
                    if control.pause_changed && self.ui.is_some() {
                        let action = if self.controls.paused() {
                            crate::game_ui::Action::Menu
                        } else {
                            crate::game_ui::Action::Continue
                        };
                        if let Err(error) = self.ui_action(action, event_loop) {
                            self.error = Some(error);
                            event_loop.exit();
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
        // Threaded bridges advance even when redraws are throttled/minimized.
        // Publish releases/focus loss/pause immediately at the event boundary.
        self.publish_input(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "input-actions")]
        self.controls.set_focused(false);
        self.keys = Keys::default();
        self.publish_input(event_loop);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}

/// Opens the window and runs until it closes (or `opts.seconds` pass).
pub fn run<B: Bridge<Arena>>(bridge: B, opts: Options) -> Result<Summary, String> {
    run_inner(
        bridge,
        opts,
        #[cfg(feature = "sprites")]
        None,
        #[cfg(feature = "game-ui")]
        None,
    )
}

/// Runs the same Arena simulation with installed sprite assets and local-player camera follow.
#[cfg(feature = "sprites")]
pub fn run_sprites<B: Bridge<Arena>>(
    bridge: B,
    opts: Options,
    root: &std::path::Path,
) -> Result<Summary, String> {
    let scene = crate::sprite_scene::SpriteScene::open(root)?;
    run_inner(
        bridge,
        opts,
        Some(scene),
        #[cfg(feature = "game-ui")]
        None,
    )
}

fn run_inner<B: Bridge<Arena>>(
    bridge: B,
    opts: Options,
    #[cfg(feature = "sprites")] sprites: Option<crate::sprite_scene::SpriteScene>,
    #[cfg(feature = "game-ui")] restart: Option<Box<dyn FnMut() -> Result<B, String>>>,
) -> Result<Summary, String> {
    let audio = ArenaAudio::open(opts.audio)?;
    eprintln!("audio: {}", audio.status());
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let now = Instant::now();
    let local_slot = bridge.local_slot().0;
    let extractor = ArenaExtractor {
        remote_mode: opts.remote_mode,
        local_slot,
    };
    #[cfg(feature = "input-actions")]
    #[allow(unused_mut)]
    let mut controls = crate::arena_input::ArenaControls::new(
        opts.input_map
            .clone()
            .unwrap_or_else(crate::arena_input::default_map),
    )?;
    #[cfg(feature = "game-ui")]
    let ui = opts
        .game_ui_project
        .as_ref()
        .map(|root| crate::game_ui::GameUi::open(root, restart.is_some() && opts.relay.is_none()))
        .transpose()?;
    #[cfg(feature = "game-ui")]
    if ui.is_some() {
        controls.set_paused(true);
        controls.set_ui_capture(true);
    }
    let mut app = App {
        bridge,
        #[cfg(feature = "game-ui")]
        restart,
        #[cfg(feature = "game-ui")]
        ui,
        #[cfg(feature = "sprites")]
        sprites,
        audio,
        view: ViewWorld::new(extractor, opts.view),
        opts,
        gfx: None,
        keys: Keys::default(),
        #[cfg(feature = "input-actions")]
        controls,
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
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("run: {e}"))?;
    if let Some(e) = app.error {
        return Err(e);
    }
    let seconds = app.started.elapsed().as_secs_f32();
    app.summary.audio_status = app.audio.status().to_string();
    app.summary.audio_started = app.audio.started();
    app.summary.frames = app.frames;
    app.summary.seconds = seconds;
    app.summary.fps = app.frames as f32 / seconds.max(0.001);
    app.summary.worst_frame_ms = app.worst_frame.as_secs_f32() * 1000.0;
    Ok(app.summary)
}

/// Local-only restart factory. Relay callers use `run` and never receive this capability.
#[cfg(feature = "game-ui")]
pub fn run_restartable<B: Bridge<Arena> + 'static>(
    mut factory: impl FnMut() -> Result<B, String> + 'static,
    opts: Options,
    sprite_root: Option<&std::path::Path>,
) -> Result<Summary, String> {
    if opts.relay.is_some() {
        return Err("relay restart is not supported".into());
    }
    let bridge = factory()?;
    #[cfg(feature = "sprites")]
    let sprites = sprite_root
        .map(crate::sprite_scene::SpriteScene::open)
        .transpose()?;
    #[cfg(not(feature = "sprites"))]
    if sprite_root.is_some() {
        return Err("sprite mode requires sprites feature".into());
    }
    run_inner(
        bridge,
        opts,
        #[cfg(feature = "sprites")]
        sprites,
        Some(Box::new(factory)),
    )
}

#[cfg(all(test, feature = "game-ui"))]
mod game_ui_tests {
    use super::*;
    use crate::arena_view::{arena_bridge_config, loopback_pair, Loopback};
    use orr_bridge::InProc;
    fn local() -> InProc<Arena, orr_bridge::LoopbackPair<Arena>> {
        InProc::new(loopback_pair(Loopback::default()), arena_bridge_config())
    }
    #[test]
    fn repeated_local_restart_discards_bridge_view_audio_and_input_history() {
        let now = Instant::now();
        let opts = Options {
            label: "test".into(),
            input_map: None,
            game_ui_project: None,
            vsync: false,
            audio: AudioMode::Off,
            seconds: None,
            remote_mode: InterpMode::Snapshot,
            view: ViewConfig::default(),
            relay: None,
        };
        let mut app = App {
            bridge: local(),
            restart: Some(Box::new(|| Ok(local()))),
            ui: None,
            #[cfg(feature = "sprites")]
            sprites: None,
            audio: ArenaAudio::open(AudioMode::Off).unwrap(),
            view: ViewWorld::new(
                ArenaExtractor {
                    remote_mode: opts.remote_mode,
                    local_slot: 0,
                },
                opts.view,
            ),
            opts,
            gfx: None,
            keys: Keys::default(),
            controls: crate::arena_input::ArenaControls::new(crate::arena_input::default_map())
                .unwrap(),
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
        for _ in 0..3 {
            app.controls.button(
                orr_input::Button::Keyboard {
                    key: orr_input::Key::D,
                },
                true,
                false,
                false,
            );
            app.controls.publish(&mut app.bridge).unwrap();
            app.bridge.update(Duration::from_millis(200));
            assert!(app.bridge.snapshot().unwrap().tick() > 0);
            let update = app.bridge.poll_view();
            app.view.update_from_bridge(0.1, &update);
            app.view.render_items(&mut app.items);
            app.summary.predicted_hits = 10;
            app.sent_keys = Some(app.controls.keys());
            app.restart_local().unwrap();
            assert_eq!(app.controls.keys(), Keys::default());
            assert!(app.sent_keys.is_none());
            assert!(app.items.is_empty());
            assert_eq!(app.audio.started(), 0);
            assert_eq!(app.summary.predicted_hits, 0);
            app.view.render_items(&mut app.items);
            assert!(app.items.is_empty());
            let fresh = local();
            assert_eq!(
                app.bridge.snapshot().map(|s| s.tick()),
                fresh.snapshot().map(|s| s.tick())
            );
            app.controls.publish(&mut app.bridge).unwrap();
            app.bridge.update(Duration::from_millis(50));
            assert!(app.bridge.snapshot().is_some());
        }
        app.restart = None;
        assert!(app.restart_local().unwrap_err().contains("unavailable"));
    }
}
