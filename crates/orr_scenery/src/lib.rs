//! `orr_scenery`: an outdoor scene renderer on [`orr_rhi`] and the Forest
//! Valley showcase built with it.
//!
//! A frame is drawn in this order:
//!
//! 1. Two sun shadow maps: a sharp one around the camera and a wide one.
//! 2. The scene into a multisampled linear HDR target: terrain, instanced
//!    trees, rocks and ground cover, the sky, then the stream surface.
//! 3. The post chain: bloom, light shafts, exposure, tone mapping, grading
//!    and display encoding.
//!
//! [`World`] generates the valley from one seed. [`Renderer`] owns all GPU
//! resources. [`Camera`] and [`Environment`] describe one frame, and
//! [`Settings`] holds every value of the look that a person can tune.
//!
//! This is view layer code: it uses floats and wall time. Simulation crates
//! never depend on it.
#![allow(clippy::float_arithmetic)]

pub mod math;
#[cfg(feature = "passes")]
pub mod dragon;
#[cfg(feature = "passes")]
pub mod passes;
pub mod mesh;
pub mod renderer;
pub mod settings;
pub mod world;

pub use renderer::Renderer;
pub use settings::{Preset, Quality, Settings};
pub use world::World;

use math::{cross, look_at, mul, normalize, perspective, Mat4, V3};

pub const NEAR_PLANE: f32 = 0.3;
pub const FAR_PLANE: f32 = 9000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: V3,
    /// Radians around the vertical axis. Zero looks along +z.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
}

impl Camera {
    /// A camera at `position` that looks at `target`.
    pub fn looking_at(position: V3, target: V3) -> Self {
        let d = normalize(math::sub(target, position));
        Self { position, yaw: d[0].atan2(d[2]), pitch: d[1].clamp(-1.0, 1.0).asin(), fov_y: 55.0_f32.to_radians() }
    }

    pub fn forward(&self) -> V3 {
        [self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos()]
    }

    pub fn right(&self) -> V3 {
        normalize(cross(self.forward(), [0.0, 1.0, 0.0]))
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = look_at(self.position, math::add(self.position, self.forward()), [0.0, 1.0, 0.0]);
        mul(&perspective(self.fov_y, aspect, NEAR_PLANE, FAR_PLANE), &view)
    }

    /// Turns the view by the given angles in radians. Positive `pitch` looks up.
    pub fn turn(&mut self, yaw: f32, pitch: f32) {
        if yaw.is_finite() && pitch.is_finite() {
            self.yaw = (self.yaw + yaw).rem_euclid(std::f32::consts::TAU);
            self.pitch = (self.pitch + pitch).clamp(-1.45, 1.45);
        }
    }

    /// Height of the camera above the ground or the water under it.
    pub fn height_above(&self, world: &World) -> f32 {
        let [x, y, z] = self.position;
        y - world.height(x, z).max(world.stream(z).level)
    }

    /// Moves the camera for `seconds`. Walking follows the ground at eye
    /// height and ignores `motion.lift`. Flying is free above the ground.
    /// The camera stays inside the detailed part of the world.
    pub fn travel(&mut self, world: &World, motion: Motion, walking: bool, seconds: f32) {
        const EYE_HEIGHT: f32 = 1.7;
        // The camera stays this far inside the detailed square.
        const LIMIT: f32 = world::HALF - 42.0;
        let seconds = if seconds.is_finite() { seconds.clamp(0.0, 0.25) } else { 0.0 };
        let (forward, right) = (self.forward(), self.right());
        let speed = if walking { 5.0 } else { 22.0 } * if motion.fast { 4.0 } else { 1.0 } * seconds;
        let (ahead, side, lift) = (motion.ahead.clamp(-1.0, 1.0), motion.side.clamp(-1.0, 1.0), motion.lift.clamp(-1.0, 1.0));
        let p = &mut self.position;
        if walking {
            // Walking ignores where the camera looks up or down.
            let flat = normalize([forward[0], 0.0, forward[2]]);
            p[0] += (flat[0] * ahead + right[0] * side) * speed;
            p[2] += (flat[2] * ahead + right[2] * side) * speed;
        } else {
            for k in 0..3 {
                p[k] += (forward[k] * ahead + right[k] * side) * speed;
            }
            p[1] += lift * speed;
        }
        p[0] = p[0].clamp(-LIMIT, LIMIT);
        p[2] = p[2].clamp(-LIMIT, LIMIT);
        let floor = world.height(p[0], p[2]).max(world.stream(p[2]).level);
        if walking {
            // Ease to eye height so steps in the ground do not jolt the view.
            p[1] += (floor + EYE_HEIGHT - p[1]) * (1.0 - (-12.0 * seconds).exp());
        } else {
            p[1] = p[1].clamp(floor + 0.6, 600.0);
        }
    }
}

/// Where a person wants the camera to go, each axis from -1 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub ahead: f32,
    /// Positive is to the right.
    pub side: f32,
    /// Positive is up. Used only while flying.
    pub lift: f32,
    pub fast: bool,
}

/// What changes from frame to frame without a person changing it.
/// Everything a person tunes is in [`Settings`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    /// Seconds. Drives wind, water and clouds. It stands still while the scene is paused.
    pub time: f32,
    /// Real seconds since the previous frame. Auto exposure follows this, also while paused.
    pub frame_seconds: f32,
    /// World height where the fog is densest, normally the water level near the camera.
    pub fog_height: f32,
}

impl Environment {
    /// The environment at `time` seconds for a camera in `world`, `frame_seconds` after the previous frame.
    pub fn at(world: &World, camera: &Camera, time: f32, frame_seconds: f32) -> Self {
        Self { time, frame_seconds, fog_height: world.stream(camera.position[2]).level }
    }
}

/// Camera positions that show the valley well, in walking order from upstream.
pub fn viewpoints(world: &World) -> Vec<Camera> {
    let eye = |x: f32, z: f32, above: f32| -> V3 { [x, world.height(x, z).max(world.stream(z).level) + above, z] };
    let on_bank = |z: f32, side: f32, out: f32, above: f32| -> V3 {
        let s = world.stream(z);
        eye(s.center + side * (s.half_width + out), z, above)
    };
    let water = |z: f32, above: f32| -> V3 {
        let s = world.stream(z);
        [s.center, s.level + above, z]
    };
    // Cascades sit at the end of each 72 m step; step k ends at z = -512 + 72 * (k + 1).
    let cascade = |k: f32| -512.0 + 72.0 * (k + 1.0) - 6.0;
    vec![
        // On the bank below a cascade, looking upstream with the sun to the side.
        Camera::looking_at(on_bank(cascade(8.0) + 26.0, 1.0, 0.5, 1.6), water(cascade(8.0) - 4.0, 1.4)),
        // Standing in the shallows of a pool, looking downstream.
        Camera::looking_at(on_bank(cascade(5.0) + 18.0, -1.0, -1.5, 1.4), water(cascade(5.0) + 62.0, 0.3)),
        // From the forest edge across a meadow to the stream.
        Camera::looking_at(on_bank(cascade(7.0) + 40.0, -1.0, 34.0, 1.8), water(cascade(7.0) + 8.0, 1.0)),
        // Low over a pool, looking upstream into the light.
        Camera::looking_at(on_bank(cascade(6.0) + 30.0, 1.0, 1.5, 1.5), water(cascade(6.0), 1.2)),
        // High above the valley, looking upstream along it.
        Camera::looking_at(eye(world.stream(120.0).center + 120.0, 120.0, 95.0), water(-160.0, 0.0)),
        // Above a slope, looking down the valley to the far ridges.
        Camera::looking_at(eye(world.stream(-140.0).center - 130.0, -140.0, 38.0), water(120.0, 6.0)),
    ]
}
