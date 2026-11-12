//! The Forest Valley world: a mountain stream that steps down through pools
//! and cascades, forested slopes, boulders and ground cover. Everything comes
//! from one seed. This is view data only; no simulation reads it.

use crate::math::{fbm2, hash2, lerp, noise2, normalize, ridged2, smoothstep, Rng, V3};

/// Half of the side of the detailed square, in meters.
pub const HALF: f32 = 512.0;
/// Vertices per side of the detailed height grid.
pub const GRID: usize = 513;
/// Meters between detailed height samples.
pub const CELL: f32 = 2.0;
/// Half of the side of the coarse backdrop square, in meters.
pub const FAR_HALF: f32 = 3072.0;
/// Vertices per side of the backdrop grid.
pub const FAR_GRID: usize = 257;
/// Side of one ground-cover chunk, in meters.
pub const CHUNK: f32 = 32.0;

const STEP_LENGTH: f32 = 72.0;
const STEP_COUNT: usize = 18;
const CASCADE_START: f32 = 0.84;

/// One placed object. `variant` selects one of the meshes of its family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: V3,
    pub scale: f32,
    pub yaw: f32,
    pub tint: V3,
    pub variant: u8,
}

/// Ground cover of one square chunk, as ranges into `World::cover`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverChunk {
    pub center: V3,
    pub first: u32,
    /// Instances per cover variant, stored back to back from `first`.
    pub counts: [u32; COVER_VARIANTS],
}

pub const CONIFER_VARIANTS: usize = 3;
pub const BROADLEAF_VARIANTS: usize = 3;
pub const ROCK_VARIANTS: usize = 3;
/// Grass tuft, flowering tuft, fern.
pub const COVER_VARIANTS: usize = 3;

/// A sample of the stream at one downstream coordinate `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stream {
    /// World `x` of the centerline.
    pub center: f32,
    /// Water surface height.
    pub level: f32,
    pub half_width: f32,
    /// 0 in a pool, 1 in the middle of a cascade.
    pub cascade: f32,
    /// Deepest water at the centerline.
    pub depth: f32,
}

pub struct World {
    pub seed: u32,
    drops: [f32; STEP_COUNT + 1],
    /// Detailed heights, row-major over `z` then `x`.
    pub heights: Vec<f32>,
    pub conifers: Vec<Placed>,
    pub broadleaves: Vec<Placed>,
    pub rocks: Vec<Placed>,
    pub cover: Vec<Placed>,
    pub cover_chunks: Vec<CoverChunk>,
}

impl World {
    pub fn generate(seed: u32) -> Self {
        let mut drops = [0.0; STEP_COUNT + 1];
        for i in 0..STEP_COUNT {
            drops[i + 1] = drops[i] + 1.6 + 2.6 * hash2(i as i32, 7, seed);
        }
        let mut world = Self {
            seed,
            drops,
            heights: Vec::new(),
            conifers: Vec::new(),
            broadleaves: Vec::new(),
            rocks: Vec::new(),
            cover: Vec::new(),
            cover_chunks: Vec::new(),
        };
        let mut heights = Vec::with_capacity(GRID * GRID);
        for j in 0..GRID {
            for i in 0..GRID {
                heights.push(world.ground(grid_coordinate(i), grid_coordinate(j)));
            }
        }
        world.heights = heights;
        world.place_trees();
        world.place_rocks();
        world.place_cover();
        world
    }

    /// The stream at downstream coordinate `z`. Defined for every `z`.
    pub fn stream(&self, z: f32) -> Stream {
        let seed = self.seed;
        let center = 62.0 * (z * 0.0061 + 0.7).sin() + 26.0 * (z * 0.0173 + 2.1).sin() + 8.0 * (z * 0.041).sin();
        let u = ((z + HALF) / STEP_LENGTH).clamp(0.0, STEP_COUNT as f32 - 0.001);
        let step = u.floor() as usize;
        let f = u - step as f32;
        let fall = smoothstep(CASCADE_START, 1.0, f);
        let level = 64.0 - lerp(self.drops[step], self.drops[step + 1], fall) - 0.25 * u;
        // Whitewater starts a little before the drop and runs out into the next pool.
        let cascade = (smoothstep(CASCADE_START - 0.05, CASCADE_START + 0.05, f) + (1.0 - smoothstep(0.0, 0.1, f))).min(1.0);
        let wide = 7.0 + 3.0 * noise2(z * 0.03, 3.0, seed);
        Stream {
            center,
            level,
            half_width: lerp(wide, 3.6, cascade * 0.8),
            cascade,
            depth: lerp(1.25, 0.4, cascade),
        }
    }

    /// Analytic ground height. Defined for every `x` and `z`.
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        let seed = self.seed;
        let s = self.stream(z);
        let across = (x - s.center).abs();
        if across < s.half_width {
            let t = across / s.half_width;
            let bed = s.level - s.depth * (1.0 - t * t).powf(0.8);
            return bed + 0.12 * (noise2(x * 0.9, z * 0.9, seed ^ 0x51) - 0.5);
        }
        let e = across - s.half_width;
        // Gravel bank, then a bench of meadow, then the valley wall.
        let wall_start = lerp(10.0, 46.0, noise2(z * 0.007, 11.0, seed));
        let ridge = 0.55 + 0.9 * ridged2(x * 0.0035, z * 0.0035, 4, seed ^ 0x77);
        let wall = 150.0 * ridge * smoothstep(wall_start, wall_start + 300.0, e).powf(1.25);
        let bench = 0.07 * e.min(60.0) + 5.0 * (fbm2(x * 0.018, z * 0.018, 4, seed ^ 0x13) - 0.4) * smoothstep(3.0, 40.0, e);
        let detail = (fbm2(x * 0.11, z * 0.11, 3, seed ^ 0x29) - 0.5) * 1.1 * smoothstep(0.5, 8.0, e);
        let far = 190.0 * ridged2(x * 0.0011, z * 0.0011, 5, seed ^ 0x99) * smoothstep(240.0, 1200.0, e);
        s.level + bench + wall + detail + far
    }

    /// Bilinear height inside the detailed square; analytic outside it.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let (gx, gz) = ((x + HALF) / CELL, (z + HALF) / CELL);
        let limit = (GRID - 1) as f32;
        if !(0.0..limit).contains(&gx) || !(0.0..limit).contains(&gz) {
            return self.ground(x, z);
        }
        let (i, j) = (gx as usize, gz as usize);
        let (tx, tz) = (gx - i as f32, gz - j as f32);
        let at = |i: usize, j: usize| self.heights[j * GRID + i];
        lerp(lerp(at(i, j), at(i + 1, j), tx), lerp(at(i, j + 1), at(i + 1, j + 1), tx), tz)
    }

    pub fn normal(&self, x: f32, z: f32) -> V3 {
        let d = 1.5;
        normalize([
            self.height(x - d, z) - self.height(x + d, z),
            2.0 * d,
            self.height(x, z - d) - self.height(x, z + d),
        ])
    }

    /// Water surface minus ground. Positive under water. Far from the stream
    /// the value is a large negative number.
    pub fn water_depth(&self, x: f32, z: f32, ground: f32) -> f32 {
        let s = self.stream(z);
        if (x - s.center).abs() < s.half_width + 6.0 {
            s.level - ground
        } else {
            -100.0
        }
    }

    /// Meters from the water edge; zero inside the stream.
    pub fn bank_distance(&self, x: f32, z: f32) -> f32 {
        let s = self.stream(z);
        ((x - s.center).abs() - s.half_width).max(0.0)
    }

    /// Tree cover in `[0, 1]` at a point. Low in clearings, on cliffs and next
    /// to the water.
    pub fn forest(&self, x: f32, z: f32) -> f32 {
        let bank = self.bank_distance(x, z);
        let patches = fbm2(x * 0.012 + 40.0, z * 0.012, 3, self.seed ^ 0x3d);
        let cover = smoothstep(0.3, 0.5, patches + 0.25 * smoothstep(30.0, 110.0, bank));
        cover * smoothstep(4.0, 16.0, bank)
    }

    fn place_trees(&mut self) {
        let mut rng = Rng::new((u64::from(self.seed) << 8) | 1);
        let spacing = 5.5;
        let cells = (2.0 * HALF / spacing) as i32;
        for cj in 0..cells {
            for ci in 0..cells {
                let x = -HALF + (ci as f32 + rng.unit()) * spacing;
                let z = -HALF + (cj as f32 + rng.unit()) * spacing;
                let (pick, kind, size, yaw, hue, shade) = (rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit());
                if x.abs() > HALF - 4.0 || z.abs() > HALF - 4.0 {
                    continue;
                }
                let normal = self.normal(x, z);
                let forest = self.forest(x, z);
                // Sparse trees still grow in the meadow; cliffs stay bare.
                if normal[1] < 0.72 || pick > 0.04 + 0.9 * forest || self.bank_distance(x, z) < 3.0 {
                    continue;
                }
                let y = self.height(x, z);
                let bank = self.bank_distance(x, z);
                let above = y - self.stream(z).level;
                // Broadleaf trees keep to the valley floor, conifers to the slopes.
                let conifer_share = 0.25 + 0.65 * smoothstep(8.0, 70.0, above);
                let placed = Placed {
                    position: [x, y - 0.25, z],
                    scale: 0.75 + 0.6 * size * size + 0.15 * smoothstep(10.0, 60.0, bank),
                    yaw: yaw * std::f32::consts::TAU,
                    tint: [hue, 0.82 + 0.3 * shade, 0.0],
                    variant: 0,
                };
                if kind < conifer_share {
                    self.conifers.push(Placed { variant: (rng.next_u32() % CONIFER_VARIANTS as u32) as u8, ..placed });
                } else {
                    self.broadleaves.push(Placed { variant: (rng.next_u32() % BROADLEAF_VARIANTS as u32) as u8, ..placed });
                }
            }
        }
    }

    fn place_rocks(&mut self) {
        let mut rng = Rng::new((u64::from(self.seed) << 8) | 2);
        // Boulders in and beside the stream, crowded at the cascades.
        let mut z = -HALF + 6.0;
        while z < HALF - 6.0 {
            let s = self.stream(z);
            let count = 1 + (s.cascade * 5.0) as u32;
            for _ in 0..count {
                let side = rng.range(-1.0, 1.0);
                let reach = s.half_width + 5.0;
                let x = s.center + side * reach;
                let zz = z + rng.range(-1.5, 1.5);
                let in_water = side.abs() * reach < s.half_width;
                let big = rng.unit();
                let scale = if in_water { 0.35 + 1.0 * big * big + 0.5 * s.cascade } else { 0.4 + 1.9 * big * big * big };
                let y = self.height(x, zz);
                self.rocks.push(Placed {
                    position: [x, y - 0.25 * scale, zz],
                    scale,
                    yaw: rng.range(0.0, std::f32::consts::TAU),
                    tint: [rng.unit(), 0.8 + 0.35 * rng.unit(), if in_water { 1.0 } else { 0.0 }],
                    variant: (rng.next_u32() % ROCK_VARIANTS as u32) as u8,
                });
            }
            z += lerp(3.2, 1.3, s.cascade);
        }
        // Outcrops on the slopes.
        for _ in 0..2600 {
            let x = rng.range(-HALF + 8.0, HALF - 8.0);
            let z = rng.range(-HALF + 8.0, HALF - 8.0);
            let big = rng.unit();
            let (yaw, hue, shade, variant) = (rng.unit(), rng.unit(), rng.unit(), rng.next_u32());
            let normal = self.normal(x, z);
            if self.bank_distance(x, z) < 3.0 || (normal[1] > 0.93 && big < 0.6) {
                continue;
            }
            let scale = 0.5 + 2.6 * big * big;
            self.rocks.push(Placed {
                position: [x, self.height(x, z) - 0.3 * scale, z],
                scale,
                yaw: yaw * std::f32::consts::TAU,
                tint: [hue, 0.8 + 0.35 * shade, 0.0],
                variant: (variant % ROCK_VARIANTS as u32) as u8,
            });
        }
    }

    fn place_cover(&mut self) {
        let mut rng = Rng::new((u64::from(self.seed) << 8) | 3);
        let chunks = (2.0 * HALF / CHUNK) as i32;
        for cj in 0..chunks {
            for ci in 0..chunks {
                let (x0, z0) = (-HALF + ci as f32 * CHUNK, -HALF + cj as f32 * CHUNK);
                let center_x = x0 + CHUNK * 0.5;
                let center_z = z0 + CHUNK * 0.5;
                // Ground cover is only worth its triangles near the stream, where the camera walks.
                if self.bank_distance(center_x, center_z) > 150.0 {
                    continue;
                }
                let mut lists: [Vec<Placed>; COVER_VARIANTS] = Default::default();
                let per_side = 44;
                let pitch = CHUNK / per_side as f32;
                for j in 0..per_side {
                    for i in 0..per_side {
                        let x = x0 + (i as f32 + rng.unit()) * pitch;
                        let z = z0 + (j as f32 + rng.unit()) * pitch;
                        let (pick, kind, size, yaw, hue, shade) = (rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit(), rng.unit());
                        let y = self.height(x, z);
                        let depth = self.water_depth(x, z, y);
                        let bank = self.bank_distance(x, z);
                        if depth > -0.12 || bank > 130.0 || self.normal(x, z)[1] < 0.8 {
                            continue;
                        }
                        let forest = self.forest(x, z);
                        let gravel = 1.0 - smoothstep(0.6, 3.5, bank);
                        let meadow = noise2(x * 0.05, z * 0.05, self.seed ^ 0x5f);
                        // Thick in the open, thin on gravel and under the trees.
                        let density = (1.0 - 0.75 * forest) * (1.0 - 0.9 * gravel) * (0.45 + 0.75 * meadow) * (1.0 - 0.5 * smoothstep(60.0, 130.0, bank));
                        if pick > density {
                            continue;
                        }
                        let variant = if kind < 0.1 + 0.5 * forest {
                            2
                        } else if kind > 0.93 - 0.1 * meadow {
                            1
                        } else {
                            0
                        };
                        lists[variant].push(Placed {
                            position: [x, y - 0.03, z],
                            scale: 0.7 + 0.7 * size,
                            yaw: yaw * std::f32::consts::TAU,
                            tint: [hue, 0.8 + 0.4 * shade, 0.0],
                            variant: variant as u8,
                        });
                    }
                }
                let total: usize = lists.iter().map(Vec::len).sum();
                if total == 0 {
                    continue;
                }
                let first = self.cover.len() as u32;
                let counts = std::array::from_fn(|v| lists[v].len() as u32);
                for list in lists {
                    self.cover.extend(list);
                }
                self.cover_chunks.push(CoverChunk {
                    center: [center_x, self.height(center_x, center_z), center_z],
                    first,
                    counts,
                });
            }
        }
    }
}

/// World coordinate of detailed grid index `i`.
pub fn grid_coordinate(i: usize) -> f32 {
    -HALF + i as f32 * CELL
}

/// World coordinate of backdrop grid index `i`.
pub fn far_coordinate(i: usize) -> f32 {
    -FAR_HALF + i as f32 * (2.0 * FAR_HALF / (FAR_GRID - 1) as f32)
}
