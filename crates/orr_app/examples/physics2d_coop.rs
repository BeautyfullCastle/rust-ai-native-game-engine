//! Two players on one screen: `cargo run -p orr_app --release --example physics2d_coop`.
//! Player 1 (slot 0): W A S D move the paddle, Space shoots.
//! Player 2 (slot 1): arrow keys move the paddle, Enter shoots. Escape quits.
#![allow(clippy::float_arithmetic)]
use orr_app::{App, Entity, EntityView, EventState, FrameView, InterpMode, PlayerSlot, RawPlayerSignal, SceneView, ViewContext};
use orr_app::{fp_to_f32, solid_quad, Camera, Extracted, Extractor, RenderItem, View2D};
use orr_games::physics_game::{PaddleTag, PhysConfig, PhysGame, PhysInput, Scene, SceneMode, TICK_RATE};
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

/// One per paddle. `is_local` is true for both paddles here: this client controls both players.
#[derive(Default)]
struct PaddleView;

impl EntityView<PhysGame> for PaddleView {
    fn on_create(&mut self, ctx: &mut ViewContext<'_, PhysGame>, entity: Entity) {
        let Some(tag) = ctx.predicted().get::<PaddleTag>(entity) else { return };
        let slot = PlayerSlot(tag.slot as u8);
        println!("paddle of slot {} appeared at tick {} (local: {})", slot.0, ctx.tick(), ctx.is_local(slot));
    }
}

/// Prints the player signals the app sends without a server. `PhysGame` makes its paddles in
/// `setup`, so it does not use `Players`; a game that does would create them from `Added`.
struct Roll;

impl SceneView<PhysGame> for Roll {
    fn on_player(&mut self, ctx: &mut ViewContext<'_, PhysGame>, signal: &RawPlayerSignal, state: EventState) {
        println!("player signal {:?} of slot {} at tick {} ({state:?})", signal.kind, signal.slot.0, ctx.tick());
    }
}

fn main() -> Result<(), orr_app::Error> {
    App::<PhysGame>::with_config(PhysConfig::new(600, SceneMode::Rain))
        .tick_rate(TICK_RATE)
        .local_players(2)
        .input(|keys, slot| {
            let stick = keys.stick_for_slot(slot);
            PhysInput::new(stick.x, stick.y, 0, stick.action)
        })
        .entity_view::<PaddleTag>(PaddleView::default)
        .scene_view(Roll)
        .view(Look(PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }))
        .title("Orrery two players (orr_app)")
        .run()
}
