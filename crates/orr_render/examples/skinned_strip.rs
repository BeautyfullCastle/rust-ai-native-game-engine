//! Package-backed skeletal animation viewer. Run with --help for installation.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
use orr_model::{
    animation::{AnimatedModel, AnimationPlayer, PlaybackMode},
    IDENTITY,
};
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{
    Acquire, Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions,
};
use orr_render::{Camera3D, Lighting, SkinnedInstance, SkinnedModelRenderer};
use std::{path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

fn load(project: &std::path::Path) -> Result<AnimatedModel, String> {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("animation".into());
    let project = Project::open(project, runtime).map_err(|e| e.to_string())?;
    let bytes = project
        .read_asset("sample-animation", "animated.glb")
        .map_err(|e| format!("{e}; install assets/animation_demo first"))?;
    let imported =
        orr_model::animation_import::import_with_resolver("animated.glb", &bytes, |uri| {
            project
                .read_asset("sample-animation", uri)
                .map_err(|e| orr_model::Error::message(e.to_string()))
        })
        .map_err(|e| e.to_string())?;
    AnimatedModel::from_bytes(&imported.to_bytes().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 1.5, 7.0], [0.0, 1.5, 0.0], 4.0)
}
fn lighting() -> Lighting {
    Lighting {
        shadows: false,
        tonemap: false,
        ambient: 1.0,
        intensity: 0.0,
        sky: [1.0; 3],
        ground: [1.0; 3],
        ..Default::default()
    }
}
fn draw(
    renderer: &mut SkinnedModelRenderer<Wgpu>,
    view: &<Wgpu as Rhi>::TextureView,
    size: (u32, u32),
    players: &[AnimationPlayer; 2],
) -> Result<(), String> {
    let left = players[0]
        .pose(renderer.model())
        .map_err(|e| e.to_string())?;
    let right = players[1]
        .pose(renderer.model())
        .map_err(|e| e.to_string())?;
    let mut a = IDENTITY;
    a[3][0] = -1.2;
    let mut b = IDENTITY;
    b[3][0] = 1.2;
    renderer
        .draw(
            view,
            size,
            &camera(),
            &lighting(),
            &[
                SkinnedInstance {
                    pose: &left,
                    transform: a,
                },
                SkinnedInstance {
                    pose: &right,
                    transform: b,
                },
            ],
        )
        .map_err(|e| e.to_string())
}
// Stop clears clip selection. Arrow keys remain safe until R or 1/2 selects one.
fn seek_selected(
    player: &mut AnimationPlayer,
    model: &AnimatedModel,
    delta: f32,
) -> Result<(), orr_model::Error> {
    if player.clip().is_none() {
        return Ok(());
    }
    player.seek(model, (player.time() + delta).max(0.0))
}

fn toggle_pause(player: &mut AnimationPlayer) {
    if player.state() == orr_model::animation::PlaybackState::Playing {
        player.pause();
    } else {
        player.resume();
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: <Wgpu as Rhi>::Surface,
    renderer: SkinnedModelRenderer<Wgpu>,
}
struct App {
    model: AnimatedModel,
    players: [AnimationPlayer; 2],
    gfx: Option<Gfx>,
    last: Instant,
    looping: bool,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let result = (|| -> Result<Gfx, String> {
            let window=Arc::new(event_loop.create_window(Window::default_attributes().with_title("Orrery skinning | Space pause | Left/Right seek | 1/2 clip | S stop | R play | L loop").with_inner_size(winit::dpi::LogicalSize::new(960,640))).map_err(|e|e.to_string())?);
            let size = window.inner_size();
            let (rhi, surface) = Wgpu::for_window(
                window.clone(),
                (size.width, size.height),
                true,
                WgpuOptions::default(),
            )?;
            let renderer = SkinnedModelRenderer::new(
                rhi.clone(),
                rhi.surface_format(&surface),
                self.model.clone(),
            )
            .map_err(|e| e.to_string())?;
            Ok(Gfx {
                window,
                surface,
                renderer,
            })
        })();
        match result {
            Ok(gfx) => {
                self.gfx = Some(gfx);
                self.last = Instant::now();
            }
            Err(e) => self.fail(event_loop, e),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    if size.width > 0 && size.height > 0 {
                        g.renderer
                            .rhi()
                            .resize_surface(&mut g.surface, size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let PhysicalKey::Code(key) = event.physical_key else {
                    return;
                };
                let mode = if self.looping {
                    PlaybackMode::Loop
                } else {
                    PlaybackMode::Once
                };
                let result: Result<(), orr_model::Error> = match key {
                    KeyCode::Escape => {
                        event_loop.exit();
                        Ok(())
                    }
                    KeyCode::Space => {
                        toggle_pause(&mut self.players[0]);
                        Ok(())
                    }
                    KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                        let delta = if key == KeyCode::ArrowLeft { -0.1 } else { 0.1 };
                        seek_selected(&mut self.players[0], &self.model, delta)
                    }
                    KeyCode::Digit1 | KeyCode::Digit2 => {
                        let clip = if key == KeyCode::Digit1 { 0 } else { 1 };
                        self.players[0].play(&self.model, clip, mode)
                    }
                    KeyCode::KeyS => {
                        self.players[0].stop();
                        Ok(())
                    }
                    KeyCode::KeyR => self.players[0].play(&self.model, 0, mode),
                    KeyCode::KeyL => {
                        self.looping = !self.looping;
                        let clip = self.players[0].clip().unwrap_or(0);
                        self.players[0].play(
                            &self.model,
                            clip,
                            if self.looping {
                                PlaybackMode::Loop
                            } else {
                                PlaybackMode::Once
                            },
                        )
                    }
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    self.fail(event_loop, e);
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                for p in &mut self.players {
                    if let Err(e) = p.advance(&self.model, dt) {
                        self.fail(event_loop, e);
                        return;
                    }
                }
                if let Some(g) = &mut self.gfx {
                    let size = g.window.inner_size();
                    if size.width == 0 || size.height == 0 {
                        return;
                    }
                    if let Acquire::Frame(frame) = g.renderer.rhi().acquire_frame(&mut g.surface) {
                        let view = g.renderer.rhi().frame_view(&frame).clone();
                        if let Err(e) = draw(
                            &mut g.renderer,
                            &view,
                            (size.width, size.height),
                            &self.players,
                        ) {
                            self.fail(event_loop, e);
                            return;
                        }
                        g.renderer.rhi().present(frame);
                    }
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut project = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("Install: mkdir -p /tmp/animation-project; cargo run -p orr_package --bin orr_pkg -- install /tmp/animation-project --path assets/animation_demo\nRun: cargo run -p orr_render --features animation --example skinned_strip -- --project /tmp/animation-project [--offscreen /tmp/skinned.ppm]\nThe left instance is controllable; the right keeps playing independently. Space pause/resume; arrows seek; 1/2 switch clips; S stop/rest; R restart; L loop/once. Offscreen imports the installed asset, cooks/reloads, draws two independent poses and saves an actual GPU readback.");
                return Ok(());
            }
            "--project" => {
                project = Some(PathBuf::from(args.next().ok_or("missing project path")?))
            }
            "--offscreen" => output = Some(PathBuf::from(args.next().ok_or("missing PPM path")?)),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    let model = load(&project.ok_or("--project path required; see --help")?)?;
    let mut players = [AnimationPlayer::default(), AnimationPlayer::default()];
    for (i, p) in players.iter_mut().enumerate() {
        p.play(&model, 0, PlaybackMode::Loop)
            .map_err(|e| e.to_string())?;
        p.seek(&model, if i == 0 { 0.5 } else { 1.0 })
            .map_err(|e| e.to_string())?;
    }
    if let Some(output) = output {
        let rhi = Wgpu::headless(WgpuOptions::default())?;
        eprintln!(
            "adapter: {} (software: {})",
            rhi.adapter_name(),
            rhi.is_software()
        );
        let target = rhi.create_texture(&TextureDesc {
            label: "skinned package demo",
            width: 960,
            height: 640,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&target, None);
        let mut renderer =
            SkinnedModelRenderer::new(rhi.clone(), TextureFormat::Rgba8UnormSrgb, model)
                .map_err(|e| e.to_string())?;
        draw(&mut renderer, &view, (960, 640), &players)?;
        let pixels = rhi.read_texture(&target);
        let mut ppm = b"P6\n960 640\n255\n".to_vec();
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&p[..3]);
        }
        std::fs::write(output, ppm).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut app = App {
        model,
        players,
        gfx: None,
        last: Instant::now(),
        looping: true,
        error: None,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(e) = app.error {
        return Err(e);
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("skinned_strip: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrows_after_stop_preserve_rest_and_allow_restart() {
        let model = orr_model::animation_import::import_with_resolver(
            "animated.glb",
            include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
            |_| panic!("embedded fixture"),
        )
        .unwrap();
        let mut player = AnimationPlayer::default();
        player.play(&model, 0, PlaybackMode::Loop).unwrap();
        player.advance(&model, 0.5).unwrap();
        player.stop();
        seek_selected(&mut player, &model, -0.1).unwrap();
        seek_selected(&mut player, &model, 0.1).unwrap();
        assert_eq!(player.clip(), None);
        assert_eq!(player.time(), 0.0);
        assert_eq!(
            player.pose(&model).unwrap().global(),
            model.rest_pose().unwrap().global()
        );
        player.play(&model, 1, PlaybackMode::Loop).unwrap();
        player.pause();
        seek_selected(&mut player, &model, 0.1).unwrap();
        assert_eq!(player.time(), 0.1);
        assert_eq!(player.state(), orr_model::animation::PlaybackState::Paused);
        toggle_pause(&mut player);
        assert_eq!(player.state(), orr_model::animation::PlaybackState::Playing);
        player.play(&model, 0, PlaybackMode::Once).unwrap();
        player.advance(&model, 2.0).unwrap();
        seek_selected(&mut player, &model, -0.1).unwrap();
        toggle_pause(&mut player);
        assert_eq!(player.state(), orr_model::animation::PlaybackState::Playing);
    }
}
