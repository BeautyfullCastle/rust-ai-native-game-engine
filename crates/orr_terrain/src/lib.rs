//! Optional GPU-free Y-up/XZ bounded heightfields. No scene, physics or editor integration.
//! Each cell uses the a-d diagonal: [a,c,d], [a,d,b], where b is +X and c is +Z.
//! Samples use that same piecewise plane, not bilinear interpolation. Interior seams
//! belong to the +X/+Z cell; outer edges belong to the final cell. A hole owns its
//! boundaries under this rule. Heights round once toward negative infinity.
#![deny(clippy::float_arithmetic)]
use orr_fp::FP;
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_SIDE: u32 = 129;
pub const MAX_ASSET_ID_BYTES: usize = 256;
pub const MAX_FILE_BYTES: usize = 8
    + 2
    + MAX_ASSET_ID_BYTES
    + 8
    + 24
    + (MAX_SIDE * MAX_SIDE) as usize * 8
    + ((MAX_SIDE - 1) * (MAX_SIDE - 1)) as usize;
pub const MAX_EDITS: usize = 4096;
pub const MAX_HISTORY: usize = 32;
const MAGIC: &[u8; 8] = b"ORRTHF\x01\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(&'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

/// Surface orientation is a deterministic fixed-point unit normal (quantized).
/// Slope is rise/run, not an angle; None means it exceeds the FP range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: FP,
    pub normal: [FP; 3],
    pub slope: Option<FP>,
}
/// Inclusive dirty cell rectangle. A changed vertex dirties every adjacent cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRegion {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

/// Validated immutable content. Identity is caller-chosen; revision is SHA-256 of canonical bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terrain {
    asset_id: String,
    width: u32,
    depth: u32,
    origin: [FP; 2],
    spacing: FP,
    heights: Vec<FP>,
    holes: Vec<bool>,
}
impl Terrain {
    pub fn new(
        asset_id: String,
        width: u32,
        depth: u32,
        origin: [FP; 2],
        spacing: FP,
        heights: Vec<FP>,
        holes: Vec<bool>,
    ) -> Result<Self, Error> {
        if asset_id.is_empty()
            || asset_id.len() > MAX_ASSET_ID_BYTES
            || !asset_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-/ .".contains(&c))
            || asset_id.starts_with('/')
            || asset_id
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(Error("invalid asset identity"));
        }
        if !(2..=MAX_SIDE).contains(&width) || !(2..=MAX_SIDE).contains(&depth) {
            return Err(Error("terrain dimension limit"));
        }
        if spacing.raw() <= 0 {
            return Err(Error("spacing must be positive"));
        }
        for (o, side) in origin.iter().zip([width, depth]) {
            let end = i128::from(o.raw()) + i128::from(side - 1) * i128::from(spacing.raw());
            if i64::try_from(end).is_err() {
                return Err(Error("terrain coordinate overflow"));
            }
        }
        if heights.len() != (width * depth) as usize
            || holes.len() != ((width - 1) * (depth - 1)) as usize
        {
            return Err(Error("height or hole count mismatch"));
        }
        Ok(Self {
            asset_id,
            width,
            depth,
            origin,
            spacing,
            heights,
            holes,
        })
    }
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn depth(&self) -> u32 {
        self.depth
    }
    pub fn origin(&self) -> [FP; 2] {
        self.origin
    }
    pub fn spacing(&self) -> FP {
        self.spacing
    }
    pub fn heights(&self) -> &[FP] {
        &self.heights
    }
    pub fn holes(&self) -> &[bool] {
        &self.holes
    }
    pub fn revision(&self) -> [u8; 32] {
        Sha256::digest(self.cook()).into()
    }
    pub fn vertex_position(&self, index: u32) -> Option<[FP; 3]> {
        let height = *self.heights.get(index as usize)?;
        let x = index % self.width;
        let z = index / self.width;
        let coordinate = |o: FP, n: u32| {
            FP::from_raw(
                (i128::from(o.raw()) + i128::from(n) * i128::from(self.spacing.raw())) as i64,
            )
        };
        Some([
            coordinate(self.origin[0], x),
            height,
            coordinate(self.origin[1], z),
        ])
    }
    /// Canonical counterclockwise (+Y) triangles; holes generate no geometry.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut result = Vec::with_capacity(self.holes.len() * 2);
        for z in 0..self.depth - 1 {
            for x in 0..self.width - 1 {
                if self.holes[(z * (self.width - 1) + x) as usize] {
                    continue;
                }
                let a = z * self.width + x;
                let b = a + 1;
                let c = a + self.width;
                let d = c + 1;
                result.extend([[a, c, d], [a, d, b]]);
            }
        }
        result
    }
    pub fn sample(&self, x: FP, z: FP) -> Option<FP> {
        let spacing = i128::from(self.spacing.raw());
        let axis = |v: FP, origin: FP, side: u32| {
            let delta = i128::from(v.raw()) - i128::from(origin.raw());
            if delta < 0 || delta > i128::from(side - 1) * spacing {
                return None;
            }
            let cell = (delta / spacing).min(i128::from(side - 2));
            Some((cell as u32, delta - cell * spacing))
        };
        let (cx, u) = axis(x, self.origin[0], self.width)?;
        let (cz, v) = axis(z, self.origin[1], self.depth)?;
        if self.holes[(cz * (self.width - 1) + cx) as usize] {
            return None;
        }
        let a = (cz * self.width + cx) as usize;
        let b = a + 1;
        let c = a + self.width as usize;
        let d = c + 1;
        // Nonnegative barycentric weights sum exactly to spacing. This bounds
        // the wide sum even for i64::MIN/MAX heights and maximum legal spacing.
        let terms = if u <= v {
            [(a, spacing - v), (c, v - u), (d, u)]
        } else {
            [(a, spacing - u), (d, v), (b, u - v)]
        };
        let numerator: i128 = terms
            .into_iter()
            .map(|(i, w)| i128::from(self.heights[i].raw()) * w)
            .sum();
        Some(FP::from_raw(numerator.div_euclid(spacing) as i64))
    }
    /// Height and the plane orientation of the same canonical owner triangle.
    /// Normals scale coefficients before integer square root to prevent overflow;
    /// precision is Q16 and near-vertical normals may quantize Y to zero.
    pub fn surface(&self, x: FP, z: FP) -> Option<Surface> {
        let height = self.sample(x, z)?;
        let spacing = i128::from(self.spacing.raw());
        let dx = i128::from(x.raw()) - i128::from(self.origin[0].raw());
        let dz = i128::from(z.raw()) - i128::from(self.origin[1].raw());
        let cx = (dx / spacing).min(i128::from(self.width - 2));
        let cz = (dz / spacing).min(i128::from(self.depth - 2));
        let a = (cz * i128::from(self.width) + cx) as usize;
        let b = a + 1;
        let c = a + self.width as usize;
        let d = c + 1;
        let h = |i: usize| i128::from(self.heights[i].raw());
        let (hx, hz) = if dx - cx * spacing <= dz - cz * spacing {
            (h(d) - h(c), h(c) - h(a))
        } else {
            (h(b) - h(a), h(d) - h(b))
        };
        let max = hx.abs().max(hz.abs()).max(spacing);
        let coefficients = [-hx, spacing, -hz].map(|v| v * (1i128 << 30) / max);
        let magnitude = isqrt(coefficients.iter().map(|v| (v * v) as u128).sum()) as i128;
        let normal = coefficients.map(|v| FP::from_raw((v * 65536 / magnitude) as i64));
        // Divide before squaring: slope's 64-bit raw answer may overflow but
        // each gradient raw fits in i128. Saturate only the optional slope result.
        let gx = hx.abs() * 65536 / spacing;
        let gz = hz.abs() * 65536 / spacing;
        let slope = if gx > i128::from(i64::MAX) || gz > i128::from(i64::MAX) {
            None
        } else {
            i64::try_from(isqrt((gx * gx + gz * gz) as u128))
                .ok()
                .map(FP::from_raw)
        };
        Some(Surface {
            height,
            normal,
            slope,
        })
    }
    /// Version 1: magic/version (8), identity length u16 + UTF-8, dimensions u32,
    /// origin X/Z and spacing i64, row-major height i64, one 0/1 byte per cell.
    /// All integers little endian; no padding, trailing bytes or alternate encodings.
    pub fn cook(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            MAX_FILE_BYTES
                .min(64 + self.asset_id.len() + self.heights.len() * 8 + self.holes.len()),
        );
        out.extend(MAGIC);
        out.extend((self.asset_id.len() as u16).to_le_bytes());
        out.extend(self.asset_id.as_bytes());
        out.extend(self.width.to_le_bytes());
        out.extend(self.depth.to_le_bytes());
        for v in [self.origin[0], self.origin[1], self.spacing] {
            out.extend(v.raw().to_le_bytes());
        }
        for h in &self.heights {
            out.extend(h.raw().to_le_bytes());
        }
        out.extend(self.holes.iter().map(|h| u8::from(*h)));
        out
    }
    pub fn load(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error("terrain byte limit"));
        }
        let mut reader = Reader(bytes);
        if reader.take(8)? != MAGIC {
            return Err(Error("unknown terrain format/version"));
        }
        let id_len = usize::from(u16::from_le_bytes(reader.array()?));
        if id_len > MAX_ASSET_ID_BYTES {
            return Err(Error("identity byte limit"));
        }
        let id = std::str::from_utf8(reader.take(id_len)?)
            .map_err(|_| Error("invalid identity UTF-8"))?
            .to_owned();
        let width = u32::from_le_bytes(reader.array()?);
        let depth = u32::from_le_bytes(reader.array()?);
        if !(2..=MAX_SIDE).contains(&width) || !(2..=MAX_SIDE).contains(&depth) {
            return Err(Error("terrain dimension limit"));
        }
        let mut next_fp = || reader.array().map(|b| FP::from_raw(i64::from_le_bytes(b)));
        let origin = [next_fp()?, next_fp()?];
        let spacing = next_fp()?;
        let count = (width * depth) as usize;
        let hole_count = ((width - 1) * (depth - 1)) as usize;
        if reader.0.len() != count * 8 + hole_count {
            return Err(Error("terrain byte count mismatch"));
        }
        let mut heights = Vec::with_capacity(count);
        for _ in 0..count {
            heights.push(FP::from_raw(i64::from_le_bytes(reader.array()?)));
        }
        let mut holes = Vec::with_capacity(hole_count);
        for &b in reader.take(hole_count)? {
            match b {
                0 => holes.push(false),
                1 => holes.push(true),
                _ => return Err(Error("noncanonical hole mask")),
            }
        }
        Self::new(id, width, depth, origin, spacing, heights, holes)
    }
}
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if n > self.0.len() {
            return Err(Error("truncated terrain"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error("truncated terrain"))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    SetHeight { x: u32, z: u32, height: FP },
    SetHole { x: u32, z: u32, hole: bool },
}
/// Bounded snapshot undo/redo. Transactions validate completely before publication.
/// Undo restores content and revision; history is intentionally not part of cooked assets.
#[derive(Clone)]
pub struct TerrainDocument {
    terrain: Terrain,
    undo: Vec<Terrain>,
    redo: Vec<Terrain>,
    dirty: Option<DirtyRegion>,
}
impl TerrainDocument {
    pub fn new(terrain: Terrain) -> Self {
        Self {
            terrain,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: None,
        }
    }
    pub fn terrain(&self) -> &Terrain {
        &self.terrain
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Most recent successful content change, not an accumulated upload queue.
    /// No-ops preserve it. Rejected transactions leave it untouched.
    pub fn dirty_region(&self) -> Option<DirtyRegion> {
        self.dirty
    }
    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }
    pub fn apply(&mut self, edits: &[Edit]) -> Result<(), Error> {
        if edits.len() > MAX_EDITS {
            return Err(Error("edit batch limit"));
        }
        let mut candidate = self.terrain.clone();
        for edit in edits {
            match *edit {
                Edit::SetHeight { x, z, height } => {
                    if x >= candidate.width || z >= candidate.depth {
                        return Err(Error("height edit outside terrain"));
                    }
                    candidate.heights[(z * candidate.width + x) as usize] = height;
                }
                Edit::SetHole { x, z, hole } => {
                    if x >= candidate.width - 1 || z >= candidate.depth - 1 {
                        return Err(Error("hole edit outside terrain"));
                    }
                    candidate.holes[(z * (candidate.width - 1) + x) as usize] = hole;
                }
            }
        }
        if candidate != self.terrain {
            let mut min = [candidate.width - 2, candidate.depth - 2];
            let mut max = [0, 0];
            for z in 0..candidate.depth - 1 {
                for x in 0..candidate.width - 1 {
                    let cell = (z * (candidate.width - 1) + x) as usize;
                    let a = (z * candidate.width + x) as usize;
                    let corners = [
                        a,
                        a + 1,
                        a + candidate.width as usize,
                        a + candidate.width as usize + 1,
                    ];
                    if candidate.holes[cell] != self.terrain.holes[cell]
                        || corners
                            .iter()
                            .any(|&i| candidate.heights[i] != self.terrain.heights[i])
                    {
                        min = [min[0].min(x), min[1].min(z)];
                        max = [max[0].max(x), max[1].max(z)];
                    }
                }
            }
            self.dirty = Some(DirtyRegion { min, max });
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.terrain, candidate));
            self.redo.clear();
        }
        Ok(())
    }
    fn mark_all_dirty(&mut self) {
        self.dirty = Some(DirtyRegion {
            min: [0, 0],
            max: [self.terrain.width - 2, self.terrain.depth - 2],
        });
    }
    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.terrain, previous));
            self.mark_all_dirty();
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.terrain, next));
            self.mark_all_dirty();
            true
        } else {
            false
        }
    }
}
