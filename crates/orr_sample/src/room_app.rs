//! Single-player authored room runtime. Admission is completed before GPU startup;
//! restart uses the retained initial frame and never reopens project files.
use crate::{
    room_project::PreparedProject,
    room_view::{self, RoomRenderer},
};
use orr_bridge::FrameView;
use orr_games::room_escape_game::{RoomEscapeV1, RoomInput, RoomRun, INTERACT, TICK_RATE};
use orr_render::orr_rhi::{Acquire, Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::OrbitCamera;
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

pub const HELP: &str = "room_escape --project PATH [--headless [--ticks 0..6000] [--hold left,right,up,down,interact] [--capture NEW.png]]\nWASD/arrows: world X/Z movement; E: interact; R: restart; Escape: quit.\nMouse left drag: orbit; right/middle drag: pan; wheel: zoom (view only).\nHeadless defaults to zero ticks; capture requires an available GPU adapter.";

#[derive(Debug)]
pub struct Options {
    pub project: PathBuf,
    pub headless: bool,
    pub ticks: u32,
    pub held: RoomInput,
    pub capture: Option<PathBuf>,
}
impl Options {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut project = None;
        let mut headless = false;
        let mut ticks = 0;
        let mut held = RoomInput::default();
        let mut capture = None;
        let mut seen = HashSet::new();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if !seen.insert(flag.clone()) {
                return Err(format!("duplicate option: {flag}"));
            }
            match flag.as_str() {
                "--project" => {
                    project = Some(PathBuf::from(
                        args.next().ok_or("--project requires a path")?,
                    ))
                }
                "--headless" => headless = true,
                "--ticks" => {
                    ticks = args
                        .next()
                        .ok_or("--ticks requires a value")?
                        .parse::<u32>()
                        .map_err(|_| "invalid --ticks")?;
                    if ticks > 6000 {
                        return Err("--ticks must be between 0 and 6000".into());
                    }
                }
                "--capture" => {
                    capture = Some(PathBuf::from(
                        args.next().ok_or("--capture requires a new PNG path")?,
                    ))
                }
                "--hold" => {
                    let value = args
                        .next()
                        .ok_or("--hold requires comma-separated controls")?;
                    let mut directions = HashSet::new();
                    for key in value.split(',') {
                        if !directions.insert(key) {
                            return Err(format!("duplicate held control: {key}"));
                        }
                        match key {
                            "left" | "right" | "up" | "down" | "interact" => {}
                            _ => return Err(format!("unknown held control: {key}")),
                        }
                    }
                    held.move_x = i8::from(directions.contains("right"))
                        - i8::from(directions.contains("left"));
                    held.move_z =
                        i8::from(directions.contains("down")) - i8::from(directions.contains("up"));
                    held.buttons = if directions.contains("interact") {
                        INTERACT
                    } else {
                        0
                    };
                }
                _ => return Err(format!("unknown option: {flag}\n{HELP}")),
            }
        }
        if !headless && (seen.contains("--ticks") || seen.contains("--hold") || capture.is_some()) {
            return Err("--ticks, --hold and --capture require --headless".into());
        }
        Ok(Self {
            project: project.ok_or("--project is required; no fallback room is created")?,
            headless,
            ticks,
            held,
            capture,
        })
    }
}
fn step(sim: &mut Simulation<RoomEscapeV1>, input: RoomInput) {
    let mut inputs = TickInputs::new(sim.tick() + 1, 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}
pub fn run(options: Options) -> Result<(), String> {
    let project = PreparedProject::open_with_options(
        &options.project,
        cfg!(feature = "room-ui"),
        if cfg!(all(feature = "room-checkpoint", target_os = "linux")) {
            crate::room_project::CheckpointSupport::MetadataOnly
        } else {
            crate::room_project::CheckpointSupport::Disabled
        },
    )?;
    if options.headless {
        headless(
            &project,
            options.ticks,
            options.held,
            options.capture.as_deref(),
        )
    } else {
        run_window(project)
    }
}
pub fn headless(
    project: &PreparedProject,
    ticks: u32,
    held: RoomInput,
    capture: Option<&Path>,
) -> Result<(), String> {
    if ticks > 6000 {
        return Err("--ticks must be between 0 and 6000".into());
    }
    let mut sim = project.scene().simulation()?;
    println!("room initial checksum: 0x{:016x}", sim.frame().checksum());
    for _ in 0..ticks {
        step(&mut sim, held);
    }
    let run = sim.frame().singleton::<RoomRun>();
    println!(
        "room tick: {} checksum: 0x{:016x}\nroom key: {} won: {}",
        sim.tick(),
        sim.frame().checksum(),
        run.key_collected,
        run.won
    );
    if let Some(path) = capture {
        // Check an obvious collision early, then atomically enforce it at write.
        if path.exists() {
            return Err(format!(
                "capture destination already exists: {}",
                path.display()
            ));
        }
        let gpu = Wgpu::headless(WgpuOptions::default())?;
        let size = (1024, 768);
        let mut renderer = RoomRenderer::new(&gpu, size, project.models())?;
        renderer.render(
            FrameView::of(sim.frame()),
            project.scene().index(),
            project.models(),
            &match project.camera() {
                Some(camera) => camera.document.camera(&camera.document.orbit(), size)?,
                None => room_view::camera(size),
            },
        )?;
        #[cfg(feature = "room-ui")]
        if let Some(prepared) = project.ui() {
            let mut ui = crate::collect_ui::CollectUi::new_for(
                prepared.document.clone(),
                prepared.font.clone(),
                crate::authored_ui::Profile::Room,
            )?;
            // Headless captures show authoritative playing/terminal state; no UI action is executed.
            ui.apply(crate::authored_ui::Action::Continue);
            let mut overlay = crate::game_ui_gpu::GpuOverlay::new(&gpu, room_view::TARGET_FORMAT);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0 as f32, size.1 as f32),
                )),
                ..Default::default()
            };
            let (output, _) = ui.show_room(
                input,
                crate::collect_ui::RoomHud {
                    key_acquired: run.key_collected != 0,
                    won: run.won != 0,
                },
            );
            overlay.paint(
                &gpu,
                renderer.target().render_view(),
                size,
                &ui.context,
                output,
            )?;
        }
        let rgba = renderer.read_rgba8();
        if rgba.len() != (size.0 * size.1 * 4) as usize {
            return Err("GPU capture returned an invalid image size".into());
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("capture {}: {e}", path.display()))?;
        let mut encoder = png17::Encoder::new(BufWriter::new(file), size.0, size.1);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        println!("room capture adapter: {}", gpu.adapter_name());
    }
    Ok(())
}

#[cfg(feature = "room-ui")]
fn ui_pointer_input(
    position: Option<winit::dpi::PhysicalPosition<f64>>,
    scale: f32,
) -> egui::RawInput {
    let mut input = egui::RawInput::default();
    let point = position
        .filter(|p| scale.is_finite() && scale > 0.0 && p.x.is_finite() && p.y.is_finite())
        .map(|p| egui::pos2(p.x as f32 / scale, p.y as f32 / scale));
    input
        .events
        .push(point.map_or(egui::Event::PointerGone, egui::Event::PointerMoved));
    input
}

#[derive(Default)]
struct Keys {
    down: HashSet<KeyCode>,
    blocked: HashSet<KeyCode>,
    neutral_pending: bool,
}
impl Keys {
    fn clear(&mut self) {
        self.blocked.extend(self.down.drain());
        self.neutral_pending = true;
    }
    fn event(
        &mut self,
        code: KeyCode,
        pressed: bool,
        synthetic: bool,
        repeat: bool,
        focused: bool,
    ) -> bool {
        if !pressed {
            self.down.remove(&code);
            self.blocked.remove(&code);
            return false;
        }
        if synthetic || !focused {
            self.blocked.insert(code);
            self.down.remove(&code);
            return false;
        }
        if repeat || self.blocked.contains(&code) {
            return false;
        }
        self.down.insert(code)
    }
    fn sample(&mut self) -> RoomInput {
        if std::mem::take(&mut self.neutral_pending) {
            return RoomInput::default();
        }
        let any = |a, b| self.down.contains(&a) || self.down.contains(&b);
        RoomInput {
            move_x: i8::from(any(KeyCode::KeyD, KeyCode::ArrowRight))
                - i8::from(any(KeyCode::KeyA, KeyCode::ArrowLeft)),
            move_z: i8::from(any(KeyCode::KeyS, KeyCode::ArrowDown))
                - i8::from(any(KeyCode::KeyW, KeyCode::ArrowUp)),
            buttons: if self.down.contains(&KeyCode::KeyE) {
                INTERACT
            } else {
                0
            },
            reserved: 0,
        }
    }
}
struct Graphics {
    window: Arc<Window>,
    rhi: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    renderer: RoomRenderer,
    size: (u32, u32),
    format: TextureFormat,
}
#[cfg(feature = "room-ui")]
struct WindowUi {
    state: egui_winit::State,
    gpu: crate::game_ui_gpu::GpuOverlay,
    pending: crate::game_ui_gpu::PendingOverlay,
}
#[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckpointAction {
    Resume,
    NewGame,
    Continue,
    Restart,
    Quit,
}
struct App {
    #[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
    checkpoint: Option<crate::room_checkpoint::CheckpointSession>,
    #[cfg(feature = "room-ui")]
    ui: Option<crate::collect_ui::CollectUi>,
    #[cfg(feature = "room-ui")]
    window_ui: Option<WindowUi>,
    #[cfg(feature = "room-ui")]
    quit: bool,
    #[cfg(feature = "room-ui")]
    ui_pointer: Option<winit::dpi::PhysicalPosition<f64>>,
    project: PreparedProject,
    sim: Simulation<RoomEscapeV1>,
    graphics: Option<Graphics>,
    keys: Keys,
    focused: bool,
    last: Instant,
    accumulated: Duration,
    orbit: OrbitCamera,
    drag: Option<MouseButton>,
    cursor: Option<[f32; 2]>,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
    fn step_authoritative(&mut self, input: RoomInput) {
        step(&mut self.sim, input);
        #[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
        if let Some(checkpoint) = &mut self.checkpoint {
            checkpoint.observe(*self.sim.frame().singleton::<RoomRun>());
        }
    }
    fn restart(&mut self) -> Result<(), String> {
        self.sim = self.project.scene().simulation()?;
        if let Some(camera) = self.project.camera() {
            self.orbit = camera.document.orbit();
            self.drag = None;
            self.cursor = None;
        }
        self.keys.clear();
        self.accumulated = Duration::ZERO;
        self.last = Instant::now();
        Ok(())
    }
    #[cfg(feature = "room-ui")]
    fn ui_frame(&mut self, input: egui::RawInput) -> Result<Vec<egui::FullOutput>, String> {
        #[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
        if self.checkpoint.is_some()
            && self.ui.as_ref().is_some_and(|ui| {
                matches!(
                    ui.screen(),
                    crate::authored_ui::Screen::Title | crate::authored_ui::Screen::Menu
                )
            })
        {
            return self.checkpoint_ui_frame(input);
        }
        let run = self.sim.frame().singleton::<RoomRun>();
        let ui = self.ui.as_mut().ok_or("Room UI is not admitted")?;
        let mut fresh_input = input.clone();
        fresh_input.events.clear();
        let (mut output, action) = ui.show_room(
            input,
            crate::collect_ui::RoomHud {
                key_acquired: run.key_collected != 0,
                won: run.won != 0,
            },
        );
        if let Some(action) = action {
            self.apply_ui_action(action)?;
            if !self.quit {
                // Keep texture epochs in order, but never paint pre-action terminal
                // geometry over a freshly restarted Frame/camera.
                output.shapes.clear();
                let run = self.sim.frame().singleton::<RoomRun>();
                let (fresh, repeated_action) = self.ui.as_mut().expect("admitted UI").show_room(
                    fresh_input,
                    crate::collect_ui::RoomHud {
                        key_acquired: run.key_collected != 0,
                        won: run.won != 0,
                    },
                );
                debug_assert!(repeated_action.is_none(), "no event replay after action");
                return Ok(vec![output, fresh]);
            }
        }
        Ok(vec![output])
    }
    #[cfg(feature = "room-ui")]
    fn apply_ui_action(&mut self, action: crate::authored_ui::Action) -> Result<(), String> {
        if action == crate::authored_ui::Action::Quit {
            self.quit = true;
            return Ok(());
        }
        if matches!(
            action,
            crate::authored_ui::Action::Restart | crate::authored_ui::Action::Play
        ) {
            self.restart()?; // Authoritative synchronous Frame and declared-camera replacement.
        }
        self.keys.clear();
        self.drag = None;
        self.cursor = None;
        if let Some(ui) = &mut self.ui {
            ui.apply(action);
            ui.acknowledge_restart();
        }
        Ok(())
    }
    #[cfg(feature = "room-ui")]
    fn draw_ui(&mut self) -> Result<(), String> {
        let input = match (&mut self.window_ui, &self.graphics) {
            (Some(window_ui), Some(g)) => window_ui.state.take_egui_input(&g.window),
            _ => return Ok(()),
        };
        for mut output in self.ui_frame(input)? {
            if let (Some(window_ui), Some(g)) = (&mut self.window_ui, &self.graphics) {
                window_ui
                    .state
                    .handle_platform_output(&g.window, std::mem::take(&mut output.platform_output));
                window_ui.pending.push(output)?;
            } else {
                output.drop_without_applying_deltas();
            }
        }
        Ok(())
    }
    fn draw(&mut self) -> Result<(), String> {
        #[cfg(feature = "room-ui")]
        self.draw_ui()?;
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last);
        self.last = now;
        let Some(size) = self.graphics.as_ref().map(|g| g.size) else {
            return Ok(());
        };
        if size.0 == 0 || size.1 == 0 {
            self.accumulated = Duration::ZERO;
            return Ok(());
        }
        if self.focused {
            self.accumulated += elapsed.min(Duration::from_millis(250));
            let interval = Duration::from_secs_f64(1.0 / f64::from(TICK_RATE));
            while self.accumulated >= interval {
                #[cfg(feature = "room-ui")]
                let blocked = self.ui.as_ref().is_some_and(|ui| ui.blocks_controls());
                #[cfg(not(feature = "room-ui"))]
                let blocked = false;
                let input = self.keys.sample();
                self.step_authoritative(if blocked { RoomInput::default() } else { input });
                self.accumulated -= interval;
            }
        } else {
            self.accumulated = Duration::ZERO;
        }
        let g = self
            .graphics
            .as_mut()
            .expect("graphics checked before stepping");
        let run = self.sim.frame().singleton::<RoomRun>();
        g.window.set_title(&format!("Room Escape | key: {} | {} | WASD/arrows move, E interact, R restart, Esc quit | mouse orbit/pan/zoom", run.key_collected, if run.won != 0 { "WON" } else if run.key_collected == 0 { "exit locked" } else { "exit unlocked" }));
        // The backend reconfigures lost/outdated surfaces and returns Skip.
        if let Acquire::Frame(surface_frame) = g.rhi.acquire_frame(&mut g.surface) {
            let view = g.rhi.frame_view(&surface_frame);
            g.renderer.render_to(
                FrameView::of(self.sim.frame()),
                self.project.scene().index(),
                self.project.models(),
                &match self.project.camera() {
                    Some(camera) => camera.document.camera(&self.orbit, g.size)?,
                    None => self.orbit.camera(),
                },
                orr_render::ImportedSceneTarget {
                    view,
                    size: g.size,
                    format: g.format,
                    sample_count: 1,
                },
            )?;
            #[cfg(feature = "room-ui")]
            if let (Some(ui), Some(window_ui)) = (&self.ui, &mut self.window_ui) {
                window_ui
                    .pending
                    .paint(&mut window_ui.gpu, &g.rhi, view, g.size, &ui.context)?;
            }
            g.rhi.present(surface_frame);
        }
        Ok(())
    }
}

#[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
impl App {
    fn checkpoint_ui_frame(
        &mut self,
        input: egui::RawInput,
    ) -> Result<Vec<egui::FullOutput>, String> {
        let checkpoint = self.checkpoint.as_ref().ok_or("checkpoint missing")?;
        let ui = self
            .ui
            .as_mut()
            .ok_or("checkpoint requires authored Room UI")?;
        let menu = ui.screen() == crate::authored_ui::Screen::Menu;
        // The checkpoint chooser owns pointer input; discard authored hit regions.
        ui.invalidate_pointer_layout();
        let context = ui.context.clone();
        let available = checkpoint.available();
        let status = checkpoint.status().to_owned();
        let mut action = None;
        let mut fresh_input = input.clone();
        fresh_input.events.clear();
        let mut output = context.run_ui(input, |root| {
            let width = root.available_width().min(440.0);
            root.add_space(12.0);
            root.horizontal(|row| {
                row.add_space(((row.available_width() - width) * 0.5).max(0.0));
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(16, 22, 32))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::same(12))
                    .show(row, |panel| {
                        let content_width = (width - 24.0).max(1.0);
                        panel.set_min_width(content_width);
                        panel.set_max_width(content_width);
                        panel.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
                        panel.spacing_mut().button_padding = egui::vec2(14.0, 10.0);
                        panel.spacing_mut().item_spacing.y = 7.0;
                        panel.vertical_centered(|root| {
                            root.heading("Room checkpoint");
                            root.label(&status);
                            root.label(
                    "Resume restores the key at the authored spawn. New Game clears the saved key.",
                );
                            if root
                                .add_enabled(available, egui::Button::new("Resume"))
                                .clicked()
                            {
                                action = Some(CheckpointAction::Resume);
                            }
                            if root.button("New Game").clicked() {
                                action = Some(CheckpointAction::NewGame);
                            }
                            if menu && root.button("Continue current session").clicked() {
                                action = Some(CheckpointAction::Continue);
                            }
                            if root
                                .button("Restart session (does not clear checkpoint)")
                                .clicked()
                            {
                                action = Some(CheckpointAction::Restart);
                            }
                            if root.button("Quit").clicked() {
                                action = Some(CheckpointAction::Quit);
                            }
                        });
                    });
            });
        });
        if let Some(action) = action {
            self.apply_checkpoint_action(action)?;
            if !self.quit {
                output.shapes.clear();
                let mut outputs = vec![output];
                outputs.extend(self.ui_frame(fresh_input)?);
                return Ok(outputs);
            }
        }
        Ok(vec![output])
    }
    fn apply_checkpoint_action(&mut self, action: CheckpointAction) -> Result<(), String> {
        match action {
            CheckpointAction::Quit => {
                self.quit = true;
                return Ok(());
            }
            CheckpointAction::NewGame => {
                if !self
                    .checkpoint
                    .as_mut()
                    .ok_or("checkpoint missing")?
                    .new_game()
                {
                    return Ok(());
                }
                self.restart()?;
            }
            CheckpointAction::Resume => {
                if !self.checkpoint.as_ref().is_some_and(|s| s.available()) {
                    return Ok(());
                }
                self.restart()?;
                self.sim = crate::room_checkpoint::restore(&self.project, true)?;
            }
            CheckpointAction::Restart => self.restart()?,
            CheckpointAction::Continue => {}
        }
        self.keys.clear();
        self.drag = None;
        self.cursor = None;
        self.accumulated = Duration::ZERO;
        self.last = Instant::now();
        if let Some(ui) = &mut self.ui {
            ui.apply(if action == CheckpointAction::Continue {
                crate::authored_ui::Action::Continue
            } else {
                crate::authored_ui::Action::Play
            });
            ui.acknowledge_restart();
        }
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let result = (|| {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Room Escape")
                            .with_inner_size(LogicalSize::new(1024.0, 768.0))
                            .with_max_inner_size(LogicalSize::new(2048.0, 2048.0)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let inner = window.inner_size();
            let size = (inner.width.max(1), inner.height.max(1));
            let (rhi, surface) = Wgpu::for_window(
                window.clone(),
                size,
                true,
                WgpuOptions {
                    allow_software_fallback: false,
                    ..Default::default()
                },
            )?;
            let format = rhi.surface_format(&surface);
            let renderer =
                RoomRenderer::new_with_format(&rhi, size, self.project.models(), format)?;
            Ok::<_, String>(Graphics {
                window,
                rhi,
                surface,
                renderer,
                size,
                format,
            })
        })();
        match result {
            Ok(g) => {
                #[cfg(feature = "room-ui")]
                if let Some(ui) = &mut self.ui {
                    ui.invalidate_pointer_layout();
                    self.window_ui = Some(WindowUi {
                        state: egui_winit::State::new(
                            ui.context.clone(),
                            egui::ViewportId::ROOT,
                            g.window.as_ref(),
                            Some(g.window.scale_factor() as f32),
                            g.window.theme(),
                            None,
                        ),
                        gpu: crate::game_ui_gpu::GpuOverlay::new(&g.rhi, g.format),
                        pending: Default::default(),
                    });
                }
                self.focused = g.window.has_focus();
                self.graphics = Some(g);
                self.last = Instant::now();
            }
            Err(e) => self.fail(event_loop, e),
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.keys.clear();
        self.focused = false;
        self.drag = None;
        self.cursor = None;
        self.graphics = None;
        self.accumulated = Duration::ZERO;
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.graphics.as_ref().is_none_or(|g| g.window.id() != id) {
            return;
        }
        #[cfg(feature = "room-ui")]
        match &event {
            WindowEvent::CursorMoved { position, .. } => self.ui_pointer = Some(*position),
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => self.ui_pointer = None,
            _ => {}
        }
        #[cfg(feature = "room-ui")]
        let ui_capture = if let (Some(ui), Some(window_ui), Some(g)) =
            (&mut self.ui, &mut self.window_ui, &self.graphics)
        {
            let response = window_ui.state.on_window_event(&g.window, &event);
            if matches!(
                event,
                WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
            ) {
                ui.invalidate_pointer_layout();
            }
            let pending = ui_pointer_input(
                self.ui_pointer,
                egui_winit::pixels_per_point(&ui.context, &g.window),
            );
            response.consumed
                || ui.blocks_controls()
                || ui.context.egui_wants_keyboard_input()
                || ui.pending_pointer_over_ui(&pending)
        } else {
            false
        };
        #[cfg(not(feature = "room-ui"))]
        let ui_capture = false;
        if ui_capture {
            self.keys.clear();
            self.drag = None;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.keys.clear();
                self.drag = None;
                self.cursor = None;
                self.accumulated = Duration::ZERO;
                self.last = Instant::now();
            }
            WindowEvent::Resized(size) => {
                if size.width > 4096 || size.height > 4096 {
                    self.fail(
                        event_loop,
                        "window exceeds the renderer's 4096-pixel size limit".into(),
                    );
                    return;
                }
                if let Some(g) = &mut self.graphics {
                    g.size = (size.width, size.height);
                    if size.width > 0 && size.height > 0 {
                        g.rhi
                            .resize_surface(&mut g.surface, size.width, size.height);
                    }
                }
                if size.width == 0 || size.height == 0 {
                    self.keys.clear();
                    self.drag = None;
                    self.cursor = None;
                }
                self.last = Instant::now();
                self.accumulated = Duration::ZERO;
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let fresh = self.keys.event(
                        code,
                        event.state == ElementState::Pressed,
                        is_synthetic,
                        event.repeat,
                        self.focused && !ui_capture,
                    );
                    if fresh {
                        match code {
                            KeyCode::Escape => event_loop.exit(),
                            KeyCode::KeyR => {
                                if let Err(e) = self.restart() {
                                    self.fail(event_loop, e);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.focused && !ui_capture => {
                let next = [position.x as f32, position.y as f32];
                if let Some(previous) = self.cursor {
                    let delta = [next[0] - previous[0], next[1] - previous[1]];
                    match self.drag {
                        Some(MouseButton::Left) => {
                            if let Some(camera) = self.project.camera() {
                                camera.document.orbit_delta(&mut self.orbit, delta);
                            } else {
                                self.orbit.orbit(delta);
                            }
                        }
                        Some(MouseButton::Middle | MouseButton::Right) => {
                            let size = self.graphics.as_ref().map_or((1, 1), |g| g.size);
                            if let Some(camera) = self.project.camera() {
                                camera.document.pan(&mut self.orbit, delta, size);
                            } else {
                                self.orbit.pan(delta, size);
                            }
                        }
                        _ => {}
                    }
                }
                self.cursor = Some(next);
            }
            WindowEvent::MouseInput { state, button, .. } if self.focused && !ui_capture => {
                self.drag = if state == ElementState::Pressed {
                    Some(button)
                } else {
                    None
                };
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused && !ui_capture => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if let Some(camera) = self.project.camera() {
                    camera.document.zoom(&mut self.orbit, steps);
                } else {
                    self.orbit.zoom(steps);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    self.fail(event_loop, e);
                }
                #[cfg(feature = "room-ui")]
                if self.quit {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let next = self.last + Duration::from_millis(16);
        if Instant::now() >= next {
            if let Some(g) = &self.graphics {
                g.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            next.max(Instant::now() + Duration::from_millis(1)),
        ));
    }
}
impl App {
    fn new(project: PreparedProject) -> Result<Self, String> {
        let sim = project.scene().simulation()?;
        let orbit = project.camera().map_or_else(
            || OrbitCamera::new([0.0, 0.5, 0.0], 0.65, 1.02, 60.0),
            |camera| camera.document.orbit(),
        );
        #[cfg(feature = "room-ui")]
        let ui = project
            .ui()
            .map(|ui| {
                crate::collect_ui::CollectUi::new_for(
                    ui.document.clone(),
                    ui.font.clone(),
                    crate::authored_ui::Profile::Room,
                )
            })
            .transpose()?;
        Ok(Self {
            #[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
            checkpoint: None,
            #[cfg(feature = "room-ui")]
            ui,
            #[cfg(feature = "room-ui")]
            window_ui: None,
            #[cfg(feature = "room-ui")]
            quit: false,
            #[cfg(feature = "room-ui")]
            ui_pointer: None,
            project,
            sim,
            graphics: None,
            keys: Keys::default(),
            focused: false,
            last: Instant::now(),
            accumulated: Duration::ZERO,
            orbit,
            drag: None,
            cursor: None,
            error: None,
        })
    }
}
pub fn run_window(project: PreparedProject) -> Result<(), String> {
    // Metadata admission is intentionally available to non-persisting tooling;
    // it must not grant support to this public standalone consuming route.
    #[cfg(not(all(feature = "room-checkpoint", target_os = "linux")))]
    if project.checkpoint().is_some() {
        return Err("Room checkpoint runtime requires the Linux room-checkpoint consumer".into());
    }
    let mut app = App::new(project)?;
    #[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
    {
        app.checkpoint = crate::room_checkpoint::CheckpointSession::open(&app.project);
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::WaitUntil(
        Instant::now() + Duration::from_millis(16),
    ));
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn strict_options() {
        assert!(Options::parse(vec![]).is_err());
        assert!(Options::parse(args(&["--project", "x", "--ticks", "0"])).is_err());
        assert!(
            Options::parse(args(&["--project", "x", "--headless", "--ticks", "6001"])).is_err()
        );
        assert!(Options::parse(args(&["--project", "x", "--headless", "--hold", "bot"])).is_err());
        let options = Options::parse(args(&[
            "--project",
            "x",
            "--headless",
            "--ticks",
            "0",
            "--hold",
            "left,right,up,interact",
        ]))
        .unwrap();
        assert_eq!(options.ticks, 0);
        assert_eq!(options.held.move_x, 0);
        assert_eq!(options.held.move_z, -1);
        assert_eq!(options.held.buttons, INTERACT);
    }
    #[test]
    fn focus_synthetic_and_restart_withhold_keys() {
        let mut keys = Keys::default();
        assert!(keys.event(KeyCode::KeyE, true, false, false, true));
        assert_eq!(keys.sample().buttons, INTERACT);
        keys.clear();
        assert_eq!(keys.sample(), RoomInput::default());
        assert!(!keys.event(KeyCode::KeyE, true, false, true, true));
        assert_eq!(keys.sample().buttons, 0);
        keys.event(KeyCode::KeyE, false, false, false, true);
        assert!(!keys.event(KeyCode::KeyE, true, true, false, true));
        assert_eq!(keys.sample().buttons, 0);
        keys.event(KeyCode::KeyE, false, false, false, true);
        assert!(keys.event(KeyCode::KeyE, true, false, false, true));
        assert_eq!(keys.sample().buttons, INTERACT);
    }
    #[test]
    fn reset_inserts_neutral_tick_before_new_press() {
        let mut keys = Keys::default();
        keys.clear();
        keys.event(KeyCode::KeyW, true, false, false, true);
        assert_eq!(keys.sample(), RoomInput::default());
        assert_eq!(keys.sample().move_z, -1);
    }
}

#[cfg(all(
    test,
    feature = "room-ui",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod room_ui_app_tests {
    use super::*;
    use crate::authored_ui::{Action, Screen};
    fn render(app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let outputs = app.ui_frame(input).unwrap();
        if outputs.len() == 2 {
            assert!(
                outputs[0].shapes.is_empty(),
                "pre-action geometry must not be painted"
            );
            if app.ui.as_ref().unwrap().screen() == Screen::Playing {
                fn text(shape: &egui::Shape, found: &mut String) {
                    match shape {
                        egui::Shape::Text(t) => found.push_str(t.galley.text()),
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                text(shape, found);
                            }
                        }
                        _ => {}
                    }
                }
                let mut visible = String::new();
                for shape in &outputs[1].shapes {
                    text(&shape.shape, &mut visible);
                }
                assert!(
                    visible.contains("메뉴"),
                    "Playing output must be regenerated after action: {visible}"
                );
                assert!(
                    !visible.contains("다시 시작"),
                    "old terminal restart button leaked into new Frame"
                );
            }
        }
        for output in outputs {
            output.drop_without_applying_deltas();
        }
    }
    fn click(app: &mut App, id: &str) {
        render(app, vec![]);
        let ui = app.ui.as_ref().unwrap();
        let node = ui
            .document()
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap();
        let point = egui::pos2(
            800.0 * f32::from(node.anchor[0]) / 1000.0
                + f32::from(node.offset[0])
                + f32::from(node.size[0]) / 2.0,
            f32::from(node.offset[1]) + f32::from(node.size[1]) / 2.0,
        );
        let event = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        render(app, vec![egui::Event::PointerMoved(point), event(true)]);
        render(app, vec![event(false)]);
    }
    #[test]
    fn native_app_widget_restart_uses_owned_frame_camera_and_neutral_controls() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("room");
        crate::project_create::create(&crate::project_create::CreateOptions {
            output: root.clone(),
            template: crate::project_create::ROOM_UI_TEMPLATE.into(),
            seed: "native-app-ui".into(),
        })
        .unwrap();
        let prepared = PreparedProject::open_with_ui(&root, true).unwrap();
        let initial = prepared.scene().frame().checksum();
        let declared = prepared.camera().unwrap().document.clone();
        let mut app = App::new(prepared).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Title);
        click(&mut app, "play");
        assert_eq!(app.sim.frame().checksum(), initial);
        assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
        assert!(app.keys.event(KeyCode::KeyE, true, false, false, true));
        click(&mut app, "menu");
        assert_eq!(app.keys.sample(), RoomInput::default());
        assert!(app.ui.as_ref().unwrap().blocks_controls());
        assert!(!app.keys.event(KeyCode::KeyE, true, false, true, true));
        click(&mut app, "continue");
        assert_eq!(app.keys.sample().buttons, 0);
        app.keys.event(KeyCode::KeyE, false, false, false, true);
        assert!(app.keys.event(KeyCode::KeyE, true, false, false, true));
        app.orbit.yaw += 0.5;
        app.sim.frame_mut().singleton_mut::<RoomRun>().key_collected = 1;
        app.sim.frame_mut().singleton_mut::<RoomRun>().won = 1;
        render(&mut app, vec![]);
        assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Terminal);
        click(&mut app, "restart");
        assert_eq!(app.sim.frame().checksum(), initial);
        assert_eq!(app.orbit.yaw, declared.orbit().yaw);
        assert_eq!(app.keys.sample(), RoomInput::default());
        assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Playing);
        assert!(!app.quit);
        app.apply_ui_action(Action::Quit).unwrap();
        assert!(app.quit);
    }
}

#[cfg(all(
    test,
    feature = "room-ui",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod room_ui_gpu_tests {
    use super::*;
    #[test]
    #[ignore = "mandatory software/physical GPU adapter and explicit capture directory"]
    fn room_ui_composited_title_play_key_win_and_projection_preserve_frame() {
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        let output =
            PathBuf::from(std::env::var_os("ORR_ROOM_UI_CAPTURE_DIR").expect("capture directory"));
        std::fs::create_dir_all(&output).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("gpu room");
        crate::project_create::create(&crate::project_create::CreateOptions {
            output: root.clone(),
            template: crate::project_create::ROOM_UI_TEMPLATE.into(),
            seed: "room-ui-gpu".into(),
        })
        .unwrap();
        let project = PreparedProject::open_with_ui(&root, true).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let gpu = Wgpu::headless(WgpuOptions::default()).unwrap();
        let prepared = project.ui().unwrap();
        for size in [(1024, 768), (480, 800)] {
            let mut renderer = RoomRenderer::new(&gpu, size, project.models()).unwrap();
            let camera = project
                .camera()
                .unwrap()
                .document
                .camera(&project.camera().unwrap().document.orbit(), size)
                .unwrap();
            let mut sim = project.scene().simulation().unwrap();
            let mut ui = crate::collect_ui::CollectUi::new_for(
                prepared.document.clone(),
                prepared.font.clone(),
                crate::authored_ui::Profile::Room,
            )
            .unwrap();
            let mut overlay = crate::game_ui_gpu::GpuOverlay::new(&gpu, room_view::TARGET_FORMAT);
            for (name, hud) in [
                ("title", crate::collect_ui::RoomHud::default()),
                ("playing", crate::collect_ui::RoomHud::default()),
                (
                    "key",
                    crate::collect_ui::RoomHud {
                        key_acquired: true,
                        won: false,
                    },
                ),
                (
                    "won",
                    crate::collect_ui::RoomHud {
                        key_acquired: true,
                        won: true,
                    },
                ),
            ] {
                if name == "playing" {
                    ui.apply(crate::authored_ui::Action::Continue);
                }
                {
                    let run = sim.frame_mut().singleton_mut::<RoomRun>();
                    run.key_collected = u32::from(hud.key_acquired);
                    run.won = u32::from(hud.won);
                }
                let frame = sim.frame();
                let initial = frame.checksum();
                renderer
                    .render(
                        FrameView::of(frame),
                        project.scene().index(),
                        project.models(),
                        &camera,
                    )
                    .unwrap();
                let baseline = renderer.read_rgba8();
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size.0 as f32, size.1 as f32),
                    )),
                    ..Default::default()
                };
                let (paint, action) = ui.show_room(raw, hud);
                assert!(action.is_none());
                overlay
                    .paint(
                        &gpu,
                        renderer.target().render_view(),
                        size,
                        &ui.context,
                        paint,
                    )
                    .unwrap();
                let pixels = renderer.read_rgba8();
                let changed = pixels
                    .chunks_exact(4)
                    .zip(baseline.chunks_exact(4))
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(
                    changed > 400,
                    "real overlay pixels absent: {name} {changed}"
                );
                assert_eq!(frame.checksum(), initial);
                let path = output.join(format!("{name}-{}x{}.png", size.0, size.1));
                let file = std::fs::File::create(path).unwrap();
                let mut encoder = png17::Encoder::new(file, size.0, size.1);
                encoder.set_color(png17::ColorType::Rgba);
                encoder.set_depth(png17::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&pixels)
                    .unwrap();
            }
        }
    }
}

#[cfg(all(test, feature = "room-ui"))]
mod room_pointer_capture_tests {
    use super::*;
    #[test]
    fn key_capture_uses_physical_position_after_scale_without_cursor_move() {
        use crate::authored_ui::{Document, Kind, Node, Profile, Screen};
        let document = Document {
            schema: 1,
            nodes: vec![Node {
                id: "panel".into(),
                parent: None,
                kind: Kind::Container,
                screen: Screen::Title,
                anchor: [0, 0],
                offset: [100, 100],
                size: [100, 100],
            }],
        };
        let mut ui = crate::collect_ui::CollectUi::new_for(
            document,
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
            Profile::Room,
        )
        .unwrap();
        let physical = winit::dpi::PhysicalPosition::new(300.0, 300.0);
        let mut raw = ui_pointer_input(Some(physical), 1.0);
        raw.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        ));
        ui.show_room(raw, Default::default())
            .0
            .drop_without_applying_deltas();
        let mut resized = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        resized
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(2.0);
        // The scale/redraw does not generate a physical CursorMoved.
        ui.show_room(resized, Default::default())
            .0
            .drop_without_applying_deltas();
        assert!(ui.pending_pointer_over_ui(&ui_pointer_input(Some(physical), 2.0)));
        assert!(!ui.pending_pointer_over_ui(&ui_pointer_input(Some(physical), 1.0)));
        let mut keys = Keys::default();
        let captured = ui.pending_pointer_over_ui(&ui_pointer_input(Some(physical), 2.0));
        assert!(!keys.event(KeyCode::KeyE, true, false, false, !captured));
        assert_eq!(keys.sample().buttons, 0);
        keys.event(KeyCode::KeyE, false, false, false, true);
        assert!(keys.event(KeyCode::KeyE, true, false, false, true));
        assert_eq!(keys.sample().buttons, INTERACT);
    }
}

#[cfg(all(
    test,
    feature = "room-checkpoint",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[path = "room_checkpoint_acceptance_tests.rs"]
mod room_checkpoint_acceptance_tests;
