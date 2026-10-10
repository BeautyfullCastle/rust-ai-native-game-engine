//! Standalone, local-only authored CollectDodgeV1 player.
use crate::project_compositor::ProjectWindow;
use crate::{
    collect_game::{self as game, CollectDodgeV1, CollectInput},
    collect_project::PreparedProject,
    collect_view,
};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_fp::FP;
use orr_render::orr_rhi::Wgpu;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

pub const HELP: &str = "collect_dodge --project DIR [--headless --ticks 0..6000 [--hold right,up] [--capture NEW.png]]\nAudio: --audio-render-check proves real offline PCM in headless mode. --audio off|auto|required (default off; native output requires collect-audio-native).\nWindow: WASD/arrows move, Space restarts on a fresh press, Escape quits.\nHeadless directions: left,right,up,down,restart. Capture requires a software GPU adapter.";
#[derive(Debug)]
pub struct Options {
    pub project: PathBuf,
    pub headless: bool,
    pub ticks: u32,
    pub held: CollectInput,
    pub capture: Option<PathBuf>,
    pub audio: crate::arena_audio::AudioMode,
    pub audio_render_check: bool,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let (mut project, mut ticks, mut hold, mut capture) = (None, None, None, None);
        let mut headless = false;
        let mut audio = crate::arena_audio::AudioMode::Off;
        let mut audio_render_check = false;
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            if !seen.insert(arg.clone()) {
                return Err(format!("duplicate option: {arg}"));
            }
            match arg.as_str() {
                "--project" => {
                    project = Some(PathBuf::from(args.next().ok_or("--project requires DIR")?))
                }
                "--headless" => headless = true,
                "--audio-render-check" => audio_render_check = true,
                "--audio" => audio = args.next().ok_or("--audio requires off|auto|required")?.parse()?,
                "--ticks" => {
                    ticks = Some(
                        args.next()
                            .ok_or("--ticks requires N")?
                            .parse::<u32>()
                            .map_err(|_| "invalid --ticks")?,
                    )
                }
                "--hold" => {
                    hold = Some(parse_hold(
                        &args.next().ok_or("--hold requires directions")?,
                    )?)
                }
                "--capture" => {
                    capture = Some(PathBuf::from(
                        args.next().ok_or("--capture requires NEW.png")?,
                    ))
                }
                _ => return Err(format!("unknown option: {arg}\n{HELP}")),
            }
        }
        if !headless && (ticks.is_some() || hold.is_some() || capture.is_some()) {
            return Err("--ticks, --hold and --capture require --headless".into());
        }
        if audio_render_check && (!headless || !cfg!(feature="collect-audio")) { return Err("--audio-render-check needs headless and collect-audio".into()); }
        if headless && audio != crate::arena_audio::AudioMode::Off { return Err("headless audio policy must be off (no device)".into()); }
        if !cfg!(feature="collect-audio") && audio != crate::arena_audio::AudioMode::Off { return Err("Collect audio support is not built".into()); }
        if headless && ticks.is_none() {
            return Err("--headless requires explicit --ticks 0..6000".into());
        }
        let ticks = ticks.unwrap_or(0);
        if ticks > 6000 {
            return Err("--ticks must be between 0 and 6000".into());
        }
        if capture
            .as_ref()
            .is_some_and(|p| p.extension().and_then(|e| e.to_str()) != Some("png"))
        {
            return Err("--capture must end in .png".into());
        }
        Ok(Self {
            project: project.ok_or("--project DIR is required")?,
            headless,
            ticks,
            held: hold.unwrap_or_default(),
            capture,
            audio,
            audio_render_check,
        })
    }
}
fn parse_hold(value: &str) -> Result<CollectInput, String> {
    let mut seen = BTreeSet::new();
    for key in value.split(',') {
        if !matches!(key, "left" | "right" | "up" | "down" | "restart") || !seen.insert(key) {
            return Err(format!("invalid or duplicate held key: {key}"));
        }
    }
    if (seen.contains("left") && seen.contains("right"))
        || (seen.contains("up") && seen.contains("down"))
    {
        return Err("conflicting held directions".into());
    }
    Ok(CollectInput {
        x: axis(seen.contains("left"), seen.contains("right")),
        y: axis(seen.contains("down"), seen.contains("up")),
        buttons: if seen.contains("restart") {
            game::RESTART
        } else {
            0
        },
        ..Default::default()
    })
}
fn axis(negative: bool, positive: bool) -> FP {
    FP::from_int(i32::from(positive) - i32::from(negative))
}
#[cfg(feature = "collect-ui")]
fn pointer_in_ui(
    position: winit::dpi::PhysicalPosition<f64>,
    pixels_per_point: f32,
) -> Option<egui::Pos2> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }
    let point = egui::pos2(
        (position.x / f64::from(pixels_per_point)) as f32,
        (position.y / f64::from(pixels_per_point)) as f32,
    );
    point.is_finite().then_some(point)
}
type LocalBridge = InProc<CollectDodgeV1, PlayHost<CollectDodgeV1>>;
fn bridge(project: &PreparedProject) -> Result<LocalBridge, String> {
    Ok(InProc::new(
        PlayHost::new(project.scene().session()?, PlayerSlot(0)),
        BridgeConfig::default(),
    ))
}
pub fn run(options: Options) -> Result<(), String> {
    // All package verification and scene admission precede any GPU/window work.
    let support = if cfg!(all(feature = "collect-progress", target_os = "linux")) {
        crate::collect_project::ProgressSupport::MetadataOnly
    } else {
        crate::collect_project::ProgressSupport::Unsupported
    };
    let project = PreparedProject::open_with_audio(
        &options.project,
        support,
        crate::collect_project::compiled_sprite_support(),
        cfg!(feature = "collect-ui"),
        cfg!(feature = "collect-audio"),
    )?;
    if options.headless {
        headless_with_audio_check(
            &project,
            options.ticks,
            options.held,
            options.capture.as_deref(),
            options.audio_render_check,
        )
    } else {
        run_window_with_audio(project, options.audio)
    }
}

// At most eight fresh restart taps await simulation delivery. Excess taps
// coalesce at the cap; focus loss cancels all undelivered taps.
const MAX_PENDING_RESTARTS: u8 = 8;
#[derive(Default)]
struct Keys {
    pressed: BTreeSet<KeyCode>,
    blocked: BTreeSet<KeyCode>,
    focused: bool,
    pending_restarts: u8,
    observed_restart_held: bool,
    observed_tick: u64,
}
impl Keys {
    #[cfg(feature = "collect-ui")]
    fn ui_action(&mut self, action: crate::authored_ui::Action) {
        // Navigation neutralizes movement without dropping an authorized edge.
        self.blocked.append(&mut self.pressed);
        if matches!(
            action,
            crate::authored_ui::Action::Play | crate::authored_ui::Action::Restart
        ) {
            self.pending_restarts = self
                .pending_restarts
                .saturating_add(1)
                .min(MAX_PENDING_RESTARTS);
        }
    }
    fn focus(&mut self, focused: bool) {
        if !focused {
            self.blocked.append(&mut self.pressed);
            self.pending_restarts = 0;
        }
        self.focused = focused;
    }
    fn captured_event(
        &mut self,
        code: KeyCode,
        down: bool,
        repeat: bool,
        synthetic: bool,
        capture: bool,
    ) {
        if capture && down {
            self.blocked.insert(code);
        }
        self.event(code, down, repeat, synthetic);
    }
    fn event(&mut self, code: KeyCode, down: bool, repeat: bool, synthetic: bool) {
        if !down {
            self.pressed.remove(&code);
            self.blocked.remove(&code);
            return;
        }
        if !self.focused || synthetic {
            self.blocked.insert(code);
            return;
        }
        if !repeat
            && !self.blocked.contains(&code)
            && self.pressed.insert(code)
            && code == KeyCode::Space
        {
            self.pending_restarts = self
                .pending_restarts
                .saturating_add(1)
                .min(MAX_PENDING_RESTARTS);
        }
    }
    // Only an authoritative advanced tick acknowledges delivery. A render/update
    // that simulated zero ticks must not consume an edge or a neutral rearm.
    fn observe(&mut self, tick: u64, restart_held: bool) {
        if tick <= self.observed_tick {
            return;
        }
        if restart_held && !self.observed_restart_held {
            self.pending_restarts = self.pending_restarts.saturating_sub(1);
        }
        self.observed_tick = tick;
        self.observed_restart_held = restart_held;
    }
    fn input(&self) -> CollectInput {
        if !self.focused {
            return CollectInput::default();
        }
        let any = |a, b| self.pressed.contains(&a) || self.pressed.contains(&b);
        CollectInput {
            x: axis(
                any(KeyCode::KeyA, KeyCode::ArrowLeft),
                any(KeyCode::KeyD, KeyCode::ArrowRight),
            ),
            y: axis(
                any(KeyCode::KeyS, KeyCode::ArrowDown),
                any(KeyCode::KeyW, KeyCode::ArrowUp),
            ),
            buttons: if if self.pending_restarts > 0 {
                // Separate queued presses with a simulation-observed low sample.
                !self.observed_restart_held
            } else {
                self.pressed.contains(&KeyCode::Space)
            } {
                game::RESTART
            } else {
                0
            },
            ..Default::default()
        }
    }
}
#[cfg(all(feature = "collect-progress", target_os = "linux"))]
type WindowBridge = InProc<CollectDodgeV1, crate::collect_progress_host::ProgressHost>;
#[cfg(not(all(feature = "collect-progress", target_os = "linux")))]
type WindowBridge = LocalBridge;
struct App {
    #[cfg(feature="collect-audio")]
    audio: Option<crate::collect_audio_output::Playback>,
    bridge: WindowBridge,
    #[cfg(all(feature = "collect-progress", target_os = "linux"))]
    progress: crate::collect_progress::ProgressSession,
    gfx: Option<(Arc<Window>, ProjectWindow)>,
    presentation: crate::project_runtime::ProjectPresentation,
    keys: Keys,
    last: Instant,
    error: Option<String>,
    #[cfg(feature = "collect-ui")]
    ui: Option<crate::collect_ui::CollectUi>,
    #[cfg(feature = "collect-ui")]
    window_ui: Option<WindowUi>,
    #[cfg(feature = "collect-ui")]
    ui_pointer: Option<winit::dpi::PhysicalPosition<f64>>,
}
#[cfg(feature = "collect-ui")]
struct WindowUi {
    state: egui_winit::State,
    gpu: crate::game_ui_gpu::GpuOverlay,
    pending: crate::game_ui_gpu::PendingOverlay,
}
impl App {
    #[cfg(all(feature = "collect-progress", target_os = "linux"))]
    fn persist_completed(&mut self) {
        self.progress
            .observe_completed(self.bridge.host().completed_best());
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last;
        self.last = now;
        #[cfg(feature = "collect-ui")]
        if let (Some(ui), Some(window_ui), Some((window, _))) =
            (&mut self.ui, &mut self.window_ui, &self.gfx)
        {
            let input = window_ui.state.take_egui_input(window);
            let capture = ui.blocks_controls()
                || ui.pending_pointer_over_ui(&input)
                || ui.context.egui_wants_keyboard_input();
            if capture {
                self.keys.blocked.append(&mut self.keys.pressed);
            }
            let snapshot = self.bridge.snapshot();
            let run = snapshot
                .as_ref()
                .map(|s| {
                    let run = s.predicted().singleton::<game::CollectRun>();
                    (run.score, run.phase)
                })
                .unwrap_or((0, game::PLAYING));
            #[cfg(all(feature = "collect-progress", target_os = "linux"))]
            let best = self
                .progress
                .best()
                .map_or_else(|| "-".into(), |v| v.to_string());
            #[cfg(not(all(feature = "collect-progress", target_os = "linux")))]
            let best = "-".into();
            let (mut output, action) = ui.show(
                input,
                crate::collect_ui::Hud {
                    score: run.0,
                    phase: run.1,
                    best,
                },
            );
            window_ui
                .state
                .handle_platform_output(window, std::mem::take(&mut output.platform_output));
            if let Err(error) = window_ui.pending.push(output) {
                self.fail(event_loop, error);
                return;
            }
            if let Some(action) = action {
                if action == crate::authored_ui::Action::Quit {
                    event_loop.exit();
                    return;
                }
                self.keys.ui_action(action);
                ui.apply(action);
            }
        }
        if let Err(e) = self.bridge.set_input(PlayerSlot(0), self.keys.input()) {
            self.fail(event_loop, e.to_string());
            return;
        }
        self.bridge.update(dt);
        #[cfg(all(feature = "collect-progress", target_os = "linux"))]
        self.persist_completed();
        let update = self.bridge.poll_view(); // Drain events, including after terminal states.
        #[cfg(feature="collect-audio")]
        if let Some(audio) = &mut self.audio {
            if let Err(error) = audio.update(0, &update, |event| event.kind == game::EVENT_COLLECTED) { self.fail(event_loop, error); return; }
        }
        if let Some(snapshot) = &update.snapshot {
            #[cfg(feature = "collect-ui")]
            if snapshot.tick() > self.keys.observed_tick
                && snapshot
                    .predicted()
                    .singleton::<game::CollectRun>()
                    .restart_held
                    != 0
                && !self.keys.observed_restart_held
            {
                if let Some(ui) = &mut self.ui {
                    ui.acknowledge_restart();
                }
            }
            self.keys.observe(
                snapshot.tick(),
                snapshot
                    .predicted()
                    .singleton::<game::CollectRun>()
                    .restart_held
                    != 0,
            );
        }
        if let (Some(snapshot), Some((window, renderer))) = (update.snapshot, self.gfx.as_mut()) {
            let frame = snapshot.predicted();
            if let Err(error) = self.presentation.update(Some(&snapshot)) {
                self.fail(event_loop, error);
                return;
            }
            let title = collect_view::title(frame);
            #[cfg(all(feature = "collect-progress", target_os = "linux"))]
            let title = format!("{title} | {}", self.progress.status());
            window.set_title(&title);
            #[cfg(feature = "collect-ui")]
            let result = if let (Some(ui), Some(window_ui)) = (&self.ui, &mut self.window_ui) {
                renderer.render_with_overlay(
                    self.presentation.shapes(),
                    self.presentation.sprites(),
                    &self.presentation.camera,
                    |gpu, view, size| {
                        window_ui
                            .pending
                            .paint(&mut window_ui.gpu, gpu, view, size, &ui.context)
                    },
                )
            } else {
                renderer.render(
                    self.presentation.shapes(),
                    self.presentation.sprites(),
                    &self.presentation.camera,
                )
            };
            #[cfg(not(feature = "collect-ui"))]
            let result = renderer.render(
                self.presentation.shapes(),
                self.presentation.sprites(),
                &self.presentation.camera,
            );
            if let Err(error) = result {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("CollectDodgeV1")
            .with_inner_size(LogicalSize::new(900.0, 900.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.fail(event_loop, e.to_string());
                return;
            }
        };
        match ProjectWindow::new(window.clone(), true, self.presentation.assets()) {
            Ok(renderer) => {
                println!("collect adapter: {}", renderer.adapter_name());
                self.keys.focus(window.has_focus());
                #[cfg(feature = "collect-ui")]
                if let Some(ui) = &mut self.ui {
                    ui.invalidate_pointer_layout();
                    self.window_ui = Some(WindowUi {
                        state: egui_winit::State::new(
                            ui.context.clone(),
                            egui::ViewportId::ROOT,
                            window.as_ref(),
                            Some(window.scale_factor() as f32),
                            window.theme(),
                            None,
                        ),
                        gpu: crate::game_ui_gpu::GpuOverlay::new(renderer.rhi(), renderer.format()),
                        pending: Default::default(),
                    });
                }
                self.gfx = Some((window, renderer));
                self.last = Instant::now();
            }
            Err(e) => self.fail(event_loop, e),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        #[cfg(feature = "collect-ui")]
        if let (Some(window_ui), Some((window, _))) = (&mut self.window_ui, &self.gfx) {
            let response = window_ui.state.on_window_event(window, &event);
            if response.consumed {
                self.keys.blocked.append(&mut self.keys.pressed);
            }
        }
        #[cfg(feature = "collect-ui")]
        if matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Focused(false)
        ) {
            if let Some(ui) = &mut self.ui {
                ui.invalidate_pointer_layout();
            }
        }
        #[cfg(feature = "collect-ui")]
        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                self.ui_pointer = Some(*position);
            }
            WindowEvent::CursorLeft { .. } => self.ui_pointer = None,
            _ => {}
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                #[cfg(feature = "collect-ui")]
                if !focused {
                    if let Some(ui) = &mut self.ui {
                        ui.acknowledge_restart();
                    }
                }
                self.keys.focus(focused);
                if let Err(e) = self.bridge.set_input(PlayerSlot(0), self.keys.input()) {
                    self.fail(event_loop, e.to_string());
                }
            }
            WindowEvent::Resized(size) => {
                if let Some((_, renderer)) = &mut self.gfx {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    if code == KeyCode::Escape && down && !is_synthetic && self.keys.focused {
                        event_loop.exit();
                    }
                    #[cfg(feature = "collect-ui")]
                    let capture = self.ui.as_ref().is_some_and(|ui| {
                        let mut pending = egui::RawInput::default();
                        let scale = self
                            .gfx
                            .as_ref()
                            .map_or(ui.context.pixels_per_point(), |(window, _)| {
                                egui_winit::pixels_per_point(&ui.context, window)
                            });
                        let point = self
                            .ui_pointer
                            .and_then(|position| pointer_in_ui(position, scale));
                        pending.events.push(
                            point.map_or(egui::Event::PointerGone, egui::Event::PointerMoved),
                        );
                        ui.blocks_controls()
                            || ui.context.egui_wants_keyboard_input()
                            || ui.pending_pointer_over_ui(&pending)
                    });
                    #[cfg(not(feature = "collect-ui"))]
                    let capture = false;
                    self.keys
                        .captured_event(code, down, event.repeat, is_synthetic, capture);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some((window, _)) = &self.gfx {
            window.request_redraw();
        }
    }
}
fn window_app(project: &PreparedProject) -> Result<App, String> {
    #[cfg(not(all(feature = "collect-progress", target_os = "linux")))]
    let bridge = bridge(project)?;
    #[cfg(all(feature = "collect-progress", target_os = "linux"))]
    let bridge = InProc::new(
        crate::collect_progress_host::ProgressHost::new(PlayHost::new(
            project.scene().session()?,
            PlayerSlot(0),
        )),
        BridgeConfig::default(),
    );
    #[cfg(all(feature = "collect-progress", target_os = "linux"))]
    let progress = crate::collect_progress::ProgressSession::open(project);
    Ok(App {
        #[cfg(feature="collect-audio")]
        audio: project.audio().cloned().map(|p| crate::collect_audio_output::Playback::open(p,crate::arena_audio::AudioMode::Off)).transpose()?,
        bridge,
        #[cfg(all(feature = "collect-progress", target_os = "linux"))]
        progress,
        gfx: None,
        presentation: project.presentation(),
        keys: Keys::default(),
        last: Instant::now(),
        error: None,
        #[cfg(feature = "collect-ui")]
        ui: project
            .ui()
            .map(|p| crate::collect_ui::CollectUi::new(p.document.clone(), p.font.clone()))
            .transpose()?,
        #[cfg(feature = "collect-ui")]
        window_ui: None,
        #[cfg(feature = "collect-ui")]
        ui_pointer: None,
    })
}
pub fn run_window(project: PreparedProject) -> Result<(), String> {
    run_window_with_audio(project, crate::arena_audio::AudioMode::Off)
}
fn run_window_with_audio(project: PreparedProject, mode: crate::arena_audio::AudioMode) -> Result<(), String> {
    let mut app = window_app(&project)?;
    #[cfg(feature="collect-audio")]
    {
        app.audio = project.audio().cloned().map(|p| crate::collect_audio_output::Playback::open(p,mode)).transpose()?;
        if let Some(audio) = &app.audio { eprintln!("Collect audio: {}",audio.status()); }
        else if mode == crate::arena_audio::AudioMode::Required { return Err("required audio needs an authored pickup cue".into()); }
        else if mode == crate::arena_audio::AudioMode::Auto { eprintln!("Collect audio: unavailable: no authored pickup cue"); }
    }
    #[cfg(not(feature="collect-audio"))]
    if mode != crate::arena_audio::AudioMode::Off { return Err("Collect audio support is not built".into()); }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}
pub fn headless(
    project: &PreparedProject,
    ticks: u32,
    held: CollectInput,
    capture: Option<&Path>,
) -> Result<(), String> {
    headless_with_audio_check(project,ticks,held,capture,false)
}
fn headless_with_audio_check(project: &PreparedProject, ticks:u32, held:CollectInput, capture:Option<&Path>, audio_render_check:bool) -> Result<(),String> {
    if ticks > 6000 {
        return Err("--ticks must be between 0 and 6000".into());
    }
    #[cfg(feature="collect-audio")]
    let mut audio = if audio_render_check { Some(crate::collect_audio_output::Playback::offline(project.audio().cloned().ok_or("audio render check requires an authored pickup cue")?)?) } else { None };
    #[cfg(feature="collect-audio")]
    let (mut audio_peak, mut audio_frames) = (0.0f32, 0u64);
    #[cfg(not(feature="collect-audio"))]
    if audio_render_check { return Err("Collect audio support is not built".into()); }
    let mut bridge = bridge(project)?;
    let initial = bridge.snapshot().ok_or("missing initial snapshot")?;
    let mut presentation = project.presentation();
    presentation.update(Some(&initial))?;
    println!(
        "collect initial checksum: 0x{:016x}",
        initial.predicted().checksum()
    );
    bridge
        .set_input(PlayerSlot(0), held)
        .map_err(|e| e.to_string())?;
    for _ in 0..ticks {
        bridge.step(1);
        let update = bridge.poll_view();
        #[cfg(feature="collect-audio")]
        if let Some(audio) = &mut audio {
            audio.update(0,&update,|event|event.kind==game::EVENT_COLLECTED)?;
            let mut pcm = [0.0;1600];
            audio.render(&mut pcm)?;
            for sample in pcm { if !sample.is_finite() || sample.abs()>1.0 { return Err("offline Kira output invalid".into()); } audio_peak=audio_peak.max(sample.abs()); }
            audio_frames += 800;
        }
        presentation.update(update.snapshot.as_ref())?;
    }
    let snapshot = bridge.snapshot().ok_or("missing snapshot")?;
    let frame = snapshot.predicted();
    println!(
        "collect tick: {} checksum: 0x{:016x}\n{}",
        snapshot.tick(),
        frame.checksum(),
        collect_view::title(frame)
    );
    #[cfg(feature="collect-audio")]
    if let Some(audio) = &audio {
        if audio.stats().started==0 || audio_peak==0.0 { return Err("audio render check observed no audible authoritative pickup".into()); }
        println!("collect audio: offline Kira; device=none; pickups={}; frames={audio_frames}; peak={audio_peak:.8}",audio.stats().started);
    }
    if let Some(path) = capture {
        use orr_render::orr_rhi::{Rhi, TextureFormat, WgpuOptions};
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        })?;
        if !gpu.is_software() {
            return Err("capture requires a verified software GPU adapter".into());
        }
        println!(
            "collect capture adapter: {} (software: true)",
            gpu.adapter_name()
        );
        let size = (1024, 1024);
        let target =
            orr_render::OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
        let mut compositor = crate::project_compositor::ProjectCompositor::new(
            gpu.clone(),
            TextureFormat::Rgba8UnormSrgb,
            presentation.assets(),
        )?;
        compositor.draw(
            target.render_view(),
            size,
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )?;
        #[cfg(feature = "collect-ui")]
        if let Some(prepared) = project.ui() {
            let mut ui = crate::collect_ui::CollectUi::new(
                prepared.document.clone(),
                prepared.font.clone(),
            )?;
            ui.apply(crate::authored_ui::Action::Play);
            let run = frame.singleton::<game::CollectRun>();
            let mut overlay =
                crate::game_ui_gpu::GpuOverlay::new(&gpu, TextureFormat::Rgba8UnormSrgb);
            // Settle egui layout/font atlas through the identical runtime document path.
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size.0 as f32, size.1 as f32),
                    )),
                    ..Default::default()
                };
                let (output, _) = ui.show(
                    input,
                    crate::collect_ui::Hud {
                        score: run.score,
                        phase: run.phase,
                        best: "-".into(),
                    },
                );
                compositor.draw_with_overlay(
                    target.render_view(),
                    size,
                    presentation.shapes(),
                    presentation.sprites(),
                    &presentation.camera,
                    |gpu, view, size| overlay.paint(gpu, view, size, &ui.context, output),
                )?;
            }
        }
        let rgba = target.read_rgba8();
        let file = std::fs::File::create_new(path)
            .map_err(|e| format!("capture {}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(all(test, feature = "collect-progress", target_os = "linux"))]
pub(crate) fn exercise_window_progress(project: &PreparedProject, mode: &str) {
    // Exact production App constructor and persistence dispatch; no OS window
    // is claimed by this source-hidden test harness.
    let mut app = window_app(project).unwrap();
    if mode == "untrusted-ancestor" {
        assert_eq!(
            app.progress.status(),
            "progress ancestor must be trusted and not writable by other users"
        );
        return;
    }
    if mode == "relaunch" {
        assert_eq!(app.progress.status(), "Best collected: 2");
        return;
    }
    assert_eq!(mode, "win");
    assert_eq!(app.progress.status(), "Best collected: 0");
    app.bridge
        .set_input(
            PlayerSlot(0),
            CollectInput {
                x: FP::ONE,
                ..Default::default()
            },
        )
        .unwrap();
    app.bridge.step(20);
    assert_eq!(app.bridge.host().completed_best(), Some(2));
    let restart = CollectInput {
        buttons: game::RESTART,
        ..Default::default()
    };
    #[cfg(feature = "collect-ui")]
    let restart = if let Some(ui) = &mut app.ui {
        use crate::authored_ui::{Action, Kind, Screen};
        ui.apply(Action::Play);
        let hud = crate::collect_ui::Hud {
            score: 2,
            phase: game::WON,
            best: "0".into(),
        };
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 900.0),
            )),
            ..Default::default()
        };
        let (mut output, _) = ui.show(input(), hud.clone());
        output.textures_delta.clear();
        let node = ui
            .document()
            .nodes
            .iter()
            .find(|n| {
                n.screen == Screen::Terminal
                    && matches!(
                        n.kind,
                        Kind::Button {
                            action: Action::Restart,
                            ..
                        }
                    )
            })
            .expect("authored terminal Restart");
        assert!(
            node.parent.is_none(),
            "test fixture uses a root restart control"
        );
        let pos = egui::pos2(
            900.0 * f32::from(node.anchor[0]) / 1000.0
                + f32::from(node.offset[0])
                + f32::from(node.size[0]) / 2.0,
            900.0 * f32::from(node.anchor[1]) / 1000.0
                + f32::from(node.offset[1])
                + f32::from(node.size[1]) / 2.0,
        );
        let mut click = input();
        for pressed in [true, false] {
            click.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        let (mut output, action) = ui.show(click, hud);
        output.textures_delta.clear();
        assert_eq!(action, Some(Action::Restart));
        app.keys.focus(true);
        app.keys.ui_action(action.unwrap());
        ui.apply(action.unwrap());
        app.keys.input()
    } else {
        restart
    };
    app.bridge.set_input(PlayerSlot(0), restart).unwrap();
    app.bridge.step(1);
    assert_eq!(
        app.bridge
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<game::CollectRun>()
            .score,
        0
    );
    app.persist_completed();
    assert_eq!(app.progress.status(), "Best collected: 2 (saved)");
    app.persist_completed();
    assert_eq!(app.progress.status(), "Best collected: 2 (saved)");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Result<Options, String> {
        Options::parse(s.split_whitespace().map(str::to_owned))
    }
    #[cfg(feature = "collect-ui")]
    #[test]
    fn ui_navigation_preserves_restart_until_authoritative_tick() {
        use crate::authored_ui::Action;
        let mut keys = Keys::default();
        keys.focus(true);
        keys.event(KeyCode::KeyD, true, false, false);
        keys.ui_action(Action::Restart);
        keys.ui_action(Action::Menu);
        keys.ui_action(Action::Continue);
        for _ in 0..8 {
            keys.observe(0, false);
            assert_eq!(keys.input().buttons, game::RESTART);
            assert_eq!(keys.input().x, FP::ZERO);
        }
        keys.observe(1, true);
        assert_eq!(keys.pending_restarts, 0);
        assert_eq!(keys.input().buttons, 0);
        keys.ui_action(Action::Restart);
        keys.focus(false);
        assert_eq!(keys.pending_restarts, 0);
    }
    #[test]
    fn captured_fresh_restart_is_rejected_but_prior_authorized_edge_survives() {
        let mut keys = Keys::default();
        keys.focus(true);
        keys.captured_event(KeyCode::Space, true, false, false, true);
        assert_eq!(keys.pending_restarts, 0);
        assert_eq!(keys.input().buttons, 0);
        keys.captured_event(KeyCode::Space, false, false, false, true);
        keys.captured_event(KeyCode::Space, true, false, false, false);
        assert_eq!(keys.pending_restarts, 1);
        keys.captured_event(KeyCode::Space, false, false, false, true);
        assert_eq!(keys.input().buttons, game::RESTART);
    }
    #[cfg(feature = "collect-ui")]
    #[test]
    fn pointer_capture_recomputes_physical_point_for_dpi_and_zoom() {
        let point = winit::dpi::PhysicalPosition::new(240.0, 120.0);
        assert_eq!(pointer_in_ui(point, 1.0), Some(egui::pos2(240.0, 120.0)));
        assert_eq!(
            pointer_in_ui(point, 2.0 * 1.5),
            Some(egui::pos2(80.0, 40.0))
        );
        assert!(pointer_in_ui(point, 0.0).is_none());
    }
    #[test]
    fn strict_cli() {
        assert!(parse("--project demo --headless --ticks 0 --hold right,up").is_ok());
        for bad in [
            "--project demo --headless",
            "--project demo --ticks 3",
            "--project demo --headless --ticks 6001",
            "--project demo --project other",
            "--project demo --headless --ticks 2 --hold right,left",
            "--project demo --connect nowhere",
            "--project demo --headless --ticks 1 --hold up,up",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn focus_requires_fresh_press_and_aliases_are_independent() {
        let mut keys = Keys::default();
        keys.focus(true);
        keys.event(KeyCode::KeyD, true, false, false);
        keys.event(KeyCode::ArrowRight, true, false, false);
        keys.event(KeyCode::KeyD, false, false, false);
        assert_eq!(keys.input().x, FP::ONE);
        keys.event(KeyCode::Space, true, false, false);
        keys.focus(false);
        assert_eq!(keys.input(), CollectInput::default());
        keys.focus(true);
        keys.event(KeyCode::Space, true, true, false);
        assert_eq!(keys.input().buttons, 0);
        keys.event(KeyCode::Space, false, false, false);
        keys.event(KeyCode::Space, true, false, false);
        assert_eq!(keys.input().buttons, game::RESTART);
        keys.focus(false);
        keys.focus(true);
        keys.event(KeyCode::KeyW, true, false, true);
        assert_eq!(keys.input().y, FP::ZERO);
    }
    fn tap(keys: &mut Keys) {
        keys.event(KeyCode::Space, true, false, false);
        keys.event(KeyCode::Space, false, false, false);
    }
    #[test]
    fn subtick_tap_survives_zero_tick_updates() {
        let mut keys = Keys::default();
        keys.focus(true);
        tap(&mut keys);
        assert_eq!(keys.input().buttons, game::RESTART);
        for _ in 0..3 {
            keys.observe(0, false);
            assert_eq!(keys.input().buttons, game::RESTART);
        }
        keys.observe(1, true);
        assert_eq!(keys.pending_restarts, 0);
        assert_eq!(keys.input().buttons, 0);
    }
    #[test]
    fn consecutive_quick_taps_require_observed_neutral() {
        let mut keys = Keys::default();
        keys.focus(true);
        tap(&mut keys);
        tap(&mut keys);
        keys.event(KeyCode::KeyD, true, false, false);
        assert_eq!(keys.pending_restarts, 2);
        assert_eq!(keys.input().buttons, game::RESTART);
        keys.observe(1, true);
        assert_eq!(keys.pending_restarts, 1);
        assert_eq!(keys.input().buttons, 0);
        keys.observe(1, false); // No simulated neutral tick yet.
        assert_eq!(keys.input().buttons, 0);
        keys.observe(2, false);
        assert_eq!(keys.input().buttons, game::RESTART);
        assert_eq!(keys.input().x, FP::ONE);
        keys.observe(3, true);
        assert_eq!(keys.pending_restarts, 0);
        assert_eq!(keys.input().buttons, 0);
    }
    #[test]
    fn focus_loss_cancels_pending_and_regain_requires_fresh_press() {
        let mut keys = Keys::default();
        keys.focus(true);
        tap(&mut keys);
        keys.focus(false);
        assert_eq!(keys.pending_restarts, 0);
        keys.focus(true);
        assert_eq!(keys.input().buttons, 0);
        keys.event(KeyCode::Space, true, false, true);
        keys.event(KeyCode::Space, true, true, false);
        assert_eq!(keys.input().buttons, 0);
        keys.event(KeyCode::Space, false, false, false);
        tap(&mut keys);
        assert_eq!(keys.input().buttons, game::RESTART);
        keys.observe(1, true);
        tap(&mut keys);
        keys.focus(false);
        keys.focus(true);
        assert_eq!(keys.pending_restarts, 0);
        tap(&mut keys);
        assert_eq!(keys.input().buttons, 0);
        keys.observe(2, false);
        assert_eq!(keys.input().buttons, game::RESTART);
    }
    #[test]
    fn restart_queue_is_bounded_and_repeat_does_not_enqueue() {
        let mut keys = Keys::default();
        keys.focus(true);
        for _ in 0..100 {
            tap(&mut keys);
        }
        assert_eq!(keys.pending_restarts, MAX_PENDING_RESTARTS);
        keys.focus(false);
        keys.focus(true);
        keys.event(KeyCode::Space, true, true, false);
        assert_eq!(keys.pending_restarts, 0);
    }
    #[test]
    fn restart_taps_reach_real_bridge_only_after_simulation_ticks() {
        use orr_fp::FPVec2;
        use std::time::Duration;
        let level = game::CollectLevel::new(
            FPVec2::ZERO,
            vec![FPVec2::new(FP::from_int(20), FP::ZERO)],
            vec![],
            60,
        )
        .unwrap();
        let simulation = orr_sim::Simulation::<CollectDodgeV1>::new(level, 60, 42);
        let mut config = orr_bridge::PlayConfig::new(1, 42, 60);
        config.start_paused = false;
        let session =
            orr_bridge::PlaySession::<CollectDodgeV1>::from_frame(config, simulation.frame())
                .unwrap();
        let mut bridge = InProc::new(
            PlayHost::new(session, PlayerSlot(0)),
            BridgeConfig::default(),
        );
        let mut keys = Keys::default();
        keys.focus(true);
        tap(&mut keys);
        tap(&mut keys);
        let deliver = |keys: &mut Keys, bridge: &mut LocalBridge, dt| {
            bridge.set_input(PlayerSlot(0), keys.input()).unwrap();
            bridge.update(dt);
            let snapshot = bridge.poll_view().snapshot.unwrap();
            keys.observe(
                snapshot.tick(),
                snapshot
                    .predicted()
                    .singleton::<game::CollectRun>()
                    .restart_held
                    != 0,
            );
            snapshot
        };
        let zero = deliver(&mut keys, &mut bridge, Duration::ZERO);
        assert_eq!(zero.tick(), 0);
        assert_eq!(keys.pending_restarts, 2);
        let first = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(first.tick(), 1);
        assert_eq!(keys.pending_restarts, 1);
        assert_eq!(
            first
                .predicted()
                .singleton::<game::CollectRun>()
                .elapsed_ticks,
            0
        );
        let neutral = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(neutral.tick(), 2);
        assert_eq!(keys.pending_restarts, 1);
        assert_eq!(
            neutral
                .predicted()
                .singleton::<game::CollectRun>()
                .elapsed_ticks,
            1
        );
        let second = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(second.tick(), 3);
        assert_eq!(keys.pending_restarts, 0);
        assert_eq!(
            second
                .predicted()
                .singleton::<game::CollectRun>()
                .elapsed_ticks,
            0
        );
    }
}
