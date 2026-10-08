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
    let project = PreparedProject::open(&options.project)?;
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
struct App {
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
    fn draw(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last);
        self.last = now;
        let Some(g) = &mut self.graphics else {
            return Ok(());
        };
        if g.size.0 == 0 || g.size.1 == 0 {
            self.accumulated = Duration::ZERO;
            return Ok(());
        }
        if self.focused {
            self.accumulated += elapsed.min(Duration::from_millis(250));
            let interval = Duration::from_secs_f64(1.0 / f64::from(TICK_RATE));
            while self.accumulated >= interval {
                step(&mut self.sim, self.keys.sample());
                self.accumulated -= interval;
            }
        } else {
            self.accumulated = Duration::ZERO;
        }
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
            g.rhi.present(surface_frame);
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
                        self.focused,
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
            WindowEvent::CursorMoved { position, .. } if self.focused => {
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
            WindowEvent::MouseInput { state, button, .. } if self.focused => {
                self.drag = if state == ElementState::Pressed {
                    Some(button)
                } else {
                    None
                };
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused => {
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
pub fn run_window(project: PreparedProject) -> Result<(), String> {
    let sim = project.scene().simulation()?;
    let orbit = project.camera().map_or_else(
        || OrbitCamera::new([0.0, 0.5, 0.0], 0.65, 1.02, 60.0),
        |camera| camera.document.orbit(),
    );
    let mut app = App {
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
    };
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
