//! The 2D physics demo on `orr_app`: `cargo run -p orr_app --release --example physics2d`.
//! WASD or arrows move the blue paddle, Q/E turn it, space shoots, Escape quits.
#![allow(clippy::float_arithmetic)]
use orr_app::{fp_to_f32, solid_quad, App, Camera, Extracted, Extractor, FrameView, InterpMode, KeyCode as K, RenderItem, View2D};
use orr_games::physics_game::{PhysConfig, PhysGame, PhysInput, Scene, SceneMode, TICK_RATE};
use orr_sample::physics_host::phys_bot;
use orr_sample::physics_view::PhysExtractor;

/// Bodies come from the sample's extractor; the box around them comes from the `Scene` singleton.
struct Look(PhysExtractor);

impl Extractor for Look {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        self.0.extract(frame, out);
    }
}

impl View2D for Look {
    fn camera(&self, frame: FrameView<'_>) -> Camera {
        let (half_w, height) = box_size(frame);
        Camera::new([0.0, height / 2.0], half_w.max(height / 2.0) * 1.06)
    }

    fn backdrop(&self, frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        let (half_w, height) = box_size(frame);
        out.push(solid_quad([0.0, height / 2.0], [half_w, height / 2.0], [0.07, 0.07, 0.11, 1.0]));
    }
}

fn box_size(frame: FrameView<'_>) -> (f32, f32) {
    let scene = frame.singleton::<Scene>();
    (fp_to_f32(scene.half_w), fp_to_f32(scene.height))
}

fn main() -> Result<(), orr_app::Error> {
    let mut bot = phys_bot(1234);
    App::<PhysGame>::with_config(PhysConfig::new(1000, SceneMode::Rain))
        .tick_rate(TICK_RATE)
        .players(2)
        .bot(move |_slot, tick| bot(tick).0)
        .input(|k, _slot| PhysInput::new(k.axis(&[K::KeyA, K::ArrowLeft], &[K::KeyD, K::ArrowRight]), k.axis(&[K::KeyS, K::ArrowDown], &[K::KeyW, K::ArrowUp]), k.axis(&[K::KeyE], &[K::KeyQ]), k.held(K::Space)))
        .view(Look(PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }))
        .title("Orrery physics (orr_app)")
        .run()
}
