//! Drawing a [`ViewFrame`] into a character grid: a pure function of the frame, the camera and
//! `alpha`. The terminal UI colors the cells; `--dump` writes their characters.

use orr_viewstream::{EntityRecord, ViewFrame, MODE_NONE, SHAPE_CAPSULE, SHAPE_CIRCLE};

use crate::schema::ViewSchema;

/// One terminal cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub rgba: [u8; 4],
}

const EMPTY: Cell = Cell { ch: ' ', rgba: [0; 4] };

/// A `w` x `h` grid of cells, row 0 at the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn new(w: usize, h: usize) -> Grid {
        Grid { w, h, cells: vec![EMPTY; w * h] }
    }

    pub fn row(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.w..(y + 1) * self.w]
    }

    /// The characters, one line per row, trailing blanks trimmed.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity((self.w + 1) * self.h);
        for y in 0..self.h {
            let line: String = self.row(y).iter().map(|c| c.ch).collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

/// The part of the world shown: a box in world units (y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// Radius of a circle that contains the entity's shape.
fn reach(e: &EntityRecord) -> f32 {
    match e.shape {
        SHAPE_CIRCLE => e.size,
        SHAPE_CAPSULE => e.size + e.half_y,
        _ => e.size.hypot(if e.half_y > 0.0 { e.half_y } else { e.size }),
    }
}

/// Half extent along x and y of the entity's shape at rotation `rot` (its axis-aligned box).
fn extent(e: &EntityRecord, rot: f32) -> [f32; 2] {
    let (s, c) = (rot.sin().abs(), rot.cos().abs());
    match e.shape {
        SHAPE_CIRCLE => [e.size, e.size],
        SHAPE_CAPSULE => [c * e.size + e.half_y, s * e.size + e.half_y],
        _ => {
            let hy = if e.half_y > 0.0 { e.half_y } else { e.size };
            [c * e.size + s * hy, s * e.size + c * hy]
        }
    }
}

impl Camera {
    /// Fits the scene: the box around the entities that do not move (walls), or around all of
    /// them if nothing is static, with a small margin.
    pub fn fit(frame: &ViewFrame) -> Camera {
        let any_static = frame.entities.iter().any(|e| e.mode == MODE_NONE);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for e in frame.entities.iter().filter(|e| !any_static || e.mode == MODE_NONE) {
            let r = extent(e, e.cur[2]);
            for k in 0..2 {
                lo[k] = lo[k].min(e.cur[k] - r[k]);
                hi[k] = hi[k].max(e.cur[k] + r[k]);
            }
        }
        if lo[0] > hi[0] {
            return Camera { min: [-1.0, -1.0], max: [1.0, 1.0] };
        }
        for k in 0..2 {
            let margin = ((hi[k] - lo[k]) * 0.03).max(0.05);
            lo[k] -= margin;
            hi[k] += margin;
        }
        Camera { min: lo, max: hi }
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut d = (b - a) % tau;
    if d > std::f32::consts::PI {
        d -= tau;
    } else if d < -std::f32::consts::PI {
        d += tau;
    }
    a + d * t
}

/// `[x, y, rotation]` of an entity at `alpha` (0 = previous tick, 1 = current tick).
pub fn pose(e: &EntityRecord, alpha: f32) -> [f32; 3] {
    if e.mode == MODE_NONE || alpha >= 1.0 {
        return e.cur;
    }
    [e.prev[0] + (e.cur[0] - e.prev[0]) * alpha, e.prev[1] + (e.cur[1] - e.prev[1]) * alpha, lerp_angle(e.prev[2], e.cur[2], alpha)]
}

/// Is the world point inside the shape at `pose`?
fn inside(e: &EntityRecord, p: [f32; 3], x: f32, y: f32) -> bool {
    let (dx, dy) = (x - p[0], y - p[1]);
    match e.shape {
        SHAPE_CIRCLE => dx * dx + dy * dy <= e.size * e.size,
        shape => {
            let (s, c) = p[2].sin_cos();
            let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
            if shape == SHAPE_CAPSULE {
                let cx = lx.clamp(-e.size, e.size);
                (lx - cx) * (lx - cx) + ly * ly <= e.half_y * e.half_y
            } else {
                let hy = if e.half_y > 0.0 { e.half_y } else { e.size };
                lx.abs() <= e.size && ly.abs() <= hy
            }
        }
    }
}

/// The character of an entity, from its kind name (the schema) and shape.
fn glyph(schema: &ViewSchema, e: &EntityRecord, unicode: bool) -> char {
    match schema.kind_name(e.kind) {
        "static" | "wall" => '#',
        "paddle" => '=',
        _ => match e.shape {
            SHAPE_CIRCLE => 'o',
            SHAPE_CAPSULE => '=',
            _ if unicode => '\u{25A0}',
            _ => 'B',
        },
    }
}

/// Draws the frame at `alpha` into a `w` x `h` grid. Static entities first, then the rest in
/// record order. Terminal cells are about twice as tall as wide; the scale accounts for it.
pub fn render(frame: &ViewFrame, schema: &ViewSchema, cam: &Camera, alpha: f32, w: usize, h: usize, unicode: bool) -> Grid {
    let mut grid = Grid::new(w, h);
    if w == 0 || h == 0 {
        return grid;
    }
    let (ww, wh) = ((cam.max[0] - cam.min[0]).max(1e-3), (cam.max[1] - cam.min[1]).max(1e-3));
    let sx = (w as f32 / ww).min(h as f32 / wh * 2.0); // cells per world unit, horizontally
    let sy = sx / 2.0;
    let (cx, cy) = ((cam.min[0] + cam.max[0]) / 2.0, (cam.min[1] + cam.max[1]) / 2.0);
    let (half_w, half_h) = (w as f32 / 2.0, h as f32 / 2.0);
    for pass_static in [true, false] {
        for e in frame.entities.iter().filter(|e| (e.mode == MODE_NONE) == pass_static) {
            if e.rgba[3] == 0 {
                continue;
            }
            let p = pose(e, alpha);
            let cell = Cell { ch: glyph(schema, e, unicode), rgba: e.rgba };
            let r = reach(e);
            let to_col = |wx: f32| (wx - cx) * sx + half_w;
            let to_row = |wy: f32| (cy - wy) * sy + half_h;
            let (i0, i1) = (to_col(p[0] - r).floor() as i64, to_col(p[0] + r).ceil() as i64);
            let (j0, j1) = (to_row(p[1] + r).floor() as i64, to_row(p[1] - r).ceil() as i64);
            let mut drawn = false;
            for j in j0.max(0)..j1.min(h as i64) {
                for i in i0.max(0)..i1.min(w as i64) {
                    let wx = cx + (i as f32 + 0.5 - half_w) / sx;
                    let wy = cy - (j as f32 + 0.5 - half_h) / sy;
                    if inside(e, p, wx, wy) {
                        grid.cells[j as usize * w + i as usize] = cell;
                        drawn = true;
                    }
                }
            }
            if !drawn {
                // Smaller than a cell: still show where it is.
                let (i, j) = (to_col(p[0]).floor() as i64, to_row(p[1]).floor() as i64);
                if (0..w as i64).contains(&i) && (0..h as i64).contains(&j) {
                    grid.cells[j as usize * w + i as usize] = cell;
                }
            }
        }
    }
    grid
}
