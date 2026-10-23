//! Native application shell. Authoritative simulation remains behind the bridge.
use crate::game_contract::{admit, read_scene, Admitted, HeldKeys};
use crate::presentation::StandaloneGame;
use orr_bridge::{Bridge, BridgeConfig, FrameView, InProc, PlayHost, SimControl, Snapshot};
use orr_input::{Action, ActionMap, Button, Input, Key};
use orr_render::orr_rhi::Wgpu;
use orr_render::{extract_items, Camera, RenderList, WindowRenderer};
use orr_session::{ControlOp, ReplayReader};
use orr_sim::PlayerSlot;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const OBSERVATION_LIMIT: usize = 64 * 1024 * 1024;
const REPLAY_LIMIT: usize = 4 * 1024 * 1024;

// This clock only schedules the view-side bridge pump. It never enters game state.
#[allow(clippy::disallowed_types)]
mod view_clock {
    use std::time::{Duration, Instant};
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);
    pub struct Clock {
        last_update: Instant,
        next_frame: Instant,
    }
    impl Clock {
        pub fn new() -> Self {
            let now = Instant::now();
            Self {
                last_update: now,
                next_frame: now,
            }
        }
        pub fn elapsed(&mut self) -> Duration {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(self.last_update);
            self.last_update = now;
            elapsed
        }
        pub fn request_frame(&mut self) -> bool {
            self.request_frame_at(Instant::now())
        }
        fn request_frame_at(&mut self, now: Instant) -> bool {
            if now < self.next_frame {
                return false;
            }
            // A redraw wakes the event loop too. Keep this deadline across those
            // wakes rather than unconditionally scheduling another redraw.
            // Slow frames skip missed view deadlines, never a catch-up burst.
            self.next_frame = now + FRAME_INTERVAL;
            true
        }
        pub fn next_frame(&self) -> Instant {
            self.next_frame
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn redraw_wakes_do_not_schedule_another_frame_before_deadline() {
            let mut clock = Clock::new();
            let start = clock.next_frame();
            assert!(clock.request_frame_at(start));
            for millis in 0..16 {
                assert!(!clock.request_frame_at(start + Duration::from_millis(millis)));
                assert_eq!(clock.next_frame(), start + FRAME_INTERVAL);
            }
            assert!(clock.request_frame_at(start + FRAME_INTERVAL));
            assert!(!clock.request_frame_at(start + FRAME_INTERVAL));
        }

        #[test]
        fn delayed_wake_schedules_one_frame_without_catch_up_burst() {
            let mut clock = Clock::new();
            let late = clock.next_frame() + Duration::from_secs(20);
            assert!(clock.request_frame_at(late));
            assert_eq!(clock.next_frame(), late + FRAME_INTERVAL);
            assert!(!clock.request_frame_at(late));
        }
    }
}

fn mapping() -> ActionMap {
    let bindings = [
        ("left", vec![Key::A, Key::ArrowLeft]),
        ("right", vec![Key::D, Key::ArrowRight]),
        ("up", vec![Key::W, Key::ArrowUp]),
        ("down", vec![Key::S, Key::ArrowDown]),
        ("action", vec![Key::Space]),
    ];
    ActionMap {
        version: 1,
        actions: bindings
            .into_iter()
            .map(|(name, keys)| Action {
                name: name.into(),
                bindings: keys
                    .into_iter()
                    .map(|key| Button::Keyboard { key })
                    .collect(),
            })
            .collect(),
    }
}

/// Platform-independent local ownership lifecycle. This is NOT an ERP lease.
pub struct Controller<G: StandaloneGame> {
    bridge: InProc<G, PlayHost<G>>,
    input: Input,
    #[cfg(feature = "static-game-actions")]
    actions: Option<(
        orr_input::HeldActionInput,
        crate::presentation::FrozenActionInputs<G>,
    )>,
    replay: Option<ReplayReader<G>>,
    focused: bool,
    neutral_pending: bool,
    resume_count: u64,
    resume_from_tick: u64,
    neutral_consumed_tick: Option<u64>,
    consumed: Vec<Value>,
}
impl<G: StandaloneGame> Controller<G> {
    pub fn new(admitted: &Admitted<G>, replay: Option<&[u8]>) -> Result<Self, String> {
        let session = if let Some(bytes) = replay {
            admitted.open_replay(bytes)?
        } else {
            admitted.session()?
        };
        let replay = replay
            .map(ReplayReader::<G>::parse)
            .transpose()
            .map_err(|e| e.to_string())?;
        #[cfg(feature = "static-game-actions")]
        let actions = G::action_profile()
            .map(|profile| {
                let frozen = crate::presentation::FrozenActionInputs::<G>::admit(profile.clone())?;
                let mut input = orr_input::HeldActionInput::new(profile);
                input.set_focused(false);
                input.set_blocked(true);
                Ok::<_, String>((input, frozen))
            })
            .transpose()?;
        let mut input = Input::new(mapping())?;
        input.set_focused(false);
        input.set_blocked(true);
        Ok(Self {
            bridge: InProc::new(
                PlayHost::new(session, PlayerSlot(0)),
                BridgeConfig::default(),
            ),
            input,
            #[cfg(feature = "static-game-actions")]
            actions,
            replay,
            focused: false,
            neutral_pending: false,
            resume_count: 0,
            resume_from_tick: 0,
            neutral_consumed_tick: None,
            consumed: Vec::new(),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.bridge
            .snapshot()
            .expect("play host publishes initial snapshot")
    }
    pub fn paused(&self) -> bool {
        !self.bridge.host().session().is_playing()
    }
    pub fn focused(&self) -> bool {
        self.focused
    }
    pub fn neutral_pending(&self) -> bool {
        self.neutral_pending
    }
    pub fn held(&self) -> G::Input {
        if self.paused() || self.neutral_pending || !self.focused || self.replay.is_some() {
            G::Input::default()
        } else {
            #[cfg(feature = "static-game-actions")]
            if let Some((input, frozen)) = &self.actions {
                return frozen
                    .get(input.held())
                    .expect("validated action resolver domain");
            }
            G::held_input(HeldKeys {
                x: self.input.axis("left", "right"),
                y: self.input.axis("down", "up"),
                action: self.input.state("action").held,
            })
        }
    }
    fn publish_input(&mut self) -> Result<(), String> {
        if self.replay.is_none() {
            self.bridge
                .set_input(PlayerSlot(0), self.held())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn pause(&mut self) -> Result<(), String> {
        self.input.set_blocked(true);
        #[cfg(feature = "static-game-actions")]
        if let Some((input, _)) = &mut self.actions {
            input.set_blocked(true);
        }
        // Clear the coalesced held sample before pausing. Do not advance a hidden tick.
        if self.replay.is_none() {
            self.bridge
                .set_input(PlayerSlot(0), G::Input::default())
                .map_err(|e| e.to_string())?;
        }
        self.bridge
            .control(ControlOp::Pause)
            .map_err(|e| e.to_string())?;
        self.neutral_pending = false;
        Ok(())
    }
    pub fn focus(&mut self, focused: bool) -> Result<(), String> {
        self.focused = focused;
        self.input.set_focused(focused);
        #[cfg(feature = "static-game-actions")]
        if let Some((input, _)) = &mut self.actions {
            input.set_focused(focused);
        }
        if !focused {
            self.pause()?;
        }
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), String> {
        if !self.focused || !self.paused() {
            return Ok(());
        }
        self.input.set_blocked(true);
        #[cfg(feature = "static-game-actions")]
        if let Some((input, _)) = &mut self.actions {
            input.set_blocked(true);
        }
        self.neutral_pending = self.replay.is_none();
        self.resume_from_tick = self.snapshot().tick();
        self.neutral_consumed_tick = None;
        self.resume_count = self
            .resume_count
            .checked_add(1)
            .ok_or("resume counter overflow")?;
        self.publish_input()?;
        self.bridge
            .control(ControlOp::Play)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Releases always clear state. Synthetic/repeated presses cannot arm gameplay.
    pub fn key(
        &mut self,
        key: Key,
        down: bool,
        repeat: bool,
        synthetic: bool,
    ) -> Result<(), String> {
        if !down {
            #[cfg(feature = "static-game-actions")]
            if let Some((input, _)) = &mut self.actions {
                input.key(key, false, repeat, false);
            }
            self.input
                .button(Button::Keyboard { key }, false, repeat, false);
            return self.publish_input();
        }
        if synthetic || repeat || !self.focused {
            return Ok(());
        }
        match key {
            Key::Enter => self.resume()?,
            Key::Escape => self.pause()?,
            _ => {
                #[cfg(feature = "static-game-actions")]
                if let Some((input, _)) = &mut self.actions {
                    input.key(key, true, false, false);
                }
                self.input
                    .button(Button::Keyboard { key }, true, false, false);
                self.publish_input()?;
            }
        }
        Ok(())
    }
    /// Only the production fixed-tick bridge advances simulation.
    pub fn update(&mut self, elapsed: Duration) -> Result<(), String> {
        let before = self.snapshot().tick();
        let input = self.held();
        self.publish_input()?;
        self.bridge.update(elapsed);
        let after = self.snapshot().tick();
        self.consumed.clear();
        for tick in before.saturating_add(1)..=after {
            let actual = if let Some(reader) = &self.replay {
                reader
                    .tick(tick)
                    .and_then(|(inputs, _)| inputs.first())
                    .copied()
                    .ok_or("missing consumed replay input")?
            } else {
                input
            };
            let frame = self
                .bridge
                .host()
                .session()
                .frame_at(tick)
                .ok_or("missing consumed frame")?;
            self.consumed.push(json!({"tick":tick,"input":G::input_value(&actual),
                "checksum":format!("0x{:016x}",frame.checksum()),"state":G::state_value(FrameView::of(frame))}));
        }
        if self.neutral_pending && after > before {
            self.neutral_consumed_tick = Some(before + 1);
            self.neutral_pending = false;
            // A fresh physical press is required after the neutral boundary.
            self.input.set_blocked(false);
            #[cfg(feature = "static-game-actions")]
            if let Some((input, _)) = &mut self.actions {
                input.set_blocked(false);
            }
        }
        Ok(())
    }
    pub fn take_consumed(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.consumed)
    }
    pub fn recording(&self) -> Result<Vec<u8>, String> {
        if self.replay.is_some() {
            return Err("cannot record a read-only replay".into());
        }
        if self.bridge.host().session().last_tick() > 36_000 {
            return Err("recording exceeds the supported 36000 ticks".into());
        }
        let bytes = self.bridge.host().session().save_replay();
        if bytes.len() > REPLAY_LIMIT {
            return Err("recording exceeds 4 MiB".into());
        }
        Ok(bytes)
    }
    fn poll_view(&mut self) -> orr_bridge::ViewUpdate<G::Event> {
        self.bridge.poll_view()
    }
}

#[derive(Default, Debug)]
pub struct Options {
    pub scene: Option<PathBuf>,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub observe: Option<PathBuf>,
    pub identity: bool,
    pub provenance: bool,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--identity" if !out.identity => out.identity = true,
                "--provenance" if !out.provenance => out.provenance = true,
                "--scene" | "--record" | "--replay" | "--observe" => {
                    let slot = match flag.as_str() {
                        "--scene" => &mut out.scene,
                        "--record" => &mut out.record,
                        "--replay" => &mut out.replay,
                        _ => &mut out.observe,
                    };
                    if slot.is_some() {
                        return Err(format!("duplicate {flag}"));
                    }
                    let value = args
                        .next()
                        .filter(|v| !v.starts_with("--"))
                        .ok_or_else(|| format!("missing {flag} path"))?;
                    *slot = Some(PathBuf::from(value));
                }
                _ => return Err(format!("unknown or duplicate option {flag}")),
            }
        }
        if out.provenance {
            if out.identity
                || out.scene.is_some()
                || out.record.is_some()
                || out.replay.is_some()
                || out.observe.is_some()
            {
                return Err("--provenance must be alone".into());
            }
        } else if out.scene.is_none() {
            return Err("--scene PATH is required".into());
        }
        if out.identity && (out.record.is_some() || out.replay.is_some() || out.observe.is_some()) {
            return Err("identity query cannot run or record".into());
        }
        if out.record.is_some() && out.replay.is_some() {
            return Err("record and replay are mutually exclusive".into());
        }
        for p in [&out.record, &out.replay, &out.observe]
            .into_iter()
            .flatten()
        {
            if !p.is_absolute() {
                return Err("record/replay/observe paths must be absolute".into());
            }
        }
        if out.record.is_some() && out.record == out.observe {
            return Err("record and observation outputs must differ".into());
        }
        Ok(out)
    }
}
pub fn compiled_source_sha() -> Result<&'static str, String> {
    let Some(sha) = option_env!("ORR_STATIC_STANDALONE_BUILD_SHA") else {
        return Err("compiled source provenance is absent".into());
    };
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("compiled source provenance is invalid".into());
    }
    Ok(sha)
}
fn output_destination(path: &Path, runtime: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("output path must be absolute".into());
    }
    let parent = path
        .parent()
        .ok_or("missing output parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let destination = parent.join(path.file_name().ok_or("missing output filename")?);
    let executable = runtime.canonicalize().map_err(|e| e.to_string())?;
    let executable_dir = executable.parent().ok_or("missing executable directory")?;
    let protected = if executable_dir.file_name().is_some_and(|name| name == "bin") {
        executable_dir.parent().ok_or("missing bundle root")?
    } else {
        executable_dir
    };
    if destination.starts_with(protected) {
        return Err("record/observe output must be outside the executable/bundle directory".into());
    }
    Ok(destination)
}
fn new_output(path: &Path) -> Result<File, String> {
    if !path.is_absolute() {
        return Err("output path must be absolute".into());
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}
fn replay_bytes(path: &Path) -> Result<Vec<u8>, String> {
    crate::game_contract::read_regular(path, REPLAY_LIMIT)
}

struct Observer {
    file: File,
    bytes: usize,
}
impl Observer {
    fn write(&mut self, value: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        line.push(b'\n');
        self.bytes = self
            .bytes
            .checked_add(line.len())
            .filter(|n| *n <= OBSERVATION_LIMIT)
            .ok_or("observation limit exceeded")?;
        self.file
            .write_all(&line)
            .and_then(|_| self.file.flush())
            .map_err(|e| e.to_string())
    }
}
struct Graphics {
    window: Arc<Window>,
    renderer: WindowRenderer<Wgpu>,
    camera: Camera,
    adapter: Value,
}
struct Native<G: StandaloneGame> {
    controller: Controller<G>,
    identity: Value,
    graphics: Option<Graphics>,
    observer: Option<Observer>,
    record: Option<File>,
    clock: view_clock::Clock,
    sequence: u64,
    pending: Vec<Value>,
    reason: &'static str,
    error: Option<String>,
}
// Pure view geometry. Floating-point arithmetic never enters the simulation.
#[allow(clippy::float_arithmetic)]
fn fit_camera(items: &[orr_view::RenderItem]) -> Camera {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for item in items {
        let p = [item.transform.pos.x, item.transform.pos.y];
        for n in 0..2 {
            lo[n] = lo[n].min(p[n] - item.style.size);
            hi[n] = hi[n].max(p[n] + item.style.size);
        }
    }
    if items.is_empty() {
        return Camera::new([0.0, 0.0], 8.0);
    }
    Camera::new(
        [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5],
        ((hi[0] - lo[0]).max(hi[1] - lo[1]) * 0.75).max(5.0),
    )
}
fn key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::A,
        KeyCode::KeyB => Key::B,
        KeyCode::KeyC => Key::C,
        KeyCode::KeyE => Key::E,
        KeyCode::KeyF => Key::F,
        KeyCode::KeyG => Key::G,
        KeyCode::KeyH => Key::H,
        KeyCode::KeyI => Key::I,
        KeyCode::KeyJ => Key::J,
        KeyCode::KeyK => Key::K,
        KeyCode::KeyL => Key::L,
        KeyCode::KeyM => Key::M,
        KeyCode::KeyN => Key::N,
        KeyCode::KeyO => Key::O,
        KeyCode::KeyP => Key::P,
        KeyCode::KeyQ => Key::Q,
        KeyCode::KeyR => Key::R,
        KeyCode::KeyT => Key::T,
        KeyCode::KeyU => Key::U,
        KeyCode::KeyV => Key::V,
        KeyCode::KeyX => Key::X,
        KeyCode::KeyY => Key::Y,
        KeyCode::KeyZ => Key::Z,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyW => Key::W,
        KeyCode::KeyS => Key::S,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        _ => return None,
    })
}
impl<G: StandaloneGame> Native<G> {
    fn frame(&mut self) -> Result<(), String> {
        self.controller.update(self.clock.elapsed())?;
        let update = self.controller.poll_view();
        let snapshot = update
            .snapshot
            .unwrap_or_else(|| self.controller.snapshot());
        let mut items = Vec::new();
        G::extract(snapshot.predicted(), &mut items);
        let gfx = self.graphics.as_mut().ok_or("window is not initialized")?;
        if update.resync.is_some() {
            gfx.camera = fit_camera(&items);
        }
        let mut list = RenderList::default();
        extract_items(&items, &mut list);
        let consumed = self.controller.take_consumed();
        if self.observer.is_some() {
            self.pending.extend(consumed);
            if self.pending.len() > 600 {
                return Err("unpresented observation backlog exceeded".into());
            }
        }
        if !gfx.renderer.render(&list, &gfx.camera) {
            return Ok(());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("presentation counter overflow")?;
        let state = G::state_value(snapshot.predicted());
        let status = if self.controller.paused() {
            "paused: Enter starts/resumes"
        } else if state["won"].as_u64() == Some(1) {
            "won: release then Space restarts"
        } else {
            "WASD/arrows move; Space acts; Escape pauses"
        };
        #[cfg(feature = "static-game-actions")]
        let status = if self.controller.actions.is_some() && !self.controller.paused() {
            "WASD/arrows move; custom action controls; Escape pauses"
        } else {
            status
        };
        gfx.window.set_title(&format!("{} | {status}", G::ID));
        if let Some(observer) = &mut self.observer {
            let size = gfx.renderer.size();
            observer.write(&json!({"schema":"orr.static-standalone-observation/1","sequence":self.sequence,"reason":self.reason,
                "process_id":std::process::id(),"identity":self.identity,"source_sha":compiled_source_sha().ok(),
                "tick":snapshot.tick(),"checksum":format!("0x{:016x}",snapshot.predicted().checksum()),
                "paused":self.controller.paused(),"focused":self.controller.focused(),"resume_neutral_pending":self.controller.neutral_pending(),
                "resume_count":self.controller.resume_count,"resume_from_tick":self.controller.resume_from_tick,"neutral_consumed_tick":self.controller.neutral_consumed_tick,
                "held":G::input_value(&self.controller.held()),"consumed":self.pending,"state":state,"adapter":gfx.adapter,
                "camera":{"center":gfx.camera.center,"half_extent":gfx.camera.half_extent},"size":[size.0,size.1]}))?;
            self.pending.clear();
        }
        self.reason = "redraw";
        Ok(())
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
}
impl<G: StandaloneGame + 'static> ApplicationHandler for Native<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let result = (|| {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title(G::ID)
                            .with_inner_size(LogicalSize::new(960, 720)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let size = window.inner_size();
            let renderer = WindowRenderer::new(window.clone(), (size.width, size.height), true)?;
            let info = renderer.renderer.rhi().adapter().get_info();
            let adapter = json!({"name":info.name,"driver":info.driver,"driver_info":info.driver_info,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type)});
            let mut items = Vec::new();
            G::extract(self.controller.snapshot().predicted(), &mut items);
            self.controller.focus(window.has_focus())?;
            self.graphics = Some(Graphics {
                window,
                renderer,
                camera: fit_camera(&items),
                adapter,
            });
            self.reason = "resumed";
            Ok::<(), String>(())
        })();
        if let Err(e) = result {
            self.fail(event_loop, e);
        }
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.controller.focus(false) {
            self.fail(event_loop, e);
        }
        self.reason = "suspended";
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.graphics.as_ref().is_none_or(|g| g.window.id() != id) {
            return;
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                self.reason = "close";
                let result = self.controller.pause();
                event_loop.exit();
                result
            }
            WindowEvent::Focused(focused) => {
                self.reason = if focused {
                    "focus_gained"
                } else {
                    "focus_lost"
                };
                self.controller.focus(focused)
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                self.reason = "keyboard";
                if let PhysicalKey::Code(code) = event.physical_key {
                    if let Some(key) = key(code) {
                        self.controller.key(
                            key,
                            event.state == ElementState::Pressed,
                            event.repeat,
                            is_synthetic,
                        )
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
            WindowEvent::Resized(size) => {
                self.graphics
                    .as_mut()
                    .unwrap()
                    .renderer
                    .resize(size.width, size.height);
                Ok(())
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.fail(event_loop, e);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.clock.request_frame() {
            if let Some(gfx) = &self.graphics {
                gfx.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.clock.next_frame()));
    }
}

pub fn main_for<G: StandaloneGame + 'static>() -> Result<(), String> {
    run::<G>(Options::parse(std::env::args().skip(1))?)
}
pub fn run<G: StandaloneGame + 'static>(options: Options) -> Result<(), String> {
    if options.provenance {
        println!("{}", json!({"source_sha":compiled_source_sha()?}));
        return Ok(());
    }
    let text = read_scene(options.scene.as_deref().ok_or("missing scene")?)?;
    let admitted = admit::<G>(&text)?;
    if options.identity {
        println!("{}", admitted.identity());
        return Ok(());
    }
    let replay = options.replay.as_deref().map(replay_bytes).transpose()?;
    let controller = Controller::new(&admitted, replay.as_deref())?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let record_path = options
        .record
        .as_deref()
        .map(|p| output_destination(p, &executable))
        .transpose()?;
    let observe_path = options
        .observe
        .as_deref()
        .map(|p| output_destination(p, &executable))
        .transpose()?;
    if record_path.is_some() && record_path == observe_path {
        return Err("record and observe outputs must differ after normalization".into());
    }
    let record = record_path.as_deref().map(new_output).transpose()?;
    let observer = observe_path
        .as_deref()
        .map(|p| new_output(p).map(|file| Observer { file, bytes: 0 }))
        .transpose()?;
    let mut app = Native {
        controller,
        identity: admitted.identity().clone(),
        graphics: None,
        observer,
        record,
        clock: view_clock::Clock::new(),
        sequence: 0,
        pending: Vec::new(),
        reason: "startup",
        error: None,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(error) = app.error {
        return Err(error);
    }
    if let Some(mut file) = app.record.take() {
        let bytes = app.controller.recording()?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_stay_outside_runtime_bundle_and_normalize_parents() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("bundle");
        let bin = bundle.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("standalone");
        std::fs::write(&executable, b"fixture").unwrap();
        let evidence = temp.path().join("evidence");
        std::fs::create_dir(&evidence).unwrap();
        assert!(output_destination(&bundle.join("new.orrp"), &executable).is_err());
        assert!(output_destination(&bin.join("new.ndjson"), &executable).is_err());
        assert!(output_destination(Path::new("relative.orrp"), &executable).is_err());
        assert_eq!(
            output_destination(&evidence.join("../evidence/new.orrp"), &executable).unwrap(),
            output_destination(&evidence.join("new.orrp"), &executable).unwrap()
        );
        #[cfg(unix)]
        {
            let alias = temp.path().join("bundle-alias");
            std::os::unix::fs::symlink(&bundle, &alias).unwrap();
            assert!(output_destination(&alias.join("new.orrp"), &executable).is_err());
        }
    }

    #[test]
    fn output_creation_never_clobbers_existing_files_or_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("keep");
        std::fs::write(&existing, b"preserve").unwrap();
        assert!(new_output(&existing).is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"preserve");
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(&existing, &alias).unwrap();
            assert!(new_output(&alias).is_err());
            assert_eq!(std::fs::read(&existing).unwrap(), b"preserve");
        }
    }
}
