//! Collider surface wires for the editor only; no simulation or picking authority.
use orr_render::RenderList3D;
use orr_view::{RenderItem3, Shape3};

const STEPS: usize = 32;
const MAX_SEGMENTS: usize = 132;
type Segment = ([f32; 3], [f32; 3]);

/// Append a complete valid outline, or leave the list unchanged.
pub(super) fn append(list: &mut RenderList3D, item: &RenderItem3) {
    if let Some(segments) = segments(item) {
        for (a, b) in segments {
            list.line(a, b, 2.0, [1.0, 0.85, 0.2, 1.0]);
        }
    }
}

fn segments(item: &RenderItem3) -> Option<Vec<Segment>> {
    let position = item.transform.pos.to_array();
    let rotation = item.transform.rot.to_array();
    if !position
        .iter()
        .chain(rotation.iter())
        .all(|v| v.is_finite())
    {
        return None;
    }
    let norm: f32 = rotation.iter().map(|v| v * v).sum();
    if !norm.is_finite() || (norm - 1.0).abs() > 0.001 {
        return None;
    }
    let positive = |v: f32| v.is_finite() && v > 0.0;
    let mut lines = Vec::with_capacity(MAX_SEGMENTS);
    match item.style.shape {
        Shape3::Box { half } => {
            if !half.into_iter().all(positive) {
                return None;
            }
            let corner = |i: usize| {
                std::array::from_fn(|axis| {
                    if i & (1 << axis) == 0 {
                        -half[axis]
                    } else {
                        half[axis]
                    }
                })
            };
            for i in 0..8 {
                for bit in [1, 2, 4] {
                    if i & bit == 0 {
                        lines.push((corner(i), corner(i | bit)));
                    }
                }
            }
        }
        Shape3::Sphere { radius } => {
            if !positive(radius) {
                return None;
            }
            for axis in 0..3 {
                curve(&mut lines, STEPS, std::f32::consts::TAU, |a| {
                    let mut p = [0.0; 3];
                    p[(axis + 1) % 3] = radius * a.cos();
                    p[(axis + 2) % 3] = radius * a.sin();
                    p
                });
            }
        }
        Shape3::Capsule {
            half_length,
            radius,
        } => {
            if !positive(radius) || !half_length.is_finite() || half_length < 0.0 {
                return None;
            }
            for sign in [-1.0, 1.0] {
                curve(&mut lines, STEPS, std::f32::consts::TAU, |a| {
                    [radius * a.cos(), sign * half_length, radius * a.sin()]
                });
                for axis in [0, 2] {
                    curve(&mut lines, STEPS / 2, std::f32::consts::PI, |a| {
                        let mut p = [0.0; 3];
                        p[axis] = radius * a.cos();
                        p[1] = sign * (half_length + radius * a.sin());
                        p
                    });
                }
            }
            for axis in [0, 2] {
                for sign in [-1.0, 1.0] {
                    let mut a = [0.0, -half_length, 0.0];
                    a[axis] = sign * radius;
                    let mut b = a;
                    b[1] = half_length;
                    lines.push((a, b));
                }
            }
        }
        Shape3::Plane { half_x, half_z } => {
            if !positive(half_x) || !positive(half_z) {
                return None;
            }
            let corners = [
                [-half_x, 0.0, -half_z],
                [half_x, 0.0, -half_z],
                [half_x, 0.0, half_z],
                [-half_x, 0.0, half_z],
            ];
            for i in 0..4 {
                lines.push((corners[i], corners[(i + 1) % 4]));
            }
        }
    }
    // fill_list deliberately renders planes horizontally, regardless of body rotation.
    let horizontal = matches!(item.style.shape, Shape3::Plane { .. });
    for (a, b) in &mut lines {
        for point in [a, b] {
            let local = if horizontal {
                *point
            } else {
                orr_render::math3::quat_rotate(rotation, *point)
            };
            *point = std::array::from_fn(|axis| position[axis] + local[axis]);
            if !point.iter().all(|v| v.is_finite()) {
                return None;
            }
        }
    }
    (lines.len() <= MAX_SEGMENTS).then_some(lines)
}

fn curve(lines: &mut Vec<Segment>, steps: usize, angle: f32, point: impl Fn(f32) -> [f32; 3]) {
    for i in 0..steps {
        lines.push((
            point(angle * i as f32 / steps as f32),
            point(angle * (i + 1) as f32 / steps as f32),
        ));
    }
}

#[cfg(test)]
#[path = "selection_outline/tests.rs"]
mod tests;
