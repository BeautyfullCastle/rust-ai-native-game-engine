//! Native Stormglass Foundry. Deterministic session at 60 Hz, responsive vector presentation.
use crate::{
    defense_ui::DefenseUi,
    game_ui_gpu::{GpuOverlay, PendingOverlay},
    project_compositor::{ProjectCompositor, ProjectWindow},
};
use orr_games::coop_defense_game as game;
use orr_render::{
    orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions},
    Camera, RenderList,
};
use orr_session::{LocalInputSource, PlayerSlot, Session, SessionConfig};
use std::{collections::VecDeque, path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

type DefenseSession = Session<game::DefenseGame, LocalInputSource>;
pub const HELP:&str="Stormglass Foundry\ncoop_defense [--scene PATH] [--headless --ticks N [--demo] [--capture NEW.png]]\nSpace summon, click your unit then empty tile to move or identical role/tier to merge, U upgrade, I invest, P/Escape pause, Enter continue, R restart. AI partner is local, not online. Headless capture uses a software GPU.";
#[derive(Debug, Default)]
pub struct Options {
    pub headless: bool,
    pub ticks: u32,
    pub demo: bool,
    pub capture: Option<PathBuf>,
    pub scene: Option<PathBuf>,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.into_iter();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(a) = args.next() {
            if !seen.insert(a.clone()) {
                return Err(format!("duplicate option {a}"));
            }
            match a.as_str() {
                "--headless" => out.headless = true,
                "--demo" => out.demo = true,
                "--ticks" => {
                    out.ticks = args
                        .next()
                        .ok_or("--ticks needs N")?
                        .parse()
                        .map_err(|_| "invalid ticks")?
                }
                "--scene" => out.scene = Some(args.next().ok_or("--scene needs PATH")?.into()),
                "--capture" => {
                    out.capture = Some(args.next().ok_or("--capture needs NEW.png")?.into())
                }
                _ => return Err(format!("unknown option {a}\n{HELP}")),
            }
        }
        if out.ticks > 36000 {
            return Err("ticks must not exceed 36000".into());
        }
        if !out.headless && (out.ticks != 0 || out.capture.is_some() || out.demo) {
            return Err("--ticks, --capture and --demo require --headless".into());
        }
        if out
            .capture
            .as_ref()
            .is_some_and(|p| p.extension().and_then(|s| s.to_str()) != Some("png"))
        {
            return Err("capture must end in .png".into());
        }
        Ok(out)
    }
}
/// Resolve and validate source before creating any window, GPU, or session.
fn prepare_config(options: &Options) -> Result<game::DefenseConfig, String> {
    use orr_games::coop_defense_scene::{parse_config, MAX_SOURCE_BYTES};
    use std::io::Read;
    let Some(path) = &options.scene else {
        return Ok(game::DefenseConfig::default());
    };
    let error = |e: std::io::Error| format!("scene {}: {e}", path.display());
    let metadata = std::fs::symlink_metadata(path).map_err(error)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err("scene must be a regular file of at most 64 KiB (no symlink)".into());
    }
    // NOFOLLOW and NONBLOCK prevent a swapped FIFO/symlink from hanging the
    // CLI between path inspection and open. Verify the opened handle as well.
    #[cfg(unix)]
    let file: std::fs::File = {
        use rustix::fs::{open, Mode, OFlags};
        open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| format!("scene {}: {e}", path.display()))?
        .into()
    };
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: inspect rather than follow a raced link.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0020_0000)
            .open(path)
            .map_err(error)?
    };
    #[cfg(not(any(unix, windows)))]
    let file = std::fs::File::open(path).map_err(error)?;
    let opened = file.metadata().map_err(error)?;
    if !opened.file_type().is_file() || opened.len() > MAX_SOURCE_BYTES as u64 {
        return Err("opened scene must be a regular file of at most 64 KiB".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("scene grew beyond 64 KiB".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "scene must be UTF-8".to_string())?;
    parse_config(text)
}

fn session(config: game::DefenseConfig) -> DefenseSession {
    let mut session_config =
        SessionConfig::new(1, PlayerSlot(0), u64::from(config.seed), game::TICK_RATE);
    session_config.build_id = game::SCENARIO_BUILD_ID;
    Session::new(config, session_config, LocalInputSource)
}
pub fn run(options: Options) -> Result<(), String> {
    let config = prepare_config(&options)?;
    if options.headless {
        return headless_prepared(options, config);
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        session: session(config),
        ui: DefenseUi::new(),
        gfx: None,
        last: Instant::now(),
        accumulator: 0.0,
        commands: VecDeque::new(),
        error: None,
        focused: true,
        focus_lost_pending: false,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}
struct Graphics {
    window: Arc<Window>,
    renderer: ProjectWindow,
    state: egui_winit::State,
    overlay: GpuOverlay,
    pending: PendingOverlay,
}
struct App {
    session: DefenseSession,
    ui: DefenseUi,
    gfx: Option<Graphics>,
    last: Instant,
    accumulator: f64,
    commands: VecDeque<game::DefenseInput>,
    error: Option<String>,
    focused: bool,
    focus_lost_pending: bool,
}
impl App {
    // Shared by the window redraw path and CPU-only admission tests.
    fn process_input(&mut self, mut input: egui::RawInput, delta: f64) -> egui::FullOutput {
        input.focused &= self.focused;
        if std::mem::take(&mut self.focus_lost_pending) {
            input.events.push(egui::Event::WindowFocused(false));
        }
        let focus_lost = !self.focused
            || !input.focused
            || input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)));
        if focus_lost
            && self.ui.started
            && game::run(self.session.predicted_frame()).phase == game::PLAYING
        {
            self.ui.pause();
        }
        let was_started = self.ui.started;
        let was_paused = self.ui.paused;
        let (output, command) = self.ui.show(input, self.session.predicted_frame());
        if focus_lost || !was_started || was_paused || self.ui.paused {
            self.commands.clear();
            self.accumulator = 0.0;
            return output;
        }
        if let Some(command) = command.filter(|_| self.focused) {
            if self.commands.len() < 32 {
                self.commands.push_back(command);
            }
        }
        self.accumulator += if self.ui.started && self.focused {
            delta.clamp(0.0, 0.1)
        } else {
            0.0
        };
        while self.accumulator >= 1.0 / 60.0 {
            let command = self.commands.pop_front().unwrap_or_default();
            self.session.advance(command, vec![]);
            self.accumulator -= 1.0 / 60.0;
        }
        output
    }
    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.accumulator = 0.0;
        self.last = Instant::now();
        if !focused {
            self.focus_lost_pending = true;
            self.commands.clear();
            if self.ui.started && game::run(self.session.predicted_frame()).phase == game::PLAYING {
                self.ui.pause();
            }
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let delta = (now - self.last).as_secs_f64().min(0.1);
        self.last = now;
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        let input = gfx.state.take_egui_input(&gfx.window);
        let mut output = self.process_input(input, delta);
        let gfx = self.gfx.as_mut().expect("graphics initialized above");
        gfx.state
            .handle_platform_output(&gfx.window, std::mem::take(&mut output.platform_output));
        if let Err(e) = gfx.pending.push(output) {
            self.fail(event_loop, e);
            return;
        }
        let result = gfx.renderer.render_with_overlay(
            &RenderList::default(),
            &[],
            &Camera::new([0.0, 0.0], 1.0),
            |gpu, view, size| {
                gfx.pending
                    .paint(&mut gfx.overlay, gpu, view, size, &self.ui.context)
            },
        );
        if let Err(e) = result {
            self.fail(event_loop, e);
        }
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Stormglass Foundry · local cooperative defense")
                .with_inner_size(LogicalSize::new(1200.0, 940.0))
                .with_min_inner_size(LogicalSize::new(600.0, 470.0)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.fail(event_loop, e.to_string());
                return;
            }
        };
        let renderer = match ProjectWindow::new(window.clone(), true, &Default::default()) {
            Ok(r) => r,
            Err(e) => {
                self.fail(event_loop, e);
                return;
            }
        };
        println!("Stormglass native adapter: {}", renderer.adapter_name());
        let state = egui_winit::State::new(
            self.ui.context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        let overlay = GpuOverlay::new(renderer.rhi(), renderer.format());
        self.gfx = Some(Graphics {
            window,
            renderer,
            state,
            overlay,
            pending: Default::default(),
        });
        self.last = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        let _ = gfx.state.on_window_event(&gfx.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gfx.renderer.resize(size.width, size.height);
            }
            WindowEvent::Focused(focused) => {
                self.set_focused(focused);
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}
pub fn headless(options: Options) -> Result<(), String> {
    let config = prepare_config(&options)?;
    headless_prepared(options, config)
}
fn headless_prepared(options: Options, config: game::DefenseConfig) -> Result<(), String> {
    let mut session = session(config);
    let mut sequence = 0;
    for tick in 0..options.ticks {
        let input = if options.demo && tick % 45 == 0 {
            sequence += 1;
            game::strategy_input(
                &game::run(session.predicted_frame()),
                &game::units(session.predicted_frame()),
                0,
                sequence,
            )
        } else {
            game::DefenseInput::default()
        };
        session.advance(input, vec![]);
    }
    let frame = session.predicted_frame();
    let run = game::run(frame);
    println!(
        "Stormglass tick={} wave={} phase={} reactor={} kills={} merges={} checksum=0x{:016x}",
        run.tick,
        run.wave,
        run.phase,
        run.reactor_hp,
        run.kills,
        run.merges,
        frame.checksum()
    );
    if let Some(path) = options.capture {
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        })?;
        if !gpu.is_software() {
            return Err("capture requires a verified software adapter".into());
        }
        println!("Stormglass capture adapter: {}", gpu.adapter_name());
        let size = (1440, 1128);
        let target =
            orr_render::OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
        let mut compositor = ProjectCompositor::new(
            gpu.clone(),
            TextureFormat::Rgba8UnormSrgb,
            &Default::default(),
        )?;
        let mut overlay = GpuOverlay::new(&gpu, TextureFormat::Rgba8UnormSrgb);
        let mut ui = DefenseUi::new();
        ui.started = options.demo || options.ticks > 0;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0 as f32, size.1 as f32),
                )),
                ..Default::default()
            };
            let (output, _) = ui.show(input, frame);
            compositor.draw_with_overlay(
                target.render_view(),
                size,
                &RenderList::default(),
                &[],
                &Camera::new([0.0, 0.0], 1.0),
                |gpu, view, size| overlay.paint(gpu, view, size, &ui.context, output),
            )?;
        }
        let file = std::fs::File::create_new(&path)
            .map_err(|e| format!("capture {}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&target.read_rgba8())
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_tracks_scenario_build_identity() {
        assert_eq!(
            session(game::DefenseConfig::default()).build_hash(),
            orr_sim::build_hash_of(game::SCENARIO_BUILD_ID, 0)
        );
        assert_ne!(session(game::DefenseConfig::default()).build_hash(), 0);
    }
    #[test]
    fn bounded_cli() {
        assert!(Options::parse(["--headless", "--ticks", "36001"].map(String::from)).is_err());
        assert!(Options::parse(["--capture", "x.png"].map(String::from)).is_err());
        assert!(Options::parse(["--headless", "--headless"].map(String::from)).is_err());
        assert!(
            Options::parse(["--headless", "--ticks", "300", "--demo"].map(String::from)).is_ok()
        );
    }
    #[test]
    fn scene_cli_is_explicit_and_available_in_both_modes() {
        assert!(Options::parse(["--scene"].map(String::from)).is_err());
        assert!(Options::parse(["--scene", "a", "--scene", "b"].map(String::from)).is_err());
        assert_eq!(
            Options::parse(["--scene", "a"].map(String::from))
                .unwrap()
                .scene,
            Some("a".into())
        );
        assert!(Options::parse(["--scene", "a", "--headless"].map(String::from)).is_ok());
    }
    #[test]
    fn campaign_only_scene_preserves_default_scenario_and_authored_campaign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("campaign.yaml");
        std::fs::write(
            &path,
            "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Campaign: {last_wave: 3}}\n",
        )
        .unwrap();
        let config = prepare_config(&Options {
            scene: Some(path.clone()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.scenario, game::Scenario::default());
        assert_eq!(config.scenario.starting_gold, 150);
        assert_eq!(config.scenario.reactor_hp, 30);
        assert_eq!(config.campaign.last_wave, 3);

        let native = session(config);
        let frame = native.predicted_frame();
        assert_eq!(frame.singleton::<game::Campaign>().last_wave, 3);
        assert_eq!(
            frame.singleton::<game::Scenario>(),
            &game::Scenario::default()
        );
    }
    #[test]
    fn bounded_scene_loading_preserves_defaults_and_authored_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scene.yaml");
        let options = Options {
            scene: Some(path.clone()),
            ..Default::default()
        };
        assert!(prepare_config(&options).is_err());
        std::fs::write(&path, "schema: orr.scene/1\nentities: {}\n").unwrap();
        let explicit_default = session(prepare_config(&options).unwrap());
        assert_eq!(
            session(prepare_config(&Options::default()).unwrap())
                .predicted_frame()
                .to_bytes(),
            explicit_default.predicted_frame().to_bytes()
        );
        std::fs::write(&path, "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}, CoopDefense::Campaign: {last_wave: 3}, CoopDefense::WavePacing: {spawn_interval_ticks: 70}}\n").unwrap();
        let mut native = session(prepare_config(&options).unwrap());
        assert_eq!(
            native
                .predicted_frame()
                .singleton::<game::Campaign>()
                .last_wave,
            3
        );
        assert_eq!(game::run(native.predicted_frame()).gold, [250; 2]);
        assert_eq!(game::run(native.predicted_frame()).reactor_hp, 75);
        let delay = native.config().input_delay;
        for action in [game::SUMMON, game::RESTART] {
            native.advance(
                game::DefenseInput {
                    action,
                    sequence: action,
                    ..Default::default()
                },
                vec![],
            );
            for _ in 0..delay {
                native.advance(game::DefenseInput::default(), vec![]);
            }
        }
        assert_eq!(game::run(native.predicted_frame()).gold, [250; 2]);
        assert_eq!(game::run(native.predicted_frame()).reactor_hp, 75);
        assert_eq!(
            native
                .predicted_frame()
                .singleton::<game::Campaign>()
                .last_wave,
            3
        );
        assert_eq!(game::run(native.predicted_frame()).generation, 1);
        assert_eq!(
            native
                .predicted_frame()
                .singleton::<game::WavePacing>()
                .spawn_interval_ticks,
            70
        );
        let active_frame_before_invalid_source = native.predicted_frame().to_bytes();
        for invalid in [
            vec![0xff],
            vec![b' '; 65537],
            b"schema: invalid\n".to_vec(),
            b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Campaign: {last_wave: 11}}\n".to_vec(),
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(prepare_config(&options).is_err());
            assert_eq!(native.predicted_frame().to_bytes(), active_frame_before_invalid_source);
            assert!(run(Options {
                scene: Some(path.clone()),
                ..Default::default()
            })
            .is_err());
        }
        assert!(prepare_config(&Options {
            scene: Some(dir.path().to_owned()),
            ..Default::default()
        })
        .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn scene_loader_rejects_symlinks_and_fifos_before_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, "schema: orr.scene/1\nentities: {}\n").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(prepare_config(&Options {
            scene: Some(link),
            ..Default::default()
        })
        .is_err());
        let fifo = dir.path().join("fifo");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        assert!(prepare_config(&Options {
            scene: Some(fifo),
            ..Default::default()
        })
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_scene_fails_before_startup() {
        use std::os::unix::fs::PermissionsExt;
        // A privileged test process can read mode-000 files; do not claim that
        // platform proves unprivileged permission denial.
        if rustix::process::geteuid().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unreadable");
        std::fs::write(&path, "schema: orr.scene/1\nentities: {}\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
        let result = run(Options {
            scene: Some(path.clone()),
            ..Default::default()
        });
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;
    fn app() -> App {
        let mut ui = DefenseUi::new();
        ui.started = true;
        App {
            session: session(game::DefenseConfig::default()),
            ui,
            gfx: None,
            last: Instant::now(),
            accumulator: 0.0,
            commands: VecDeque::new(),
            error: None,
            focused: true,
            focus_lost_pending: false,
        }
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    fn frame(app: &mut App, events: Vec<egui::Event>, delta: f64) {
        app.process_input(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 940.0),
                )),
                events,
                ..Default::default()
            },
            delta,
        );
    }
    #[test]
    fn manual_pause_keeps_actual_session_frame_unchanged() {
        let mut app = app();
        frame(&mut app, vec![], 1.0 / 60.0);
        let before = app.session.predicted_frame().to_bytes();
        frame(&mut app, vec![key(egui::Key::P)], 0.1);
        assert_eq!(
            app.session.predicted_frame().to_bytes(),
            before,
            "manual pause must freeze Session"
        );
    }
    #[test]
    fn focus_return_requires_explicit_continue() {
        let mut app = app();
        frame(&mut app, vec![], 1.0 / 60.0);
        let before = app.session.predicted_frame().to_bytes();
        app.set_focused(false);
        app.set_focused(true);
        frame(&mut app, vec![], 0.1);
        assert_eq!(
            app.session.predicted_frame().to_bytes(),
            before,
            "focus return must not resume Session"
        );
    }
    fn pulse(key: egui::Key) -> Vec<egui::Event> {
        let down = self::key(key);
        let mut up = down.clone();
        if let egui::Event::Key { pressed, .. } = &mut up {
            *pressed = false;
        }
        vec![down, up]
    }
    fn button(x: f32, y: f32, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn click(x: f32, y: f32) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(egui::pos2(x, y)),
            button(x, y, true),
            button(x, y, false),
        ]
    }
    fn unchanged(app: &App, bytes: &[u8]) {
        assert!(
            app.session.predicted_frame().to_bytes() == bytes,
            "Session Frame bytes changed"
        );
    }
    #[test]
    fn long_pause_freezes_frame_tick_economy_and_resumes_without_catchup() {
        let mut app = app();
        frame(&mut app, vec![], 0.1);
        frame(&mut app, pulse(egui::Key::Escape), 0.01);
        let before = app.session.predicted_frame().to_bytes();
        let tick = app.session.predicted_frame().tick();
        for _ in 0..100 {
            frame(&mut app, vec![], 3600.0);
        }
        unchanged(&app, &before);
        frame(&mut app, pulse(egui::Key::Enter), 3600.0);
        assert!(!app.ui.paused);
        unchanged(&app, &before);
        frame(&mut app, vec![], 1.0 / 60.0);
        assert_eq!(app.session.predicted_frame().tick(), tick + 1);
    }
    #[test]
    fn pause_clears_unsubmitted_queue_selection_and_partial_time() {
        let mut app = app();
        frame(&mut app, pulse(egui::Key::Space), 0.001);
        assert_eq!(app.commands.len(), 1);
        app.ui.selected = Some(0);
        let before = app.session.predicted_frame().to_bytes();
        frame(&mut app, pulse(egui::Key::P), 0.1);
        assert!(app.commands.is_empty());
        assert_eq!(app.accumulator, 0.0);
        assert_eq!(app.ui.selected, None);
        unchanged(&app, &before);
        frame(&mut app, pulse(egui::Key::Enter), 0.1);
        frame(&mut app, vec![], 0.1);
        assert_eq!(game::run(app.session.predicted_frame()).gold[0], 150);
        assert_eq!(game::run(app.session.predicted_frame()).last_sequence, 0);
    }
    #[test]
    fn pause_and_continue_frames_do_not_submit_mixed_gameplay() {
        let mut app = app();
        let before = app.session.predicted_frame().to_bytes();
        let mut mixed = pulse(egui::Key::Space);
        mixed.extend(pulse(egui::Key::R));
        mixed.extend(pulse(egui::Key::P));
        mixed.extend(pulse(egui::Key::Enter));
        frame(&mut app, mixed, 0.1);
        assert!(app.ui.paused);
        unchanged(&app, &before);
        let mut mixed = pulse(egui::Key::Enter);
        for key in [egui::Key::Space, egui::Key::U, egui::Key::I, egui::Key::R] {
            mixed.extend(pulse(key));
        }
        mixed.extend(click(1050.0, 870.0));
        frame(&mut app, mixed, 0.1);
        assert!(!app.ui.paused);
        unchanged(&app, &before);
        frame(&mut app, vec![], 0.1);
        let run = game::run(app.session.predicted_frame());
        assert_eq!(
            (run.generation, run.last_sequence, run.gold[0]),
            (0, 0, 150)
        );
        let before = app.session.predicted_frame().to_bytes();
        let mut clicks = click(350.0, 870.0); // Summon first, then genuine Pause.
        clicks.extend(click(1050.0, 762.0));
        clicks.extend(pulse(egui::Key::Enter));
        frame(&mut app, clicks, 0.1);
        assert!(app.ui.paused);
        assert!(app.commands.is_empty());
        unchanged(&app, &before);
    }
    #[test]
    fn focus_false_dominates_continue_and_gameplay_in_any_event_order() {
        for reverse in [false, true] {
            let mut app = app();
            let before = app.session.predicted_frame().to_bytes();
            let mut events = pulse(egui::Key::Enter);
            events.extend(pulse(egui::Key::Space));
            events.extend(pulse(egui::Key::R));
            events.extend(click(600.0, 517.0));
            events.push(egui::Event::WindowFocused(false));
            events.push(egui::Event::WindowFocused(true));
            if reverse {
                events.reverse();
            }
            frame(&mut app, events, 0.1);
            assert!(app.ui.paused);
            unchanged(&app, &before);
        }
        let mut app = app();
        app.set_focused(false);
        app.set_focused(true);
        frame(&mut app, pulse(egui::Key::Enter), 0.1);
        assert!(
            app.ui.paused,
            "native focus-loss latch must survive focus return before redraw"
        );
        frame(&mut app, pulse(egui::Key::Enter), 0.1);
        assert!(!app.ui.paused);
    }
    #[test]
    fn unfocused_raw_input_blocks_start_resume_and_commands() {
        let mut app = app();
        let before = app.session.predicted_frame().to_bytes();
        app.process_input(
            egui::RawInput {
                focused: false,
                events: pulse(egui::Key::Enter),
                ..Default::default()
            },
            0.1,
        );
        assert!(app.ui.paused);
        unchanged(&app, &before);
        app.ui.started = false;
        app.ui.paused = false;
        app.process_input(
            egui::RawInput {
                focused: false,
                events: pulse(egui::Key::Space),
                ..Default::default()
            },
            0.1,
        );
        assert!(!app.ui.started);
    }
    #[test]
    fn repeated_keys_do_not_pause_or_resume_and_space_remains_summon() {
        let mut app = app();
        let repeated = |key| {
            let mut event = self::key(key);
            if let egui::Event::Key { repeat, .. } = &mut event {
                *repeat = true;
            }
            event
        };
        frame(&mut app, vec![repeated(egui::Key::P)], 0.0);
        assert!(!app.ui.paused);
        frame(&mut app, pulse(egui::Key::Escape), 0.0);
        for key in [
            egui::Key::P,
            egui::Key::Escape,
            egui::Key::Enter,
            egui::Key::Space,
        ] {
            frame(&mut app, vec![repeated(key)], 0.1);
            assert!(app.ui.paused);
        }
        frame(&mut app, pulse(egui::Key::Space), 0.1);
        assert!(app.ui.paused);
        let mut release_enter = self::key(egui::Key::Enter);
        if let egui::Event::Key { pressed, .. } = &mut release_enter {
            *pressed = false;
        }
        frame(&mut app, vec![release_enter], 0.0);
        frame(&mut app, pulse(egui::Key::Enter), 0.1);
        frame(&mut app, pulse(egui::Key::Space), 0.1);
        assert_eq!(game::run(app.session.predicted_frame()).gold[0], 125);
    }
    #[test]
    fn pause_continue_buttons_are_event_attributed_and_consume_frame() {
        let mut app = app();
        let mut unrelated = click(40.0, 100.0);
        unrelated.push(egui::Event::PointerMoved(egui::pos2(1050.0, 762.0)));
        frame(&mut app, unrelated, 0.0);
        assert!(!app.ui.paused);
        let mut events = click(1050.0, 762.0);
        events.push(egui::Event::PointerMoved(egui::pos2(1050.0, 870.0)));
        frame(&mut app, events, 0.1);
        assert!(app.ui.paused);
        let before = app.session.predicted_frame().to_bytes();
        let mut unrelated = click(1050.0, 870.0);
        unrelated.push(egui::Event::PointerMoved(egui::pos2(600.0, 517.0)));
        frame(&mut app, unrelated, 0.1);
        assert!(app.ui.paused);
        let mut events = click(600.0, 517.0);
        events.extend(click(1050.0, 870.0));
        frame(&mut app, events, 0.1);
        assert!(!app.ui.paused);
        unchanged(&app, &before);
        frame(&mut app, vec![], 0.1);
        assert_eq!(game::run(app.session.predicted_frame()).generation, 0);
    }
    #[test]
    fn held_gestures_cannot_cross_pause_or_continue() {
        for (x, y) in [(350.0, 870.0), (1050.0, 870.0), (600.0, 517.0)] {
            for focus_loss in [false, true] {
                let mut app = app();
                frame(&mut app, vec![button(x, y, true)], 0.0);
                if focus_loss {
                    app.set_focused(false);
                    app.set_focused(true);
                    frame(&mut app, vec![], 0.0);
                } else {
                    frame(&mut app, pulse(egui::Key::P), 0.0);
                }
                frame(&mut app, vec![button(x, y, false)], 0.1);
                assert!(app.ui.paused);
                frame(&mut app, vec![button(600.0, 517.0, true)], 0.1);
                frame(&mut app, pulse(egui::Key::Enter), 0.1);
                frame(&mut app, vec![button(1050.0, 870.0, false)], 0.1);
                let run = game::run(app.session.predicted_frame());
                assert_eq!((run.generation, run.last_sequence), (0, 0));
            }
        }
    }
    #[test]
    fn admitted_delayed_input_is_frozen_then_completes_after_explicit_continue() {
        let mut app = app();
        assert_eq!(app.session.config().input_delay, 2);
        frame(&mut app, pulse(egui::Key::Space), 1.0 / 60.0);
        assert!(app.commands.is_empty());
        assert_eq!(game::run(app.session.predicted_frame()).gold[0], 150);
        frame(&mut app, pulse(egui::Key::P), 0.1);
        let before = app.session.predicted_frame().to_bytes();
        frame(&mut app, vec![], 100.0);
        unchanged(&app, &before);
        frame(&mut app, pulse(egui::Key::Enter), 100.0);
        unchanged(&app, &before);
        frame(&mut app, vec![], 2.0 / 60.0);
        assert_eq!(game::run(app.session.predicted_frame()).gold[0], 125);
        assert_eq!(game::run(app.session.predicted_frame()).last_sequence, 1);
    }
    #[test]
    fn ready_terminal_and_authored_restart_contracts_survive_pause() {
        let mut app = app();
        app.ui.started = false;
        let before = app.session.predicted_frame().to_bytes();
        frame(&mut app, pulse(egui::Key::P), 0.1);
        assert!(!app.ui.started && !app.ui.paused);
        frame(&mut app, pulse(egui::Key::Space), 0.1);
        assert!(app.ui.started);
        unchanged(&app, &before);
        let config = game::DefenseConfig {
            scenario: game::Scenario {
                starting_gold: 250,
                reactor_hp: 75,
                ..Default::default()
            },
            campaign: game::Campaign { last_wave: 3 },
            ..Default::default()
        };
        app.session = session(config);
        frame(&mut app, pulse(egui::Key::P), 0.1);
        frame(&mut app, pulse(egui::Key::Enter), 0.1);
        frame(&mut app, pulse(egui::Key::R), 0.1);
        assert_eq!(game::run(app.session.predicted_frame()).generation, 1);
        assert_eq!(game::run(app.session.predicted_frame()).gold, [250; 2]);
        assert_eq!(
            app.session
                .predicted_frame()
                .singleton::<game::Campaign>()
                .last_wave,
            3
        );
        // Reach the actual terminal game state through deterministic Session ticks.
        while game::run(app.session.predicted_frame()).phase == game::PLAYING {
            app.session.advance(Default::default(), vec![]);
        }
        app.set_focused(false);
        app.set_focused(true);
        frame(&mut app, vec![], 0.0);
        frame(&mut app, pulse(egui::Key::P), 0.0);
        assert!(!app.ui.paused);
        frame(&mut app, pulse(egui::Key::R), 0.1);
        assert_eq!(game::run(app.session.predicted_frame()).generation, 2);
        assert_eq!(game::run(app.session.predicted_frame()).reactor_hp, 75);
    }
    #[test]
    fn responsive_pause_controls_render_and_use_matching_hit_rects() {
        for size in [
            egui::vec2(1200.0, 940.0),
            egui::vec2(750.0, 900.0),
            egui::vec2(1800.0, 800.0),
        ] {
            let mut app = app();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let layout = crate::defense_view::Layout::new(screen);
            let draw = |app: &mut App, events| {
                app.process_input(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events,
                        ..Default::default()
                    },
                    0.0,
                )
            };
            let has_text = |out: &egui::FullOutput, needle: &str| {
                out.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(needle))
            })
            };
            let active = draw(&mut app, vec![]);
            assert!(has_text(&active, "PAUSE"));
            let pause = layout.p(1050.0, 762.0);
            let paused = draw(&mut app, click(pause.x, pause.y));
            assert!(app.ui.paused);
            assert!(has_text(&paused, "DEFENSE PAUSED"));
            assert!(has_text(&paused, "CONTINUE"));
            let resume = layout.p(600.0, 517.0);
            draw(&mut app, click(resume.x, resume.y));
            assert!(!app.ui.paused);
        }
    }
}
