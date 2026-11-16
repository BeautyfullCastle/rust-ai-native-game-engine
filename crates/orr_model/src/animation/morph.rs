//! Morph targets (blend shapes): per-vertex position and normal offsets of a
//! primitive, mixed by weights before skinning. A face's smile or a blink is a
//! target; weight 0 leaves the mesh as modeled and 1 adds the whole offset.
//!
//! Weights live beside the [`super::Pose`], not in it: a pose graph blends
//! joints only. [`AnimatedModel::rest_weights`] gives the modeled defaults and
//! [`AnimatedModel::sample_weights`] the weights of a clip at a time.
use super::{add_budget, MAX_ANIMATION_BYTES, MAX_ANIMATION_KEYS, sample_interval, AnimatedModel, AnimatedSource, Interpolation};
use std::collections::BTreeSet;
use crate::{invalid, Error, Vertex, MAX_DECODED_BYTES};
use serde::{Deserialize, Serialize};

/// Targets one primitive can have.
pub const MAX_MORPH_TARGETS: usize = 8;
/// Largest weight magnitude. Weights above 1 exaggerate a target.
pub const MAX_MORPH_WEIGHT: f32 = 16.0;

/// The morph targets of a model and the weight tracks of its clips.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Morphs {
    /// One entry per primitive that has targets, in primitive order.
    pub primitives: Vec<PrimitiveMorph>,
    /// Weight tracks, each in one clip and on one node.
    pub channels: Vec<MorphChannel>,
}

impl Morphs {
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty() && self.channels.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveMorph {
    /// Index into [`AnimatedSource::primitives`].
    pub primitive: u32,
    pub targets: Vec<MorphTarget>,
    /// Weights when no clip says otherwise, one per target.
    pub weights: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphTarget {
    pub name: String,
    /// Offset of every vertex's position, in the primitive's vertex order.
    pub positions: Vec<[f32; 3]>,
    /// Offset of every vertex's normal, or empty for none.
    pub normals: Vec<[f32; 3]>,
}

/// The weights of every target of the primitives on `node` over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphChannel {
    /// Index into [`AnimatedSource::clips`].
    pub clip: u32,
    pub node: u32,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// All weights of the first time, then of the second, and so on. A cubic
    /// spline track holds three blocks per time: in-tangents, values, then
    /// out-tangents, each one weight per target.
    pub weights: Vec<f32>,
}

/// Weights of each [`PrimitiveMorph`], in the same order.
pub type MorphWeights = Vec<Vec<f32>>;

fn weight_ok(w: f32) -> bool {
    w.is_finite() && w.abs() <= MAX_MORPH_WEIGHT
}

/// Checks the targets and tracks against the rest of `source`.
pub(super) fn validate(source: &AnimatedSource, decoded: &mut usize, keys: &mut usize, animation_bytes: &mut usize) -> Result<(), Error> {
    validate_targets(source, decoded)?;
    validate_channels(source, decoded, keys, animation_bytes)
}

fn validate_targets(source: &AnimatedSource, decoded: &mut usize) -> Result<(), Error> {
    let mut last = None;
    for p in &source.morph.primitives {
        let primitive = source.primitives.get(p.primitive as usize).ok_or_else(|| invalid("morph primitive out of range"))?;
        if last.is_some_and(|l| l >= p.primitive) {
            return Err(invalid("morph primitives must be in increasing order"));
        }
        last = Some(p.primitive);
        if p.targets.is_empty() || p.targets.len() > MAX_MORPH_TARGETS || p.weights.len() != p.targets.len() || !p.weights.iter().all(|w| weight_ok(*w)) {
            return Err(invalid("morph target count or default weights"));
        }
        let count = primitive.vertices.len();
        for target in &p.targets {
            super::validate_name(&target.name)?;
            let finite = |v: &[[f32; 3]]| v.iter().flatten().all(|c| c.is_finite() && c.abs() <= 1.0e6);
            if target.positions.len() != count || !(target.normals.is_empty() || target.normals.len() == count) || !finite(&target.positions) || !finite(&target.normals) {
                return Err(invalid("morph target offsets must match the vertices and be finite"));
            }
            add_budget(decoded, target.name.len() + (target.positions.len() + target.normals.len()) * 12, MAX_DECODED_BYTES, "decoded model")?;
        }
    }
    Ok(())
}

fn validate_channels(source: &AnimatedSource, decoded: &mut usize, keys: &mut usize, animation_bytes: &mut usize) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for channel in &source.morph.channels {
        if channel.clip as usize >= source.clips.len() || !seen.insert((channel.clip, channel.node)) {
            return Err(invalid("morph channel clip out of range, or duplicate morph channel target"));
        }
        let k = targets_on(source, channel.node).ok_or_else(|| invalid("morph channel node has no targets, or differing counts"))?;
        let cubic = channel.interpolation == Interpolation::CubicSpline;
        let per_key = if cubic { k * 3 } else { k };
        if channel.times.is_empty()
            || (cubic && channel.times.len() < 2)
            || channel.times.iter().any(|t| !t.is_finite() || *t < 0.0)
            || channel.times.windows(2).any(|t| t[0] >= t[1])
            || channel.times.len().checked_mul(per_key) != Some(channel.weights.len())
            || !channel.weights.chunks_exact(per_key).all(|key| key.iter().enumerate().all(|(i, w)| if cubic && i / k != 1 { w.is_finite() } else { weight_ok(*w) }))
        {
            return Err(invalid("morph channel times/weights"));
        }
        let bytes = (channel.times.len() + channel.weights.len()) * 4;
        add_budget(keys, channel.weights.len(), MAX_ANIMATION_KEYS, "aggregate animation key")?;
        add_budget(animation_bytes, bytes, MAX_ANIMATION_BYTES, "aggregate animation byte")?;
        add_budget(decoded, bytes, MAX_DECODED_BYTES, "decoded model")?;
    }
    Ok(())
}

/// The target count shared by every morphed primitive on `node`.
fn targets_on(source: &AnimatedSource, node: u32) -> Option<usize> {
    let mut counts = source.morph.primitives.iter().filter(|p| source.primitives[p.primitive as usize].node == node).map(|p| p.targets.len());
    let first = counts.next()?;
    counts.all(|c| c == first).then_some(first)
}

impl AnimatedModel {
    /// The default weights of every morphed primitive.
    pub fn rest_weights(&self) -> MorphWeights {
        self.source.morph.primitives.iter().map(|p| p.weights.clone()).collect()
    }

    /// The weights of clip `clip` at `time`: the defaults, replaced by the
    /// clip's tracks. Time clamps to each track.
    pub fn sample_weights(&self, clip: u32, time: f32) -> Result<MorphWeights, Error> {
        if !time.is_finite() {
            return Err(invalid("nonfinite animation sample time"));
        }
        if clip as usize >= self.source.clips.len() {
            return Err(invalid("animation clip out of range"));
        }
        let mut weights = self.rest_weights();
        for channel in self.source.morph.channels.iter().filter(|c| c.clip == clip) {
            let (a, b, t) = sample_interval(&channel.times, time, channel.interpolation);
            let sampled: Vec<f32> = if channel.interpolation == Interpolation::CubicSpline {
                let k = channel.weights.len() / (channel.times.len() * 3);
                cubic_weights(&channel.weights, k, a, b, t, channel.times[b] as f64 - channel.times[a] as f64)?
            } else {
                let k = channel.weights.len() / channel.times.len();
                let (wa, wb) = (&channel.weights[a * k..(a + 1) * k], &channel.weights[b * k..(b + 1) * k]);
                wa.iter().zip(wb).map(|(x, y)| x + (y - x) * t).collect()
            };
            for (p, out) in self.source.morph.primitives.iter().zip(&mut weights) {
                if self.source.primitives[p.primitive as usize].node == channel.node {
                    out.clone_from(&sampled);
                }
            }
        }
        Ok(weights)
    }

    /// Checks that `weights` fit this model's targets.
    pub fn validate_weights(&self, weights: &[Vec<f32>]) -> Result<(), Error> {
        let primitives = &self.source.morph.primitives;
        if weights.len() != primitives.len() || weights.iter().zip(primitives).any(|(w, p)| w.len() != p.targets.len() || !w.iter().all(|w| weight_ok(*w))) {
            return Err(invalid("morph weights do not match the targets"));
        }
        Ok(())
    }

    /// The targets of primitive `index` with their weights, when it has any.
    /// `weights` must have passed [`Self::validate_weights`].
    pub fn primitive_morph<'a>(&'a self, index: usize, weights: &'a [Vec<f32>]) -> Option<(&'a PrimitiveMorph, &'a [f32])> {
        let primitives = &self.source.morph.primitives;
        let slot = primitives.binary_search_by_key(&(index as u32), |p| p.primitive).ok()?;
        Some((&primitives[slot], &weights[slot]))
    }
}

/// glTF 2.0 Appendix C: Hermite spline of the weights between keys `a` and
/// `b`. Tangents are per second, so the out- and in-tangents scale by `delta`,
/// the seconds between the keys. The result is not clamped, but it must stay
/// within the weight limit that [`AnimatedModel::validate_weights`] enforces.
fn cubic_weights(weights: &[f32], k: usize, a: usize, b: usize, t: f32, delta: f64) -> Result<Vec<f32>, Error> {
    let block = |key: usize, part: usize, i: usize| weights[key * 3 * k + part * k + i] as f64;
    let t = t as f64;
    let (t2, t3) = (t * t, t * t * t);
    let out: Vec<f32> = (0..k)
        .map(|i| {
            if a == b || t == 0.0 {
                return block(a, 1, i) as f32;
            }
            ((2.0 * t3 - 3.0 * t2 + 1.0) * block(a, 1, i) + delta * (t3 - 2.0 * t2 + t) * block(a, 2, i) + (-2.0 * t3 + 3.0 * t2) * block(b, 1, i) + delta * (t3 - t2) * block(b, 0, i)) as f32
        })
        .collect();
    if !out.iter().all(|w| weight_ok(*w)) {
        return Err(invalid("cubic morph weight outside the weight limit"));
    }
    Ok(out)
}

/// Vertex `index` of a primitive with its targets' offsets added by weight.
/// The normal is left unnormalized; skinning normalizes it.
pub fn morph_vertex(base: Vertex, index: usize, morph: Option<(&PrimitiveMorph, &[f32])>) -> Vertex {
    let Some((p, weights)) = morph else { return base };
    let mut v = base;
    for (target, &w) in p.targets.iter().zip(weights).filter(|(_, w)| **w != 0.0) {
        for c in 0..3 {
            v.position[c] += target.positions[index][c] * w;
            if let Some(n) = target.normals.get(index) {
                v.normal[c] += n[c] * w;
            }
        }
    }
    v
}
