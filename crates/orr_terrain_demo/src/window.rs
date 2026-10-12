//! Optional native presentation. Wall time, floats and winit stay outside the
//! fixed-point controller. Offscreen tests do not establish native interaction.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]

use crate::{render, status_title, ControlKey, Controls, Course, FixedClock, Game};
use orr_render::{
    orr_rhi::{Acquire, Rhi, Wgpu, WgpuOptions},
    OrbitCamera,
};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    None,
    Restart,
    Exit,
}

/// The actual event-state reducer is independent of a window/device, so all
/// lifecycle tests exercise the same state changes as ApplicationHandler.
#[derive(Default)]
struct Interaction {
    controls: Controls,
    clock: FixedClock,
    live: bool,
    focused: bool,
    occluded: bool,
    size: (u32, u32),
    dragging: bool,
    cursor: Option<[f64; 2]>,
    last: Option<Instant>,
    restart_held: bool,
}

impl Interaction {
    fn drawable(&self) -> bool {
        self.live
            && !self.occluded
            && self.size.0 > 0
            && self.size.1 > 0
            && self.size.0 <= 8192
            && self.size.1 <= 8192
    }

    fn running(&self) -> bool {
        self.drawable() && self.focused
    }

    fn clear(&mut self) {
        self.controls.clear();
        self.clock.reset();
        self.dragging = false;
        self.cursor = None;
        self.last = None;
    }

    fn resume(&mut self, size: (u32, u32), focused: bool) -> bool {
        if self.live {
            return false;
        }
        self.clear();
        self.live = true;
        self.focused = focused;
        self.occluded = false;
        self.size = size;
        self.restart_held = false;
        true
    }

    fn suspend(&mut self) {
        self.clear();
        self.live = false;
        self.restart_held = false;
    }

    fn focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.clear();
            self.restart_held = false;
            self.focused = focused;
        }
    }

    fn resize(&mut self, size: (u32, u32)) -> bool {
        if size == self.size {
            return false;
        }
        let was_drawable = self.drawable();
        self.size = size;
        if !self.drawable() || !was_drawable {
            self.clear();
            self.restart_held = false;
        }
        true
    }

    fn occlusion(&mut self, occluded: bool) {
        if self.occluded != occluded {
            self.clear();
            self.restart_held = false;
            self.occluded = occluded;
        }
    }

    fn key(&mut self, key: KeyCode, pressed: bool, repeat: bool, synthetic: bool) -> Action {
        if key == KeyCode::KeyR && !pressed {
            self.restart_held = false;
        }
        if let Some(control) = control_key(key) {
            // Synthetic releases still clear keys; synthetic presses after
            // focus restoration must not restore stale physical-key state.
            if !pressed || (self.running() && !synthetic && !repeat) {
                self.controls.set(control, pressed);
            }
            return Action::None;
        }
        if !pressed || repeat || synthetic || !self.running() {
            return Action::None;
        }
        match key {
            KeyCode::Escape => Action::Exit,
            KeyCode::KeyR if !self.restart_held => {
                self.restart_held = true;
                self.clear();
                Action::Restart
            }
            _ => Action::None,
        }
    }

    fn mouse(&mut self, pressed: bool) {
        self.dragging = pressed && self.running();
        if !self.dragging {
            self.cursor = None;
        }
    }

    fn pointer(&mut self, position: [f64; 2]) -> Option<[f32; 2]> {
        if !self.running() || !position.into_iter().all(f64::is_finite) {
            self.cursor = None;
            return None;
        }
        let previous = self.cursor.replace(position)?;
        if !self.dragging {
            return None;
        }
        // Bound in f64 before narrowing. Even finite f64 subtraction can
        // overflow; clamping maps those spikes to a bounded view-only delta.
        Some(std::array::from_fn(|i| {
            (position[i] - previous[i]).clamp(-2048.0, 2048.0) as f32
        }))
    }

    fn ticks(&mut self, now: Instant) -> u32 {
        if !self.running() {
            self.last = None;
            self.clock.reset();
            return 0;
        }
        let Some(last) = self.last.replace(now) else {
            return 0;
        };
        self.clock.advance(now.saturating_duration_since(last))
    }
}

fn control_key(key: KeyCode) -> Option<ControlKey> {
    Some(match key {
        KeyCode::KeyW => ControlKey::W,
        KeyCode::KeyA => ControlKey::A,
        KeyCode::KeyS => ControlKey::S,
        KeyCode::KeyD => ControlKey::D,
        KeyCode::ArrowUp => ControlKey::Up,
        KeyCode::ArrowLeft => ControlKey::Left,
        KeyCode::ArrowDown => ControlKey::Down,
        KeyCode::ArrowRight => ControlKey::Right,
        _ => return None,
    })
}

struct Graphics {
    // Drop the surface before its window when suspending (notably on Android).
    surface: <Wgpu as Rhi>::Surface,
    window: Arc<Window>,
    rhi: Wgpu,
    scene: render::Scene<Wgpu>,
    configured_size: (u32, u32),
    title: String,
}

struct App {
    game: Game,
    camera: OrbitCamera,
    interaction: Interaction,
    graphics: Option<Graphics>,
    error: Option<String>,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        for _ in 0..self.interaction.ticks(Instant::now()) {
            self.game.tick(self.interaction.controls.input());
        }
        render::follow_target(&mut self.camera, &self.game.state());
        if !self.interaction.drawable() {
            return;
        }
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };
        // A resize may have arrived while occluded, when configuration was
        // intentionally deferred. Restore the real extent before any draw.
        if graphics.configured_size != self.interaction.size {
            graphics.rhi.resize_surface(
                &mut graphics.surface,
                self.interaction.size.0,
                self.interaction.size.1,
            );
            graphics.configured_size = self.interaction.size;
        }
        let title = status_title(&self.game);
        if title != graphics.title {
            graphics.window.set_title(&title);
            graphics.title = title;
        }
        if let Acquire::Frame(frame) = graphics.rhi.acquire_frame(&mut graphics.surface) {
            let view = graphics.rhi.frame_view(&frame).clone();
            if let Err(error) = graphics.scene.draw(
                &view,
                self.interaction.size,
                &self.game,
                &self.camera.camera(),
            ) {
                self.fail(event_loop, error);
                return;
            }
            graphics.rhi.present(frame);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Redundant Resumed must not allocate another device or terrain upload.
        if self.graphics.is_some() {
            return;
        }
        let result = (|| -> Result<Graphics, String> {
            let title = status_title(&self.game);
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title(&title)
                            .with_inner_size(winit::dpi::PhysicalSize::new(960, 640)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let size = window.inner_size();
            let configured_size = (size.width.clamp(1, 8192), size.height.clamp(1, 8192));
            let (rhi, surface) = Wgpu::for_window(
                window.clone(),
                configured_size,
                true,
                WgpuOptions::default(),
            )?;
            let scene = render::Scene::new(
                rhi.clone(),
                rhi.surface_format(&surface),
                self.game.course(),
            )?;
            Ok(Graphics {
                surface,
                window,
                rhi,
                scene,
                configured_size,
                title,
            })
        })();
        match result {
            Ok(graphics) => {
                let size = graphics.window.inner_size();
                self.interaction
                    .resume((size.width, size.height), graphics.window.has_focus());
                graphics.window.request_redraw();
                self.graphics = Some(graphics);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.interaction.suspend();
        self.graphics = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self
            .graphics
            .as_ref()
            .is_some_and(|g| g.window.id() == window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => self.interaction.focus(focused),
            WindowEvent::Occluded(occluded) => self.interaction.occlusion(occluded),
            WindowEvent::Resized(size) => {
                self.interaction.resize((size.width, size.height));
                if self.interaction.drawable() {
                    if let Some(graphics) = self.graphics.as_mut() {
                        if graphics.configured_size != self.interaction.size {
                            graphics.rhi.resize_surface(
                                &mut graphics.surface,
                                size.width,
                                size.height,
                            );
                            graphics.configured_size = self.interaction.size;
                        }
                        graphics.window.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    match self.interaction.key(
                        key,
                        event.state == ElementState::Pressed,
                        event.repeat,
                        is_synthetic,
                    ) {
                        Action::None => {}
                        Action::Exit => event_loop.exit(),
                        Action::Restart => {
                            self.game.restart();
                            self.camera = render::follow_camera(&self.game.state());
                        }
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.interaction.mouse(state == ElementState::Pressed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(delta) = self.interaction.pointer([position.x, position.y]) {
                    render::orbit(&mut self.camera, delta);
                }
            }
            WindowEvent::CursorLeft { .. } => self.interaction.mouse(false),
            WindowEvent::MouseWheel { delta, .. } if self.interaction.running() => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) if position.y.is_finite() => {
                        (position.y / 40.0).clamp(-12.0, 12.0) as f32
                    }
                    MouseScrollDelta::PixelDelta(_) => return,
                };
                render::zoom(&mut self.camera, steps);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if self.interaction.running() {
            if let Some(graphics) = self.graphics.as_ref() {
                graphics.window.request_redraw();
            }
        }
    }
}

pub fn run(course: Course) -> Result<(), String> {
    let game = Game::new(course);
    let camera = render::follow_camera(&game.state());
    let mut app = App {
        game,
        camera,
        interaction: Interaction::default(),
        graphics: None,
        error: None,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Input;
    use std::time::Duration;

    fn active() -> Interaction {
        let mut interaction = Interaction::default();
        assert!(interaction.resume((960, 640), true));
        interaction
    }

    #[test]
    fn focus_and_suspension_clear_input_drag_and_elapsed_backlog() {
        let mut input = active();
        let now = Instant::now();
        input.key(KeyCode::KeyW, true, false, false);
        input.mouse(true);
        input.pointer([10.0, 20.0]);
        assert_eq!(input.ticks(now), 0);
        assert_eq!(input.ticks(now + Duration::from_millis(17)), 1);
        input.focus(false);
        assert_eq!(input.controls.input(), Input::default());
        assert!(!input.dragging);
        assert!(input.cursor.is_none());
        assert_eq!(input.ticks(now + Duration::from_secs(20)), 0);
        input.focus(true);
        input.key(KeyCode::KeyW, true, false, true);
        assert_eq!(input.controls.input(), Input::default());
        assert_eq!(input.ticks(now + Duration::from_secs(21)), 0);
        input.key(KeyCode::KeyD, true, false, false);
        input.suspend();
        input.suspend();
        assert_eq!(input.controls.input(), Input::default());
        assert!(input.resume((960, 640), true));
        assert_eq!(input.ticks(now + Duration::from_secs(60)), 0);
    }

    #[test]
    fn redundant_resume_resize_minimize_and_occlusion_do_not_advance_stale_input() {
        let mut input = active();
        input.key(KeyCode::ArrowRight, true, false, false);
        assert!(!input.resume((1, 1), false));
        assert_eq!(input.size, (960, 640));
        assert_eq!(input.controls.input().x, 1);
        assert!(!input.resize((960, 640)));
        assert!(input.resize((0, 640)));
        assert!(!input.running());
        assert_eq!(input.controls.input(), Input::default());
        assert_eq!(input.ticks(Instant::now()), 0);
        input.resize((320, 240));
        assert!(input.running());
        assert_eq!(input.ticks(Instant::now()), 0);
        input.key(KeyCode::KeyA, true, false, false);
        input.occlusion(true);
        assert_eq!(input.controls.input(), Input::default());
        assert!(!input.drawable());
        input.occlusion(false);
        assert_eq!(input.ticks(Instant::now()), 0);
        input.resize((u32::MAX, 240));
        assert!(!input.drawable());
    }

    #[test]
    fn restart_is_edge_triggered_and_synthetic_releases_clear_controls() {
        let mut input = active();
        input.key(KeyCode::KeyW, true, false, false);
        input.key(KeyCode::ArrowUp, true, false, false);
        input.key(KeyCode::KeyW, false, false, true);
        assert_eq!(input.controls.input().z, -1);
        input.key(KeyCode::ArrowUp, false, false, true);
        assert_eq!(input.controls.input(), Input::default());
        input.key(KeyCode::KeyD, true, false, false);
        assert_eq!(input.key(KeyCode::KeyR, true, false, true), Action::None);
        assert_eq!(
            input.key(KeyCode::KeyR, true, false, false),
            Action::Restart
        );
        assert_eq!(input.controls.input(), Input::default());
        assert_eq!(input.key(KeyCode::KeyR, true, true, false), Action::None);
        assert_eq!(input.key(KeyCode::KeyR, true, false, false), Action::None);
        input.key(KeyCode::KeyR, false, false, false);
        assert_eq!(
            input.key(KeyCode::KeyR, true, false, false),
            Action::Restart
        );
        assert_eq!(input.key(KeyCode::Escape, true, false, false), Action::Exit);
    }

    #[test]
    fn autorepeat_cannot_restore_movement_after_an_input_reset() {
        for transition in 0..5 {
            let mut input = active();
            input.key(KeyCode::KeyW, true, false, false);
            assert_eq!(input.controls.input().z, -1);
            match transition {
                0 => {
                    input.focus(false);
                    input.focus(true);
                }
                1 => {
                    assert_eq!(
                        input.key(KeyCode::KeyR, true, false, false),
                        Action::Restart
                    );
                }
                2 => {
                    input.suspend();
                    input.resume((960, 640), true);
                }
                3 => {
                    input.resize((0, 0));
                    input.resize((960, 640));
                }
                _ => {
                    input.occlusion(true);
                    input.occlusion(false);
                }
            }
            input.key(KeyCode::KeyW, true, true, false);
            input.key(KeyCode::ArrowUp, true, true, false);
            assert_eq!(input.controls.input(), Input::default());
            input.key(KeyCode::KeyW, true, false, false);
            assert_eq!(input.controls.input().z, -1);
        }
    }

    #[test]
    fn pointer_requires_fresh_finite_positions_and_bounds_huge_deltas() {
        let mut input = active();
        input.mouse(true);
        assert_eq!(input.pointer([f64::NAN, 0.0]), None);
        assert_eq!(input.pointer([-f64::MAX, 0.0]), None);
        assert_eq!(input.pointer([f64::MAX, 0.0]), Some([2048.0, 0.0]));
        input.focus(false);
        input.focus(true);
        assert_eq!(input.pointer([20.0, 30.0]), None);
        assert_eq!(input.pointer([25.0, 30.0]), None);
        input.mouse(true);
        assert_eq!(input.pointer([30.0, 35.0]), Some([5.0, 5.0]));
        input.mouse(false);
        assert_eq!(input.pointer([f64::INFINITY, 0.0]), None);
    }
}
