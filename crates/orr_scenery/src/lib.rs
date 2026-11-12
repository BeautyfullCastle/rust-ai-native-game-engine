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
pub mod mesh;
pub mod renderer;
pub mod settings;
pub mod world;

pub use renderer::Renderer;
pub use settings::{Quality, Settings};
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
