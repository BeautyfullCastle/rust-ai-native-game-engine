//! Forest Valley: walk or fly through a forested mountain stream.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]

use orr_rhi::{Acquire, Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_scenery::{viewpoints, Camera, Environment, Motion, Preset, Renderer, World};
use std::{path::PathBuf, process::ExitCode, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const HELP: &str = "forest_valley [--seed N] [--view 1..6] [--settings FILE.json] [--hour H] [--size WxH] [--no-vsync] [--capture FILE.png] [--time SECONDS]\n\
Without --capture a window opens.\n\
  W A S D     move            Shift   move fast\n\
  Q / E       down / up       F       switch between walking and flying\n\
  mouse drag  look around     1..6    jump to a viewpoint\n\
  T / G       later / earlier Space   pause wind and water\n\
  Esc         quit\n\
--settings loads a preset saved from the editor (Edit > Forest Valley scene). --seed and --hour (5 to 19) then replace its values.\n\
--size sets the window or the captured image. --no-vsync shows the frame rate the machine can reach.\n\
--capture draws one frame without a window and writes it as a PNG file.";

const HOURS: std::ops::RangeInclusive<f32> = 5.0..=19.0;

struct Options {
    preset: Preset,
    view: usize,
    time: f32,
    capture: Option<PathBuf>,
    size: Option<(u32, u32)>,
    vsync: bool,
}

fn parse() -> Result<Option<Options>, String> {
    let mut options = Options { preset: Preset::default(), view: 0, time: 20.0, capture: None, size: None, vsync: true };
    let (mut seed, mut hour) = (None, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value\n{HELP}"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--seed" => seed = Some(value()?.parse().map_err(|_| "--seed needs a whole number")?),
            "--view" => {
                let view: usize = value()?.parse().map_err(|_| "--view needs a number from 1 to 6")?;
                if !(1..=6).contains(&view) {
                    return Err("--view needs a number from 1 to 6".into());
                }
                options.view = view - 1;
            }
            "--settings" => {
                let path = value()?;
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
                options.preset = Preset::from_json(&text).map_err(|e| format!("{path}: {e}"))?;
            }
            "--hour" => {
                let parsed: f32 = value()?.parse().map_err(|_| "--hour needs a number from 5 to 19")?;
                if !HOURS.contains(&parsed) {
                    return Err("--hour needs a number from 5 to 19".into());
                }
                hour = Some(parsed);
            }
            "--time" => {
                options.time = value()?.parse().map_err(|_| "--time needs a number of seconds")?;
                if !options.time.is_finite() {
                    return Err("--time needs a number of seconds".into());
                }
            }
            "--no-vsync" => options.vsync = false,
            "--capture" => options.capture = Some(PathBuf::from(value()?)),
            "--size" => {
                let text = value()?;
                let parsed = text.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
                options.size = match parsed {
                    Some((w, h)) if (16..=8192).contains(&w) && (16..=8192).contains(&h) => Some((w, h)),
                    _ => return Err("--size needs WIDTHxHEIGHT with both from 16 to 8192".into()),
                };
            }
            _ => return Err(format!("unknown argument {arg}\n{HELP}")),
        }
    }
    if let Some(seed) = seed {
        options.preset.seed = seed;
    }
    if let Some(hour) = hour {
        options.preset.settings.sun.hour = hour;
    }
    Ok(Some(options))
}

fn capture(world: &World, camera: &Camera, options: &Options, path: &PathBuf) -> Result<(), String> {
    let rhi = Wgpu::headless(WgpuOptions { allow_software_fallback: false, ..Default::default() })?;
    let format = TextureFormat::Rgba8UnormSrgb;
    let size = options.size.unwrap_or((1920, 1080));
    let texture = rhi.create_texture(&TextureDesc {
        label: "forest valley capture",
        width: size.0,
        height: size.1,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = rhi.create_texture_view(&texture, None);
    let mut renderer = Renderer::new(rhi.clone(), format, world, options.preset.quality);
    renderer.render(&view, size, camera, &Environment::at(world, camera, options.time, 0.0), &options.preset.settings);
    let pixels = rhi.read_texture(&texture);
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    let stats = renderer.last_frame_stats();
    println!("{} ({}): {} triangles, {} instances, {} draws", path.display(), rhi.adapter_name(), stats.triangles, stats.instances, stats.draws);
    Ok(())
}

#[derive(Default)]
struct Keys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    fast: bool,
}

struct Graphics {
    window: Arc<Window>,
    rhi: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    renderer: Renderer<Wgpu>,
    size: (u32, u32),
}

struct App {
    world: World,
    views: Vec<Camera>,
    camera: Camera,
    preset: Preset,
    time: f32,
    paused: bool,
    walking: bool,
    keys: Keys,
    looking: bool,
    cursor: Option<(f64, f64)>,
    last: Option<Instant>,
    title_timer: f32,
    frames: u32,
    vsync: bool,
    window_size: Option<(u32, u32)>,
    occluded: bool,
    graphics: Option<Graphics>,
    error: Option<String>,
}

impl App {
    fn step(&mut self, dt: f32) {
        if !self.paused {
            self.time += dt;
        }
        let axis = |positive: bool, negative: bool| f32::from(u8::from(positive)) - f32::from(u8::from(negative));
        let motion = Motion {
            ahead: axis(self.keys.forward, self.keys.back),
            side: axis(self.keys.right, self.keys.left),
            lift: axis(self.keys.up, self.keys.down),
            fast: self.keys.fast,
        };
        self.camera.travel(&self.world, motion, self.walking, dt);
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) {
        match code {
            KeyCode::KeyW | KeyCode::ArrowUp => self.keys.forward = pressed,
            KeyCode::KeyS | KeyCode::ArrowDown => self.keys.back = pressed,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.keys.left = pressed,
            KeyCode::KeyD | KeyCode::ArrowRight => self.keys.right = pressed,
            KeyCode::KeyE => self.keys.up = pressed,
            KeyCode::KeyQ => self.keys.down = pressed,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.keys.fast = pressed,
            _ if !pressed => {}
            // Holding T or G keeps changing the hour.
            KeyCode::KeyT => self.preset.settings.sun.hour = (self.preset.settings.sun.hour + 0.25).min(*HOURS.end()),
            KeyCode::KeyG => self.preset.settings.sun.hour = (self.preset.settings.sun.hour - 0.25).max(*HOURS.start()),
            // A held key must not flip a switch back and forth.
            _ if repeat => {}
            KeyCode::Escape => event_loop.exit(),
            KeyCode::KeyF => self.walking = !self.walking,
            KeyCode::Space => self.paused = !self.paused,
            KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 => {
                let index = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6]
                    .iter()
                    .position(|&k| k == code)
                    .unwrap_or(0);
                self.camera = self.views[index];
                // Viewpoints above the ground are flying viewpoints.
                self.walking = self.camera.height_above(&self.world) < 3.0;
            }
            _ => {}
        }
    }

    /// False while the window is minimized or hidden: there is nothing to draw into.
    fn drawable(&self) -> bool {
        !self.occluded && self.graphics.as_ref().is_some_and(|g| g.size.0 > 0 && g.size.1 > 0)
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |last| (now - last).as_secs_f32()).min(0.1);
        self.last = Some(now);
        self.step(dt);
        if !self.drawable() {
            return;
        }
        let environment = Environment::at(&self.world, &self.camera, self.time, dt);
        let Some(graphics) = self.graphics.as_mut() else { return };
        if let Acquire::Frame(frame) = graphics.rhi.acquire_frame(&mut graphics.surface) {
            graphics.renderer.render(graphics.rhi.frame_view(&frame), graphics.size, &self.camera, &environment, &self.preset.settings);
            graphics.rhi.present(frame);
            self.frames += 1;
        }
        self.title_timer += dt;
        if self.title_timer >= 0.5 {
            let stats = graphics.renderer.last_frame_stats();
            let hour = self.preset.settings.sun.hour;
            let minutes = (hour.fract() * 60.0) as u32;
            graphics.window.set_title(&format!(
                "Forest Valley   {:02}:{minutes:02}   {:.0} fps   {:.1} M triangles   {}",
                hour as u32,
                self.frames as f32 / self.title_timer,
                stats.triangles as f32 / 1.0e6,
                if self.walking { "walking (F: fly)" } else { "flying (F: walk)" },
            ));
            self.frames = 0;
            self.title_timer = 0.0;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title("Forest Valley");
        let attributes = match self.window_size {
            Some((w, h)) => attributes.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            None => attributes.with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0)),
        };
        let created = event_loop.create_window(attributes).map_err(|e| e.to_string()).and_then(|window| {
            let window = Arc::new(window);
            let size = window.inner_size();
            let size = (size.width, size.height);
            let (rhi, surface) = Wgpu::for_window(window.clone(), size, self.vsync, WgpuOptions { allow_software_fallback: false, ..Default::default() })?;
            let renderer = Renderer::new(rhi.clone(), rhi.surface_format(&surface), &self.world, self.preset.quality);
            Ok(Graphics { window, rhi, surface, renderer, size })
        });
        match created {
            Ok(graphics) => {
                graphics.window.request_redraw();
                self.graphics = Some(graphics);
            }
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if !self.graphics.as_ref().is_some_and(|g| g.window.id() == window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.size = (size.width, size.height);
                    if size.width > 0 && size.height > 0 {
                        graphics.rhi.resize_surface(&mut graphics.surface, size.width, size.height);
                        graphics.window.request_redraw();
                    }
                }
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                // Time must not jump when the window comes back.
                self.last = None;
                if let Some(graphics) = self.graphics.as_ref().filter(|_| !occluded) {
                    graphics.window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                self.keys = Keys::default();
                self.looking = false;
                self.cursor = None;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(event_loop, code, event.state == ElementState::Pressed, event.repeat);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left | MouseButton::Right, .. } => {
                self.looking = state == ElementState::Pressed;
                self.cursor = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.looking {
                    if let Some((x, y)) = self.cursor {
                        let (dx, dy) = ((position.x - x) as f32, (position.y - y) as f32);
                        self.camera.turn(-dx * 0.0035, -dy * 0.0035);
                    }
                    self.cursor = Some((position.x, position.y));
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        // A minimized or hidden window waits for its next event and uses no CPU.
        if self.drawable() {
            if let Some(graphics) = self.graphics.as_ref() {
                graphics.window.request_redraw();
            }
        }
    }
}

fn run() -> Result<(), String> {
    let Some(options) = parse()? else {
        println!("{HELP}");
        return Ok(());
    };
    let world = World::generate(options.preset.seed);
    let views = viewpoints(&world);
    let camera = views[options.view];
    if let Some(path) = &options.capture {
        return capture(&world, &camera, &options, path);
    }
    let mut app = App {
        walking: camera.height_above(&world) < 3.0,
        world,
        views,
        camera,
        preset: options.preset,
        time: options.time,
        paused: false,
        keys: Keys::default(),
        looking: false,
        cursor: None,
        last: None,
        title_timer: 0.0,
        frames: 0,
        vsync: options.vsync,
        window_size: options.size,
        occluded: false,
        graphics: None,
        error: None,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("forest_valley: {error}");
            ExitCode::FAILURE
        }
    }
}
