//! Morph targets (blend shapes): per-vertex position and normal offsets of a
//! primitive, mixed by weights before skinning. A face's smile or a blink is a
//! target; weight 0 leaves the mesh as modeled and 1 adds the whole offset.
//!
//! Weights live beside the [`super::Pose`], not in it: a pose graph blends
//! joints only. [`AnimatedModel::rest_weights`] gives the modeled defaults and
//! [`AnimatedModel::sample_weights`] the weights of a clip at a time.
use super::{add_budget, sample_interval, AnimatedModel, AnimatedSource, Interpolation};
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
    /// Step or linear.
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// All weights of the first time, then of the second, and so on.
    pub weights: Vec<f32>,
}

/// Weights of each [`PrimitiveMorph`], in the same order.
pub type MorphWeights = Vec<Vec<f32>>;

fn weight_ok(w: f32) -> bool {
    w.is_finite() && w.abs() <= MAX_MORPH_WEIGHT
}

/// Checks the targets and tracks against the rest of `source`.
pub(super) fn validate(source: &AnimatedSource, decoded: &mut usize) -> Result<(), Error> {
    validate_targets(source, decoded)?;
    validate_channels(source, decoded)
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

fn validate_channels(source: &AnimatedSource, decoded: &mut usize) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for channel in &source.morph.channels {
        if channel.clip as usize >= source.clips.len() || !seen.insert((channel.clip, channel.node)) {
            return Err(invalid("morph channel clip out of range, or duplicate morph channel target"));
        }
        let k = targets_on(source, channel.node).ok_or_else(|| invalid("morph channel node has no targets, or differing counts"))?;
        if channel.interpolation == Interpolation::CubicSpline
            || channel.times.is_empty()
            || channel.times.iter().any(|t| !t.is_finite() || *t < 0.0)
            || channel.times.windows(2).any(|t| t[0] >= t[1])
            || channel.times.len().checked_mul(k) != Some(channel.weights.len())
            || !channel.weights.iter().all(|w| weight_ok(*w))
        {
            return Err(invalid("morph channel times/weights; only step and linear"));
        }
        add_budget(decoded, (channel.times.len() + channel.weights.len()) * 4, MAX_DECODED_BYTES, "decoded model")?;
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
            let k = channel.weights.len() / channel.times.len();
            let (wa, wb) = (&channel.weights[a * k..(a + 1) * k], &channel.weights[b * k..(b + 1) * k]);
            for (p, out) in self.source.morph.primitives.iter().zip(&mut weights) {
                if self.source.primitives[p.primitive as usize].node == channel.node {
                    *out = wa.iter().zip(wb).map(|(x, y)| x + (y - x) * t).collect();
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
