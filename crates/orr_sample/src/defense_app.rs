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
pub const HELP:&str="Stormglass Foundry\ncoop_defense [--headless --ticks N [--demo] [--capture NEW.png]]\nSpace summon, click your unit then empty tile to move or identical role/tier to merge, U upgrade, I invest, R restart. AI partner is local, not online. Headless capture uses a software GPU.";
#[derive(Debug, Default)]
pub struct Options {
    pub headless: bool,
    pub ticks: u32,
    pub demo: bool,
    pub capture: Option<PathBuf>,
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
fn session() -> DefenseSession {
    let config = game::DefenseConfig::default();
    Session::new(
        config,
        SessionConfig::new(1, PlayerSlot(0), u64::from(config.seed), game::TICK_RATE),
        LocalInputSource,
    )
}
pub fn run(options: Options) -> Result<(), String> {
    if options.headless {
        return headless(options);
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        session: session(),
        ui: DefenseUi::new(),
        gfx: None,
        last: Instant::now(),
        accumulator: 0.0,
        commands: VecDeque::new(),
        error: None,
        focused: true,
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
}
impl App {
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
        let (mut output, command) = self.ui.show(input, self.session.predicted_frame());
        gfx.state
            .handle_platform_output(&gfx.window, std::mem::take(&mut output.platform_output));
        if let Some(command) = command.filter(|_| self.focused) {
            if self.commands.len() < 32 {
                self.commands.push_back(command);
            }
        }
        if let Err(e) = gfx.pending.push(output) {
            self.fail(event_loop, e);
            return;
        }
        self.accumulator += if self.ui.started && self.focused {
            delta
        } else {
            0.0
        };
        while self.accumulator >= 1.0 / 60.0 {
            let command = self.commands.pop_front().unwrap_or_default();
            self.session.advance(command, vec![]);
            self.accumulator -= 1.0 / 60.0;
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
                self.focused = focused;
                self.accumulator = 0.0;
                self.last = Instant::now();
                if !focused {
                    self.commands.clear();
                }
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
    let mut session = session();
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
    fn bounded_cli() {
        assert!(Options::parse(["--headless", "--ticks", "36001"].map(String::from)).is_err());
        assert!(Options::parse(["--capture", "x.png"].map(String::from)).is_err());
        assert!(Options::parse(["--headless", "--headless"].map(String::from)).is_err());
        assert!(
            Options::parse(["--headless", "--ticks", "300", "--demo"].map(String::from)).is_ok()
        );
    }
}
