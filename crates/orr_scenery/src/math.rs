//! Small float math for the outdoor view: vectors, column-major matrices,
//! integer-hash noise and a seeded generator. Results depend only on the inputs.

pub type V3 = [f32; 3];
/// Column-major: `m[column][row]`, the layout WGSL `mat4x4<f32>` reads.
pub type Mat4 = [[f32; 4]; 4];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l > 1e-12 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub fn lerp3(a: V3, b: V3, t: f32) -> V3 {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [[0.0; 4]; 4];
    for (c, column) in o.iter_mut().enumerate() {
        for (r, value) in column.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    o
}

/// Right-handed view matrix looking from `eye` to `target`.
pub fn look_at(eye: V3, target: V3, up: V3) -> Mat4 {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Right-handed perspective with depth 0 (near) to 1 (far).
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

/// Right-handed orthographic box with depth 0 (near) to 1 (far).
pub fn ortho(half_width: f32, half_height: f32, near: f32, far: f32) -> Mat4 {
    [
        [1.0 / half_width, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half_height, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [0.0, 0.0, near / (near - far), 1.0],
    ]
}

pub fn transform(m: &Mat4, p: V3) -> [f32; 4] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

/// Six inward-facing planes `(nx, ny, nz, d)` of a view-projection matrix.
pub fn frustum_planes(m: &Mat4) -> [[f32; 4]; 6] {
    let row = |r: usize| [m[0][r], m[1][r], m[2][r], m[3][r]];
    let (x, y, z, w) = (row(0), row(1), row(2), row(3));
    let combine = |a: [f32; 4], b: [f32; 4], sign: f32| -> [f32; 4] {
        let p: [f32; 4] = std::array::from_fn(|i| a[i] + sign * b[i]);
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-12);
        p.map(|v| v / l)
    };
    [combine(w, x, 1.0), combine(w, x, -1.0), combine(w, y, 1.0), combine(w, y, -1.0), combine(z, [0.0; 4], 1.0), combine(w, z, -1.0)]
}

pub fn sphere_visible(planes: &[[f32; 4]; 6], center: V3, radius: f32) -> bool {
    planes.iter().all(|p| p[0] * center[0] + p[1] * center[1] + p[2] * center[2] + p[3] > -radius)
}

fn mix_bits(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^ (v >> 16)
}

/// Uniform value in `[0, 1)` for an integer lattice point.
pub fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let h = mix_bits((x as u32).wrapping_mul(0x9e37_79b1) ^ mix_bits((y as u32).wrapping_mul(0x85eb_ca6b) ^ seed));
    (h >> 8) as f32 / 16_777_216.0
}

pub fn hash3(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    hash2(x, y, mix_bits((z as u32).wrapping_mul(0xc2b2_ae35) ^ seed))
}

/// Smooth value noise in `[0, 1]`.
pub fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (xi, yi) = (xf as i32, yf as i32);
    let (tx, ty) = (x - xf, y - yf);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let a = lerp(hash2(xi, yi, seed), hash2(xi + 1, yi, seed), sx);
    let b = lerp(hash2(xi, yi + 1, seed), hash2(xi + 1, yi + 1, seed), sx);
    lerp(a, b, sy)
}

pub fn noise3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (xf as i32, yf as i32, zf as i32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy, sz) = (s(x - xf), s(y - yf), s(z - zf));
    let plane = |k: i32| {
        let a = lerp(hash3(xi, yi, zi + k, seed), hash3(xi + 1, yi, zi + k, seed), sx);
        let b = lerp(hash3(xi, yi + 1, zi + k, seed), hash3(xi + 1, yi + 1, zi + k, seed), sx);
        lerp(a, b, sy)
    };
    lerp(plane(0), plane(1), sz)
}

/// Fractal noise in `[0, 1]`.
pub fn fbm2(mut x: f32, mut y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut total) = (0.0, 0.5, 0.0);
    for octave in 0..octaves {
        sum += noise2(x, y, seed.wrapping_add(octave * 131)) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        // Rotate each octave so lattice lines do not stack.
        (x, y) = (x * 1.6 - y * 1.2, x * 1.2 + y * 1.6);
    }
    sum / total
}

/// Ridged fractal noise in `[0, 1]`: sharp crests, smooth valleys.
pub fn ridged2(mut x: f32, mut y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut total) = (0.0, 0.5, 0.0);
    for octave in 0..octaves {
        let n = 1.0 - (noise2(x, y, seed.wrapping_add(octave * 197)) * 2.0 - 1.0).abs();
        sum += n * n * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        (x, y) = (x * 1.6 - y * 1.2, x * 1.2 + y * 1.6);
    }
    sum / total
}

/// Seeded sequence (SplitMix64). The same seed gives the same values.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 32) as u32
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
}
