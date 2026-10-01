//! `orr_web_gpu`: the browser's GPU view.
//!
//! The browser client (`orr_web`, in a Web Worker) hands the page integer draw lists; this crate
//! turns them into `orr_render` render lists and draws them with the same 2D renderer the native
//! samples use, on wgpu's browser backends: **WebGPU** where the browser has it, **WebGL2**
//! otherwise (`orr_rhi::Wgpu::for_canvas`). It is a separate wasm package so the simulation
//! package stays small; the page falls back to a plain 2D canvas when neither API is available.
//!
//! View layer: floats are fine here. Sim crates never depend on it.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod draw;

pub use draw::{arena_list, phys_list, ARENA_HALF};

#[cfg(target_arch = "wasm32")]
mod wasm;
