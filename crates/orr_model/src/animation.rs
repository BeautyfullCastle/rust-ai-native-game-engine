//! Optional, separately versioned, presentation-only skeletal animation.
//!
//! Matrices are column-major. Hierarchies use `parent * local`; skin palettes
//! use `joint_global * inverse_bind`. Skinned vertices are already in model
//! space after palette application: do not apply their mesh node a second time.
//! World/entity placement belongs to the caller and never enters simulation.
use crate::{
    determinant, invalid, normal_matrix, Dependency, Error, Image, Material, ModelSource,
    Primitive, StaticModel, Vertex, IDENTITY, MAX_DECODED_BYTES, MAX_FILE_BYTES, MAX_IMAGES,
    MAX_INDICES, MAX_MATERIALS, MAX_NODES, MAX_PRIMITIVES, MAX_VERTICES,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

pub type Matrix4 = [[f32; 4]; 4];
pub const MAX_JOINTS: usize = 64;
pub const MAX_SKINS: usize = 16;
pub const MAX_CLIPS: usize = 32;
pub const MAX_CHANNELS: usize = 256;
pub const MAX_HIERARCHY_DEPTH: usize = 64;
/// Aggregate across every clip/channel, including each channel's input times.
pub const MAX_ANIMATION_KEYS: usize = 262_144;
pub const MAX_ANIMATION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trs {
    pub translation: [f32; 3],
    /// Quaternion in x, y, z, w order.
    pub rotation: [f32; 4],
    /// The bounded v1 format permits only positive, nonzero scales.
    pub scale: [f32; 3],
}
impl Default for Trs {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}
impl Trs {
    pub fn matrix(self) -> Matrix4 {
        let [x, y, z, w] = normalize_quaternion(self.rotation);
        let [sx, sy, sz] = self.scale;
        [
            [
                (1.0 - 2.0 * (y * y + z * z)) * sx,
                2.0 * (x * y + z * w) * sx,
                2.0 * (x * z - y * w) * sx,
                0.0,
            ],
            [
                2.0 * (x * y - z * w) * sy,
                (1.0 - 2.0 * (x * x + z * z)) * sy,
                2.0 * (y * z + x * w) * sy,
                0.0,
            ],
            [
                2.0 * (x * z + y * w) * sz,
                2.0 * (y * z - x * w) * sz,
                (1.0 - 2.0 * (x * x + y * y)) * sz,
                0.0,
            ],
            [
                self.translation[0],
                self.translation[1],
                self.translation[2],
                1.0,
            ],
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimatedNode {
    pub name: String,
    pub children: Vec<u32>,
    pub rest: Trs,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    pub name: String,
    /// Indices into the complete hierarchy, in palette order.
    pub joints: Vec<u32>,
    pub inverse_bind_matrices: Vec<Matrix4>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinnedVertex {
    pub vertex: Vertex,
    /// Skin-local palette indices, including zero-weight slots.
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimatedPrimitive {
    pub id: String,
    pub node: u32,
    pub skin: Option<u32>,
    pub vertices: Vec<SkinnedVertex>,
    pub indices: Vec<u32>,
    pub material: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Step,
    Linear,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "path",
    content = "values",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ChannelValues {
    Translation(Vec<[f32; 3]>),
    Rotation(Vec<[f32; 4]>),
    Scale(Vec<[f32; 3]>),
}
impl ChannelValues {
    fn len(&self) -> usize {
        match self {
            Self::Translation(v) | Self::Scale(v) => v.len(),
            Self::Rotation(v) => v.len(),
        }
    }
    fn path(&self) -> u8 {
        match self {
            Self::Translation(_) => 0,
            Self::Rotation(_) => 1,
            Self::Scale(_) => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationChannel {
    pub node: u32,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: ChannelValues,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationClip {
    pub name: String,
    pub channels: Vec<AnimationChannel>,
}
impl AnimationClip {
    pub fn duration(&self) -> f32 {
        self.channels
            .iter()
            .filter_map(|c| c.times.last().copied())
            .fold(0.0, f32::max)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimatedSource {
    pub format: String,
    pub version: u32,
    pub asset_id: String,
    pub dependencies: Vec<Dependency>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    pub nodes: Vec<AnimatedNode>,
    pub skins: Vec<Skin>,
    pub primitives: Vec<AnimatedPrimitive>,
    pub clips: Vec<AnimationClip>,
}
pub type AnimatedModelSource = AnimatedSource;

/// Validated immutable asset. Cooked load and import share these invariants.
#[derive(Clone, Debug)]
pub struct AnimatedModel {
    source: AnimatedSource,
    /// Parents precede children; source indices themselves need not be sorted.
    order: Vec<usize>,
    parents: Vec<Option<usize>>,
    owner: Arc<()>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    owner: Arc<()>,
    local: Vec<Trs>,
    global: Vec<Matrix4>,
    skin_matrices: Vec<Vec<Matrix4>>,
}
impl Pose {
    pub fn local(&self) -> &[Trs] {
        &self.local
    }
    pub fn global(&self) -> &[Matrix4] {
        &self.global
    }
    pub fn skin_matrices(&self) -> &[Vec<Matrix4>] {
        &self.skin_matrices
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl AnimatedModel {
    pub fn new(source: AnimatedSource) -> Result<Self, Error> {
        if source.format != "orr_animated_model" || source.version != 1 {
            return Err(invalid("unsupported animated model format/version"));
        }
        if source.nodes.is_empty()
            || source.nodes.len() > MAX_NODES
            || source.skins.len() > MAX_SKINS
            || source.clips.len() > MAX_CLIPS
            || source.primitives.is_empty()
            || source.primitives.len() > MAX_PRIMITIVES
            || source.materials.is_empty()
            || source.materials.len() > MAX_MATERIALS
            || source.images.is_empty()
            || source.images.len() > MAX_IMAGES
            || source.dependencies.is_empty()
            || source.dependencies.len() > 256
        {
            return Err(invalid("animated model collection limit"));
        }
        let mut decoded_bytes =
            source.asset_id.len() + source.format.len() + source.materials.len() * 32;
        for dependency in &source.dependencies {
            add_budget(
                &mut decoded_bytes,
                dependency.uri.len() + dependency.sha256.len(),
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
        }
        let mut parents = vec![None; source.nodes.len()];
        for (index, node) in source.nodes.iter().enumerate() {
            validate_name(&node.name)?;
            validate_trs(node.rest)?;
            add_budget(
                &mut decoded_bytes,
                node.name.len() + 40 + node.children.len() * 4,
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
            if node.children.len() > MAX_NODES {
                return Err(invalid("node child count exceeds limit"));
            }
            for &child in &node.children {
                let parent = parents
                    .get_mut(child as usize)
                    .ok_or_else(|| invalid("node child out of range"))?;
                if child as usize == index || parent.replace(index).is_some() {
                    return Err(invalid(
                        "duplicate child, multiple parents, or hierarchy cycle",
                    ));
                }
            }
        }
        let mut order = Vec::with_capacity(source.nodes.len());
        let mut stack: Vec<_> = parents
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_none().then_some((i, 0)))
            .collect();
        while let Some((index, depth)) = stack.pop() {
            if depth > MAX_HIERARCHY_DEPTH {
                return Err(invalid("hierarchy depth exceeds limit"));
            }
            order.push(index);
            stack.extend(
                source.nodes[index]
                    .children
                    .iter()
                    .map(|&v| (v as usize, depth + 1)),
            );
        }
        if order.len() != source.nodes.len() {
            return Err(invalid("hierarchy cycle"));
        }
        for skin in &source.skins {
            validate_name(&skin.name)?;
            if skin.joints.is_empty()
                || skin.joints.len() > MAX_JOINTS
                || skin.joints.len() != skin.inverse_bind_matrices.len()
            {
                return Err(invalid("skin joint/inverse bind count"));
            }
            let mut seen = BTreeSet::new();
            let mut root = None;
            for (&joint, &matrix) in skin.joints.iter().zip(&skin.inverse_bind_matrices) {
                if joint as usize >= source.nodes.len() || !seen.insert(joint) {
                    return Err(invalid("invalid/duplicate skin joint"));
                }
                let mut joint_root = joint as usize;
                while let Some(parent) = parents[joint_root] {
                    joint_root = parent;
                }
                if root.is_some_and(|root| root != joint_root) {
                    return Err(invalid("skin joints do not share a hierarchy root"));
                }
                root = Some(joint_root);
                validate_matrix(matrix)?;
            }
            add_budget(
                &mut decoded_bytes,
                skin.name.len() + skin.joints.len() * 68,
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
        }
        let (mut keys, mut animation_bytes) = (0usize, 0usize);
        for clip in &source.clips {
            validate_name(&clip.name)?;
            if clip.channels.is_empty() || clip.channels.len() > MAX_CHANNELS {
                return Err(invalid("clip channel count exceeds limit or is empty"));
            }
            let mut targets = BTreeSet::new();
            add_budget(
                &mut decoded_bytes,
                clip.name.len(),
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
            for channel in &clip.channels {
                if channel.node as usize >= source.nodes.len()
                    || !targets.insert((channel.node, channel.values.path()))
                {
                    return Err(invalid("invalid/duplicate animation channel target"));
                }
                if channel.times.is_empty()
                    || channel.times.len() != channel.values.len()
                    || channel.times.iter().any(|v| !v.is_finite() || *v < 0.0)
                    || channel.times.windows(2).any(|v| v[0] >= v[1])
                {
                    return Err(invalid("invalid animation times/value count"));
                }
                add_budget(
                    &mut keys,
                    channel.times.len(),
                    MAX_ANIMATION_KEYS,
                    "aggregate animation key",
                )?;
                let stride = if matches!(channel.values, ChannelValues::Rotation(_)) {
                    20
                } else {
                    16
                };
                add_budget(
                    &mut animation_bytes,
                    channel.times.len() * stride,
                    MAX_ANIMATION_BYTES,
                    "aggregate animation byte",
                )?;
                match &channel.values {
                    ChannelValues::Translation(values) => {
                        for value in values {
                            validate_translation(*value)?;
                        }
                    }
                    ChannelValues::Rotation(values) => {
                        for value in values {
                            validate_rotation(*value)?;
                        }
                    }
                    ChannelValues::Scale(values) => {
                        for value in values {
                            validate_scale(*value)?;
                        }
                    }
                }
            }
        }
        add_budget(
            &mut decoded_bytes,
            animation_bytes,
            MAX_DECODED_BYTES,
            "decoded model",
        )?;
        let (mut vertices, mut indices) = (0usize, 0usize);
        for primitive in &source.primitives {
            if primitive.node as usize >= source.nodes.len() {
                return Err(invalid("primitive node out of range"));
            }
            let skin = primitive
                .skin
                .map(|s| {
                    source
                        .skins
                        .get(s as usize)
                        .ok_or_else(|| invalid("primitive skin out of range"))
                })
                .transpose()?;
            add_budget(
                &mut vertices,
                primitive.vertices.len(),
                MAX_VERTICES,
                "vertex",
            )?;
            add_budget(&mut indices, primitive.indices.len(), MAX_INDICES, "index")?;
            add_budget(
                &mut decoded_bytes,
                primitive.id.len() + primitive.vertices.len() * 64 + primitive.indices.len() * 4,
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
            for v in &primitive.vertices {
                if !v
                    .weights
                    .iter()
                    .all(|w| w.is_finite() && (0.0..=1.0).contains(w))
                    || (v.weights.iter().sum::<f32>() - 1.0).abs() > 1.0e-5
                {
                    return Err(invalid("invalid/non-normalized skin weights"));
                }
                if let Some(skin) = skin {
                    let mut used = BTreeSet::new();
                    if v.joints
                        .iter()
                        .zip(v.weights)
                        .any(|(&joint, weight)| weight > 0.0 && !used.insert(joint))
                    {
                        return Err(invalid("duplicate nonzero skin influence"));
                    }
                    if v.joints.iter().any(|&j| j as usize >= skin.joints.len()) {
                        return Err(invalid("skin influence joint out of range"));
                    }
                } else if v.joints != [0; 4] || v.weights != [1.0, 0.0, 0.0, 0.0] {
                    return Err(invalid("rigid vertex has skin influences"));
                }
            }
        }
        for image in &source.images {
            add_budget(
                &mut decoded_bytes,
                image.rgba8.len(),
                MAX_DECODED_BYTES,
                "decoded model",
            )?;
        }
        // Keep every static geometry/material/image/dependency invariant identical.
        StaticModel::new(ModelSource {
            format: "orr_static_model".into(),
            version: 1,
            asset_id: source.asset_id.clone(),
            dependencies: source.dependencies.clone(),
            materials: source.materials.clone(),
            images: source.images.clone(),
            primitives: source
                .primitives
                .iter()
                .map(|p| Primitive {
                    id: p.id.clone(),
                    vertices: p.vertices.iter().map(|v| v.vertex).collect(),
                    indices: p.indices.clone(),
                    material: p.material,
                    transform: IDENTITY,
                })
                .collect(),
        })?;
        let model = Self {
            source,
            order,
            parents,
            owner: Arc::new(()),
        };
        // A malformed bind pose must fail cooking/loading rather than reach a GPU.
        model.bounds(&model.rest_pose()?)?;
        Ok(model)
    }
    pub fn source(&self) -> &AnimatedSource {
        &self.source
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("cooked animated model exceeds byte limit"));
        }
        Self::new(serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let bytes = serde_json::to_vec(&self.source).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("cooked animated model exceeds byte limit"));
        }
        Ok(bytes)
    }
    pub fn rest_pose(&self) -> Result<Pose, Error> {
        self.pose_from_local(self.source.nodes.iter().map(|n| n.rest).collect())
    }
    /// Each sample starts from rest. Time clamps separately to each channel;
    /// absent tracks never retain values from a previously sampled clip.
    pub fn sample_clip(&self, clip: u32, time: f32) -> Result<Pose, Error> {
        if !time.is_finite() {
            return Err(invalid("nonfinite animation sample time"));
        }
        let clip = self
            .source
            .clips
            .get(clip as usize)
            .ok_or_else(|| invalid("animation clip out of range"))?;
        let mut local: Vec<_> = self.source.nodes.iter().map(|n| n.rest).collect();
        for channel in &clip.channels {
            let (a, b, t) = sample_interval(&channel.times, time, channel.interpolation);
            let trs = &mut local[channel.node as usize];
            match &channel.values {
                ChannelValues::Translation(v) => trs.translation = lerp3(v[a], v[b], t),
                ChannelValues::Scale(v) => trs.scale = lerp3(v[a], v[b], t),
                ChannelValues::Rotation(v) => trs.rotation = slerp(v[a], v[b], t),
            }
        }
        self.pose_from_local(local)
    }
    fn pose_from_local(&self, local: Vec<Trs>) -> Result<Pose, Error> {
        let mut global = vec![IDENTITY; local.len()];
        for &i in &self.order {
            validate_trs(local[i])?;
            let m = local[i].matrix();
            global[i] = self.parents[i].map_or(m, |parent| multiply(global[parent], m));
            validate_matrix(global[i])?;
        }
        let skin_matrices = self
            .source
            .skins
            .iter()
            .map(|skin| {
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind_matrices)
                    .map(|(&j, &ib)| {
                        let matrix = multiply(global[j as usize], ib);
                        validate_matrix(matrix)?;
                        Ok(matrix)
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Pose {
            owner: self.owner.clone(),
            local,
            global,
            skin_matrices,
        })
    }
    /// Check opaque pose ownership, dimensions and matrices before
    /// indexing or uploading a palette. Only this asset or its clones own it.
    pub fn validate_pose(&self, pose: &Pose) -> Result<(), Error> {
        if !Arc::ptr_eq(&pose.owner, &self.owner) {
            return Err(invalid("pose belongs to a different animated model"));
        }
        if pose.local.len() != self.source.nodes.len()
            || pose.global.len() != self.source.nodes.len()
            || pose.skin_matrices.len() != self.source.skins.len()
        {
            return Err(invalid("pose hierarchy/skin count mismatch"));
        }
        for &local in &pose.local {
            validate_trs(local)?;
        }
        for &matrix in &pose.global {
            validate_matrix(matrix)?;
        }
        for (matrices, skin) in pose.skin_matrices.iter().zip(&self.source.skins) {
            if matrices.len() != skin.joints.len() {
                return Err(invalid("pose palette count mismatch"));
            }
            for &matrix in matrices {
                validate_matrix(matrix)?;
            }
        }
        Ok(())
    }
    /// CPU deformation oracle, with inverse-transpose blended-matrix normals.
    /// Singular/mirrored blends or degenerate normals fail closed. Skin weights
    /// are renormalized to account for the validation tolerance. Placement is
    /// deliberately omitted, so returned primitive transforms are identity.
    pub fn deform(&self, pose: &Pose) -> Result<Vec<Primitive>, Error> {
        self.validate_pose(pose)?;
        self.source
            .primitives
            .iter()
            .map(|primitive| {
                let vertices = primitive
                    .vertices
                    .iter()
                    .map(|v| deform_vertex(primitive, v, pose))
                    .collect::<Result<Vec<_>, Error>>()?;
                Ok(Primitive {
                    id: primitive.id.clone(),
                    vertices,
                    indices: primitive.indices.clone(),
                    material: primitive.material,
                    transform: IDENTITY,
                })
            })
            .collect()
    }
    /// Exact current-pose vertex bounds, before caller/entity placement.
    /// Runs the same validation/deformation oracle without allocating geometry.
    pub fn bounds(&self, pose: &Pose) -> Result<Bounds, Error> {
        self.validate_pose(pose)?;
        let mut bounds = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for primitive in &self.source.primitives {
            for source_vertex in &primitive.vertices {
                let vertex = deform_vertex(primitive, source_vertex, pose)?;
                for i in 0..3 {
                    bounds.min[i] = bounds.min[i].min(vertex.position[i]);
                    bounds.max[i] = bounds.max[i].max(vertex.position[i]);
                }
            }
        }
        Ok(bounds)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackMode {
    Once,
    Loop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}
/// Per-instance presentation state. Neither asset data nor simulation is changed.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationPlayer {
    owner: Option<Arc<()>>,
    clip: Option<u32>,
    time: f32,
    mode: PlaybackMode,
    state: PlaybackState,
}
impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            owner: None,
            clip: None,
            time: 0.0,
            mode: PlaybackMode::Once,
            state: PlaybackState::Stopped,
        }
    }
}
impl AnimationPlayer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clip(&self) -> Option<u32> {
        self.clip
    }
    pub fn time(&self) -> f32 {
        self.time
    }
    pub fn mode(&self) -> PlaybackMode {
        self.mode
    }
    pub fn state(&self) -> PlaybackState {
        self.state
    }
    /// Selecting any clip, including the same one, resets time and playback.
    pub fn play(
        &mut self,
        model: &AnimatedModel,
        clip: u32,
        mode: PlaybackMode,
    ) -> Result<(), Error> {
        if model.source.clips.get(clip as usize).is_none() {
            return Err(invalid("animation clip out of range"));
        }
        self.owner = Some(model.owner.clone());
        self.clip = Some(clip);
        self.time = 0.0;
        self.mode = mode;
        self.state = PlaybackState::Playing;
        Ok(())
    }
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }
    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.state = PlaybackState::Playing;
        }
    }
    /// Stop clears selection and restores the rest pose on the next `pose` call.
    pub fn stop(&mut self) {
        *self = Self::default();
    }
    /// Reset the selected clip to time zero, retaining paused/playing state.
    /// A completed once-only clip becomes paused; call resume to replay it.
    pub fn reset_clip(&mut self) {
        self.time = 0.0;
        if self.state == PlaybackState::Finished {
            self.state = PlaybackState::Paused;
        }
    }
    /// Seek clamps in once mode and wraps in loop mode, without resuming a
    /// paused player. Negative and nonfinite times are rejected transactionally.
    pub fn seek(&mut self, model: &AnimatedModel, time: f32) -> Result<(), Error> {
        if !time.is_finite() || time < 0.0 {
            return Err(invalid("invalid animation seek time"));
        }
        let duration = self.duration(model)?;
        let new_time = playback_time(time, duration, self.mode);
        self.time = new_time;
        if self.mode == PlaybackMode::Once && time >= duration {
            self.state = PlaybackState::Finished;
        } else if self.state == PlaybackState::Finished {
            self.state = PlaybackState::Paused;
        }
        Ok(())
    }
    /// Once holds its final pose; loop wraps at exactly duration. Zero-duration
    /// loops remain at zero. A paused/stopped/finished player does not advance.
    pub fn advance(&mut self, model: &AnimatedModel, delta: f32) -> Result<(), Error> {
        if !delta.is_finite() || delta < 0.0 {
            return Err(invalid("invalid animation delta"));
        }
        self.check_model(model)?;
        if self.state != PlaybackState::Playing {
            return Ok(());
        }
        let duration = self.duration(model)?;
        // Use f64 for adding two finite f32 values without overflowing to infinity.
        let time = f64::from(self.time) + f64::from(delta);
        if self.mode == PlaybackMode::Loop && duration > 0.0 {
            self.time = (time % f64::from(duration)) as f32;
            // The narrowed remainder may round up to the exact endpoint.
            // A loop never exposes the once-only terminal frame.
            if self.time >= duration {
                self.time = 0.0;
            }
        } else {
            self.time = time.min(f64::from(duration)) as f32;
            if self.mode == PlaybackMode::Once && time >= f64::from(duration) {
                self.state = PlaybackState::Finished;
            }
        }
        Ok(())
    }
    pub fn pose(&self, model: &AnimatedModel) -> Result<Pose, Error> {
        self.check_model(model)?;
        self.clip.map_or_else(
            || model.rest_pose(),
            |clip| model.sample_clip(clip, self.time),
        )
    }
    fn check_model(&self, model: &AnimatedModel) -> Result<(), Error> {
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| !Arc::ptr_eq(owner, &model.owner))
        {
            return Err(invalid(
                "animation player belongs to a different animated model",
            ));
        }
        Ok(())
    }
    fn duration(&self, model: &AnimatedModel) -> Result<f32, Error> {
        self.check_model(model)?;
        let clip = self
            .clip
            .ok_or_else(|| invalid("no animation clip selected"))?;
        model
            .source
            .clips
            .get(clip as usize)
            .map(AnimationClip::duration)
            .ok_or_else(|| invalid("animation clip out of range"))
    }
}

fn playback_time(time: f32, duration: f32, mode: PlaybackMode) -> f32 {
    if mode == PlaybackMode::Loop && duration > 0.0 {
        time % duration
    } else {
        time.min(duration)
    }
}
fn add_budget(total: &mut usize, amount: usize, max: usize, label: &str) -> Result<(), Error> {
    *total = total
        .checked_add(amount)
        .ok_or_else(|| invalid(format!("{label} count overflow")))?;
    if *total > max {
        return Err(invalid(format!("{label} budget exceeded")));
    }
    Ok(())
}
fn validate_name(name: &str) -> Result<(), Error> {
    if name.len() > 1024 || name.contains('\0') {
        return Err(invalid("invalid animation name"));
    }
    Ok(())
}
fn validate_translation(value: [f32; 3]) -> Result<(), Error> {
    if !value.iter().all(|v| v.is_finite() && v.abs() <= 1.0e6) {
        return Err(invalid("invalid translation"));
    }
    Ok(())
}
fn validate_rotation(value: [f32; 4]) -> Result<(), Error> {
    if !value.iter().all(|v| v.is_finite())
        || !(0.99..=1.01).contains(&value.iter().map(|v| v * v).sum::<f32>())
    {
        return Err(invalid("invalid/nonunit rotation quaternion"));
    }
    Ok(())
}
fn validate_scale(value: [f32; 3]) -> Result<(), Error> {
    if !value
        .iter()
        .all(|v| v.is_finite() && (1.0e-6..=1.0e6).contains(v))
    {
        return Err(invalid("zero, mirrored, nonfinite, or out-of-range scale"));
    }
    Ok(())
}
fn validate_trs(trs: Trs) -> Result<(), Error> {
    validate_translation(trs.translation)?;
    validate_rotation(trs.rotation)?;
    validate_scale(trs.scale)
}
fn validate_matrix(matrix: Matrix4) -> Result<(), Error> {
    normal_matrix(matrix)?;
    if determinant(matrix) <= 0.0 {
        return Err(invalid("mirrored animation transform or blend"));
    }
    Ok(())
}
/// Column-major matrix multiplication, including affine translations.
pub fn multiply(a: Matrix4, b: Matrix4) -> Matrix4 {
    let mut out = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}
fn deform_vertex(
    primitive: &AnimatedPrimitive,
    vertex: &SkinnedVertex,
    pose: &Pose,
) -> Result<Vertex, Error> {
    let matrix = match primitive.skin {
        Some(skin) => blend_matrix(vertex, &pose.skin_matrices[skin as usize]),
        None => pose.global[primitive.node as usize],
    };
    let normal_transform = normal_matrix(matrix)?;
    if determinant(matrix) <= 0.0 {
        return Err(invalid("mirrored animation transform or blend"));
    }
    let position = transform_point(matrix, vertex.vertex.position);
    let normal = transform_vector(normal_transform, vertex.vertex.normal);
    let length2 = dot3(normal, normal);
    if !position.iter().all(|x| x.is_finite() && x.abs() <= 1.0e9)
        || !length2.is_finite()
        || length2 < 1.0e-20
    {
        return Err(invalid(
            "nonfinite deformed position or degenerate blend normal",
        ));
    }
    let inverse_length = length2.sqrt().recip();
    Ok(Vertex {
        position,
        normal: normal.map(|v| v * inverse_length),
        uv: vertex.vertex.uv,
    })
}
fn blend_matrix(vertex: &SkinnedVertex, palette: &[Matrix4]) -> Matrix4 {
    let mut result = [[0.0; 4]; 4];
    let total = vertex.weights.iter().sum::<f32>();
    for i in 0..4 {
        let weight = vertex.weights[i] / total;
        for c in 0..4 {
            for r in 0..3 {
                result[c][r] += palette[vertex.joints[i] as usize][c][r] * weight;
            }
        }
    }
    // Exact affine row avoids rejecting harmless rounding in normalized sums.
    result[3][3] = 1.0;
    result
}
fn transform_point(m: Matrix4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}
fn transform_vector(m: Matrix4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2])
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
fn normalize_quaternion(q: [f32; 4]) -> [f32; 4] {
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    q.map(|v| v / length)
}
fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let a = normalize_quaternion(a);
    let mut b = normalize_quaternion(b);
    let mut dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    if dot < 0.0 {
        b = b.map(|v| -v);
        dot = -dot;
    }
    dot = dot.clamp(-1.0, 1.0);
    let result = if dot > 0.9995 {
        std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
    } else {
        let theta = dot.acos();
        let denominator = theta.sin();
        let wa = ((1.0 - t) * theta).sin() / denominator;
        let wb = (t * theta).sin() / denominator;
        std::array::from_fn(|i| a[i] * wa + b[i] * wb)
    };
    normalize_quaternion(result)
}
fn sample_interval(times: &[f32], time: f32, interpolation: Interpolation) -> (usize, usize, f32) {
    if time <= times[0] {
        return (0, 0, 0.0);
    }
    let end = times.partition_point(|&t| t <= time);
    if end == times.len() {
        return (end - 1, end - 1, 0.0);
    }
    let start = end - 1;
    if interpolation == Interpolation::Step {
        return (start, start, 0.0);
    }
    (
        start,
        end,
        (time - times[start]) / (times[end] - times[start]),
    )
}
