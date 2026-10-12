//! Original small content presets, optional with the particle renderer.
//! A sample maps genuine events to these; dust is not itself a landing detector.
use orr_view::{Vec3, particles::{EmitterDesc, ParticleBlend}};
pub fn landing_dust(origin: Vec3, seed: u64) -> EmitterDesc {
    EmitterDesc { origin, velocity: Vec3::new(0.0, 0.5, 0.0), velocity_jitter: Vec3::new(0.7, 0.2, 0.7),
        burst: 12, rate: 0.0, duration: 0.0, lifetime: 0.6, size_start: 0.14, size_end: 0.5,
        color_start: [0.65, 0.56, 0.43, 0.45], color_end: [0.65, 0.56, 0.43, 0.0], seed, blend: ParticleBlend::Alpha }
}
pub fn pickup_sparks(origin: Vec3, seed: u64) -> EmitterDesc {
    EmitterDesc { origin, velocity: Vec3::new(0.0, 1.0, 0.0), velocity_jitter: Vec3::new(1.1, 0.8, 1.1),
        burst: 16, rate: 0.0, duration: 0.0, lifetime: 0.5, size_start: 0.12, size_end: 0.0,
        color_start: [1.0, 0.72, 0.15, 0.9], color_end: [1.0, 0.3, 0.05, 0.0], seed, blend: ParticleBlend::Additive }
}
