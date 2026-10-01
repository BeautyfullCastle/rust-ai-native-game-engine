//! 3D cameras: a perspective or orthographic [`Camera3D`], and an
//! [`OrbitCamera`] that turns mouse drags into camera moves.
//!
//! World space: right handed, y up. Screen space: pixels, origin at the top
//! left, y down (like the 2D [`crate::Camera`]).

use crate::math3::{add, cross, dot, normalize, scale, sub, Mat4, Vec3};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Projection {
    /// `fov_y` is the full vertical field of view in radians.
    Perspective { fov_y: f32, near: f32, far: f32 },
    /// Shows `half_height` world units above and below the target line of sight.
    Orthographic { half_height: f32, near: f32, far: f32 },
}

/// An eye looking at a target through a projection.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera3D {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub projection: Projection,
}

impl Camera3D {
    pub fn perspective(eye: Vec3, target: Vec3, fov_y_degrees: f32) -> Self {
        Self {
            eye,
            target,
            up: [0.0, 1.0, 0.0],
            projection: Projection::Perspective { fov_y: fov_y_degrees.to_radians(), near: 0.1, far: 500.0 },
        }
    }

    pub fn orthographic(eye: Vec3, target: Vec3, half_height: f32) -> Self {
        Self {
            eye,
            target,
            up: [0.0, 1.0, 0.0],
            projection: Projection::Orthographic { half_height, near: -500.0, far: 500.0 },
        }
    }

    /// Unit vector from the eye to the target.
    pub fn forward(&self) -> Vec3 {
        normalize(sub(self.target, self.eye))
    }

    fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.forward();
        let r = normalize(cross(f, self.up));
        let u = cross(r, f);
        (f, r, u)
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.eye, self.target, self.up)
    }

    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Perspective { fov_y, near, far } => Mat4::perspective(fov_y, aspect, near, far),
            Projection::Orthographic { half_height, near, far } => {
                let half_w = half_height * aspect;
                Mat4::orthographic(-half_w, half_w, -half_height, half_height, near, far)
            }
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection_matrix(aspect).mul(&self.view())
    }

    /// Screen position (pixels, top left origin) of a world point, `None`
    /// when it is behind the camera.
    pub fn world_to_screen(&self, world: Vec3, viewport: (u32, u32)) -> Option<[f32; 2]> {
        let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
        let c = self.view_proj(w / h).transform_point4(world);
        if c[3] <= 1e-6 {
            return None;
        }
        let (x, y) = (c[0] / c[3], c[1] / c[3]);
        Some([(x * 0.5 + 0.5) * w, (0.5 - y * 0.5) * h])
    }

    /// The world space ray (origin, unit direction) through a screen position.
    pub fn screen_ray(&self, screen: [f32; 2], viewport: (u32, u32)) -> (Vec3, Vec3) {
        let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
        let (nx, ny) = (screen[0] / w * 2.0 - 1.0, 1.0 - screen[1] / h * 2.0);
        let aspect = w / h;
        let (f, r, u) = self.basis();
        match self.projection {
            Projection::Perspective { fov_y, .. } => {
                let t = (fov_y * 0.5).tan();
                let dir = add(f, add(scale(r, nx * t * aspect), scale(u, ny * t)));
                (self.eye, normalize(dir))
            }
            Projection::Orthographic { half_height, .. } => {
                let origin = add(self.eye, add(scale(r, nx * half_height * aspect), scale(u, ny * half_height)));
                (origin, f)
            }
        }
    }

    /// Where the ray through `screen` meets the horizontal plane `y = height`.
    pub fn screen_to_plane_y(&self, screen: [f32; 2], viewport: (u32, u32), height: f32) -> Option<Vec3> {
        let (o, d) = self.screen_ray(screen, viewport);
        if d[1].abs() < 1e-6 {
            return None;
        }
        let t = (height - o[1]) / d[1];
        (t > 0.0).then(|| add(o, scale(d, t)))
    }

    /// A unit vector along the camera's right.
    pub fn right(&self) -> Vec3 {
        self.basis().1
    }

    /// A unit vector along the camera's up (perpendicular to `forward`).
    pub fn camera_up(&self) -> Vec3 {
        self.basis().2
    }

    /// How far the target is from the eye.
    pub fn distance(&self) -> f32 {
        dot(sub(self.target, self.eye), self.forward())
    }
}

/// Orbits a target: drag to turn, wheel to zoom, middle or right drag to pan.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Turn around the y axis in radians (0 looks from +z toward the target).
    pub yaw: f32,
    /// Elevation above the horizon in radians, kept inside `(-pi/2, pi/2)`.
    pub pitch: f32,
    pub distance: f32,
    pub fov_y_degrees: f32,
}

impl OrbitCamera {
    pub const MIN_DISTANCE: f32 = 0.5;
    pub const MAX_DISTANCE: f32 = 400.0;
    const MAX_PITCH: f32 = 1.5;

    pub fn new(target: Vec3, yaw: f32, pitch: f32, distance: f32) -> Self {
        let mut c = Self { target, yaw, pitch, distance, fov_y_degrees: 50.0 };
        c.clamp();
        c
    }

    fn clamp(&mut self) {
        self.pitch = self.pitch.clamp(-Self::MAX_PITCH, Self::MAX_PITCH);
        self.distance = self.distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Eye position.
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        add(self.target, scale([cp * sy, sp, cp * cy], self.distance))
    }

    /// Mouse drag of `delta` pixels turns the camera around the target.
    pub fn orbit(&mut self, delta: [f32; 2]) {
        self.yaw -= delta[0] * 0.006;
        self.pitch += delta[1] * 0.006;
        self.clamp();
    }

    /// Wheel zoom: `steps > 0` moves closer (each step is 10 percent).
    pub fn zoom(&mut self, steps: f32) {
        self.distance *= 0.9f32.powf(steps);
        self.clamp();
    }

    /// Moves the target in the camera plane so the scene follows the drag.
    pub fn pan(&mut self, delta: [f32; 2], viewport: (u32, u32)) {
        let cam = self.camera();
        let per_pixel = match cam.projection {
            Projection::Perspective { fov_y, .. } => 2.0 * self.distance * (fov_y * 0.5).tan() / viewport.1.max(1) as f32,
            Projection::Orthographic { half_height, .. } => 2.0 * half_height / viewport.1.max(1) as f32,
        };
        let shift = add(scale(cam.right(), -delta[0] * per_pixel), scale(cam.camera_up(), delta[1] * per_pixel));
        self.target = add(self.target, shift);
    }

    pub fn camera(&self) -> Camera3D {
        Camera3D::perspective(self.eye(), self.target, self.fov_y_degrees)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: (u32, u32) = (800, 600);

    #[test]
    fn target_maps_to_the_screen_center() {
        let cam = Camera3D::perspective([3.0, 4.0, 10.0], [1.0, 0.0, 0.0], 60.0);
        let p = cam.world_to_screen([1.0, 0.0, 0.0], VP).unwrap();
        assert!((p[0] - 400.0).abs() < 1e-2 && (p[1] - 300.0).abs() < 1e-2, "{p:?}");
    }

    #[test]
    fn up_is_up_and_right_is_right_on_screen() {
        let cam = Camera3D::perspective([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 60.0);
        let above = cam.world_to_screen([0.0, 1.0, 0.0], VP).unwrap();
        let right = cam.world_to_screen([1.0, 0.0, 0.0], VP).unwrap();
        assert!(above[1] < 300.0 && (above[0] - 400.0).abs() < 1e-3);
        assert!(right[0] > 400.0 && (right[1] - 300.0).abs() < 1e-3);
        // tan(30 deg) * 10 = 5.77 units fill half the height: 1 unit is 300 / 5.77 px.
        assert!((above[1] - (300.0 - 300.0 / (10.0 * 30f32.to_radians().tan()))).abs() < 0.05, "{above:?}");
    }

    #[test]
    fn a_point_behind_the_camera_has_no_screen_position() {
        let cam = Camera3D::perspective([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 60.0);
        assert!(cam.world_to_screen([0.0, 0.0, 20.0], VP).is_none());
    }

    #[test]
    fn orthographic_has_no_perspective_shrink() {
        let cam = Camera3D::orthographic([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 3.0);
        let near = cam.world_to_screen([1.0, 0.0, 5.0], VP).unwrap();
        let far = cam.world_to_screen([1.0, 0.0, -50.0], VP).unwrap();
        assert!((near[0] - far[0]).abs() < 1e-3);
        // half_height 3 = 300 px: 1 unit = 100 px.
        assert!((near[0] - 500.0).abs() < 1e-2, "{near:?}");
    }

    #[test]
    fn screen_ray_round_trips_with_world_to_screen() {
        for cam in [
            Camera3D::perspective([5.0, 6.0, 7.0], [0.0, 1.0, 0.0], 50.0),
            Camera3D::orthographic([5.0, 6.0, 7.0], [0.0, 1.0, 0.0], 8.0),
        ] {
            let world = [1.5, 0.25, -2.0];
            let s = cam.world_to_screen(world, VP).unwrap();
            let (o, d) = cam.screen_ray(s, VP);
            // The world point lies on the ray.
            let t = dot(sub(world, o), d);
            let on_ray = add(o, scale(d, t));
            assert!(crate::math3::length(sub(on_ray, world)) < 1e-3, "{cam:?}");
        }
    }

    #[test]
    fn plane_pick_finds_the_ground_point() {
        let cam = Camera3D::perspective([0.0, 10.0, 10.0], [0.0, 0.0, 0.0], 50.0);
        let hit = cam.screen_to_plane_y([400.0, 300.0], VP, 0.0).unwrap();
        assert!(crate::math3::length(hit) < 1e-3, "{hit:?}");
    }

    #[test]
    fn orbit_keeps_the_distance_and_clamps_the_pitch() {
        let mut o = OrbitCamera::new([1.0, 2.0, 3.0], 0.3, 0.4, 12.0);
        let d0 = crate::math3::length(sub(o.eye(), o.target));
        o.orbit([100.0, -50.0]);
        let d1 = crate::math3::length(sub(o.eye(), o.target));
        assert!((d0 - 12.0).abs() < 1e-4 && (d1 - 12.0).abs() < 1e-4);
        o.orbit([0.0, 1.0e6]);
        assert!(o.pitch <= 1.5 + 1e-6);
        o.zoom(1.0e3);
        assert!(o.distance >= OrbitCamera::MIN_DISTANCE);
    }

    #[test]
    fn pan_moves_the_target_with_the_mouse() {
        let mut o = OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.0, 10.0);
        let world = [0.0, 0.0, 0.0];
        let before = o.camera().world_to_screen(world, VP).unwrap();
        o.pan([40.0, 0.0], VP);
        let after = o.camera().world_to_screen(world, VP).unwrap();
        assert!((after[0] - before[0] - 40.0).abs() < 0.5, "{before:?} {after:?}");
    }
}
