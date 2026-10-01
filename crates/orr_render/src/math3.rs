//! Small `f32` vector and matrix helpers for the 3D renderer (view layer).
//!
//! Vectors are plain `[f32; 3]` arrays. Matrices are column major, the
//! layout WGSL's `mat4x4<f32>` reads: `m[col][row]`. The convention is right
//! handed, y up, camera looking down -z, clip depth `0..1` (wgpu).

pub type Vec3 = [f32; 3];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// Unit vector; the zero vector stays zero.
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 1e-12 {
        scale(a, 1.0 / l)
    } else {
        [0.0; 3]
    }
}

/// Rotates `v` by the unit quaternion `q = [x, y, z, w]`.
pub fn quat_rotate(q: [f32; 4], v: Vec3) -> Vec3 {
    let qv = [q[0], q[1], q[2]];
    let t = scale(cross(qv, v), 2.0);
    add(add(v, scale(t, q[3])), cross(qv, t))
}

/// A 4x4 matrix, column major: `m[col][row]`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub const IDENTITY: Mat4 =
        Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    /// `self * rhs` (apply `rhs` first).
    pub fn mul(&self, rhs: &Mat4) -> Mat4 {
        let mut out = [[0.0f32; 4]; 4];
        for (c, col) in out.iter_mut().enumerate() {
            for (r, v) in col.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[k][r] * rhs.0[c][k]).sum();
            }
        }
        Mat4(out)
    }

    /// Transforms the point `p` (w = 1) and returns the homogeneous result.
    pub fn transform_point4(&self, p: Vec3) -> [f32; 4] {
        let m = &self.0;
        let mut out = [0.0f32; 4];
        for (r, v) in out.iter_mut().enumerate() {
            *v = m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
        }
        out
    }

    /// World to view space for an eye looking at `target`.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
        let f = normalize(sub(target, eye));
        let s = normalize(cross(f, up));
        let u = cross(s, f);
        Mat4([
            [s[0], u[0], -f[0], 0.0],
            [s[1], u[1], -f[1], 0.0],
            [s[2], u[2], -f[2], 0.0],
            [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
        ])
    }

    /// Perspective projection, `fov_y` in radians, depth `0..1`.
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y * 0.5).tan();
        let d = near - far;
        Mat4([
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / d, -1.0],
            [0.0, 0.0, near * far / d, 0.0],
        ])
    }

    /// Orthographic projection of the box `x in [l, r]`, `y in [b, t]`,
    /// distance `near..far` in front of the eye, depth `0..1`.
    pub fn orthographic(l: f32, r: f32, b: f32, t: f32, near: f32, far: f32) -> Mat4 {
        let d = near - far;
        Mat4([
            [2.0 / (r - l), 0.0, 0.0, 0.0],
            [0.0, 2.0 / (t - b), 0.0, 0.0],
            [0.0, 0.0, 1.0 / d, 0.0],
            [-(r + l) / (r - l), -(t + b) / (t - b), near / d, 1.0],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn look_at_puts_the_target_on_the_negative_z_axis() {
        let v = Mat4::look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(close(v.transform_point4([0.0, 0.0, 0.0]), [0.0, 0.0, -5.0, 1.0]));
        assert!(close(v.transform_point4([1.0, 2.0, 0.0]), [1.0, 2.0, -5.0, 1.0]));
    }

    #[test]
    fn perspective_maps_near_and_far_to_depth_0_and_1() {
        let p = Mat4::perspective(1.0, 1.5, 0.5, 50.0);
        let n = p.transform_point4([0.0, 0.0, -0.5]);
        let f = p.transform_point4([0.0, 0.0, -50.0]);
        assert!((n[2] / n[3]).abs() < 1e-5, "{n:?}");
        assert!((f[2] / f[3] - 1.0).abs() < 1e-5, "{f:?}");
    }

    #[test]
    fn orthographic_maps_the_box_to_the_unit_cube() {
        let p = Mat4::orthographic(-2.0, 2.0, -1.0, 1.0, 1.0, 11.0);
        assert!(close(p.transform_point4([2.0, 1.0, -1.0]), [1.0, 1.0, 0.0, 1.0]));
        assert!(close(p.transform_point4([-2.0, -1.0, -11.0]), [-1.0, -1.0, 1.0, 1.0]));
    }

    #[test]
    fn quaternion_rotation_turns_x_into_z() {
        // 90 degrees about +y: x -> -z.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let r = quat_rotate([0.0, h, 0.0, h], [1.0, 0.0, 0.0]);
        assert!((r[0]).abs() < 1e-6 && (r[2] + 1.0).abs() < 1e-6, "{r:?}");
    }
}
