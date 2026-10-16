//! Optional diffuse irradiance. Bounded CPU baking lives in `irradiance_bake`.
//!
//! SH9 stores RGB **irradiance** coefficients, already cosine-convolved, in the
//! positive real basis of Ramamoorthi–Hanrahan (2001), equation 3:
//! [1, y, z, x, xy, yz, 3z²−1, xz, x²−y²],
//! multiplied by [0.2820948, 0.48860252, 0.48860252, 0.48860252, 1.0925485,
//! 1.0925485, 0.31539157, 1.0925485, 0.54627424]. No cosine convolution is
//! applied while evaluating the grid; a Lambertian consumer uses E/π.
//! Coefficients are interpolated first, evaluated using a unit world normal,
//! then RGB is clamped nonnegative exactly once.
//! Reference: <https://graphics.stanford.edu/papers/envmap/envmap.pdf>.
//!
//! Nodes are x-fastest: x + nx * (y + ny * z). Coverage includes the maximum
//! face. Weight fades linearly for one cell inside the nearest boundary; it is
//! zero on every face, including the exact maximum. Outside returns no sample.
//! Consumers blend the returned irradiance with their existing diffuse ambient
//! using weight; direct sun, points, shadows, and emissive remain independent.
#![allow(clippy::float_arithmetic)]

use serde::{Deserialize, Serialize};

pub const IRRADIANCE_VERSION: u32 = 1;
pub const MAX_IRRADIANCE_BYTES: usize = 256 * 1024;
pub const MAX_PROBES: usize = 64;
pub const SH_COEFFICIENTS: usize = 9;
pub const WORLD_LIMIT: f32 = 1.0e6;
pub const MIN_SPACING: f32 = 1.0e-3;
pub const MAX_COEFFICIENT: f32 = 1.0e4;
pub const Y00: f32 = 0.282_094_8;
pub type Sh9 = [[f32; 3]; SH_COEFFICIENTS];

/// Describes the source. `Baked` is bounded static sun-only diffuse transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrradianceProvenance {
    Authored,
    Imported,
    /// Single-bounce static sun bake; see the sidecar receipt for freshness.
    Baked,
}

/// A bounded axis-aligned grid with uniform spacing along each axis.
/// Public fields support authoring; consumers must validate before any effects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrradianceGrid {
    pub version: u32,
    pub enabled: bool,
    pub provenance: IrradianceProvenance,
    pub dimensions: [u32; 3],
    pub origin: [f32; 3],
    pub spacing: [f32; 3],
    #[serde(deserialize_with = "bounded_coefficients")]
    pub coefficients: Vec<Sh9>,
}

fn bounded_coefficients<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Sh9>, D::Error> {
    struct CoefficientsVisitor;
    impl<'de> serde::de::Visitor<'de> for CoefficientsVisitor {
        type Value = Vec<Sh9>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 64 nodes with exactly nine signed RGB SH coefficients each")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut nodes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_PROBES));
            while let Some(node) = seq.next_element::<Sh9>()? {
                if nodes.len() == MAX_PROBES {
                    return Err(serde::de::Error::custom("irradiance grid exceeds 64 nodes"));
                }
                nodes.push(node);
            }
            Ok(nodes)
        }
    }
    deserializer.deserialize_seq(CoefficientsVisitor)
}

/// Blend weight is independent of irradiance; zero-weight boundary samples are
/// still inside coverage and must not cause out-of-range cell indexing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrradianceSample {
    pub irradiance: [f32; 3],
    pub weight: f32,
}

/// Shared WGSL uniform ABI: four vec4 headers and 64 × 9 RGB-padded vec4s.
/// Exactly 9,280 bytes, below WebGPU's minimum 16 KiB uniform-binding limit.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct IrradianceUniform {
    pub origin: [f32; 4],
    pub inverse_spacing: [f32; 4],
    /// xyz dimensions, w enabled (0 or 1).
    pub dimensions: [u32; 4],
    /// Actual f32 maximum world face, shared exactly with the shader.
    pub maximum: [f32; 4],
    pub coefficients: [[f32; 4]; MAX_PROBES * SH_COEFFICIENTS],
}
impl Default for IrradianceUniform {
    fn default() -> Self {
        <Self as bytemuck::Zeroable>::zeroed()
    }
}

impl Default for IrradianceGrid {
    fn default() -> Self {
        Self {
            version: IRRADIANCE_VERSION,
            enabled: false,
            provenance: IrradianceProvenance::Authored,
            dimensions: [2; 3],
            origin: [-1.0; 3],
            spacing: [2.0; 3],
            coefficients: vec![[[0.0; 3]; 9]; 8],
        }
    }
}

impl IrradianceGrid {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_IRRADIANCE_BYTES {
            return Err("irradiance JSON exceeds 256 KiB".into());
        }
        let grid: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("irradiance JSON: {e}"))?;
        grid.validate()?;
        Ok(grid)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_IRRADIANCE_BYTES {
            return Err("irradiance JSON exceeds 256 KiB".into());
        }
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != IRRADIANCE_VERSION {
            return Err(format!("unsupported irradiance version {}", self.version));
        }
        if !self.dimensions.iter().all(|d| (2..=4).contains(d)) {
            return Err("irradiance dimensions must each be in 2..=4".into());
        }
        let count = self
            .dimensions
            .iter()
            .try_fold(1_u32, |n, d| n.checked_mul(*d))
            .ok_or("irradiance node count overflow")?;
        if count as usize > MAX_PROBES || self.coefficients.len() != count as usize {
            return Err("irradiance node count must equal dimensions product (at most 64)".into());
        }
        for axis in 0..3 {
            let origin = self.origin[axis];
            let spacing = self.spacing[axis];
            if !origin.is_finite() || origin.abs() > WORLD_LIMIT {
                return Err("irradiance origin must be finite and within +/-1000000".into());
            }
            if !spacing.is_finite() || !(MIN_SPACING..=WORLD_LIMIT).contains(&spacing) {
                return Err(
                    "irradiance spacing must be finite and between 0.001 and 1000000".into(),
                );
            }
            let maximum = origin + spacing * (self.dimensions[axis] - 1) as f32;
            // Also reject a cell unrepresentable at this f32 world coordinate.
            if !maximum.is_finite()
                || maximum.abs() > WORLD_LIMIT
                || (1..self.dimensions[axis])
                    .any(|i| origin + spacing * i as f32 <= origin + spacing * (i - 1) as f32)
            {
                return Err("irradiance grid extent must fit finite +/-1000000 world coordinates with distinct nodes".into());
            }
        }
        if !self
            .coefficients
            .iter()
            .flatten()
            .flatten()
            .all(|c| c.is_finite() && c.abs() <= MAX_COEFFICIENT)
        {
            return Err(
                "irradiance SH coefficients must be finite signed RGB within +/-10000".into(),
            );
        }
        Ok(())
    }

    pub fn node_index(&self, node: [u32; 3]) -> Option<usize> {
        if node.iter().zip(self.dimensions).any(|(i, d)| *i >= d) {
            return None;
        }
        let index = node[2]
            .checked_mul(self.dimensions[1])?
            .checked_add(node[1])?
            .checked_mul(self.dimensions[0])?
            .checked_add(node[0])?;
        ((index as usize) < self.coefficients.len()).then_some(index as usize)
    }

    pub fn maximum(&self) -> [f32; 3] {
        std::array::from_fn(|axis| {
            self.origin[axis] + self.spacing[axis] * self.dimensions[axis].saturating_sub(1) as f32
        })
    }

    /// Invalid/disabled grids, invalid normals/positions, and outside coverage
    /// have no local sample. Robust normalization accepts every finite nonzero
    /// normal, including subnormal and near-f32::MAX inputs.
    pub fn sample(&self, position: [f32; 3], normal: [f32; 3]) -> Option<IrradianceSample> {
        if !self.enabled || self.validate().is_err() || !position.iter().all(|x| x.is_finite()) {
            return None;
        }
        let basis = sh_basis(normal)?;
        let maximum = self.maximum();
        let mut cell = [0_u32; 3];
        let mut fraction = [0.0; 3];
        let mut weight = 1.0_f32;
        for axis in 0..3 {
            if position[axis] < self.origin[axis] || position[axis] > maximum[axis] {
                return None;
            }
            // Clamp the local coordinate only after testing actual world bounds.
            // Rounding cannot promote an outside point into coverage.
            let q = if position[axis] == maximum[axis] {
                (self.dimensions[axis] - 1) as f32
            } else {
                ((position[axis] - self.origin[axis]) * (1.0 / self.spacing[axis]))
                    .clamp(0.0, (self.dimensions[axis] - 1) as f32)
            };
            cell[axis] = (q.floor() as u32).min(self.dimensions[axis] - 2);
            fraction[axis] = q - cell[axis] as f32;
            weight = weight.min(q).min((self.dimensions[axis] - 1) as f32 - q);
        }
        let mut coefficients = [[0.0_f32; 3]; 9];
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let corner = [x, y, z];
                    let mut w = 1.0;
                    for axis in 0..3 {
                        w *= if corner[axis] == 0 {
                            1.0 - fraction[axis]
                        } else {
                            fraction[axis]
                        };
                    }
                    let index = self.node_index([cell[0] + x, cell[1] + y, cell[2] + z])?;
                    for (output, node) in coefficients.iter_mut().zip(self.coefficients[index]) {
                        for channel in 0..3 {
                            output[channel] += node[channel] * w;
                        }
                    }
                }
            }
        }
        let mut irradiance = [0.0_f32; 3];
        for (coefficient, basis) in coefficients.iter().zip(basis) {
            for channel in 0..3 {
                irradiance[channel] += coefficient[channel] * basis;
            }
        }
        for channel in &mut irradiance {
            *channel = channel.max(0.0);
        }
        Some(IrradianceSample {
            irradiance,
            weight: weight.clamp(0.0, 1.0),
        })
    }

    pub fn packed_uniform(&self) -> Result<IrradianceUniform, String> {
        self.validate()?;
        let maximum = self.maximum();
        let mut packed = IrradianceUniform {
            origin: [self.origin[0], self.origin[1], self.origin[2], 0.0],
            maximum: [maximum[0], maximum[1], maximum[2], 0.0],
            inverse_spacing: [
                1.0 / self.spacing[0],
                1.0 / self.spacing[1],
                1.0 / self.spacing[2],
                0.0,
            ],
            dimensions: [
                self.dimensions[0],
                self.dimensions[1],
                self.dimensions[2],
                u32::from(self.enabled),
            ],
            ..IrradianceUniform::default()
        };
        for (node_index, node) in self.coefficients.iter().enumerate() {
            for (coefficient_index, c) in node.iter().enumerate() {
                packed.coefficients[node_index * SH_COEFFICIENTS + coefficient_index] =
                    [c[0], c[1], c[2], 0.0];
            }
        }
        Ok(packed)
    }
}

/// Produces constant nonnegative linear RGB irradiance, not outgoing radiance.
/// In particular c0 = E / Y00, with every directional coefficient zero.
pub fn constant_irradiance(irradiance: [f32; 3]) -> Result<Sh9, String> {
    if !irradiance
        .iter()
        .all(|v| v.is_finite() && (0.0..=MAX_COEFFICIENT * Y00).contains(v))
    {
        return Err(
            "constant irradiance must be finite, nonnegative, and at most 2820.948 per channel"
                .into(),
        );
    }
    let mut coefficients = [[0.0; 3]; 9];
    coefficients[0] = irradiance.map(|e| e / Y00);
    Ok(coefficients)
}

/// Positive real orthonormal SH, ordered by increasing band then m.
/// We explicitly omit the Condon–Shortley minus signs for l=1,m=±1.
pub fn sh_basis(normal: [f32; 3]) -> Option<[f32; 9]> {
    if !normal.iter().all(|v| v.is_finite()) {
        return None;
    }
    let maximum = normal.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    if maximum == 0.0 {
        return None;
    }
    let scaled = normal.map(|v| v / maximum);
    let length = scaled.iter().map(|v| v * v).sum::<f32>().sqrt();
    let [x, y, z] = scaled.map(|v| v / length);
    Some([
        Y00,
        0.488_602_52 * y,
        0.488_602_52 * z,
        0.488_602_52 * x,
        1.092_548_5 * x * y,
        1.092_548_5 * y * z,
        0.315_391_57 * (3.0 * z * z - 1.0),
        1.092_548_5 * x * z,
        0.546_274_24 * (x * x - y * y),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> IrradianceGrid {
        IrradianceGrid {
            enabled: true,
            origin: [0.0; 3],
            spacing: [1.0; 3],
            coefficients: vec![constant_irradiance([2.0, 3.0, 4.0]).unwrap(); 8],
            ..IrradianceGrid::default()
        }
    }
    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 2e-5, "{a} != {b}");
    }

    #[test]
    fn constant_is_irradiance_and_normalization_is_robust() {
        let grid = grid();
        for normal in [
            [0.0, 1.0, 0.0],
            [1.0, 2.0, -3.0],
            [f32::MAX; 3],
            [f32::from_bits(1), 0.0, 0.0],
        ] {
            let sample = grid.sample([0.5; 3], normal).unwrap();
            for (actual, expected) in sample.irradiance.into_iter().zip([2.0, 3.0, 4.0]) {
                near(actual, expected);
            }
            near(sample.weight, 0.5);
        }
        for normal in [[0.0; 3], [f32::NAN, 0.0, 1.0], [f32::INFINITY, 1.0, 0.0]] {
            assert!(grid.sample([0.5; 3], normal).is_none());
        }
    }
    #[test]
    fn basis_axes_and_signs() {
        let x = sh_basis([1.0, 0.0, 0.0]).unwrap();
        let y = sh_basis([0.0, 1.0, 0.0]).unwrap();
        let z = sh_basis([0.0, 0.0, 1.0]).unwrap();
        near(x[3], 0.48860252);
        near(y[1], 0.48860252);
        near(z[2], 0.48860252);
        near(x[6], -0.31539157);
        near(z[6], 0.63078314);
        near(y[8], -0.54627424);
        let negative = sh_basis([-1.0, -1.0, -1.0]).unwrap();
        assert!(negative[1] < 0.0 && negative[3] < 0.0 && negative[4] > 0.0 && negative[7] > 0.0);
    }
    #[test]
    fn eight_node_interpolation_and_single_final_clamp() {
        let mut grid = grid();
        for (index, coefficients) in grid.coefficients.iter_mut().enumerate() {
            *coefficients = [[0.0; 3]; 9];
            coefficients[0] = [(index as f32 - 3.0) / Y00; 3];
        }
        // Average [-3,-2,-1,0,1,2,3,4] is .5; per-node clamping would be 1.25.
        let sample = grid.sample([0.5; 3], [0.0, 1.0, 0.0]).unwrap();
        near(sample.irradiance[0], 0.5);
        let sample = grid.sample([0.25, 0.5, 0.75], [0.0, 1.0, 0.0]).unwrap();
        near(sample.irradiance[0], 1.25);
        assert_eq!(grid.node_index([1, 0, 0]), Some(1));
        assert_eq!(grid.node_index([0, 1, 0]), Some(2));
        assert_eq!(grid.node_index([0, 0, 1]), Some(4));
        assert_eq!(grid.node_index([2, 0, 0]), None);
    }
    #[test]
    fn signed_directional_coefficients_are_evaluated_after_interpolation() {
        let mut grid = grid();
        for node in &mut grid.coefficients {
            node[3] = [-2.0, 1.0, 0.0];
        }
        let positive = grid.sample([0.5; 3], [1.0, 0.0, 0.0]).unwrap();
        let negative = grid.sample([0.5; 3], [-1.0, 0.0, 0.0]).unwrap();
        near(positive.irradiance[0], 2.0 - 2.0 * 0.48860252);
        near(negative.irradiance[0], 2.0 + 2.0 * 0.48860252);
    }
    #[test]
    fn negative_reconstructed_channels_clamp_once_and_constant_helper_is_bounded() {
        let mut grid = grid();
        for node in &mut grid.coefficients {
            *node = [[0.0; 3]; 9];
            node[0] = [-1.0 / Y00, 2.0 / Y00, -3.0 / Y00];
        }
        let sample = grid.sample([0.5; 3], [1.0, 2.0, 3.0]).unwrap();
        assert_eq!(sample.irradiance[0], 0.0);
        near(sample.irradiance[1], 2.0);
        assert_eq!(sample.irradiance[2], 0.0);
        for invalid in [
            [-1.0, 0.0, 0.0],
            [f32::NAN; 3],
            [f32::INFINITY; 3],
            [1.0e4; 3],
        ] {
            assert!(constant_irradiance(invalid).is_err());
        }
    }

    #[test]
    fn boundaries_fade_inward_without_index_overflow() {
        let mut grid = grid();
        grid.dimensions = [4; 3];
        grid.coefficients.resize(64, grid.coefficients[0]);
        near(grid.sample([1.0; 3], [0.0, 1.0, 0.0]).unwrap().weight, 1.0);
        near(
            grid.sample([0.25, 1.0, 1.0], [0.0, 1.0, 0.0])
                .unwrap()
                .weight,
            0.25,
        );
        for p in [[0.0, 1.0, 1.0], [3.0, 1.0, 1.0], [3.0; 3]] {
            let sample = grid.sample(p, [0.0, 1.0, 0.0]).unwrap();
            near(sample.weight, 0.0);
            near(sample.irradiance[0], 2.0);
        }
        for p in [
            [-f32::EPSILON, 1.0, 1.0],
            [3.000001, 1.0, 1.0],
            [f32::NAN; 3],
        ] {
            assert!(grid.sample(p, [0.0, 1.0, 0.0]).is_none());
        }
        grid.enabled = false;
        assert!(grid.sample([1.0; 3], [0.0, 1.0, 0.0]).is_none());
    }
    #[test]
    fn uniform_layout_is_bounded_and_zero_fills_unused_nodes() {
        assert_eq!(std::mem::size_of::<IrradianceUniform>(), 9280);
        let packed = grid().packed_uniform().unwrap();
        assert_eq!(packed.dimensions, [2, 2, 2, 1]);
        assert_eq!(packed.inverse_spacing, [1.0, 1.0, 1.0, 0.0]);
        assert!(packed.coefficients[72..].iter().all(|c| *c == [0.0; 4]));
    }
    #[test]
    fn non_binary_spacing_maximum_and_next_float_outside() {
        let mut grid = grid();
        grid.dimensions = [4; 3];
        grid.coefficients.resize(64, grid.coefficients[0]);
        grid.origin = [0.1, -0.7, 0.3];
        grid.spacing = [0.2, 0.3, 0.7];
        let maximum = grid.maximum();
        let packed = grid.packed_uniform().unwrap();
        assert_eq!(packed.maximum, [maximum[0], maximum[1], maximum[2], 0.0]);
        let sample = grid.sample(maximum, [0.0, 1.0, 0.0]).unwrap();
        assert_eq!(sample.weight, 0.0);
        near(sample.irradiance[0], 2.0);
        let mut outside = maximum;
        outside[0] = f32::from_bits(maximum[0].to_bits() + 1);
        assert!(grid.sample(outside, [0.0, 1.0, 0.0]).is_none());
        assert_eq!(std::mem::offset_of!(IrradianceUniform, maximum), 48);
        assert_eq!(std::mem::offset_of!(IrradianceUniform, coefficients), 64);
    }

    #[test]
    fn json_enforces_nested_array_shape_and_bounded_node_count() {
        let source = serde_json::to_value(grid()).unwrap();
        for dimensions in [[0, 2, 2], [1, 2, 2], [2, 5, 2], [4, 4, 5]] {
            let mut invalid = source.clone();
            invalid["dimensions"] = serde_json::json!(dimensions);
            assert!(IrradianceGrid::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = source.clone();
        invalid["coefficients"][0] = serde_json::to_value([[0, 0, 0]; 8]).unwrap();
        assert!(IrradianceGrid::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        invalid = source.clone();
        invalid["coefficients"][0][0] = serde_json::json!([0, 0]);
        assert!(IrradianceGrid::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        invalid = source.clone();
        invalid["coefficients"] = serde_json::to_value(vec![grid().coefficients[0]; 65]).unwrap();
        assert!(IrradianceGrid::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        invalid = source;
        invalid["provenance"] = serde_json::json!("unsupported_realtime_gi");
        assert!(IrradianceGrid::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    #[test]
    fn json_roundtrip_and_rejection() {
        let grid = grid();
        let bytes = grid.to_json().unwrap();
        assert_eq!(IrradianceGrid::from_json(&bytes).unwrap(), grid);
        let source = String::from_utf8(bytes).unwrap();
        for invalid in [
            source.replacen("\"version\": 1", "\"version\": 2", 1),
            source.replacen("\"version\": 1", "\"version\": 1, \"unexpected\": 0", 1),
            source.replacen("\"version\": 1", "\"version\": 1, \"version\": 1", 1),
            source.replacen("0.0", "1e999", 1),
            source.replacen("0.0", "NaN", 1),
        ] {
            assert!(IrradianceGrid::from_json(invalid.as_bytes()).is_err());
        }
        assert!(IrradianceGrid::from_json(&vec![b' '; MAX_IRRADIANCE_BYTES + 1]).is_err());
        let mut invalid = grid.clone();
        invalid.coefficients.pop();
        assert!(invalid.validate().is_err());
        invalid = grid.clone();
        invalid.dimensions = [u32::MAX; 3];
        assert!(invalid.validate().is_err());
        invalid = grid.clone();
        invalid.coefficients[0][0][0] = f32::NAN;
        assert!(invalid.to_json().is_err());
        invalid = grid.clone();
        invalid.coefficients[0][0][0] = 10000.1;
        assert!(invalid.validate().is_err());
        invalid = grid.clone();
        invalid.origin[0] = WORLD_LIMIT;
        assert!(invalid.validate().is_err());
        invalid = grid.clone();
        invalid.spacing[0] = 0.0009;
        assert!(invalid.validate().is_err());
        invalid = grid;
        invalid.origin[0] = 100000.0;
        invalid.spacing[0] = MIN_SPACING;
        assert!(invalid.validate().is_err());
    }
}
