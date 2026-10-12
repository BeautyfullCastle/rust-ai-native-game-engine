//! Explicit optional Collect pickup VFX preview. The default Arena sample is unchanged.
use orr_bridge::{
    Bridge, BridgeConfig, InProc, PlayConfig, PlayHost, PlaySession, PlayerSlot, Snapshot,
};
use orr_fp::{FPVec2, FP};
use orr_games::collect_dodge_game::{
    CollectActor, CollectDodgeV1, CollectInput, CollectLevel, CollectRun, COLLECTIBLE,
    EVENT_COLLECTED, PLAYER,
};
use orr_render::orr_rhi::{Acquire, Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{
    imported_scene::ParticleBatch, particle_presets, particle_renderer::ParticleRenderer, Camera3D,
    ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Material, PointLightSettings,
    RenderList3D, Renderer3D,
};
use orr_view::{
    fp_to_f32,
    particles::{ParticleDraw, ParticlePool},
    Vec3,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

pub const HELP: &str = "particle_demo --particles off|dust|sparks [--headless] [--showcase]\n--showcase: genuine pickup, spread/fade, fresh-session cycling every two seconds.\nWASD/arrows: move and collect gold spheres. R: new session. Escape: exit.\nDust previews a preset on pickup, not a landing event. --headless is CPU-only.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Dust,
    Sparks,
}
pub struct Options {
    pub mode: Mode,
    pub headless: bool,
    pub showcase: bool,
}
impl Options {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let mut mode = None;
        let mut headless = false;
        let mut showcase = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--particles" if mode.is_none() => {
                    mode = Some(match args.next().as_deref() {
                        Some("off") => Mode::Off,
                        Some("dust") => Mode::Dust,
                        Some("sparks") => Mode::Sparks,
                        _ => return Err(HELP.into()),
                    })
                }
                "--headless" if !headless => headless = true,
                "--showcase" if !showcase => showcase = true,
                _ => return Err(HELP.into()),
            }
        }
        Ok(Self {
            mode: mode.ok_or_else(|| HELP.to_string())?,
            headless,
            showcase,
        })
    }
}
const PICKUPS: [[i64; 2]; 2] = [[8, 0], [16, 6]];
fn point(x: i64, y: i64) -> FPVec2 {
    FPVec2::new(FP(x << 16), FP(y << 16))
}
fn origin(ordinal: u32) -> Option<Vec3> {
    PICKUPS
        .get(ordinal as usize)
        .map(|p| Vec3::new(p[0] as f32 * 0.1, 0.4, p[1] as f32 * 0.1))
}
struct Demo {
    mode: Mode,
    bridge: InProc<CollectDodgeV1, PlayHost<CollectDodgeV1>>,
    snapshot: Option<Snapshot>,
    particles: ParticlePool,
    draws: Vec<ParticleDraw>,
    list: RenderList3D,
}
impl Demo {
    fn new(mode: Mode) -> Self {
        let level = CollectLevel::new(
            point(0, 0),
            PICKUPS.into_iter().map(|p| point(p[0], p[1])).collect(),
            vec![],
            36000,
        )
        .expect("small original level");
        let session = PlaySession::new(PlayConfig::new(1, 42, 60), level);
        let bridge = InProc::new(
            PlayHost::new(session, PlayerSlot(0)),
            BridgeConfig::default(),
        );
        let mut result = Self {
            mode,
            bridge,
            snapshot: None,
            particles: ParticlePool::new(),
            draws: Vec::with_capacity(1024),
            list: RenderList3D::new(),
        };
        result
            .update(Duration::ZERO, CollectInput::default())
            .expect("initial bridge state");
        result
    }
    fn update(&mut self, dt: Duration, input: CollectInput) -> Result<(), String> {
        let dt = dt.min(Duration::from_millis(250));
        self.bridge
            .set_input(PlayerSlot(0), input)
            .map_err(|e| e.to_string())?;
        self.bridge.update(dt);
        // The exact same finite poll drives event reconciliation and scene extraction.
        let update = self.bridge.poll_view();
        let mode = self.mode;
        self.particles
            .reconcile(&update, |event| {
                if event.kind != EVENT_COLLECTED {
                    return None;
                }
                let position = origin(event.ordinal)?;
                let seed = u64::from(event.ordinal) + 42;
                match mode {
                    Mode::Off => None,
                    Mode::Dust => Some(particle_presets::landing_dust(position, seed)),
                    Mode::Sparks => Some(particle_presets::pickup_sparks(position, seed)),
                }
            })
            .map_err(|e| e.to_string())?;
        self.particles
            .advance(dt.as_secs_f32())
            .map_err(|e| e.to_string())?;
        self.draws.clear();
        self.draws.extend(self.particles.draws());
        self.snapshot = update.snapshot;
        self.list.clear();
        self.list.lighting.shadows = false;
        self.list
            .plane([0.8, 0.0, 0.3], 3.0, 2.0, &Material::new([0.09, 0.13, 0.2]));
        if let Some(snapshot) = &self.snapshot {
            for (_, actor) in snapshot.predicted().iter::<CollectActor>() {
                if actor.active == 0 {
                    continue;
                }
                let color = if actor.kind == PLAYER {
                    [0.1, 0.42, 0.95]
                } else if actor.kind == COLLECTIBLE {
                    [1.0, 0.76, 0.08]
                } else {
                    [1.0, 0.1, 0.1]
                };
                self.list.sphere(
                    [
                        fp_to_f32(actor.position.x) * 0.1,
                        0.2,
                        fp_to_f32(actor.position.y) * 0.1,
                    ],
                    [0.0, 0.0, 0.0, 1.0],
                    0.2,
                    &Material::new(color),
                );
            }
        }
        Ok(())
    }
}
/// A real-input preview: collect once, release movement, then start a fresh session.
/// A frame uses one Demo update/poll. Excess cycle-boundary time is not replayed.
#[derive(Default)]
struct Showcase {
    elapsed: Duration,
    pickup_observed_at: Option<Duration>,
    completed_cycles: u64,
}
impl Showcase {
    const CYCLE: Duration = Duration::from_secs(2);
    fn score(demo: &Demo) -> u32 {
        demo.snapshot
            .as_ref()
            .map_or(0, |s| s.predicted().singleton::<CollectRun>().score)
    }
    fn restart(&mut self, demo: &mut Demo) {
        *demo = Demo::new(demo.mode);
        *self = Self::default();
    }
    fn update(&mut self, demo: &mut Demo, dt: Duration, focused: bool) -> Result<(), String> {
        if !focused {
            return Ok(());
        }
        let dt = dt.min(Duration::from_millis(250));
        if !dt.is_zero() && self.elapsed >= Self::CYCLE {
            let cycles = self.completed_cycles.saturating_add(1);
            self.restart(demo);
            self.completed_cycles = cycles;
        }
        let input = CollectInput {
            x: if Self::score(demo) == 0 {
                FP::ONE
            } else {
                FP::ZERO
            },
            ..CollectInput::default()
        };
        demo.update(dt, input)?;
        self.elapsed += dt;
        if Self::score(demo) > 0 && self.pickup_observed_at.is_none() {
            self.pickup_observed_at = Some(self.elapsed);
        }
        Ok(())
    }
}
fn geometry_settings() -> orr_render::Settings3D {
    // The composed scene owns a single-sample color/depth pair.
    orr_render::Settings3D {
        msaa: 1,
        ..Default::default()
    }
}
struct DemoRenderer<B: Rhi> {
    scene: ImportedSceneRenderer<B>,
    geometry: Renderer3D<B>,
    particles: Option<ParticleRenderer<B>>,
}
impl<B: Rhi> DemoRenderer<B> {
    fn new(gpu: &B, format: TextureFormat, mode: Mode) -> Result<Self, String> {
        let scene = ImportedSceneRenderer::new(gpu.clone(), format).map_err(|e| e.to_string())?;
        let geometry =
            Renderer3D::with_settings(gpu.clone(), scene.scene_format(), geometry_settings());
        let particles = if mode == Mode::Off {
            None
        } else {
            Some(
                ParticleRenderer::new(gpu.clone(), scene.scene_format())
                    .map_err(|e| e.to_string())?,
            )
        };
        Ok(Self {
            scene,
            geometry,
            particles,
        })
    }
    fn draw(&mut self, view: &B::TextureView, size: (u32, u32), demo: &Demo) -> Result<(), String> {
        let camera = Camera3D::perspective([2.8, 3.0, 4.0], [0.8, 0.2, 0.3], 48.0);
        let target = ImportedSceneTarget {
            view,
            size,
            format: self.scene.format(),
            sample_count: 1,
        };
        let mut batches = [ImportedBatch::Procedural {
            renderer: &mut self.geometry,
            list: &demo.list,
        }];
        let particles = self.particles.as_mut().map(|renderer| ParticleBatch {
            renderer,
            items: &demo.draws,
        });
        self.scene
            .draw_with_particles(
                target,
                &camera,
                &demo.list.lighting,
                &PointLightSettings::default(),
                &mut batches,
                particles,
            )
            .map_err(|e| e.to_string())
    }
}
struct Graphics {
    window: Arc<Window>,
    gpu: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    renderer: DemoRenderer<Wgpu>,
}
struct App {
    demo: Demo,
    graphics: Option<Graphics>,
    keys: [bool; 4],
    showcase: Option<Showcase>,
    focused: bool,
    last: Instant,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last;
        self.last = now;
        let axis = |positive: bool, negative: bool| {
            if positive == negative {
                FP::ZERO
            } else if positive {
                FP::ONE
            } else {
                -FP::ONE
            }
        };
        let update = if let Some(showcase) = &mut self.showcase {
            showcase.update(&mut self.demo, dt, self.focused)
        } else {
            self.demo.update(
                dt,
                CollectInput {
                    x: axis(self.keys[1], self.keys[0]),
                    y: axis(self.keys[3], self.keys[2]),
                    ..CollectInput::default()
                },
            )
        };
        if let Err(e) = update {
            self.fail(event_loop, e);
            return;
        }
        let Some(g) = &mut self.graphics else {
            return;
        };
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if let Acquire::Frame(frame) = g.gpu.acquire_frame(&mut g.surface) {
            if let Err(e) = g.renderer.draw(
                g.gpu.frame_view(&frame),
                (size.width, size.height),
                &self.demo,
            ) {
                self.fail(event_loop, e);
                return;
            }
            g.gpu.present(frame);
        }
        let score = self
            .demo
            .snapshot
            .as_ref()
            .map_or(0, |s| s.predicted().singleton::<CollectRun>().score);
        let controls = if self.showcase.is_some() {
            "fresh-session showcase | focus loss pauses | R restart"
        } else {
            "WASD/arrows move | R restart"
        };
        g.window.set_title(&format!(
            "Particle preset preview | {controls} | {score}/2 pickups | {:?} | {} particles",
            self.demo.mode,
            self.demo.draws.len()
        ));
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let result = (|| -> Result<Graphics, String> {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Particle preset preview")
                            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 640.0)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let size = window.inner_size();
            let (gpu, surface) = Wgpu::for_window(
                window.clone(),
                (size.width, size.height),
                true,
                WgpuOptions::default(),
            )?;
            let renderer = DemoRenderer::new(&gpu, gpu.surface_format(&surface), self.demo.mode)?;
            Ok(Graphics {
                window,
                gpu,
                surface,
                renderer,
            })
        })();
        match result {
            Ok(g) => {
                self.graphics = Some(g);
                self.last = Instant::now();
            }
            Err(e) => self.fail(event_loop, e),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.keys = [false; 4];
                self.last = Instant::now();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(g) = &mut self.graphics {
                        g.gpu
                            .resize_surface(&mut g.surface, size.width, size.height);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    match key {
                        KeyCode::KeyA | KeyCode::ArrowLeft => self.keys[0] = down,
                        KeyCode::KeyD | KeyCode::ArrowRight => self.keys[1] = down,
                        KeyCode::KeyW | KeyCode::ArrowUp => self.keys[2] = down,
                        KeyCode::KeyS | KeyCode::ArrowDown => self.keys[3] = down,
                        KeyCode::Escape if down => event_loop.exit(),
                        KeyCode::KeyR if down && !event.repeat => {
                            if let Some(showcase) = &mut self.showcase {
                                showcase.restart(&mut self.demo);
                            } else {
                                self.demo = Demo::new(self.demo.mode);
                            }
                            self.last = Instant::now();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(g) = &self.graphics {
            g.window.request_redraw();
        }
    }
}
pub fn run(options: Options) -> Result<(), String> {
    let mut demo = Demo::new(options.mode);
    if options.headless {
        let mut showcase = Showcase::default();
        for _ in 0..if options.showcase { 240 } else { 20 } {
            if options.showcase {
                showcase.update(&mut demo, Duration::from_millis(17), true)?;
                continue;
            }
            demo.update(
                Duration::from_millis(17),
                CollectInput {
                    x: FP::ONE,
                    ..CollectInput::default()
                },
            )?;
        }
        println!(
            "CPU-only pickup preview: score={} particles={}",
            demo.snapshot
                .as_ref()
                .unwrap()
                .predicted()
                .singleton::<CollectRun>()
                .score,
            demo.draws.len()
        );
        return Ok(());
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = App {
        demo,
        graphics: None,
        keys: [false; 4],
        showcase: options.showcase.then(Showcase::default),
        focused: true,
        last: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}
#[cfg(test)]
#[path = "particle_demo_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "particle_showcase_tests.rs"]
mod showcase_tests;
