//! Standalone scene-pinned point-route player. Project admission precedes GPU
//! startup; restart and rendering only use the retained immutable initial Frame.
use crate::{
    navigation_project::{PreparedProject, TICK_RATE},
    navigation_project_view::{self as view, NavigationCamera, NavigationRenderer},
};
use orr_bridge::{ControlOp, FrameView, PlaySession};
use orr_games::navigation_yard3d_game::NavigationYard3D;
use orr_render::orr_rhi::{Acquire, Rhi, TextureFormat, Wgpu, WgpuOptions};
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

pub const HELP: &str = "navigation_playground --project PATH [--headless [--ticks 0..6000] [--restart-after 0..6000] [--capture NEW.png [--capture-size WIDTHxHEIGHT] [--software-gpu]]]\nSpace/P: pause or resume; R: restart and resume; Escape: quit.\nMouse left drag: orbit; right/middle drag: pan; wheel: zoom (view only).\nHeadless defaults to zero ticks. Capture uses the production GPU renderer.\nOne automatic point agent follows its admitted terrain route; no physics, WASD movement, radius, headroom, climbing or crowds.";
pub const MAX_TICKS: u32 = 6000;

#[derive(Debug)]
pub struct Options {
    pub project: PathBuf,
    pub headless: bool,
    pub ticks: u32,
    pub restart_after: Option<u32>,
    pub capture: Option<PathBuf>,
    pub capture_size: (u32, u32),
    pub software_gpu: bool,
}
impl Options {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut project = None;
        let mut headless = false;
        let mut ticks = 0;
        let mut restart_after = None;
        let mut capture = None;
        let mut capture_size = (1024, 768);
        let mut software_gpu = false;
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
                    if ticks > MAX_TICKS {
                        return Err("--ticks must be between 0 and 6000".into());
                    }
                }
                "--restart-after" => {
                    let ticks = args
                        .next()
                        .ok_or("--restart-after requires a value")?
                        .parse::<u32>()
                        .map_err(|_| "invalid --restart-after")?;
                    if ticks > MAX_TICKS {
                        return Err("--restart-after must be between 0 and 6000".into());
                    }
                    restart_after = Some(ticks);
                }
                "--capture" => {
                    capture = Some(PathBuf::from(
                        args.next().ok_or("--capture requires a new PNG path")?,
                    ))
                }
                "--capture-size" => {
                    let value = args.next().ok_or("--capture-size requires WIDTHxHEIGHT")?;
                    let (width, height) = value
                        .split_once('x')
                        .ok_or("--capture-size requires WIDTHxHEIGHT")?;
                    capture_size = (
                        width.parse().map_err(|_| "invalid capture width")?,
                        height.parse().map_err(|_| "invalid capture height")?,
                    );
                    view::validate_size(capture_size)?;
                }
                "--software-gpu" => software_gpu = true,
                _ => return Err(format!("unknown option: {flag}\n{HELP}")),
            }
        }
        if !headless && (seen.contains("--ticks") || restart_after.is_some() || capture.is_some()) {
            return Err("--ticks, --restart-after and --capture require --headless".into());
        }
        if capture.is_none() && (seen.contains("--capture-size") || software_gpu) {
            return Err("--capture-size and --software-gpu require --capture".into());
        }
        Ok(Self {
            project: project
                .ok_or("--project is required; no fallback terrain or route is created")?,
            headless,
            ticks,
            restart_after,
            capture,
            capture_size,
            software_gpu,
        })
    }
}

pub fn run(options: Options) -> Result<(), String> {
    let mut app = App::new(PreparedProject::open(&options.project)?)?;
    if !options.headless {
        return run_window(app);
    }
    if options.ticks > MAX_TICKS {
        return Err("--ticks must be between 0 and 6000".into());
    }
    println!(
        "navigation initial checksum: 0x{:016x}",
        app.frame().checksum()
    );
    if let Some(ticks) = options.restart_after {
        app.advance_ticks(ticks)?;
        eprintln!(
            "navigation restart after tick: {} checksum: 0x{:016x}",
            app.frame().tick(),
            app.frame().checksum()
        );
        app.restart()?;
    }
    app.advance_ticks(options.ticks)?;
    let navigation = view::read_navigation(app.frame())?;
    println!(
        "navigation tick: {} checksum: 0x{:016x}",
        app.frame().tick(),
        app.frame().checksum()
    );
    eprintln!(
        "navigation status: {:?} position_raw: {:?}",
        navigation.navigator.status(),
        navigation.navigator.position().map(orr_fp::FP::raw)
    );
    eprintln!(
        "navigation state checksum: {}",
        navigation
            .navigator
            .checksum()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    if let Some(path) = options.capture {
        if path.exists() {
            return Err(format!(
                "capture destination already exists: {}",
                path.display()
            ));
        }
        view::validate_size(options.capture_size)?;
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: options.software_gpu,
            ..Default::default()
        })?;
        let mut renderer = NavigationRenderer::new(&gpu, options.capture_size, app.frame())?;
        app.render(&mut renderer)?;
        save_png_new(&path, &renderer.read_rgba8(), options.capture_size)?;
        println!(
            "navigation capture adapter: {} software: {}",
            gpu.adapter_name(),
            gpu.is_software()
        );
    }
    Ok(())
}

/// Creates a new capture only; existing evidence is never overwritten.
pub fn save_png_new(path: &Path, rgba: &[u8], size: (u32, u32)) -> Result<(), String> {
    view::validate_size(size)?;
    if rgba.len() != size.0 as usize * size.1 as usize * 4 {
        return Err("navigation capture returned an invalid image size".into());
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
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

struct Graphics {
    window: Arc<Window>,
    rhi: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    renderer: NavigationRenderer,
    size: (u32, u32),
    format: TextureFormat,
}

/// The actual production application, shared by native play, headless execution
/// and acceptance tests. Controls never synthesize gameplay inputs or alter route data.
pub struct App {
    project: PreparedProject,
    session: PlaySession<NavigationYard3D>,
    camera: NavigationCamera,
    graphics: Option<Graphics>,
    focused: bool,
    down: HashSet<KeyCode>,
    drag: Option<MouseButton>,
    cursor: Option<[f32; 2]>,
    last: Instant,
    accumulated: Duration,
    error: Option<String>,
}
impl App {
    pub fn new(project: PreparedProject) -> Result<Self, String> {
        let session = project.scene().session()?;
        view::admit_presentation(FrameView::of(session.frame()))?;
        let camera = NavigationCamera::new(FrameView::of(session.frame()))?;
        Ok(Self {
            project,
            session,
            camera,
            graphics: None,
            focused: false,
            down: HashSet::new(),
            drag: None,
            cursor: None,
            last: Instant::now(),
            accumulated: Duration::ZERO,
            error: None,
        })
    }
    pub fn frame(&self) -> FrameView<'_> {
        FrameView::of(self.session.frame())
    }
    pub fn is_paused(&self) -> bool {
        !self.session.is_playing()
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.session.control(if paused {
            ControlOp::Pause
        } else {
            ControlOp::Play
        });
        self.reset_clock();
    }
    pub fn restart(&mut self) -> Result<(), String> {
        // Construct all replacement CPU state before committing either part.
        let session = self.project.scene().session()?;
        let camera = NavigationCamera::new(FrameView::of(session.frame()))?;
        self.session = session;
        self.camera = camera;
        self.clear_pointer();
        self.reset_clock();
        Ok(())
    }
    /// Seek only within this session's recorded history, never by reopening source files.
    pub fn seek(&mut self, tick: u64) -> Result<(), String> {
        self.session.seek(tick).map_err(|e| e.to_string())?;
        self.clear_pointer();
        self.reset_clock();
        Ok(())
    }
    /// Fixed idle clock ticks, respecting the real session's pause state.
    pub fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
        if ticks > MAX_TICKS {
            return Err("navigation tick batch exceeds 6000".into());
        }
        for _ in 0..ticks {
            self.session.tick();
            let failed = self
                .session
                .frame()
                .singleton::<orr_games::navigation_yard3d_game::NavigationStepStatus>();
            if failed.failed != 0 {
                return Err(format!(
                    "navigation stopped at tick {}: {}",
                    failed.failed_tick,
                    failed.message()
                ));
            }
        }
        Ok(())
    }
    pub fn render(&self, renderer: &mut NavigationRenderer) -> Result<(), String> {
        renderer.render(self.frame(), &self.camera.camera(renderer.target().size()))
    }
    fn reset_clock(&mut self) {
        self.last = Instant::now();
        self.accumulated = Duration::ZERO;
    }
    fn clear_pointer(&mut self) {
        self.drag = None;
        self.cursor = None;
    }
    fn focus(&mut self, focused: bool) {
        self.focused = focused;
        self.down.clear();
        self.clear_pointer();
        self.reset_clock();
    }
    /// Returns true only for a fresh eligible quit key; repeated/synthetic keys
    /// cannot toggle pause or restart. Releases are always observed.
    fn key(
        &mut self,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
        synthetic: bool,
    ) -> Result<bool, String> {
        if !pressed {
            self.down.remove(&code);
            return Ok(false);
        }
        let fresh = self.down.insert(code);
        if !self.focused || repeat || synthetic || !fresh {
            return Ok(false);
        }
        match code {
            KeyCode::Space | KeyCode::KeyP => self.set_paused(!self.is_paused()),
            KeyCode::KeyR => self.restart()?,
            KeyCode::Escape => return Ok(true),
            _ => {}
        }
        Ok(false)
    }
    fn advance_clock(&mut self, elapsed: Duration, visible: bool) -> Result<(), String> {
        if !self.focused || !visible || self.is_paused() {
            self.accumulated = Duration::ZERO;
            return Ok(());
        }
        self.accumulated += elapsed.min(Duration::from_millis(250));
        let interval = Duration::from_secs_f64(1.0 / f64::from(TICK_RATE));
        while self.accumulated >= interval {
            self.advance_ticks(1)?;
            self.accumulated -= interval;
        }
        Ok(())
    }
    fn draw(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last);
        self.last = now;
        let Some(size) = self.graphics.as_ref().map(|g| g.size) else {
            return Ok(());
        };
        self.advance_clock(elapsed, size.0 > 0 && size.1 > 0)?;
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        let status = view::read_navigation(self.frame())?.navigator.status();
        let title = format!(
            "Terrain Point Route | tick {} | {:?}{} | Space/P pause, R restart, Esc quit | mouse orbit/pan/zoom",
            self.frame().tick(), status, if self.is_paused() { " (paused)" } else { "" }
        );
        let camera = self.camera.camera(size);
        let frame = FrameView::of(self.session.frame());
        let g = self.graphics.as_mut().expect("graphics checked above");
        g.window.set_title(&title);
        if let Acquire::Frame(surface_frame) = g.rhi.acquire_frame(&mut g.surface) {
            let target = g.rhi.frame_view(&surface_frame);
            g.renderer.render_to(
                frame,
                &camera,
                orr_render::ImportedSceneTarget {
                    view: target,
                    size,
                    format: g.format,
                    sample_count: 1,
                },
            )?;
            g.rhi.present(surface_frame);
        }
        Ok(())
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
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
                            .with_title("Terrain Point Route")
                            .with_inner_size(LogicalSize::new(1024.0, 768.0))
                            .with_max_inner_size(LogicalSize::new(2048.0, 2048.0)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let inner = window.inner_size();
            let size = (inner.width.max(1), inner.height.max(1));
            let (rhi, surface) =
                Wgpu::for_window(window.clone(), size, true, WgpuOptions::default())?;
            let format = rhi.surface_format(&surface);
            let renderer = NavigationRenderer::new_with_format(&rhi, size, self.frame(), format)?;
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
                self.focus(g.window.has_focus());
                self.graphics = Some(g);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.focus(false);
        self.graphics = None;
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.graphics.as_ref().is_none_or(|g| g.window.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => self.focus(focused),
            WindowEvent::Resized(size) => {
                if size.width > view::MAX_TARGET_SIDE || size.height > view::MAX_TARGET_SIDE {
                    self.fail(
                        event_loop,
                        "window exceeds the 4096-pixel size limit".into(),
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
                self.clear_pointer();
                self.reset_clock();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match self.key(
                        code,
                        event.state == ElementState::Pressed,
                        event.repeat,
                        is_synthetic,
                    ) {
                        Ok(true) => event_loop.exit(),
                        Err(error) => self.fail(event_loop, error),
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => self.clear_pointer(),
            WindowEvent::CursorMoved { position, .. } if self.focused => {
                let next = [position.x as f32, position.y as f32];
                if next.iter().any(|v| !v.is_finite()) {
                    self.clear_pointer();
                    return;
                }
                if let Some(previous) = self.cursor {
                    let delta = [next[0] - previous[0], next[1] - previous[1]];
                    match self.drag {
                        Some(MouseButton::Left) => self.camera.orbit.orbit(delta),
                        Some(MouseButton::Middle | MouseButton::Right) => {
                            let size = self.graphics.as_ref().map_or((1, 1), |g| g.size);
                            self.camera.orbit.pan(delta, size);
                        }
                        _ => {}
                    }
                }
                self.cursor = Some(next);
            }
            WindowEvent::MouseInput { state, button, .. } if self.focused => {
                if state == ElementState::Pressed {
                    self.drag = Some(button);
                } else if self.drag == Some(button) {
                    self.drag = None;
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if steps.is_finite() {
                    self.camera.orbit.zoom(steps);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(g) = &self.graphics else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        let next = self.last + Duration::from_millis(16);
        if Instant::now() >= next {
            g.window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            next.max(Instant::now() + Duration::from_millis(1)),
        ));
    }
}
pub fn run_window(mut app: App) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::{fp, FP};
    use orr_games::navigation_yard3d_game::{NavigationAgentSpec, NavigationScenePin, PIN_NAME};
    use orr_navigation::{AgentProfile, NavigationStatus, TerrainGraph};
    use orr_reflect::Scene;
    use orr_sim::Simulation;

    // A genuine pinned sloped heightfield with a hole barrier and a nontrivial
    // detour. No rendered proxy steers or substitutes for its Frame-owned agent.
    fn fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let mut holes = vec![false; 64];
        for z in 2..6 {
            for x in 3..5 {
                holes[z * 8 + x] = true;
            }
        }
        let terrain = orr_terrain::Terrain::new(
            "navigation/playground-acceptance.orrt".into(),
            9,
            9,
            [fp!(-4); 2],
            FP::ONE,
            (0..81).map(|i| FP::from_raw((i % 9) * 16384)).collect(),
            holes,
        )
        .unwrap();
        let spec = NavigationAgentSpec {
            start: [fp!(-3), FP::ZERO],
            goal: [fp!(3), FP::ZERO],
            max_slope: fp!(0.5),
            distance_per_tick: fp!(0.125),
        };
        let graph = TerrainGraph::build(
            &terrain,
            AgentProfile {
                max_slope: spec.max_slope,
                ..Default::default()
            },
        )
        .unwrap();
        let mut frame = orr_ecs::Frame::new(Simulation::<NavigationYard3D>::build_registry());
        frame.set_singleton(
            NavigationScenePin::new("terrain.orrt", &terrain, &graph, spec).unwrap(),
        );
        let mut scene = Scene::unbake(&crate::navigation_project::types(), &frame, None).unwrap();
        scene.singletons.retain(|(name, _)| name == PIN_NAME);
        std::fs::write(root.path().join("point.scene.yaml"), scene.to_yaml()).unwrap();
        std::fs::write(root.path().join("terrain.orrt"), terrain.cook()).unwrap();
        std::fs::write(root.path().join("orr.project.json"),
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"terrain-point-route-3d-v1","scene":"point.scene.yaml"}}"#).unwrap();
        let app = App::new(PreparedProject::open(root.path()).unwrap()).unwrap();
        (root, app)
    }
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn cli_is_closed_and_capture_is_bounded() {
        assert!(Options::parse(vec![]).is_err());
        assert!(Options::parse(args(&["--project", "x", "--ticks", "1"])).is_err());
        assert!(
            Options::parse(args(&["--project", "x", "--headless", "--ticks", "6001"])).is_err()
        );
        assert!(Options::parse(args(&["--project", "x", "--headless", "--headless"])).is_err());
        assert!(Options::parse(args(&["--project", "x", "--capture-size", "80x60"])).is_err());
        assert!(Options::parse(args(&["--project", "x", "--software-gpu"])).is_err());
        assert!(Options::parse(args(&["--project", "x", "--restart-after", "1"])).is_err());
        assert!(Options::parse(args(&[
            "--project",
            "x",
            "--headless",
            "--restart-after",
            "6001"
        ]))
        .is_err());
        assert_eq!(
            Options::parse(args(&[
                "--project",
                "x",
                "--headless",
                "--restart-after",
                "20"
            ]))
            .unwrap()
            .restart_after,
            Some(20)
        );
        for size in ["0x100", "100x4097", "90x2x3", "NaNx40"] {
            assert!(Options::parse(args(&[
                "--project",
                "x",
                "--headless",
                "--capture",
                "x.png",
                "--capture-size",
                size
            ]))
            .is_err());
        }
        assert!(Options::parse(args(&["--project", "x", "--headless", "--hold", "left"])).is_err());
        let parsed = Options::parse(args(&[
            "--project",
            "x",
            "--headless",
            "--ticks",
            "2",
            "--capture",
            "new.png",
            "--capture-size",
            "480x720",
            "--software-gpu",
        ]))
        .unwrap();
        assert_eq!(parsed.capture_size, (480, 720));
        assert!(parsed.software_gpu);
    }
    #[test]
    fn production_app_pause_restart_and_focus_use_retained_initial_frame() {
        let (root, mut app) = fixture();
        let initial = app.session.frame().to_bytes();
        let initial_camera = app.camera.orbit;
        assert_eq!(app.session.player_count(), 2);
        app.advance_ticks(20).unwrap();
        assert_ne!(app.session.frame().to_bytes(), initial);
        app.focus(true);
        app.key(KeyCode::Space, true, false, false).unwrap();
        let paused = app.session.frame().to_bytes();
        app.advance_ticks(20).unwrap();
        app.advance_clock(Duration::from_secs(3), true).unwrap();
        assert_eq!(app.session.frame().to_bytes(), paused);
        app.key(KeyCode::Space, true, true, false).unwrap();
        assert!(app.is_paused(), "held repeat must not toggle pause");
        app.key(KeyCode::Space, false, false, false).unwrap();
        app.key(KeyCode::Space, true, false, false).unwrap();
        app.advance_ticks(1).unwrap();
        assert_eq!(app.frame().tick(), 21);
        app.focus(false);
        let unfocused = app.session.frame().to_bytes();
        app.advance_clock(Duration::from_secs(3), true).unwrap();
        app.key(KeyCode::KeyR, true, false, false).unwrap();
        assert_eq!(app.session.frame().to_bytes(), unfocused);
        app.focus(true);
        app.advance_clock(Duration::from_secs(3), false).unwrap();
        assert_eq!(
            app.session.frame().to_bytes(),
            unfocused,
            "minimized clock stays still"
        );
        app.key(KeyCode::KeyR, true, false, true).unwrap();
        assert_eq!(app.frame().tick(), 21, "synthetic press is not a restart");
        app.key(KeyCode::KeyR, false, false, false).unwrap();
        app.camera.orbit.orbit([20.0, 10.0]);
        app.camera.orbit.zoom(2.0);
        app.drag = Some(MouseButton::Left);
        app.cursor = Some([1.0, 2.0]);
        std::fs::remove_dir_all(root.path()).unwrap();
        app.key(KeyCode::KeyR, true, false, false).unwrap();
        assert_eq!(app.session.frame().to_bytes(), initial);
        assert_eq!(app.camera.orbit, initial_camera);
        assert!(!app.is_paused());
        assert!(app.drag.is_none() && app.cursor.is_none());
        app.advance_ticks(1).unwrap();
        app.key(KeyCode::KeyR, true, true, false).unwrap();
        assert_eq!(
            app.frame().tick(),
            1,
            "held restart cannot reset each frame"
        );
        // Presentation controls never inject Yard gameplay input.
        let before = app.session.frame().to_bytes();
        for key in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
            app.key(key, true, false, false).unwrap();
        }
        assert_eq!(app.session.frame().to_bytes(), before);
        let replay_checksum = app.frame().checksum();
        app.seek(0).unwrap();
        assert!(app.is_paused());
        app.set_paused(false);
        app.advance_ticks(1).unwrap();
        assert_eq!(
            app.frame().checksum(),
            replay_checksum,
            "seek/replay uses admitted Frame after deletion"
        );
        assert!(app.seek(1000).is_err());
        assert_eq!(app.frame().checksum(), replay_checksum);
    }
    #[test]
    fn capture_rejects_existing_evidence_and_invalid_pixel_count() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("capture.png");
        save_png_new(&path, &[20, 40, 60, 255], (1, 1)).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(save_png_new(&path, &[0; 4], (1, 1)).is_err());
        assert_eq!(before, std::fs::read(&path).unwrap());
        assert!(save_png_new(&root.path().join("invalid.png"), &[0; 3], (1, 1)).is_err());
        assert!(!root.path().join("invalid.png").exists());
    }
    fn color_count(bytes: &[u8], kind: u8) -> usize {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| match kind {
                0 => p[0] < 100 && p[1] >= 165 && p[2] >= 175, // cyan route, not green or sky
                1 => p[0] >= 160 && p[1] < 110 && p[2] >= 140, // magenta agent
                _ => {
                    p[0] > 30
                        && p[0] < 135
                        && p[1] > p[0].saturating_add(25)
                        && p[2] < p[1]
                        && p[2] < 150
                } // checker terrain, not route/grid
            })
            .count()
    }
    fn evidence(app: &App, renderer: &mut NavigationRenderer, gpu: &Wgpu, name: &str) -> Vec<u8> {
        let checksum = app.frame().checksum();
        app.render(renderer).unwrap();
        let rgba = renderer.read_rgba8();
        assert_eq!(app.frame().checksum(), checksum, "render is read-only");
        assert!(color_count(&rgba, 0) > 20, "cyan path missing in {name}");
        assert!(
            color_count(&rgba, 1) > 10,
            "magenta Frame agent missing in {name}"
        );
        assert!(color_count(&rgba, 2) > 200, "terrain missing in {name}");
        if let Some(root) = std::env::var_os("NAVIGATION_PLAYGROUND_CAPTURE_DIR") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            save_png_new(
                &root.join(format!("{name}.png")),
                &rgba,
                renderer.target().size(),
            )
            .unwrap();
            let navigation = view::read_navigation(app.frame()).unwrap();
            let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            let metadata = serde_json::json!({
                "tick": app.frame().tick(), "frame_checksum": format!("0x{checksum:016x}"),
                "position_raw": navigation.navigator.position().map(FP::raw),
                "status": format!("{:?}", navigation.navigator.status()),
                "terrain_revision": hex(&navigation.terrain.revision()),
                "graph_revision": hex(&navigation.graph.revision()),
                "navigator_checksum": hex(&navigation.navigator.checksum()),
                "adapter": gpu.adapter_name(), "software": gpu.is_software(),
                "size": renderer.target().size(), "terrain_models": 1, "overlay_models": 1,
                "source_sha": std::env::var("NAVIGATION_SOURCE_SHA").ok(),
            });
            std::fs::write(
                root.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
        }
        rgba
    }
    /// Mandatory real software GPU, not a skipped test or a standalone model draw.
    #[test]
    fn production_software_gpu_start_mid_reached_pause_restart_resize() {
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        })
        .expect("mandatory terrain point-route software GPU adapter");
        assert!(gpu.is_software());
        eprintln!(
            "terrain point-route acceptance adapter: {}",
            gpu.adapter_name()
        );
        let (root, mut app) = fixture();
        let mut renderer = NavigationRenderer::new(&gpu, (800, 600), app.frame()).unwrap();
        let start = evidence(&app, &mut renderer, &gpu, "01-start");
        let start_position = view::read_navigation(app.frame())
            .unwrap()
            .navigator
            .position();
        app.advance_ticks(20).unwrap();
        let mid = evidence(&app, &mut renderer, &gpu, "02-mid");
        let nav = view::read_navigation(app.frame()).unwrap();
        assert_eq!(nav.navigator.status(), NavigationStatus::Moving);
        assert_ne!(nav.navigator.position(), start_position);
        assert!(
            start
                .as_chunks::<4>()
                .0
                .iter()
                .zip(mid.as_chunks::<4>().0.iter())
                .filter(|(a, b)| a != b)
                .count()
                > 40
        );
        app.focus(true);
        app.key(KeyCode::Space, true, false, false).unwrap();
        app.advance_ticks(30).unwrap();
        let paused = evidence(&app, &mut renderer, &gpu, "03-paused");
        assert_eq!(paused, mid);
        app.key(KeyCode::Space, false, false, false).unwrap();
        app.key(KeyCode::Space, true, false, false).unwrap();
        app.advance_ticks(240).unwrap();
        let reached = evidence(&app, &mut renderer, &gpu, "04-reached");
        let nav = view::read_navigation(app.frame()).unwrap();
        assert_eq!(nav.navigator.status(), NavigationStatus::Arrived);
        assert_eq!(
            nav.navigator.position(),
            nav.graph
                .project(&nav.terrain, nav.spec.goal)
                .unwrap()
                .position
        );
        assert_ne!(reached, mid);
        app.seek(20).unwrap();
        let sought = evidence(&app, &mut renderer, &gpu, "04b-sought-mid");
        assert_eq!(sought, mid, "seek reuses the same Frame-owned geometry");
        app.set_paused(false);
        app.advance_ticks(240).unwrap();
        app.render(&mut renderer).unwrap();
        assert_eq!(
            renderer.read_rgba8(),
            reached,
            "recorded replay renders identically"
        );
        let before_resize = app.frame().checksum();
        renderer.resize((480, 720)).unwrap();
        let resized = evidence(&app, &mut renderer, &gpu, "05-portrait");
        assert_eq!(app.frame().checksum(), before_resize);
        assert_eq!(resized.len(), 480 * 720 * 4);
        assert!(renderer.resize((0, 600)).is_err());
        assert_eq!(renderer.target().size(), (480, 720));
        assert_eq!(renderer.read_rgba8(), resized);
        let mut invalid = app.session.frame().clone();
        let mut pin = *invalid.singleton::<NavigationScenePin>();
        pin.agent.distance_per_tick += FP::from_raw(1);
        invalid.set_singleton(pin);
        assert!(renderer
            .render(FrameView::of(&invalid), &app.camera.camera((480, 720)))
            .is_err());
        assert_eq!(
            renderer.read_rgba8(),
            resized,
            "rejected Frame preserves last good image"
        );
        let mut bad_camera = app.camera.camera((480, 720));
        bad_camera.eye[0] = f32::NAN;
        assert!(renderer.render(app.frame(), &bad_camera).is_err());
        assert_eq!(
            renderer.read_rgba8(),
            resized,
            "rejected camera preserves last good image"
        );
        std::fs::remove_dir_all(root.path()).unwrap();
        app.key(KeyCode::KeyR, true, false, false).unwrap();
        renderer.resize((800, 600)).unwrap();
        let restarted = evidence(&app, &mut renderer, &gpu, "06-restarted-without-files");
        assert_eq!(restarted, start);
        assert_eq!(app.frame().tick(), 0);
        app.camera.orbit.orbit([45.0, 15.0]);
        app.camera.orbit.zoom(0.5);
        let camera_only = evidence(&app, &mut renderer, &gpu, "07-view-controls");
        assert_ne!(camera_only, start);
        assert_eq!(app.frame().tick(), 0);
    }
}
