//! Step 1: an empty project. One square sweeps left and right in a window.
//!
//! `cargo run -p star_pick --release --bin step1`

use bytemuck::{Pod, Zeroable};
use orr_app::{
    fp_to_f32, solid_quad, App, Camera, Extracted, Extractor, FrameView, InterpMode, RenderItem, Shape, Style, Transform2, Vec2, View2D,
};
use orr_sim::{fn_systems, ComponentRegistryBuilder, Frame, Game, SimContext, System, FP};

/// Where something stands. `FP` is the fixed-point number the simulation uses instead of `f32`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Pos {
    pub x: FP,
    pub y: FP,
}

/// The game. It has no data of its own: the data lives in the frame.
pub struct StarPick;

impl Game for StarPick {
    type Input = (); // nobody steers yet
    type Command = ();
    type Event = ();
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Pos>("Pos");
    }

    fn setup(frame: &mut Frame, _config: &()) {
        let square = frame.spawn();
        frame.add(square, Pos { x: FP::ZERO, y: FP::ZERO });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        fn_systems([sweep])
    }
}

/// Every tick: put the square on its place of the sweep. One sweep takes 160 ticks.
fn sweep(ctx: &mut SimContext<StarPick>) {
    let phase = (ctx.tick % 160) as i32; // 0..159
    let along = if phase < 80 { phase - 40 } else { 120 - phase }; // -40..40, and back
    for (_, (pos,)) in ctx.frame.query::<(&mut Pos,)>() {
        pos.x = FP::from_int(along).div_int(10); // -4.0 .. 4.0
    }
}

/// How the window shows the frame. This is view code, so `f32` is fine here.
struct Look;

impl Extractor for Look {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        for (entity, pos) in frame.iter::<Pos>() {
            out.push(Extracted {
                entity,
                transform: Transform2::new(Vec2::new(fp_to_f32(pos.x), fp_to_f32(pos.y)), 0.0),
                mode: InterpMode::Prediction, // smooth movement between two ticks
                style: Style { shape: Shape::Quad, size: 0.4, half_y: 0.0, color: [0.35, 0.7, 0.95, 1.0] },
            });
        }
    }
}

impl View2D for Look {
    fn camera(&self, _frame: FrameView<'_>) -> Camera {
        Camera::new([0.0, 0.0], 5.5) // the picture shows 11 units from the bottom to the top
    }

    fn backdrop(&self, _frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        out.push(solid_quad([0.0, 0.0], [4.5, 4.5], [0.09, 0.10, 0.15, 1.0])); // the field
    }
}

/// The whole program of step 1 as an `App`. `main` only calls `run`.
pub fn app() -> App<StarPick> {
    App::<StarPick>::new().input(|_keys, _slot| ()).view(Look).title("Star Pick 1")
}
