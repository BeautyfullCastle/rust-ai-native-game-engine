//! Immutable, GPU-free static models. Floating point is presentation-only.
//! The versioned `orr_static_model` format is separate from ORAM and simulation.
//! Import tools are optional; runtime loading needs no glTF parser/image codec.
#![allow(clippy::float_arithmetic)]
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

#[cfg(feature = "import")]
pub mod import;

pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_VERTICES: usize = 250_000;
pub const MAX_INDICES: usize = 750_000;
pub const MAX_PRIMITIVES: usize = 1024;
pub const MAX_NODES: usize = 1024;
pub const MAX_IMAGES: usize = 32;
pub const MAX_MATERIALS: usize = 256;
pub const MAX_IMAGE_EXTENT: u32 = 2048;
pub const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub(crate) String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
pub(crate) fn invalid(s: impl Into<String>) -> Error {
    Error(s.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Primitive {
    /// Stable within the same source node/mesh/primitive slots, independent of bytes.
    pub id: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: u32,
    /// Column-major local-to-world transform, including all ancestor transforms.
    pub transform: [[f32; 4]; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrap {
    Clamp,
    Repeat,
    Mirror,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub base_color: [f32; 4],
    pub image: u32,
    pub linear_filter: bool,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom, sRGB RGB with linear alpha. Opaque renderer ignores alpha.
    pub rgba8: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// `$source` or a normalized source-relative dependency URI.
    pub uri: String,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSource {
    pub format: String,
    pub version: u32,
    /// Caller-chosen logical source path, stable across content reimports.
    pub asset_id: String,
    pub dependencies: Vec<Dependency>,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
}

/// No mutable access: import and cooked load share all runtime invariants.
#[derive(Clone, Debug)]
pub struct StaticModel(ModelSource);
impl StaticModel {
    pub fn new(source: ModelSource) -> Result<Self, Error> {
        if source.format != "orr_static_model" || source.version != 1 {
            return Err(invalid("unsupported static model format/version"));
        }
        valid_path(&source.asset_id)?;
        if source.primitives.is_empty()
            || source.primitives.len() > MAX_PRIMITIVES
            || source.materials.is_empty()
            || source.materials.len() > MAX_MATERIALS
            || source.images.is_empty()
            || source.images.len() > MAX_IMAGES
            || source.dependencies.is_empty()
            || source.dependencies.len() > 256
        {
            return Err(invalid("model collection limit"));
        }
        let mut deps = BTreeSet::new();
        for d in &source.dependencies {
            if d.uri != "$source" {
                valid_path(&d.uri)?;
            }
            if !deps.insert(&d.uri)
                || d.sha256.len() != 64
                || !d
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(invalid("invalid/duplicate dependency digest"));
            }
        }
        if !deps.contains(&"$source".to_owned()) {
            return Err(invalid("missing source digest"));
        }
        let mut image_bytes = 0usize;
        for image in &source.images {
            if image.width == 0
                || image.height == 0
                || image.width > MAX_IMAGE_EXTENT
                || image.height > MAX_IMAGE_EXTENT
                || image.rgba8.len() != image.width as usize * image.height as usize * 4
            {
                return Err(invalid("invalid RGBA image extent/length"));
            }
            image_bytes += image.rgba8.len();
            if image_bytes > MAX_DECODED_BYTES {
                return Err(invalid("decoded image budget exceeded"));
            }
        }
        for m in &source.materials {
            if m.image as usize >= source.images.len()
                || !m
                    .base_color
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            {
                return Err(invalid("invalid material image/factor"));
            }
        }
        let mut ids = BTreeSet::new();
        let (mut vertices, mut indices) = (0usize, 0usize);
        for p in &source.primitives {
            vertices = vertices
                .checked_add(p.vertices.len())
                .ok_or_else(|| invalid("vertex count overflow"))?;
            indices = indices
                .checked_add(p.indices.len())
                .ok_or_else(|| invalid("index count overflow"))?;
            if vertices > MAX_VERTICES
                || indices > MAX_INDICES
                || p.vertices.is_empty()
                || p.indices.is_empty()
                || p.indices.len() % 3 != 0
                || p.material as usize >= source.materials.len()
                || p.id.len() > 2048
                || !p.id.starts_with(&format!("{}#node=", source.asset_id))
                || !ids.insert(&p.id)
            {
                return Err(invalid("invalid primitive ID/count/material"));
            }
            normal_matrix(p.transform)?;
            for v in &p.vertices {
                if !v
                    .position
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .all(|x| x.is_finite())
                    || v.position.iter().any(|x| x.abs() > 1.0e6)
                    || v.uv.iter().any(|x| x.abs() > 65536.0)
                    || !(0.99..=1.01).contains(&v.normal.iter().map(|x| x * x).sum::<f32>())
                {
                    return Err(invalid("nonfinite/out-of-range vertex or nonunit normal"));
                }
                for r in 0..3 {
                    let world = p.transform[0][r] * v.position[0]
                        + p.transform[1][r] * v.position[1]
                        + p.transform[2][r] * v.position[2]
                        + p.transform[3][r];
                    if !world.is_finite() || world.abs() > 1.0e9 {
                        return Err(invalid("world vertex exceeds range"));
                    }
                }
            }
            if p.indices.iter().any(|&i| i as usize >= p.vertices.len()) {
                return Err(invalid("index out of range"));
            }
        }
        Ok(Self(source))
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("cooked model exceeds byte limit"));
        }
        Self::new(serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let bytes = serde_json::to_vec(&self.0).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("cooked model exceeds byte limit"));
        }
        Ok(bytes)
    }
    pub fn source(&self) -> &ModelSource {
        &self.0
    }
}

/// Inverse transpose of the affine linear part. Negative determinants are
/// supported; the renderer reverses triangle winding for mirrored nodes.
pub fn normal_matrix(m: [[f32; 4]; 4]) -> Result<[[f32; 4]; 4], Error> {
    if !m
        .iter()
        .flatten()
        .all(|v| v.is_finite() && v.abs() <= 1.0e6)
        || m[0][3] != 0.0
        || m[1][3] != 0.0
        || m[2][3] != 0.0
        || m[3][3] != 1.0
    {
        return Err(invalid("nonfinite/non-affine/out-of-range node transform"));
    }
    let cross = |a: [f32; 4], b: [f32; 4]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let cols = [cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1])];
    let det = determinant(m);
    if !det.is_finite() || det.abs() < 1e-12 {
        return Err(invalid("singular node transform"));
    }
    let mut out = IDENTITY;
    for c in 0..3 {
        for r in 0..3 {
            out[c][r] = cols[c][r] / det;
        }
    }
    if !out
        .iter()
        .flatten()
        .all(|v| v.is_finite() && v.abs() <= 1.0e6)
    {
        return Err(invalid("normal transform overflow"));
    }
    Ok(out)
}
pub fn determinant(m: [[f32; 4]; 4]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[1][0] * (m[0][1] * m[2][2] - m[0][2] * m[2][1])
        + m[2][0] * (m[0][1] * m[1][2] - m[0][2] * m[1][1])
}
pub(crate) fn valid_path(uri: &str) -> Result<(), Error> {
    if uri.is_empty()
        || uri.len() > 1024
        || uri.contains(['\\', ':', '%', '?', '#', '\0'])
        || uri
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(invalid("expected normalized relative asset path"));
    }
    Ok(())
}
