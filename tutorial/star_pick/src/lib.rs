//! The Star Pick tutorial game (docs/tutorial). One module per step: each is a complete
//! program, and each step extends the one before it. The binaries `step1` .. `step5` call
//! `stepN::app().run()`.
//!
//! Simulation code (the game, its components and systems) uses `FP` only, no `f32`, no
//! `HashMap`, no clock. Only the view code (what the window draws) uses `f32`, and it says so.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

pub mod step1;
pub mod step2;
pub mod step3;
pub mod step4;
pub mod step5;
