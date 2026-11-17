//! The 2D physics demo on `orr_app`: `cargo run -p orr_app --release --example physics2d`.
//! WASD or arrows move the blue paddle, Q/E turn it, space shoots, Escape quits.
#![allow(clippy::float_arithmetic)]
use orr_app::{
    fp_to_f32, solid_quad, App, Camera, Entity, EntityView, EventState, Extracted, Extractor, FrameView, InterpMode, KeyCode as K, RenderItem,
    SceneView, Transform2, View2D, ViewContext,
};
use orr_games::physics_game::{PaddleTag, PhysConfig, PhysEvent, PhysGame, PhysInput, Scene, SceneMode, TICK_RATE};
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

/// One per paddle: a place for a sound, a trail or a name tag that follows the paddle.
#[derive(Default)]
struct PaddleView;

impl EntityView<PhysGame> for PaddleView {
    fn on_create(&mut self, ctx: &mut ViewContext<'_, PhysGame>, entity: Entity) {
        let slot = ctx.predicted().get::<PaddleTag>(entity).map(|tag| tag.slot);
        println!("paddle of slot {slot:?} appeared at tick {}", ctx.tick());
    }

    fn on_update(&mut self, _ctx: &mut ViewContext<'_, PhysGame>, _entity: Entity, pose: Option<Transform2>) {
        let _ = pose; // the interpolated position: move the sound or the trail here
    }
}

/// Counts the physics events. A `Predicted` event may still be taken back (`Canceled`);
/// only the `Verified` ones are final, so the total uses those.
#[derive(Default)]
struct EventCounter {
    verified: u64,
}

impl SceneView<PhysGame> for EventCounter {
    fn on_event(&mut self, ctx: &mut ViewContext<'_, PhysGame>, _event: &PhysEvent, state: EventState) {
        if state == EventState::Verified {
            self.verified += 1;
            if self.verified % 100 == 1 {
                println!("{} physics events verified (tick {})", self.verified, ctx.verified_tick());
            }
        }
    }
}

fn main() -> Result<(), orr_app::Error> {
    let mut bot = phys_bot(1234);
    App::<PhysGame>::with_config(PhysConfig::new(1000, SceneMode::Rain))
        .tick_rate(TICK_RATE)
        .players(2)
        .bot(move |_slot, tick| bot(tick).0)
        .input(|k, _slot| PhysInput::new(k.axis(&[K::KeyA, K::ArrowLeft], &[K::KeyD, K::ArrowRight]), k.axis(&[K::KeyS, K::ArrowDown], &[K::KeyW, K::ArrowUp]), k.axis(&[K::KeyE], &[K::KeyQ]), k.held(K::Space)))
        .entity_view::<PaddleTag>(PaddleView::default)
        .scene_view(EventCounter::default())
        .view(Look(PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }))
        .title("Orrery physics (orr_app)")
        .run()
}
