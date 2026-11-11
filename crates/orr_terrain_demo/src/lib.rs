//! A small, optional terrain course. CPU state uses fixed-point point anchors.
//! This is not a character collider, navigation system, network game or exporter.
#![deny(clippy::float_arithmetic)]

pub mod controller;
pub mod course;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "window")]
pub mod window;

pub use controller::{ControlKey, Controls, FixedClock, Game, Input, State};
pub use course::{Course, DEFAULT_COURSE, DEFAULT_TERRAIN};

pub const CONTROLS: &str = "WASD/arrows move | drag orbit | wheel zoom | R restart | Esc exit";

pub fn status_title(game: &Game) -> String {
    let state = game.state();
    let progress = if state.finished {
        "SUMMIT REACHED! R to play again".to_owned()
    } else if state.collected == 4 {
        "4/4 beacons | Reach the summit finish".to_owned()
    } else {
        format!(
            "{}/4 beacons | Next: {}",
            state.collected,
            state.collected + 1
        )
    };
    format!("{} | {progress} | {CONTROLS}", game.course().title())
}
