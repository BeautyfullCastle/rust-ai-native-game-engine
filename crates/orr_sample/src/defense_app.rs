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

pub(crate) type DefenseSession = Session<game::DefenseGame, LocalInputSource>;
pub const HELP:&str="Stormglass Foundry\ncoop_defense [--scene PATH] [--run-directory ABSOLUTE_PRIVATE_DIRECTORY (Linux run-save feature)] [--headless --ticks N [--demo] [--capture NEW.png]]\nSpace summon, click your unit then empty tile to move or identical role/tier to merge, U upgrade, I invest, P/Escape pause, Enter continue, R restart. AI partner is local, not online. Headless capture uses a software GPU. Optional coop-defense-audio: --audio-bank ABSOLUTE_BANK_DIRECTORY --audio off|auto|required; native output additionally needs audio-native. Starts muted; Linux bank loading only.";
#[derive(Debug, Default)]
pub struct Options {
    #[cfg(feature = "coop-defense-audio")]
    pub audio_bank: Option<PathBuf>,
    #[cfg(feature = "coop-defense-audio")]
    pub audio: crate::arena_audio::AudioMode,
    #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
    pub run_directory: Option<PathBuf>,
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
                #[cfg(feature = "coop-defense-audio")]
                "--audio-bank" => {
                    out.audio_bank = Some(args.next().ok_or("--audio-bank needs PATH")?.into())
                }
                #[cfg(feature = "coop-defense-audio")]
                "--audio" => {
                    out.audio = args
                        .next()
                        .ok_or("--audio needs off|auto|required")?
                        .parse()?
                }
                #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
                "--run-directory" => {
                    out.run_directory =
                        Some(args.next().ok_or("--run-directory needs PATH")?.into())
                }
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
        #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
        if out.headless && out.run_directory.is_some() {
            return Err("--run-directory requires the native interactive player".into());
        }
        #[cfg(feature = "coop-defense-audio")]
        validate_audio_options(&out)?;
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
#[cfg(feature = "coop-defense-audio")]
fn validate_audio_options(options: &Options) -> Result<(), String> {
    if options.audio == crate::arena_audio::AudioMode::Required {
        if options.headless {
            return Err("required audio needs the native interactive player".into());
        }
        if options.audio_bank.is_none() {
            return Err("required audio needs --audio-bank".into());
        }
    }
    Ok(())
}
#[cfg(feature = "coop-defense-audio")]
fn prepare_audio(
    options: &Options,
) -> Result<Option<orr_package::coop_audio_admission::AdmittedBank>, String> {
    validate_audio_options(options)?;
    #[cfg(target_os = "linux")]
    {
        options
            .audio_bank
            .as_deref()
            .map(orr_package::coop_audio_admission::load_bank)
            .transpose()
            .map_err(|e| {
                e.chars()
                    .take(240)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect()
            })
    }
    #[cfg(not(target_os = "linux"))]
    {
        if options.audio_bank.is_some() {
            Err("authored audio bank loading currently requires Linux".into())
        } else {
            Ok(None)
        }
    }
}
/// Resolve and validate source before creating any window, GPU, or session.
fn prepare_scene(options: &Options) -> Result<Option<String>, String> {
    use orr_games::coop_defense_scene::MAX_SOURCE_BYTES;
    use std::io::Read;
    let Some(path) = &options.scene else {
        return Ok(None);
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
    Ok(Some(text.to_owned()))
}
fn prepare_config(options: &Options) -> Result<game::DefenseConfig, String> {
    match prepare_scene(options)? {
        Some(text) => orr_games::coop_defense_scene::parse_config(&text),
        None => Ok(game::DefenseConfig::default()),
    }
}

pub(crate) fn session(config: game::DefenseConfig) -> DefenseSession {
    let mut session_config =
        SessionConfig::new(1, PlayerSlot(0), u64::from(config.seed), game::TICK_RATE);
    session_config.build_id = game::SCENARIO_BUILD_ID;
    Session::new(config, session_config, LocalInputSource)
}
pub fn run(options: Options) -> Result<(), String> {
    #[cfg(feature = "coop-defense-audio")]
    let audio_bank = prepare_audio(&options)?;
    let scene = prepare_scene(&options)?;
    let config = match &scene {
        Some(text) => orr_games::coop_defense_scene::parse_config(text)?,
        None => game::DefenseConfig::default(),
    };
    if options.headless {
        return headless_prepared(options, config);
    }
    #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
    let saves = if let Some(path) = &options.run_directory {
        let mut forbidden = Vec::new();
        if let Ok(root) = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
        {
            forbidden.push(root);
        }
        if let Some(parent) = std::env::current_exe().map_err(|e| e.to_string())?.parent() {
            forbidden.push(parent.to_owned());
        }
        if let Some(scene) = &options.scene {
            let scene = scene.canonicalize().map_err(|e| e.to_string())?;
            if let Some(parent) = scene.parent() {
                forbidden.push(parent.to_owned());
            }
        }
        Some(RunSaves {
            store: crate::defense_run_save_store::RunStore::open(path, &forbidden)?,
            identity: crate::defense_run_save::executable_identity()?,
            journal: crate::defense_run_save::Journal::new(
                crate::defense_run_save::Origin { scene },
                &session(config),
            ),
            candidate: None,
        })
    } else {
        None
    };
    #[cfg(feature = "coop-defense-audio")]
    let audio = crate::defense_audio::DefenseAudio::open(audio_bank.as_ref(), options.audio)?;
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        #[cfg(feature = "coop-defense-audio")]
        audio,
        #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
        saves,
        session: session(config),
        ui: DefenseUi::new(),
        gfx: None,
        last: Instant::now(),
        accumulator: 0.0,
        commands: VecDeque::new(),
        error: None,
        focused: true,
        focus_lost_pending: false,
        board: PendingBoard::default(),
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
fn bounded_save_error(error: &str) -> String {
    let mut chars = error.chars();
    let mut message: String = chars
        .by_ref()
        .take(240)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        message.push('…');
    }
    message
}
struct Graphics {
    window: Arc<Window>,
    renderer: ProjectWindow,
    state: egui_winit::State,
    overlay: GpuOverlay,
    pending: PendingOverlay,
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
struct RunSaves {
    store: crate::defense_run_save_store::RunStore,
    identity: String,
    journal: crate::defense_run_save::Journal,
    candidate: Option<crate::defense_run_save::Candidate>,
}
struct App {
    #[cfg(feature = "coop-defense-audio")]
    audio: crate::defense_audio::DefenseAudio,
    #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
    saves: Option<RunSaves>,
    session: DefenseSession,
    ui: DefenseUi,
    gfx: Option<Graphics>,
    last: Instant,
    accumulator: f64,
    commands: VecDeque<game::DefenseInput>,
    error: Option<String>,
    focused: bool,
    focus_lost_pending: bool,
    board: PendingBoard,
}
// Presentation-only admission state. Session ticks never reset on Restart.
#[derive(Default)]
struct PendingBoard {
    through: Option<u64>,
    selection: Option<ExpectedSelection>,
}
struct ExpectedSelection {
    generation: u32,
    cell: u32,
    id: u32,
    role: u32,
    tier: u32,
    consumed_source: Option<u32>,
}
fn affects_board(command: &game::DefenseInput) -> bool {
    command.sequence != 0
        && matches!(
            command.action,
            game::SUMMON | game::MOVE_OR_MERGE | game::RESTART
        )
}
impl ExpectedSelection {
    fn capture(frame: &orr_ecs::Frame, command: game::DefenseInput) -> Option<Self> {
        let units = game::units(frame);
        let source = units
            .iter()
            .find(|u| u.board == 0 && u.cell == command.source_cell && u.active != 0)?;
        let target = units
            .iter()
            .find(|u| u.board == 0 && u.cell == command.target_cell)?;
        let merge = target.active != 0;
        Some(Self {
            generation: game::run(frame).generation,
            cell: command.target_cell,
            id: if merge { target.id } else { source.id },
            role: source.role,
            tier: source.tier + u32::from(merge),
            consumed_source: merge.then_some(source.id),
        })
    }
    fn matches(&self, frame: &orr_ecs::Frame) -> bool {
        let units = game::units(frame);
        game::run(frame).generation == self.generation
            && units.iter().any(|u| {
                u.board == 0
                    && u.active != 0
                    && u.cell == self.cell
                    && u.id == self.id
                    && u.role == self.role
                    && u.tier == self.tier
            })
            && self
                .consumed_source
                .is_none_or(|id| !units.iter().any(|u| u.active != 0 && u.id == id))
    }
}
impl App {
    fn refresh_board(&mut self) {
        let frame = self.session.predicted_frame();
        if self.board.through.is_some_and(|tick| frame.tick() >= tick) {
            self.board.through = None;
        }
        let pending = self.board.through.is_some() || self.commands.iter().any(affects_board);
        if !pending {
            if let Some(expected) = self.board.selection.take() {
                self.ui.selected = expected.matches(frame).then_some(expected.cell);
            }
        }
        self.ui.set_board_pending(pending);
    }
    fn cancel_board_intent(&mut self) {
        self.commands.clear();
        self.ui.selected = None;
        self.board.selection = None;
        // Capture once at cancellation, not on each redraw after resuming.
        self.board.through = Some(self.session.next_send_tick().saturating_sub(1));
        self.refresh_board();
    }
    // Shared by the window redraw path and CPU-only admission tests.
    fn process_input(&mut self, mut input: egui::RawInput, delta: f64) -> egui::FullOutput {
        #[cfg(feature = "coop-defense-audio")]
        {
            if let Err(error) = self.audio.poll() {
                self.error = Some(error);
            }
            self.ui.audio.enabled = self.audio.enabled();
            self.ui.audio.status = self.audio.status().to_owned();
        }
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
        #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
        {
            self.ui.run_save.enabled = self.saves.is_some();
            let was_saveable = self.ui.run_save.can_save;
            self.ui.run_save.can_save = self.saves.as_ref().is_some_and(|s| s.journal.can_save());
            if was_saveable && !self.ui.run_save.can_save {
                self.ui.run_save.status =
                    "Recording limit reached; gameplay continues, new saves disabled".into();
            }
        }
        self.refresh_board();
        let previous_selection = self.ui.selected;
        let was_started = self.ui.started;
        let was_paused = self.ui.paused;
        let (output, command) = self.ui.show(input, self.session.predicted_frame());
        if self.error.is_some() {
            self.commands.clear();
            self.accumulator = 0.0;
            return output;
        }
        #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
        if self.process_saves() {
            #[cfg(feature = "coop-defense-audio")]
            self.sync_audio(true);
            self.commands.clear();
            self.accumulator = 0.0;
            return output;
        }
        #[cfg(feature = "coop-defense-audio")]
        self.sync_audio(focus_lost || !self.ui.started || self.ui.paused);
        if self.error.is_some() || focus_lost || !was_started || was_paused || self.ui.paused {
            if focus_lost || (!was_paused && self.ui.paused) || !was_started {
                self.cancel_board_intent();
            } else {
                self.commands.clear();
                self.board.selection = None;
                self.refresh_board();
            }
            self.accumulator = 0.0;
            return output;
        }
        if let Some(command) = command.filter(|_| self.focused) {
            if self.commands.len() < 32 && command.action != 0 && command.sequence != 0 {
                if command.action == game::MOVE_OR_MERGE {
                    self.board.selection =
                        ExpectedSelection::capture(self.session.predicted_frame(), command);
                } else if command.action == game::RESTART {
                    self.board.selection = None;
                }
                self.commands.push_back(command);
            } else {
                self.ui.selected = previous_selection;
            }
        }
        self.accumulator += if self.ui.started && self.focused {
            delta.clamp(0.0, 0.1)
        } else {
            0.0
        };
        while self.accumulator >= 1.0 / 60.0 {
            let command = self.commands.pop_front().unwrap_or_default();
            if affects_board(&command) {
                self.board.through = Some(self.session.next_send_tick());
            }
            let result = self.session.advance(command, vec![]);
            #[cfg(feature = "coop-defense-audio")]
            {
                let (orr_session::AdvanceResult::Advanced { events, .. }
                | orr_session::AdvanceResult::Stalled { events }) = &result;
                if let Err(error) = self.audio.consume(events) {
                    self.error = Some(error);
                }
            }
            #[cfg(not(feature = "coop-defense-audio"))]
            let _ = result;
            #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
            if let Some(saves) = &mut self.saves {
                saves.journal.record(command);
            }
            self.accumulator -= 1.0 / 60.0;
            if self.error.is_some() {
                self.commands.clear();
                self.accumulator = 0.0;
                break;
            }
        }
        self.refresh_board();
        output
    }
    #[cfg(feature = "coop-defense-audio")]
    fn sync_audio(&mut self, paused: bool) {
        if let Err(error) = self.audio.controls(
            self.ui.audio.muted,
            f32::from(self.ui.audio.gain_percent) / 100.0,
            paused,
            self.focused,
        ) {
            self.error = Some(error);
        }
    }
    #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
    fn process_saves(&mut self) -> bool {
        use crate::defense_ui::{RunSaveAction as A, RunSaveMode as M};
        let action = self.ui.run_save.action.take();
        let modal = !matches!(self.ui.run_save.mode, M::Closed);
        let Some(saves) = &mut self.saves else {
            return false;
        };
        let active = action.is_some() || modal || saves.candidate.is_some();
        let result: Result<(), String> = (|| {
            match action {
                Some(A::Cancel) => {
                    saves.candidate = None;
                    self.ui.run_save.mode = M::Closed;
                }
                Some(A::Open) => {
                    self.ui.run_save.mode = M::List(saves.store.list()?);
                }
                Some(A::Save) if self.ui.started && self.ui.paused => {
                    let bytes = saves.journal.encode(
                        &self.session,
                        self.ui.high_water(),
                        &saves.identity,
                    )?;
                    let name = saves.store.save(&bytes)?;
                    self.ui.run_save.status = format!(
                        "Saved {name} at tick {}",
                        self.session.predicted_frame().tick()
                    );
                }
                Some(A::Select(name)) => {
                    if self.ui.started {
                        self.ui.run_save.mode = M::Confirm(name);
                    } else {
                        saves.candidate = Some(crate::defense_run_save::Candidate::parse(
                            &saves.store.read(&name)?,
                            &saves.identity,
                        )?);
                        self.ui.run_save.mode = M::Loading;
                    }
                }
                Some(A::Confirm(name)) => {
                    saves.candidate = Some(crate::defense_run_save::Candidate::parse(
                        &saves.store.read(&name)?,
                        &saves.identity,
                    )?);
                    self.ui.run_save.mode = M::Loading;
                }
                _ => {}
            }
            if let Some(candidate) = &mut saves.candidate {
                if candidate.step()? {
                    let (session, journal, high_water) = saves
                        .candidate
                        .take()
                        .expect("candidate present")
                        .finish()?;
                    self.session = session;
                    self.board = PendingBoard {
                        through: Some(self.session.next_send_tick().saturating_sub(1)),
                        selection: None,
                    };
                    saves.journal = journal;
                    self.ui.restore_paused(high_water);
                    self.ui.run_save.status = format!(
                        "Loaded tick {}. Continue when ready.",
                        self.session.predicted_frame().tick()
                    );
                    self.ui.run_save.mode = M::Closed;
                    #[cfg(feature = "coop-defense-audio")]
                    if let Err(error) = self.audio.replace(
                        self.session.predicted_frame().tick(),
                        game::run(self.session.predicted_frame()).generation,
                    ) {
                        self.error = Some(error);
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            saves.candidate = None;
            self.ui.run_save.mode = M::Closed;
            self.ui.run_save.status = bounded_save_error(&error);
        }
        active
    }
    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.accumulator = 0.0;
        self.last = Instant::now();
        if !focused {
            #[cfg(feature = "coop-defense-audio")]
            self.sync_audio(true);
            self.focus_lost_pending = true;
            self.cancel_board_intent();
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
        if let Some(error) = self.error.clone() {
            self.fail(event_loop, error);
            return;
        }
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
pub fn headless(mut options: Options) -> Result<(), String> {
    // Library callers may not have passed through CLI parsing/run admission.
    // This entry point is headless regardless of the caller's flag value.
    options.headless = true;
    #[cfg(feature = "coop-defense-audio")]
    let _audio_bank = prepare_audio(&options)?;
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
    pub(super) fn app() -> App {
        let mut ui = DefenseUi::new();
        ui.started = true;
        App {
            #[cfg(feature = "coop-defense-audio")]
            audio: Default::default(),
            #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
            saves: None,
            session: session(game::DefenseConfig::default()),
            ui,
            gfx: None,
            last: Instant::now(),
            accumulator: 0.0,
            commands: VecDeque::new(),
            error: None,
            focused: true,
            focus_lost_pending: false,
            board: PendingBoard::default(),
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
    pub(super) fn frame(app: &mut App, events: Vec<egui::Event>, delta: f64) {
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
    pub(super) fn pulse(key: egui::Key) -> Vec<egui::Event> {
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
    pub(super) fn click(x: f32, y: f32) -> Vec<egui::Event> {
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
    fn rapid_board_click_preserves_selection_across_input_delay() {
        let mut app = app();
        assert_eq!(app.session.config().input_delay, 2);
        let tick = 1.0 / 60.0;
        let layout = crate::defense_view::Layout::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 940.0),
        ));
        let cell_click = |cell| {
            let center = layout.cell(0, cell).center();
            click(center.x, center.y)
        };

        // Establish the unit through the production egui -> App -> Session path.
        frame(&mut app, pulse(egui::Key::Space), tick);
        for _ in 0..2 {
            frame(&mut app, vec![], tick);
        }
        let units = game::units(app.session.predicted_frame());
        let unit = units
            .iter()
            .find(|unit| unit.board == 0 && unit.cell == 0 && unit.active != 0)
            .expect("the consumed summon must occupy cell 0");
        let unit_id = unit.id;
        assert!(units
            .iter()
            .filter(|unit| unit.board == 0 && matches!(unit.cell, 1 | 2))
            .all(|unit| unit.active == 0));
        let summon_sequence = game::run(app.session.predicted_frame()).last_sequence;
        let unit_cell = |app: &App| {
            game::units(app.session.predicted_frame())
                .into_iter()
                .find(|unit| unit.board == 0 && unit.id == unit_id && unit.active != 0)
                .expect("the original unit must survive the move chain")
                .cell
        };

        frame(&mut app, cell_click(0), 0.0);
        assert_eq!(app.ui.selected, Some(0));
        frame(&mut app, cell_click(1), 0.0);
        let first_move = *app.commands.front().expect("first move must be queued");
        assert_eq!(app.commands.len(), 1);
        assert_eq!(
            (
                first_move.action,
                first_move.source_cell,
                first_move.target_cell
            ),
            (game::MOVE_OR_MERGE, 0, 1)
        );
        assert!(first_move.sequence > summon_sequence);

        // App has submitted the move, but delay2 has not consumed it. An empty
        // App queue alone must not make the stale board safe for the next click.
        frame(&mut app, vec![], tick);
        assert!(app.commands.is_empty());
        assert_eq!(app.ui.selected, Some(1));
        assert_eq!(unit_cell(&app), 0);
        assert_eq!(
            game::run(app.session.predicted_frame()).last_sequence,
            summon_sequence
        );
        frame(&mut app, cell_click(2), 0.0);
        assert_eq!(app.ui.selected, Some(1));
        assert!(app.ui.board_pending);
        assert!(
            app.commands.is_empty(),
            "pending clicks must not be retained"
        );

        // The already submitted first move must still reach its destination.
        // Pending clicks are ignored, never automatically replayed.
        for _ in 0..2 {
            frame(&mut app, vec![], tick);
        }
        assert_eq!(
            unit_cell(&app),
            1,
            "the original unit's already submitted move must be consumed"
        );
        assert_eq!(
            game::run(app.session.predicted_frame()).last_sequence,
            first_move.sequence
        );
        assert!(!app.ui.board_pending);
        assert_eq!(app.ui.selected, Some(1));
        frame(&mut app, vec![], 0.1);
        assert_eq!(unit_cell(&app), 1);
        frame(&mut app, cell_click(2), tick);
        assert!(app.ui.board_pending);
        for _ in 0..2 {
            frame(&mut app, vec![], tick);
        }
        assert_eq!(unit_cell(&app), 2);
        assert_eq!(app.ui.selected, Some(2));
        assert!(!app.ui.board_pending);
        assert!(app.error.is_none());
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

#[cfg(all(test, feature = "coop-defense-run-save", target_os = "linux"))]
mod run_save_tests {
    use super::*;
    fn private_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn app(path: &std::path::Path) -> App {
        let session = session(game::DefenseConfig::default());
        let journal = crate::defense_run_save::Journal::new(
            crate::defense_run_save::Origin { scene: None },
            &session,
        );
        App {
            #[cfg(feature = "coop-defense-audio")]
            audio: Default::default(),
            session,
            ui: DefenseUi::new(),
            gfx: None,
            last: Instant::now(),
            accumulator: 0.0,
            commands: VecDeque::new(),
            error: None,
            focused: true,
            focus_lost_pending: false,
            board: PendingBoard::default(),
            saves: Some(RunSaves {
                store: crate::defense_run_save_store::RunStore::open(path, &[]).unwrap(),
                identity: crate::defense_run_save::executable_identity().unwrap(),
                journal,
                candidate: None,
            }),
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
    fn key(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
            .collect()
    }
    fn click(app: &mut App, x: f32, y: f32) {
        let pos = egui::pos2(x, y);
        frame(app, vec![], 0.0);
        frame(
            app,
            [true, false]
                .into_iter()
                .map(|pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                })
                .collect(),
            0.0,
        );
    }
    fn prepare(app: &mut App) {
        frame(app, key(egui::Key::Enter), 0.0);
        frame(app, key(egui::Key::Space), 1.0 / 60.0);
        frame(app, key(egui::Key::I), 1.0 / 60.0);
        // This issued command is never admitted; pause must preserve its high-water.
        frame(app, key(egui::Key::U), 0.0);
        frame(app, key(egui::Key::P), 0.0);
        assert!(app.ui.paused);
        assert_eq!(app.ui.high_water(), 3);
        assert_eq!(app.session.predicted_frame().tick(), 2);
    }
    fn load_title(app: &mut App) {
        click(app, 600.0, 682.0);
        assert!(matches!(
            app.ui.run_save.mode,
            crate::defense_ui::RunSaveMode::List(_)
        ));
        click(app, 600.0, 321.0);
        for _ in 0..302 {
            if app.saves.as_ref().unwrap().candidate.is_none() {
                break;
            }
            frame(app, vec![], 0.1);
        }
        assert!(
            app.ui.started && app.ui.paused,
            "{}",
            app.ui.run_save.status
        );
    }
    fn trace(app: &mut App) -> (Vec<u8>, Vec<(u64, u64)>) {
        let mut checksums = vec![(
            app.session.head_tick(),
            app.session.predicted_frame().checksum(),
        )];
        let mut bytes = app.session.predicted_frame().to_bytes();
        frame(app, key(egui::Key::Enter), 0.1);
        assert_eq!(app.session.predicted_frame().tick(), 2);
        for n in 0..30 {
            frame(
                app,
                if n == 0 {
                    key(egui::Key::Space)
                } else {
                    vec![]
                },
                1.0 / 60.0,
            );
            bytes.extend_from_slice(&app.session.predicted_frame().to_bytes());
            checksums.push((
                app.session.head_tick(),
                app.session.predicted_frame().checksum(),
            ));
        }
        bytes.extend_from_slice(&app.ui.high_water().to_le_bytes());
        (bytes, checksums)
    }
    #[test]
    fn actual_app_save_title_load_and_next_input_parity() {
        let dir = private_dir();
        let mut original = app(dir.path());
        prepare(&mut original);
        let saved = original.session.predicted_frame().to_bytes();
        click(&mut original, 450.0, 582.0);
        assert_eq!(
            original.saves.as_ref().unwrap().store.list().unwrap().len(),
            1,
            "{}",
            original.ui.run_save.status
        );
        assert_eq!(original.session.predicted_frame().to_bytes(), saved);
        let mut restored = app(dir.path());
        load_title(&mut restored);
        assert_eq!(restored.ui.high_water(), 3);
        assert_eq!(trace(&mut original), trace(&mut restored));
    }
    #[test]
    fn actual_app_repeated_save_replace_cancel_and_focus() {
        let dir = private_dir();
        let mut a = app(dir.path());
        prepare(&mut a);
        click(&mut a, 450.0, 582.0);
        click(&mut a, 450.0, 582.0);
        assert_eq!(a.saves.as_ref().unwrap().store.list().unwrap().len(), 2);
        let before = a.session.predicted_frame().to_bytes();
        click(&mut a, 750.0, 582.0);
        click(&mut a, 600.0, 321.0);
        assert!(matches!(
            a.ui.run_save.mode,
            crate::defense_ui::RunSaveMode::Confirm(_)
        ));
        click(&mut a, 800.0, 642.0);
        assert_eq!(a.session.predicted_frame().to_bytes(), before);
        click(&mut a, 750.0, 582.0);
        click(&mut a, 600.0, 321.0);
        click(&mut a, 450.0, 642.0);
        assert!(a.ui.paused);
        assert_eq!(a.session.predicted_frame().to_bytes(), before);
        a.set_focused(false);
        a.set_focused(true);
        frame(&mut a, vec![], 0.1);
        assert_eq!(a.session.predicted_frame().to_bytes(), before);
    }
    #[test]
    fn separate_process_restore() {
        if std::env::var_os("ORR_NATIVE_SAVE_TEST_MODE").is_some() {
            separate_process_child();
            return;
        }
        let exe = std::env::current_exe().unwrap();
        // An explicit evidence directory preserves receipts even if a child fails.
        let evidence = std::env::var_os("ORR_NATIVE_SAVE_EVIDENCE_DIR");
        let temp = if let Some(path) = &evidence {
            tempfile::Builder::new()
                .prefix("native-run-save-")
                .tempdir_in(path)
                .unwrap()
        } else {
            private_dir()
        };
        let (root, _temporary) = if evidence.is_some() {
            (temp.keep(), None)
        } else {
            (temp.path().to_owned(), Some(temp))
        };
        let store = root.join("store");
        std::fs::create_dir(&store).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
        for mode in ["produce", "consume"] {
            let result = std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "defense_app::run_save_tests::separate_process_restore",
                ])
                .env("ORR_NATIVE_SAVE_TEST_MODE", mode)
                .env("ORR_NATIVE_SAVE_TEST_ROOT", &root)
                .output()
                .unwrap();
            std::fs::write(root.join(format!("{mode}.stdout.log")), &result.stdout).unwrap();
            std::fs::write(root.join(format!("{mode}.stderr.log")), &result.stderr).unwrap();
            assert!(
                result.status.success(),
                "{}: {} {}",
                mode,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(
            std::fs::read(root.join("produce.trace")).unwrap(),
            std::fs::read(root.join("consume.trace")).unwrap()
        );
        let read = |mode| -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(root.join(format!("{mode}.json"))).unwrap())
                .unwrap()
        };
        let produce = read("produce");
        let consume = read("consume");
        assert_ne!(produce["pid"], consume["pid"]);
        assert_eq!(produce["executable_sha256"], consume["executable_sha256"]);
        assert_eq!(produce["frame_checksums"], consume["frame_checksums"]);
        let receipt = serde_json::json!({"kind":"native CPU egui App/Session separate-process restore", "parent_pid":std::process::id(), "producer":produce,"consumer":consume,"frame_bytes_equal":true,"frame_checksums_equal":true,"os_window_physical_input_gpu_verified":false});
        std::fs::write(
            root.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
    fn separate_process_child() {
        let Ok(mode) = std::env::var("ORR_NATIVE_SAVE_TEST_MODE") else {
            return;
        };
        let root = PathBuf::from(std::env::var_os("ORR_NATIVE_SAVE_TEST_ROOT").unwrap());
        let mut a = app(&root.join("store"));
        if mode == "produce" {
            prepare(&mut a);
            click(&mut a, 450.0, 582.0);
            assert_eq!(a.saves.as_ref().unwrap().store.list().unwrap().len(), 1);
        } else {
            assert_eq!(mode, "consume");
            load_title(&mut a);
        }
        let saved_tick = a.session.head_tick();
        let saved_checksum = a.session.predicted_frame().checksum();
        let saved_high_water = a.ui.high_water();
        let (bytes, checksums) = trace(&mut a);
        std::fs::write(root.join(format!("{mode}.trace")), bytes).unwrap();
        let receipt = serde_json::json!({"mode":mode,"pid":std::process::id(),"executable_sha256":a.saves.as_ref().unwrap().identity,"origin":"default", "saved_tick":saved_tick,"saved_checksum":saved_checksum,"saved_high_water":saved_high_water,"continued_high_water":a.ui.high_water(),"frame_checksums":checksums});
        std::fs::write(
            root.join(format!("{mode}.json")),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}

#[cfg(all(test, feature = "coop-defense-run-save", target_os = "linux"))]
#[path = "defense_run_save_app_tests.rs"]
mod run_save_acceptance_tests;

#[cfg(all(test, feature = "coop-defense-audio"))]
#[path = "defense_audio_app_tests.rs"]
pub(crate) mod audio_tests;

#[cfg(test)]
#[path = "defense_pending_board_tests.rs"]
mod pending_board_tests;
