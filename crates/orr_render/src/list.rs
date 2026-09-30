//! The render list: everything to draw this frame, as plain GPU-ready data.
//!
//! The view layer fills it each frame ("extract", see [`crate::extract`]);
//! the renderer only reads it and never sees the sim.

use bytemuck::{Pod, Zeroable};

pub const SHAPE_CIRCLE: u32 = 0;
pub const SHAPE_QUAD: u32 = 1;
pub const SHAPE_CAPSULE: u32 = 2;

/// One filled shape as the shader reads it (40 bytes).
///
/// Circle: `half_size = [r, r]`. Quad: half extents. Capsule: `half_size =
/// [half length of the axis segment along local x, radius]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ShapeInstance {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub rot: f32,
    pub shape: u32,
    pub color: [f32; 4],
}

/// One line segment as the shader reads it (40 bytes). `width` is in pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineInstance {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub width: f32,
    pub pad: u32,
    pub color: [f32; 4],
}

/// Shapes (drawn first, in order, later on top) and lines (drawn over all shapes).
#[derive(Default, Clone, Debug)]
pub struct RenderList {
    pub shapes: Vec<ShapeInstance>,
    pub lines: Vec<LineInstance>,
}

impl RenderList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.shapes.clear();
        self.lines.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.lines.is_empty()
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, color: [f32; 4]) {
        self.shapes.push(ShapeInstance { center, half_size: [radius; 2], rot: 0.0, shape: SHAPE_CIRCLE, color });
    }

    /// A circle that shows its rotation with a dark dot.
    pub fn circle_rot(&mut self, center: [f32; 2], radius: f32, rot: f32, color: [f32; 4]) {
        self.shapes.push(ShapeInstance { center, half_size: [radius; 2], rot, shape: SHAPE_CIRCLE, color });
    }

    pub fn quad(&mut self, center: [f32; 2], half_size: [f32; 2], rot: f32, color: [f32; 4]) {
        self.shapes.push(ShapeInstance { center, half_size, rot, shape: SHAPE_QUAD, color });
    }

    /// A capsule: the segment of half length `half_length` along the local x
    /// axis (turned by `rot`), grown by `radius`.
    pub fn capsule(&mut self, center: [f32; 2], half_length: f32, radius: f32, rot: f32, color: [f32; 4]) {
        self.shapes.push(ShapeInstance {
            center,
            half_size: [half_length, radius],
            rot,
            shape: SHAPE_CAPSULE,
            color,
        });
    }

    /// A segment `width` pixels wide.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 4]) {
        self.lines.push(LineInstance { a, b, width, pad: 0, color });
    }

    pub fn polyline(&mut self, points: &[[f32; 2]], closed: bool, width: f32, color: [f32; 4]) {
        for w in points.windows(2) {
            self.line(w[0], w[1], width, color);
        }
        if closed && points.len() > 2 {
            self.line(points[points.len() - 1], points[0], width, color);
        }
    }

    /// Outline of an axis aligned box (a collider AABB).
    pub fn aabb(&mut self, min: [f32; 2], max: [f32; 2], width: f32, color: [f32; 4]) {
        let c = [[min[0], min[1]], [max[0], min[1]], [max[0], max[1]], [min[0], max[1]]];
        self.polyline(&c, true, width, color);
    }

    /// Outline of a box turned by `rot` around `center`.
    pub fn obb(&mut self, center: [f32; 2], half_size: [f32; 2], rot: f32, width: f32, color: [f32; 4]) {
        let (s, c) = rot.sin_cos();
        let p = |x: f32, y: f32| [center[0] + c * x - s * y, center[1] + s * x + c * y];
        let (hx, hy) = (half_size[0], half_size[1]);
        self.polyline(&[p(-hx, -hy), p(hx, -hy), p(hx, hy), p(-hx, hy)], true, width, color);
    }

    /// Outline of a circle (`segments` >= 3).
    pub fn circle_outline(&mut self, center: [f32; 2], radius: f32, segments: u32, width: f32, color: [f32; 4]) {
        let n = segments.max(3);
        let at = |i: u32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
        };
        for i in 0..n {
            self.line(at(i), at((i + 1) % n), width, color);
        }
    }

    /// Outline of a capsule (same parameters as [`RenderList::capsule`]).
    pub fn capsule_outline(
        &mut self,
        center: [f32; 2],
        half_length: f32,
        radius: f32,
        rot: f32,
        segments: u32,
        width: f32,
        color: [f32; 4],
    ) {
        let n = (segments.max(4) / 2).max(2);
        let (s, c) = rot.sin_cos();
        let to_world = |x: f32, y: f32| [center[0] + c * x - s * y, center[1] + s * x + c * y];
        let mut pts = Vec::with_capacity(2 * n as usize + 2);
        // +x cap from -90 to +90 degrees, then the -x cap from 90 to 270.
        for i in 0..=n {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / n as f32;
            pts.push(to_world(half_length + radius * a.cos(), radius * a.sin()));
        }
        for i in 0..=n {
            let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / n as f32;
            pts.push(to_world(-half_length + radius * a.cos(), radius * a.sin()));
        }
        self.polyline(&pts, true, width, color);
    }

    /// A plus mark of half size `half` world units (a contact point).
    pub fn cross(&mut self, p: [f32; 2], half: f32, width: f32, color: [f32; 4]) {
        self.line([p[0] - half, p[1]], [p[0] + half, p[1]], width, color);
        self.line([p[0], p[1] - half], [p[0], p[1] + half], width, color);
    }

    /// A segment with an arrow head at `to` (a ray, a contact normal).
    /// `head` is the head length in world units.
    pub fn arrow(&mut self, from: [f32; 2], to: [f32; 2], head: f32, width: f32, color: [f32; 4]) {
        self.line(from, to, width, color);
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let len = dx.hypot(dy);
        if len <= f32::EPSILON {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let head = head.min(len);
        for side in [-1.0f32, 1.0] {
            // Barbs go back from the tip, 25 degrees off the shaft.
            let (s, c) = (side * 0.4226, 0.9063);
            let (bx, by) = (-(ux * c - uy * s), -(ux * s + uy * c));
            self.line(to, [to[0] + bx * head, to[1] + by * head], width, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_are_40_bytes() {
        assert_eq!(std::mem::size_of::<ShapeInstance>(), 40);
        assert_eq!(std::mem::size_of::<LineInstance>(), 40);
    }

    #[test]
    fn debug_shapes_emit_the_expected_line_counts() {
        let mut l = RenderList::new();
        l.aabb([0.0; 2], [1.0; 2], 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 4);
        l.clear();
        l.circle_outline([0.0; 2], 1.0, 16, 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 16);
        l.clear();
        l.arrow([0.0; 2], [2.0, 0.0], 0.5, 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 3);
        l.clear();
        l.cross([0.0; 2], 1.0, 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 2);
    }
}
