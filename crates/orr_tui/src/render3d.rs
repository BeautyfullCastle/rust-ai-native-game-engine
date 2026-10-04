//! Orthographic XZ character rendering for version-2 3D view frames.
//!
//! World y is up and is intentionally projected away. All drawing is a pure function of the
//! frame, camera and interpolation alpha; it does not depend on a renderer or GPU.

use orr_viewstream::{
    EntityRecord3, MODE_NONE, Pose3, SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE,
    STYLE_CHECKER, ViewFrame3,
};

use crate::render::{Cell, Grid};
use crate::schema::ViewSchema;

/// The part of the world shown, with screen horizontal = world x and screen vertical = world z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3 {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// Interpolates position linearly and orientation with shortest-arc unit-quaternion slerp.
pub fn pose(e: &EntityRecord3, alpha: f32) -> Pose3 {
    if e.mode == MODE_NONE || alpha >= 1.0 {
        return e.cur;
    }
    let t = alpha.clamp(0.0, 1.0);
    let mut q0 = normalized(e.prev.rot);
    let mut q1 = normalized(e.cur.rot);
    let mut dot = dot4(q0, q1);
    if dot < 0.0 {
        q1 = q1.map(|v| -v);
        dot = -dot;
    }
    let rot = if dot > 0.9995 {
        normalized(std::array::from_fn(|i| q0[i] + (q1[i] - q0[i]) * t))
    } else {
        dot = dot.clamp(-1.0, 1.0);
        let theta = dot.acos();
        let sin_theta = theta.sin();
        if sin_theta.abs() < 1e-6 {
            q0
        } else {
            let a = ((1.0 - t) * theta).sin() / sin_theta;
            let b = (t * theta).sin() / sin_theta;
            normalized(std::array::from_fn(|i| q0[i] * a + q1[i] * b))
        }
    };
    q0 = rot;
    Pose3 {
        pos: std::array::from_fn(|i| e.prev.pos[i] + (e.cur.pos[i] - e.prev.pos[i]) * t),
        rot: q0,
    }
}

impl Camera3 {
    /// Fits static geometry, or all entities when there are no static entities, with a margin.
    pub fn fit(frame: &ViewFrame3) -> Camera3 {
        let any_static = frame.entities.iter().any(|e| e.mode == MODE_NONE);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for e in frame
            .entities
            .iter()
            .filter(|e| !any_static || e.mode == MODE_NONE)
        {
            let p = e.cur;
            let (extent, _) = footprint(e, p);
            for k in 0..2 {
                lo[k] = lo[k].min(p.pos[if k == 0 { 0 } else { 2 }] - extent[k]);
                hi[k] = hi[k].max(p.pos[if k == 0 { 0 } else { 2 }] + extent[k]);
            }
        }
        if lo[0] > hi[0] {
            return Camera3 {
                min: [-1.0, -1.0],
                max: [1.0, 1.0],
            };
        }
        for k in 0..2 {
            let margin = ((hi[k] - lo[k]) * 0.03).max(0.05);
            lo[k] -= margin;
            hi[k] += margin;
        }
        Camera3 { min: lo, max: hi }
    }
}

/// Draws a 3D frame into an existing character grid using an orthographic XZ projection.
/// Static geometry is drawn first; dynamic geometry follows in stream order.
pub fn render(
    frame: &ViewFrame3,
    schema: &ViewSchema,
    cam: &Camera3,
    alpha: f32,
    w: usize,
    h: usize,
    unicode: bool,
) -> Grid {
    let mut grid = Grid::new(w, h);
    if w == 0 || h == 0 {
        return grid;
    }
    let (ww, wh) = (
        (cam.max[0] - cam.min[0]).max(1e-3),
        (cam.max[1] - cam.min[1]).max(1e-3),
    );
    let sx = (w as f32 / ww).min(h as f32 / wh * 2.0);
    let sy = sx / 2.0;
    let (cx, cz) = (
        (cam.min[0] + cam.max[0]) / 2.0,
        (cam.min[1] + cam.max[1]) / 2.0,
    );
    let (half_w, half_h) = (w as f32 / 2.0, h as f32 / 2.0);
    let to_col = |x: f32| (x - cx) * sx + half_w;
    let to_row = |z: f32| (cz - z) * sy + half_h;

    for pass_static in [true, false] {
        for e in frame
            .entities
            .iter()
            .filter(|e| (e.mode == MODE_NONE) == pass_static)
        {
            if e.rgba[3] == 0 {
                continue;
            }
            let p = pose(e, alpha);
            let (extent, polygon) = footprint(e, p);
            let cell = Cell {
                ch: glyph(schema, e, unicode),
                rgba: e.rgba,
            };
            let x = p.pos[0];
            let z = p.pos[2];
            let (i0, i1) = (
                to_col(x - extent[0]).floor() as i64,
                to_col(x + extent[0]).ceil() as i64,
            );
            let (j0, j1) = (
                to_row(z + extent[1]).floor() as i64,
                to_row(z - extent[1]).ceil() as i64,
            );
            let mut drawn = false;
            for j in j0.max(0)..j1.min(h as i64) {
                for i in i0.max(0)..i1.min(w as i64) {
                    let wx = cx + (i as f32 + 0.5 - half_w) / sx;
                    let wz = cz - (j as f32 + 0.5 - half_h) / sy;
                    if inside(e, p, &polygon, wx, wz) {
                        let mut shown = cell;
                        if e.shape == SHAPE3_PLANE
                            && e.style_flags & STYLE_CHECKER != 0
                            && ((i + j) & 1) == 0
                        {
                            shown.ch = if unicode { '\u{2591}' } else { '.' };
                        }
                        grid.cells[j as usize * w + i as usize] = shown;
                        drawn = true;
                    }
                }
            }
            if !drawn {
                let (i, j) = (to_col(x).floor() as i64, to_row(z).floor() as i64);
                if (0..w as i64).contains(&i) && (0..h as i64).contains(&j) {
                    grid.cells[j as usize * w + i as usize] = cell;
                }
            }
        }
    }
    grid
}

fn inside(e: &EntityRecord3, p: Pose3, polygon: &[[f32; 2]], x: f32, z: f32) -> bool {
    match e.shape {
        SHAPE3_SPHERE => {
            let r = e.size[0];
            (x - p.pos[0]).powi(2) + (z - p.pos[2]).powi(2) <= r * r
        }
        SHAPE3_CAPSULE => {
            let axis = rotate(p.rot, [0.0, e.size[1], 0.0]);
            let a = [p.pos[0] - axis[0], p.pos[2] - axis[2]];
            let b = [p.pos[0] + axis[0], p.pos[2] + axis[2]];
            let d = [b[0] - a[0], b[1] - a[1]];
            let len2 = d[0] * d[0] + d[1] * d[1];
            let t = if len2 > 1e-12 {
                (((x - a[0]) * d[0] + (z - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dx = x - (a[0] + d[0] * t);
            let dz = z - (a[1] + d[1] * t);
            dx * dx + dz * dz <= e.size[0] * e.size[0]
        }
        SHAPE3_BOX | SHAPE3_PLANE => inside_polygon(polygon, [x - p.pos[0], z - p.pos[2]]),
        _ => false,
    }
}

fn inside_polygon(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross.abs() <= 1e-5 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if sign * cross < 0.0 {
            return false;
        }
    }
    sign != 0.0
}

/// Projected half extents and the polygon footprint (empty for round shapes).
fn footprint(e: &EntityRecord3, p: Pose3) -> ([f32; 2], Vec<[f32; 2]>) {
    match e.shape {
        SHAPE3_SPHERE => ([e.size[0]; 2], Vec::new()),
        SHAPE3_CAPSULE => {
            let axis = rotate(p.rot, [0.0, e.size[1], 0.0]);
            let radius = e.size[0];
            ([axis[0].abs() + radius, axis[2].abs() + radius], Vec::new())
        }
        SHAPE3_BOX => polygon_footprint(p, e.size),
        SHAPE3_PLANE => polygon_footprint(p, [e.size[0], 0.0, e.size[2]]),
        _ => ([0.0; 2], Vec::new()),
    }
}

fn polygon_footprint(p: Pose3, half: [f32; 3]) -> ([f32; 2], Vec<[f32; 2]>) {
    let mut points = Vec::with_capacity(8);
    for mask in 0..8 {
        if half[1] == 0.0 && mask & 2 != 0 {
            continue;
        }
        let local = [
            if mask & 1 == 0 { -half[0] } else { half[0] },
            if mask & 2 == 0 { -half[1] } else { half[1] },
            if mask & 4 == 0 { -half[2] } else { half[2] },
        ];
        let world = rotate(p.rot, local);
        points.push([world[0], world[2]]);
    }
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    points.dedup_by(|a, b| (a[0] - b[0]).abs() <= 1e-6 && (a[1] - b[1]).abs() <= 1e-6);
    let mut hull = Vec::with_capacity(points.len() * 2);
    for point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 1e-6 {
            hull.pop();
        }
        hull.push(*point);
    }
    let lower = hull.len();
    for point in points.iter().rev().skip(1) {
        while hull.len() > lower
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 1e-6
        {
            hull.pop();
        }
        hull.push(*point);
    }
    if hull.len() > 1 {
        hull.pop();
    }
    let extent = [
        hull.iter().map(|p| p[0].abs()).fold(0.0, f32::max),
        hull.iter().map(|p| p[1].abs()).fold(0.0, f32::max),
    ];
    (extent, hull)
}

fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn glyph(schema: &ViewSchema, e: &EntityRecord3, unicode: bool) -> char {
    match schema.kind_name(e.kind) {
        "static" | "wall" | "ground" => '#',
        _ => match e.shape {
            SHAPE3_SPHERE => 'o',
            SHAPE3_CAPSULE => '=',
            SHAPE3_PLANE => {
                if unicode {
                    '\u{2592}'
                } else {
                    '#'
                }
            }
            _ if unicode => '\u{25A0}',
            _ => 'B',
        },
    }
}

fn normalized(q: [f32; 4]) -> [f32; 4] {
    let len2 = dot4(q, q);
    if !len2.is_finite() || len2 < 1e-12 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inv = len2.sqrt().recip();
    q.map(|v| v * inv)
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// Rotates a vector by quaternion `[x, y, z, w]`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = normalized(q);
    let u = [x, y, z];
    let uv = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let uuv = [
        u[1] * uv[2] - u[2] * uv[1],
        u[2] * uv[0] - u[0] * uv[2],
        u[0] * uv[1] - u[1] * uv[0],
    ];
    std::array::from_fn(|i| v[i] + 2.0 * (w * uv[i] + uuv[i]))
}
