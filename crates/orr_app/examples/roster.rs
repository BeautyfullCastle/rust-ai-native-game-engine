//! Who is in the game, on a relay server: one avatar per player, made by the player signals.
//!
//! Offline (you steer slot 0, slot 1 stands still): `cargo run -p orr_app --release --example roster`.
//! Two windows on one PC, one player each (A and D move the avatar, Escape quits):
//!   `cargo run -p orr_app --release --example roster -- --host`
//!   `cargo run -p orr_app --release --example roster -- --connect 127.0.0.1:7777`
//! Close one window and open it again with the `--slot` and `--token` it printed: the avatar
//! dims while the player is away (`Disconnected`) and lights up again (`Reconnected`).
#![allow(clippy::float_arithmetic)]
use orr_app::{solid_quad, App, Camera, Extracted, Extractor, EventState, FrameView, RawPlayerSignal, RenderItem, SceneView, View2D, ViewContext};
use orr_testgame::roster::{Avatar, Roster, RosterInput};

/// Every avatar is a square on its own row; the row and the color come from the slot.
struct Look;

const COLORS: [[f32; 3]; 4] = [[0.95, 0.45, 0.35], [0.35, 0.7, 0.95], [0.5, 0.85, 0.45], [0.95, 0.8, 0.3]];

impl Extractor for Look {
    fn extract(&self, _frame: FrameView<'_>, _out: &mut Vec<Extracted>) {}
}

impl View2D for Look {
    fn camera(&self, _frame: FrameView<'_>) -> Camera {
        Camera::new([0.0, 0.0], 6.0)
    }

    fn backdrop(&self, frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        out.push(solid_quad([0.0, 0.0], [12.0, 6.0], [0.07, 0.07, 0.11, 1.0]));
        for (_, avatar) in frame.iter::<Avatar>() {
            let [r, g, b] = COLORS[avatar.slot as usize % COLORS.len()];
            let alpha = if avatar.away != 0 { 0.25 } else { 1.0 };
            let y = 3.0 - 1.5 * avatar.slot as f32;
            out.push(solid_quad([avatar.x as f32 * 0.05, y], [0.5, 0.5], [r, g, b, alpha]));
        }
    }
}

/// Prints each player signal with the state the view gets it in.
struct Roll;

impl SceneView<Roster> for Roll {
    fn on_player(&mut self, ctx: &mut ViewContext<'_, Roster>, signal: &RawPlayerSignal, state: EventState) {
        println!("{:?} of slot {} at tick {} ({state:?})", signal.kind, signal.slot.0, ctx.tick());
    }
}

fn main() -> Result<(), orr_app::Error> {
    App::<Roster>::new()
        .players(2)
        .input(|keys, slot| RosterInput { dx: keys.stick_for_slot(slot).x })
        .scene_view(Roll)
        .view(Look)
        .title("Orrery roster (orr_app)")
        .run()
}
