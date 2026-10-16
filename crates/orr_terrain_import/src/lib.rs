//! Explicit, bounded unsigned RAW16 -> canonical terrain authoring.
//! Conversion is integer-only. This optional tool does not install packages or
//! change simulation, editor, renderer, physics or navigation behavior.
#![deny(clippy::float_arithmetic)]
use orr_fp::FP;
use orr_terrain::{Terrain, MAX_ASSET_ID_BYTES, MAX_SIDE};
use std::fmt;

pub mod publish;

pub const MAX_RAW_BYTES: usize = 2 * (MAX_SIDE * MAX_SIDE) as usize;
const DISPLAY_LIMIT: u64 = 1_000_000 * 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// Parameters are already Q48.16 values. CLI decimal parameters use FP::parse
/// once (nearest, ties away from zero); sample conversion below uses floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub asset_id: String,
    pub width: u32,
    pub depth: u32,
    pub endian: Endian,
    pub height_offset: FP,
    pub height_scale: FP,
    pub origin: [FP; 2],
    pub spacing: FP,
    pub flip_z: bool,
}

// Scaling by 2^-16 changes the exponent, not the significant binary digits.
// Use integer exactness rather than a lossy i64 -> f64 comparison.
fn exact_f32(value: FP) -> bool {
    let raw = value.raw().unsigned_abs();
    let significant = u64::BITS - raw.leading_zeros();
    significant <= 24 || raw.trailing_zeros() >= significant - 24
}

impl Recipe {
    /// Admission before allocating heights or staging any output. In addition to
    /// core validity, spacing and all X/Z positions must satisfy editor display.
    pub fn validate(&self) -> Result<usize> {
        if !(2..=MAX_SIDE).contains(&self.width) || !(2..=MAX_SIDE).contains(&self.depth) {
            return Err(Error(format!("width and depth must be 2..={MAX_SIDE}")));
        }
        let id = &self.asset_id;
        if id.is_empty()
            || id.len() > MAX_ASSET_ID_BYTES
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-/ .".contains(&c))
            || id.starts_with('/')
            || id.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(Error("invalid terrain asset identity".into()));
        }
        if self.height_scale.raw() < 0 {
            return Err(Error("height scale must be nonnegative".into()));
        }
        self.height_offset
            .raw()
            .checked_add(self.height_scale.raw())
            .ok_or_else(|| Error("height offset + full-range scale overflows Q48.16".into()))?;
        if self.spacing.raw() <= 0 {
            return Err(Error("spacing must be positive".into()));
        }
        if !exact_f32(self.spacing) {
            return Err(Error("spacing must be exactly representable in f32 for editor Open; choose an exact binary step".into()));
        }
        for (origin, side) in self.origin.iter().zip([self.width, self.depth]) {
            for index in 0..side {
                let raw = i128::from(origin.raw())
                    .checked_add(
                        i128::from(index)
                            .checked_mul(i128::from(self.spacing.raw()))
                            .ok_or_else(|| Error("terrain coordinate overflow".into()))?,
                    )
                    .and_then(|v| i64::try_from(v).ok())
                    .ok_or_else(|| Error("terrain coordinate overflow".into()))?;
                admit_coordinate(FP::from_raw(raw))?;
            }
        }
        Ok(2 * (self.width * self.depth) as usize)
    }
}

fn admit_coordinate(value: FP) -> Result<()> {
    if value.raw().unsigned_abs() > DISPLAY_LIMIT || !exact_f32(value) {
        return Err(Error("terrain display requires exact f32 vertex coordinates within +/-1,000,000; adjust offset, scale, origin or spacing (no implicit quantization)".into()));
    }
    Ok(())
}

/// X is fastest, Y is height, rows advance +Z. flip_z reverses source rows only.
/// height_raw = offset_raw + floor(sample_u16 * scale_raw / 65535).
/// Scale is nonnegative, so integer division of the nonnegative product is floor.
pub fn import_raw16(bytes: &[u8], recipe: &Recipe) -> Result<Terrain> {
    let expected = recipe.validate()?;
    if bytes.len() != expected {
        return Err(Error(format!(
            "RAW16 byte length must be exactly {expected}, got {} (no header, padding or trailer)",
            bytes.len()
        )));
    }
    let mut heights = Vec::with_capacity((recipe.width * recipe.depth) as usize);
    for z in 0..recipe.depth {
        let source_z = if recipe.flip_z {
            recipe.depth - 1 - z
        } else {
            z
        };
        for x in 0..recipe.width {
            let start = 2 * (source_z * recipe.width + x) as usize;
            let pair = [bytes[start], bytes[start + 1]];
            let sample = match recipe.endian {
                Endian::Little => u16::from_le_bytes(pair),
                Endian::Big => u16::from_be_bytes(pair),
            };
            let scaled = i128::from(sample)
                .checked_mul(i128::from(recipe.height_scale.raw()))
                .ok_or_else(|| Error("height multiplication overflow".into()))?
                / 65535;
            let raw = i128::from(recipe.height_offset.raw())
                .checked_add(scaled)
                .and_then(|v| i64::try_from(v).ok())
                .ok_or_else(|| Error("height conversion overflow".into()))?;
            let height = FP::from_raw(raw);
            admit_coordinate(height)?;
            heights.push(height);
        }
    }
    let terrain = Terrain::new(
        recipe.asset_id.clone(),
        recipe.width,
        recipe.depth,
        recipe.origin,
        recipe.spacing,
        heights,
        vec![false; ((recipe.width - 1) * (recipe.depth - 1)) as usize],
    )
    .map_err(|e| Error(format!("terrain validation: {e}")))?;
    let bytes = terrain.cook();
    let loaded = Terrain::load(&bytes).map_err(|e| Error(format!("canonical reload: {e}")))?;
    if loaded != terrain || loaded.cook() != bytes || loaded.revision() != terrain.revision() {
        return Err(Error("canonical terrain roundtrip mismatch".into()));
    }
    orr_terrain_view::to_static_model(&terrain, &[])
        .map_err(|e| Error(format!("terrain display admission: {e}")))?
        .ok_or_else(|| Error("RAW16 terrain unexpectedly has no triangles".into()))?;
    Ok(terrain)
}
