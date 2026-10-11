//! Read-only, authored RoomEscape character state and absolute-tick poses.
//!
//! The model sidecar owns verified asset identity and local TRS. This small
//! map selects three distinct clips for its one PLAYER binding. Schema 1 keeps
//! fixed 1x; schema 2 explicitly authors a closed playback rate for every state.
//! Schema 3 adds a bounded view-only crossfade observed on simulation ticks.
//! State is derived from the same immutable frame as body placement. No animation
//! state is written into ECS, replay, input, or checkpoints.

use orr_bridge::FrameView;
use orr_games::room_escape_game::{RoomActor, RoomRun, PLAYER};
use orr_model::animation::{AnimatedModel, ChannelValues, Matrix4, Pose, Trs, MAX_CLIPS};
use orr_model_bindings::{
    animation_time::sample_clip_time_at_rate,
    model_bindings::{self, AnimationDescriptor, LoadedAsset, ModelKind, PlaybackMode},
};
use orr_reflect::{Guid, SceneIndex};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, mem::size_of, sync::Arc};

pub const MAX_BYTES: usize = 4096;
pub const MAX_CROSSFADE_TICKS: u32 = 120;
pub use orr_model_bindings::animation_time::PlaybackRate;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlaybackRates {
    pub searching: PlaybackRate,
    pub carrying: PlaybackRate,
    pub escaped: PlaybackRate,
}

impl<'de> Deserialize<'de> for PlaybackRates {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            searching: PlaybackRate,
            carrying: PlaybackRate,
            escaped: PlaybackRate,
        }
        struct Object;
        impl<'de> serde::de::Visitor<'de> for Object {
            type Value = PlaybackRates;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a room character speeds JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<PlaybackRates, M::Error> {
                let wire = Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(PlaybackRates {
                    searching: wire.searching,
                    carrying: wire.carrying,
                    escaped: wire.escaped,
                })
            }
        }
        deserializer.deserialize_map(Object)
    }
}

// Distinguish an absent schema-1 field from an explicitly null invalid value.
fn present_speeds<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PlaybackRates>, D::Error> {
    PlaybackRates::deserialize(deserializer).map(Some)
}

fn present_crossfade_ticks<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    u32::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Document {
    pub schema: u32,
    pub player: String,
    pub searching: u32,
    pub carrying: u32,
    pub escaped: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speeds: Option<PlaybackRates>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crossfade_ticks: Option<u32>,
}

// A derived struct also accepts positional arrays. Route exclusively through
// MapAccess while keeping duplicate fields visible to serde's typed visitor.
impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            player: String,
            searching: u32,
            carrying: u32,
            escaped: u32,
            #[serde(default, deserialize_with = "present_speeds")]
            speeds: Option<PlaybackRates>,
            #[serde(default, deserialize_with = "present_crossfade_ticks")]
            crossfade_ticks: Option<u32>,
        }
        struct Object;
        impl<'de> serde::de::Visitor<'de> for Object {
            type Value = Document;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a room character JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Document, M::Error> {
                let wire = Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                let document = Document {
                    schema: wire.schema,
                    player: wire.player,
                    searching: wire.searching,
                    carrying: wire.carrying,
                    escaped: wire.escaped,
                    speeds: wire.speeds,
                    crossfade_ticks: wire.crossfade_ticks,
                };
                document.validate().map_err(serde::de::Error::custom)?;
                Ok(document)
            }
        }
        deserializer.deserialize_map(Object)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Searching,
    Carrying,
    Escaped,
}

/// Won has precedence even for a diagnostic frame with no collected key.
pub fn state(frame: FrameView<'_>) -> State {
    let run = frame.singleton::<RoomRun>();
    if run.won != 0 {
        State::Escaped
    } else if run.key_collected != 0 {
        State::Carrying
    } else {
        State::Searching
    }
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("room character exceeds {MAX_BYTES} bytes"));
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("room character JSON: {e}"))?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|e| format!("room character JSON: {e}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("room character exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), String> {
        match (self.schema, self.speeds, self.crossfade_ticks) {
            (1, None, None) | (2, Some(_), None) => {}
            (3, Some(_), Some(ticks)) if ticks <= MAX_CROSSFADE_TICKS => {}
            _ => return Err("room character schema 1 requires legacy fields, schema 2 requires speeds, and schema 3 requires speeds plus crossfade_ticks 0..120".into()),
        }
        let player =
            Guid::parse(&self.player).map_err(|e| format!("room character player GUID: {e}"))?;
        if player.to_string() != self.player {
            return Err("room character player GUID must be canonical".into());
        }
        let clips = [self.searching, self.carrying, self.escaped];
        if clips.iter().any(|&clip| clip as usize >= MAX_CLIPS) {
            return Err("room character clip index exceeds the supported clip limit".into());
        }
        if self.searching == self.carrying
            || self.searching == self.escaped
            || self.carrying == self.escaped
        {
            return Err("room character requires three distinct clip indices".into());
        }
        Ok(())
    }

    pub fn clip(&self, state: State) -> u32 {
        match state {
            State::Searching => self.searching,
            State::Carrying => self.carrying,
            State::Escaped => self.escaped,
        }
    }

    pub fn speed(&self, state: State) -> PlaybackRate {
        let speeds = self.speeds.unwrap_or_default();
        match state {
            State::Searching => speeds.searching,
            State::Carrying => speeds.carrying,
            State::Escaped => speeds.escaped,
        }
    }

    /// Validate every mapped slot, including states not yet reached by the run.
    pub fn validate_clips(&self, model: &AnimatedModel) -> Result<(), String> {
        self.validate()?;
        for clip in [self.searching, self.carrying, self.escaped] {
            let duration = model
                .source()
                .clips
                .get(clip as usize)
                .ok_or_else(|| format!("room character clip {clip} does not exist"))?
                .duration();
            if !duration.is_finite() || duration < 0.0 {
                return Err(format!("room character clip {clip} has invalid duration"));
            }
        }
        Ok(())
    }

    /// Resolve only exact GUID generations and already-verified package assets.
    /// Additional static bindings are allowed; other animated bindings are not.
    pub fn validate_bindings(
        &self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &model_bindings::Document,
        assets: &BTreeMap<(String, String), LoadedAsset>,
    ) -> Result<(), String> {
        self.validate()?;
        models.validate()?;
        let guid = Guid::parse(&self.player)?;
        let entity = index
            .entity(&guid)
            .ok_or("room character player GUID is absent")?;
        if !frame.exists(entity)
            || index.guid(entity) != Some(&guid)
            || !frame
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == PLAYER)
            || frame
                .iter::<RoomActor>()
                .filter(|(_, actor)| actor.kind == PLAYER)
                .count()
                != 1
        {
            return Err("room character requires the exact sole PLAYER entity".into());
        }
        let binding = models
            .bindings
            .get(&self.player)
            .ok_or("room character PLAYER model binding is missing")?;
        if binding.kind != ModelKind::Animated
            || binding.animation
                != Some(AnimationDescriptor {
                    clip_index: self.searching,
                    playback: PlaybackMode::Loop,
                })
        {
            return Err("room character PLAYER binding must default to searching + Loop".into());
        }
        for (bound_guid, other) in &models.bindings {
            if bound_guid != &self.player
                && (other.kind != ModelKind::Static || other.animation.is_some())
            {
                return Err("room character permits only one animated PLAYER binding".into());
            }
            let loaded = assets
                .get(&(other.package.clone(), other.asset.clone()))
                .ok_or("room character binding is missing its verified asset")?;
            other.validate(loaded)?;
        }
        let loaded = assets
            .get(&(binding.package.clone(), binding.asset.clone()))
            .ok_or("room character PLAYER asset is missing")?;
        let model = loaded
            .animated_model()
            .ok_or("room character PLAYER model is not animated")?;
        if model.source().skins.is_empty()
            || !model
                .source()
                .primitives
                .iter()
                .any(|primitive| primitive.skin.is_some())
        {
            return Err("room character PLAYER model must contain skinned geometry".into());
        }
        self.validate_clips(model)?;
        validate_pose(model, &model.rest_pose().map_err(|e| e.to_string())?)?;
        // Admission probes all mapped states, not only the initial searching
        // clip. Each runtime sample is independently validated as well.
        for clip in [self.searching, self.carrying, self.escaped] {
            let duration = model.source().clips[clip as usize].duration();
            for time in [0.0, duration * 0.5, duration] {
                let pose = model.sample_clip(clip, time).map_err(|e| e.to_string())?;
                validate_pose(model, &pose)?;
            }
        }
        Ok(())
    }
}

/// Absolute frame tick at the state's authored rate (legacy 1x); state changes
/// never reset or offset phase.
/// `rest` explicitly distinguishes Edit/Stop from Play at tick zero. No call
/// mutates the frame, descriptor, asset, or any prior presentation state.
pub fn sample_pose(
    document: &Document,
    model: &AnimatedModel,
    frame: FrameView<'_>,
    tick_rate: u32,
    rest: bool,
) -> Result<Pose, String> {
    if tick_rate == 0 {
        return Err("room character cannot sample a zero tick rate".into());
    }
    document.validate_clips(model)?;
    let pose = if rest {
        model.rest_pose()
    } else {
        let state = state(frame);
        let clip = document.clip(state);
        let duration = model.source().clips[clip as usize].duration();
        let time = sample_clip_time_at_rate(
            frame.tick(),
            tick_rate,
            duration,
            PlaybackMode::Loop,
            document.speed(state),
        )?;
        model.sample_clip(clip, time)
    }
    .map_err(|e| format!("room character pose: {e}"))?;
    validate_pose(model, &pose)?;
    Ok(pose)
}

/// Context identifying a coherent presentation source. Revision is caller-owned:
/// explicit seek/restart/asset-authoring discontinuities replace it or reset the
/// cursor. Entity includes its generation; it is never serialized into gameplay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackContext {
    pub entity: orr_ecs::Entity,
    pub tick_rate: u32,
    pub rest: bool,
    pub revision: u64,
}
struct ObservedPose {
    document: Document,
    model: Arc<AnimatedModel>,
    context: PlaybackContext,
    tick: u64,
    state: State,
    current: Pose,
    outgoing: Option<(u64, Pose)>,
}
/// Bounded presentation history: at most two retained poses, never a clip graph.
/// A transition freezes the last visible pose and samples the incoming clip at
/// its unchanged absolute phase. An interrupted transition freezes its blended
/// result. Missing history snaps; it must never guess when a state changed.
#[derive(Default)]
pub struct PlaybackCursor {
    observed: Option<ObservedPose>,
}
impl PlaybackCursor {
    pub fn reset(&mut self) {
        self.observed = None;
    }
    pub fn observe(
        &mut self,
        document: &Document,
        model: &Arc<AnimatedModel>,
        frame: FrameView<'_>,
        context: PlaybackContext,
    ) -> Result<&Pose, String> {
        document.validate_clips(model)?;
        if context.tick_rate == 0 {
            return Err("room character cannot sample a zero tick rate".into());
        }
        let tick = frame.tick();
        let state = state(frame);
        let matching = self.observed.as_ref().filter(|old| {
            old.document == *document && Arc::ptr_eq(&old.model, model) && old.context == context
        });
        // Repeated draws do not even resample a pose or mutate transition state.
        if matching.is_some_and(|old| old.tick == tick && old.state == state) {
            return Ok(&self.observed.as_ref().unwrap().current);
        }
        let target = sample_pose(document, model, frame, context.tick_rate, context.rest)?;
        let duration = document.crossfade_ticks.unwrap_or(0);
        let continuous = matching
            .filter(|old| !context.rest && duration != 0 && old.tick.checked_add(1) == Some(tick));
        let (current, outgoing) = if let Some(old) = continuous {
            if old.state != state {
                (old.current.clone(), Some((tick, old.current.clone())))
            } else if let Some((start, outgoing)) = &old.outgoing {
                let elapsed = tick - start;
                if elapsed >= u64::from(duration) {
                    (target, None)
                } else {
                    let weight = elapsed as f32 / duration as f32;
                    let blended = model
                        .blend_poses(outgoing, &target, weight)
                        .map_err(|error| format!("room character blend: {error}"))?;
                    validate_pose(model, &blended)?;
                    (blended, Some((*start, outgoing.clone())))
                }
            } else {
                (target, None)
            }
        } else {
            (target, None)
        };
        self.observed = Some(ObservedPose {
            document: document.clone(),
            model: model.clone(),
            context,
            tick,
            state,
            current,
            outgoing,
        });
        Ok(&self.observed.as_ref().unwrap().current)
    }
    /// Render only a pose already observed for this exact source and tick.
    pub fn pose(
        &self,
        document: &Document,
        model: &Arc<AnimatedModel>,
        frame: FrameView<'_>,
        context: PlaybackContext,
    ) -> Result<&Pose, String> {
        self.observed
            .as_ref()
            .filter(|old| {
                old.document == *document
                    && Arc::ptr_eq(&old.model, model)
                    && old.context == context
                    && old.tick == frame.tick()
                    && old.state == state(frame)
            })
            .map(|old| &old.current)
            .ok_or_else(|| "room character playback has not observed this source and tick".into())
    }
}

fn validate_pose(model: &AnimatedModel, pose: &Pose) -> Result<(), String> {
    // bounds checks the complete finite CPU deformation as well as ownership,
    // hierarchy and palettes, without allocating a second copy of geometry.
    let bounds = model
        .bounds(pose)
        .map_err(|e| format!("room character pose: {e}"))?;
    if !bounds
        .min
        .iter()
        .chain(&bounds.max)
        .all(|value| value.is_finite())
    {
        return Err("room character pose bounds are not finite".into());
    }
    Ok(())
}

/// Conservative decoded storage for one immutable animated asset plus two
/// poses (current and transactional replacement). Charge all authored tracks,
/// including unused clips, and checked arithmetic before aggregate admission.
/// The caller combines this charge with static assets under the room limit.
pub fn decoded_model_bytes(model: &AnimatedModel) -> Result<usize, String> {
    let source = model.source();
    let mut size = 1024 * 1024; // Bounded containers, identities and allocator overhead.
    for primitive in &source.primitives {
        size = add_bytes(
            size,
            primitive.vertices.len(),
            size_of::<orr_model::animation::SkinnedVertex>(),
        )?;
        size = add_bytes(size, primitive.indices.len(), size_of::<u32>())?;
        size = add_bytes(size, primitive.id.len(), 1)?;
    }
    for image in &source.images {
        size = add_bytes(size, image.rgba8.len(), 1)?;
    }
    size = add_bytes(
        size,
        source.materials.len(),
        size_of::<orr_model::Material>(),
    )?;
    size = add_bytes(
        size,
        source.nodes.len(),
        size_of::<orr_model::animation::AnimatedNode>(),
    )?;
    for node in &source.nodes {
        size = add_bytes(size, node.name.len(), 1)?;
        size = add_bytes(size, node.children.len(), size_of::<u32>())?;
    }
    for skin in &source.skins {
        size = add_bytes(size, skin.name.len(), 1)?;
        size = add_bytes(size, skin.joints.len(), size_of::<u32>())?;
        size = add_bytes(size, skin.inverse_bind_matrices.len(), size_of::<Matrix4>())?;
        size = add_bytes(size, skin.joints.len(), 2 * size_of::<Matrix4>())?;
    }
    for clip in &source.clips {
        size = add_bytes(size, clip.name.len(), 1)?;
        size = add_bytes(
            size,
            clip.channels.len(),
            size_of::<orr_model::animation::AnimationChannel>(),
        )?;
        for channel in &clip.channels {
            size = add_bytes(size, channel.times.len(), size_of::<f32>())?;
            size = match &channel.values {
                ChannelValues::Translation(values) | ChannelValues::Scale(values) => {
                    add_bytes(size, values.len(), size_of::<[f32; 3]>())?
                }
                ChannelValues::Rotation(values) => {
                    add_bytes(size, values.len(), size_of::<[f32; 4]>())?
                }
            };
        }
    }
    // Both pose buffers include local transforms and global node matrices.
    add_bytes(
        size,
        source.nodes.len(),
        2 * (size_of::<Trs>() + size_of::<Matrix4>()),
    )
}

/// Additional three buffers for a positive schema-3 crossfade: the total five
/// covers retained current/outgoing, target, blend result and atomic outgoing
/// replacement. Legacy and zero-duration consumers are not charged this amount.
pub fn crossfade_extra_pose_bytes(model: &AnimatedModel) -> Result<usize, String> {
    let mut bytes = add_bytes(
        0,
        model.source().nodes.len(),
        3 * (size_of::<Trs>() + size_of::<Matrix4>()),
    )?;
    for skin in &model.source().skins {
        bytes = add_bytes(bytes, skin.joints.len(), 3 * size_of::<Matrix4>())?;
    }
    Ok(bytes)
}

fn add_bytes(total: usize, count: usize, stride: usize) -> Result<usize, String> {
    count
        .checked_mul(stride)
        .and_then(|size| total.checked_add(size))
        .ok_or_else(|| "room animated model size overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_model::animation::{
        AnimatedNode, AnimatedPrimitive, AnimatedSource, AnimationChannel, AnimationClip,
        Interpolation, SkinnedVertex,
    };
    use orr_model::{Dependency, Image, Material, Vertex, Wrap};

    fn document() -> Document {
        Document {
            schema: 1,
            player: "e_00000001".into(),
            searching: 0,
            carrying: 1,
            escaped: 2,
            speeds: None,
            crossfade_ticks: None,
        }
    }
    fn frame(tick: u64, key: u32, won: u32) -> Frame {
        let mut registry = ComponentRegistryBuilder::new();
        registry.register_singleton::<RoomRun>("run");
        let mut frame = Frame::new(registry.build());
        frame.set_tick(tick);
        frame.set_singleton(RoomRun {
            key_collected: key,
            won,
            ..RoomRun::default()
        });
        frame
    }
    fn model() -> AnimatedModel {
        AnimatedModel::new(AnimatedSource {
            format: "orr_animated_model".into(),
            version: 1,
            asset_id: "character.glb".into(),
            dependencies: vec![Dependency {
                uri: "$source".into(),
                sha256: "a".repeat(64),
            }],
            materials: vec![Material {
                base_color: [1.0; 4],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Clamp,
            }],
            images: vec![Image {
                width: 1,
                height: 1,
                rgba8: vec![255; 4],
            }],
            nodes: vec![AnimatedNode {
                name: "root".into(),
                children: vec![],
                rest: Trs::default(),
            }],
            skins: vec![],
            primitives: vec![AnimatedPrimitive {
                id: "character.glb#node=0/mesh=0/primitive=0".into(),
                node: 0,
                skin: None,
                vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                    .into_iter()
                    .map(|position| SkinnedVertex {
                        vertex: Vertex {
                            position,
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0; 2],
                        },
                        joints: [0; 4],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    })
                    .collect(),
                indices: vec![0, 1, 2],
                material: 0,
            }],
            clips: [
                ("searching", 1.0, 2.0),
                ("carrying", 10.0, 4.0),
                ("escaped", 20.0, 6.0),
            ]
            .into_iter()
            .map(|(name, start, duration)| AnimationClip {
                name: name.into(),
                channels: vec![AnimationChannel {
                    node: 0,
                    interpolation: Interpolation::Linear,
                    times: vec![0.0, duration],
                    values: ChannelValues::Translation(vec![
                        [start, 0.0, 0.0],
                        [start + duration, 0.0, 0.0],
                    ]),
                }],
            })
            .collect(),
        })
        .unwrap()
    }

    #[test]
    fn strict_object_schema_round_trip_and_size_bound() {
        let doc = document();
        let bytes = doc.to_bytes().unwrap();
        assert_eq!(Document::parse(&bytes).unwrap(), doc);
        let mut padded = bytes.clone();
        padded.resize(MAX_BYTES, b' ');
        assert!(Document::parse(&padded).is_ok());
        padded.push(b' ');
        assert!(Document::parse(&padded).is_err());
        let base = String::from_utf8(bytes).unwrap();
        for malformed in [
            "[]".into(),
            "[1,\"e_00000001\",0,1,2]".into(),
            "null".into(),
            base.replacen("\"schema\": 1", "\"schema\": 2", 1),
            base.replacen("\"schema\": 1", "\"schema\": 1, \"schema\": 1", 1),
            base.replacen("\"schema\": 1", "\"schema\": 1, \"extra\": 0", 1),
            base.replacen("\"searching\": 0", "\"searching\": 0, \"searching\": 0", 1),
            base.replacen("\"carrying\": 1", "\"carrying\": 1, \"carrying\": 1", 1),
            base.replacen("\"escaped\": 2", "\"escaped\": 2, \"escaped\": 2", 1),
            base.replace("e_00000001", "e_DEADBEEF"),
            format!("{base}{{}}"),
        ] {
            assert!(
                Document::parse(malformed.as_bytes()).is_err(),
                "{malformed}"
            );
        }
        for field in ["schema", "player", "searching", "carrying", "escaped"] {
            let mut value = serde_json::to_value(&doc).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                Document::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
            let mut value = serde_json::to_value(&doc).unwrap();
            value[field] = serde_json::Value::Null;
            assert!(
                Document::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        for field in ["searching", "carrying", "escaped"] {
            for invalid in [
                serde_json::json!(-1),
                serde_json::json!(32),
                serde_json::json!(4294967296u64),
                serde_json::json!(1.5),
                serde_json::json!("1"),
            ] {
                let mut value = serde_json::to_value(&doc).unwrap();
                value[field] = invalid;
                assert!(
                    Document::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                    "{field}"
                );
            }
        }
        for (searching, carrying, escaped) in [(0, 0, 2), (0, 1, 0), (0, 1, 1)] {
            assert!(Document {
                searching,
                carrying,
                escaped,
                ..doc.clone()
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn explicit_speeds_require_schema_two_and_strict_complete_objects() {
        let legacy = document();
        assert!(!String::from_utf8(legacy.to_bytes().unwrap())
            .unwrap()
            .contains("speeds"));
        for state in [State::Searching, State::Carrying, State::Escaped] {
            assert_eq!(legacy.speed(state), PlaybackRate::Normal);
        }
        for rate in PlaybackRate::ALL {
            let doc = Document {
                schema: 2,
                speeds: Some(PlaybackRates {
                    searching: rate,
                    carrying: rate,
                    escaped: rate,
                }),
                ..legacy.clone()
            };
            assert_eq!(Document::parse(&doc.to_bytes().unwrap()).unwrap(), doc);
            for state in [State::Searching, State::Carrying, State::Escaped] {
                assert_eq!(doc.speed(state), rate);
            }
        }
        let valid = r#"{"schema":2,"player":"e_00000001","searching":0,"carrying":1,"escaped":2,"speeds":{"searching":"1/4x","carrying":"2x","escaped":"4x"}}"#;
        let doc = Document::parse(valid.as_bytes()).unwrap();
        assert_eq!(doc.speed(State::Searching), PlaybackRate::Quarter);
        assert_eq!(doc.speed(State::Carrying), PlaybackRate::Double);
        assert_eq!(doc.speed(State::Escaped), PlaybackRate::Quadruple);
        for malformed in [
            valid.replace("\"schema\":2", "\"schema\":1"),
            valid.replace("\"schema\":2", "\"schema\":3"),
            valid.replace("\"schema\":2", "\"schema\":2,\"schema\":2"),
            valid.replace("\"speeds\":", "\"speeds\":null,\"speeds\":"),
            valid.replace("\"speeds\":", "\"extra\":0,\"speeds\":"),
            valid.replace("\"searching\":\"1/4x\"", "\"searching\":\"1/4x\",\"searching\":\"1/4x\""),
            valid.replace("\"carrying\":\"2x\"", "\"carrying\":\"2x\",\"carrying\":\"2x\""),
            valid.replace("\"escaped\":\"4x\"", "\"escaped\":\"4x\",\"escaped\":\"4x\""),
            valid.replace("\"escaped\":\"4x\"", "\"escaped\":\"4x\",\"extra\":\"1x\""),
            valid.replace("\"carrying\":1", "\"carrying\":0"),
            valid.replace("\"escaped\":2", "\"escaped\":32"),
            "[2,\"e_00000001\",0,1,2,{\"searching\":\"1x\",\"carrying\":\"1x\",\"escaped\":\"1x\"}]".into(),
        ] {
            assert!(Document::parse(malformed.as_bytes()).is_err(), "{malformed}");
        }
        let base: serde_json::Value = serde_json::from_str(valid).unwrap();
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!(["1x", "1x", "1x"]),
            serde_json::json!("1x"),
            serde_json::json!({}),
        ] {
            let mut value = base.clone();
            value["speeds"] = invalid;
            assert!(Document::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for field in ["searching", "carrying", "escaped"] {
            let mut value = base.clone();
            value["speeds"].as_object_mut().unwrap().remove(field);
            assert!(Document::parse(&serde_json::to_vec(&value).unwrap()).is_err());
            for invalid in [
                serde_json::Value::Null,
                serde_json::json!(1),
                serde_json::json!(0.25),
                serde_json::json!("3x"),
                serde_json::json!("1/8x"),
                serde_json::json!("8x"),
                serde_json::json!("0x"),
                serde_json::json!("-1x"),
                serde_json::json!({"1x":null}),
            ] {
                let mut value = base.clone();
                value["speeds"][field] = invalid;
                assert!(Document::parse(&serde_json::to_vec(&value).unwrap()).is_err());
            }
        }
        let mut no_speeds = base;
        no_speeds.as_object_mut().unwrap().remove("speeds");
        assert!(Document::parse(&serde_json::to_vec(&no_speeds).unwrap()).is_err());
        assert!(Document {
            schema: 2,
            ..legacy.clone()
        }
        .to_bytes()
        .is_err());
        assert!(Document {
            speeds: Some(PlaybackRates::default()),
            ..legacy
        }
        .to_bytes()
        .is_err());
    }

    #[test]
    fn rates_change_real_pose_without_history_or_frame_mutation() {
        let model = model();
        let doc = Document {
            schema: 2,
            speeds: Some(PlaybackRates {
                searching: PlaybackRate::Quarter,
                carrying: PlaybackRate::Double,
                escaped: PlaybackRate::Quadruple,
            }),
            ..document()
        };
        for (key, won, expected) in [(0, 0, 1.625), (1, 0, 11.0), (1, 1, 24.0)] {
            let frame = frame(150, key, won);
            let before = frame.to_bytes();
            let pose = sample_pose(&doc, &model, FrameView::of(&frame), 60, false).unwrap();
            assert_eq!(pose.local()[0].translation[0], expected);
            assert_ne!(
                pose,
                sample_pose(&document(), &model, FrameView::of(&frame), 60, false).unwrap()
            );
            assert_eq!(
                pose,
                sample_pose(&doc, &model, FrameView::of(&frame), 60, false).unwrap()
            );
            assert_eq!(frame.to_bytes(), before);
        }
        for (tick, expected) in [
            (150, 1.625),
            (30, 1.125),
            (0, 1.0),
            (u64::MAX, 2.0625),
            (30, 1.125),
            (0, 1.0),
        ] {
            let frame = frame(tick, 0, 0);
            assert_eq!(
                sample_pose(&doc, &model, FrameView::of(&frame), 60, false)
                    .unwrap()
                    .local()[0]
                    .translation[0],
                expected
            );
            assert_eq!(
                sample_pose(&doc, &model, FrameView::of(&frame), 60, true).unwrap(),
                model.rest_pose().unwrap()
            );
        }
    }

    #[test]
    fn state_precedence_is_read_only_and_uses_nonzero_flags() {
        for (key, won, want) in [
            (0, 0, State::Searching),
            (1, 0, State::Carrying),
            (42, 0, State::Carrying),
            (0, 1, State::Escaped),
            (1, 1, State::Escaped),
            (42, 99, State::Escaped),
        ] {
            let frame = frame(127, key, won);
            let before = frame.to_bytes();
            assert_eq!(state(FrameView::of(&frame)), want);
            assert_eq!(frame.to_bytes(), before);
        }
    }

    #[test]
    fn absolute_phase_survives_pause_seek_restart_and_state_switches() {
        let doc = document();
        let model = model();
        let searching = frame(150, 0, 0);
        let carrying = frame(150, 1, 0);
        let escaped = frame(150, 1, 1);
        for (frame, expected) in [(&searching, 1.5), (&carrying, 12.5), (&escaped, 22.5)] {
            let before = frame.to_bytes();
            let pose = sample_pose(&doc, &model, FrameView::of(frame), 60, false).unwrap();
            assert_eq!(pose.local()[0].translation[0], expected);
            assert_eq!(
                pose,
                sample_pose(&doc, &model, FrameView::of(frame), 60, false).unwrap()
            );
            assert_eq!(frame.to_bytes(), before);
        }
        // A prior escaped sample cannot carry tracks or a transition origin
        // into searching, a backwards seek, or restart.
        for (tick, expected) in [(150, 1.5), (30, 1.5), (0, 1.0), (u64::MAX, 1.25)] {
            let frame = frame(tick, 0, 0);
            let pose = sample_pose(&doc, &model, FrameView::of(&frame), 60, false).unwrap();
            assert_eq!(pose.local()[0].translation[0], expected);
        }
    }

    #[test]
    fn rest_is_explicit_and_differs_from_play_at_tick_zero() {
        let doc = document();
        let model = model();
        let frame = frame(0, 0, 0);
        let play = sample_pose(&doc, &model, FrameView::of(&frame), 60, false).unwrap();
        let rest = sample_pose(&doc, &model, FrameView::of(&frame), 60, true).unwrap();
        assert_eq!(play.local()[0].translation, [1.0, 0.0, 0.0]);
        assert_eq!(rest.local()[0].translation, [0.0; 3]);
        for rest in [false, true] {
            assert!(sample_pose(&doc, &model, FrameView::of(&frame), 0, rest).is_err());
        }
    }

    #[test]
    fn all_mapped_clips_are_validated_before_sampling_even_at_rest() {
        let doc = document();
        let full = model();
        let mut source = full.source().clone();
        source.clips.pop();
        let missing = AnimatedModel::new(source).unwrap();
        assert!(doc.validate_clips(&missing).is_err());
        let frame = frame(0, 0, 0);
        for rest in [false, true] {
            assert!(sample_pose(&doc, &missing, FrameView::of(&frame), 60, rest).is_err());
        }
        let mut source = full.source().clone();
        source.clips[2].channels[0].times = vec![0.0];
        source.clips[2].channels[0].values = ChannelValues::Translation(vec![[20.0, 0.0, 0.0]]);
        let zero = AnimatedModel::new(source).unwrap();
        let escaped = super::tests::frame(u64::MAX, 1, 1);
        assert_eq!(
            sample_pose(&doc, &zero, FrameView::of(&escaped), 60, false)
                .unwrap()
                .local()[0]
                .translation,
            [20.0, 0.0, 0.0]
        );
    }

    #[test]
    fn storage_budget_charges_all_tracks_and_checks_overflow() {
        let full = model();
        let full_bytes = decoded_model_bytes(&full).unwrap();
        let mut source = full.source().clone();
        source.clips.pop();
        let fewer = AnimatedModel::new(source).unwrap();
        assert!(full_bytes > decoded_model_bytes(&fewer).unwrap());
        assert!(full_bytes > 1024 * 1024);
        assert!(add_bytes(usize::MAX, 1, 1).is_err());
        assert!(add_bytes(0, usize::MAX, 2).is_err());
    }
    fn crossfade_document(ticks: u32) -> Document {
        Document {
            schema: 3,
            speeds: Some(PlaybackRates::default()),
            crossfade_ticks: Some(ticks),
            ..document()
        }
    }
    fn context() -> PlaybackContext {
        PlaybackContext {
            entity: orr_ecs::Entity {
                index: 1,
                version: 1,
            },
            tick_rate: 60,
            rest: false,
            revision: 0,
        }
    }
    fn observe(
        cursor: &mut PlaybackCursor,
        doc: &Document,
        model: &Arc<AnimatedModel>,
        tick: u64,
        key: u32,
        won: u32,
    ) -> Pose {
        let frame = frame(tick, key, won);
        let bytes = frame.to_bytes();
        let pose = cursor
            .observe(doc, model, FrameView::of(&frame), context())
            .unwrap()
            .clone();
        assert_eq!(frame.to_bytes(), bytes);
        pose
    }

    #[test]
    fn crossfade_schema_is_strict_bounded_and_preserves_legacy_bytes() {
        for ticks in [0, 1, MAX_CROSSFADE_TICKS] {
            let doc = crossfade_document(ticks);
            assert_eq!(Document::parse(&doc.to_bytes().unwrap()).unwrap(), doc);
        }
        let valid = serde_json::to_value(crossfade_document(6)).unwrap();
        for value in [
            serde_json::json!(null),
            serde_json::json!(-1),
            serde_json::json!(121),
            serde_json::json!(1.5),
            serde_json::json!("6"),
            serde_json::json!(4294967296u64),
        ] {
            let mut bad = valid.clone();
            bad["crossfade_ticks"] = value;
            assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        for field in ["speeds", "crossfade_ticks"] {
            let mut bad = valid.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        for schema in [1, 2, 4] {
            let mut bad = valid.clone();
            bad["schema"] = serde_json::json!(schema);
            assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        let bytes = String::from_utf8(crossfade_document(6).to_bytes().unwrap()).unwrap();
        assert!(Document::parse(
            bytes
                .replace(
                    "\"crossfade_ticks\": 6",
                    "\"crossfade_ticks\": 6, \"crossfade_ticks\": 6"
                )
                .as_bytes()
        )
        .is_err());
        let legacy = document().to_bytes().unwrap();
        assert!(!std::str::from_utf8(&legacy).unwrap().contains("crossfade"));
        assert_eq!(
            Document::parse(&legacy).unwrap().to_bytes().unwrap(),
            legacy
        );
    }

    #[test]
    fn crossfade_blends_absolute_target_and_interrupts_from_visible_pose() {
        let model = Arc::new(model());
        let doc = crossfade_document(4);
        let mut cursor = PlaybackCursor::default();
        let start = observe(&mut cursor, &doc, &model, 10, 0, 0);
        assert_eq!(observe(&mut cursor, &doc, &model, 11, 1, 0), start);
        let f = frame(12, 1, 0);
        let target = sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap();
        let expected = model.blend_poses(&start, &target, 0.25).unwrap();
        assert_eq!(observe(&mut cursor, &doc, &model, 12, 1, 0), expected);
        for _ in 0..10 {
            assert_eq!(observe(&mut cursor, &doc, &model, 12, 1, 0), expected);
        }
        assert_eq!(
            cursor
                .observed
                .as_ref()
                .unwrap()
                .outgoing
                .as_ref()
                .unwrap()
                .0,
            11
        );
        assert_eq!(observe(&mut cursor, &doc, &model, 13, 1, 1), expected);
        let f = frame(14, 1, 1);
        let target = sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap();
        assert_eq!(
            observe(&mut cursor, &doc, &model, 14, 1, 1),
            model.blend_poses(&expected, &target, 0.25).unwrap()
        );
        for tick in 15..=17 {
            observe(&mut cursor, &doc, &model, tick, 1, 1);
        }
        assert!(cursor.observed.as_ref().unwrap().outgoing.is_none());
        let f = frame(17, 1, 1);
        assert_eq!(
            cursor
                .pose(&doc, &model, FrameView::of(&f), context())
                .unwrap(),
            &sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap()
        );
    }

    #[test]
    fn crossfade_duration_endpoints_preserve_rates_and_phase_near_max_tick() {
        let model = Arc::new(model());
        for duration in [1, MAX_CROSSFADE_TICKS] {
            let mut doc = crossfade_document(duration);
            doc.speeds = Some(PlaybackRates {
                searching: PlaybackRate::Quarter,
                carrying: PlaybackRate::Double,
                escaped: PlaybackRate::Half,
            });
            let start = u64::MAX - u64::from(duration);
            let mut cursor = PlaybackCursor::default();
            let outgoing = observe(&mut cursor, &doc, &model, start - 1, 0, 0);
            for elapsed in 0..=duration {
                let tick = start + u64::from(elapsed);
                let f = frame(tick, 1, 0);
                let target = sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap();
                let expected = model
                    .blend_poses(&outgoing, &target, elapsed as f32 / duration as f32)
                    .unwrap();
                assert_eq!(observe(&mut cursor, &doc, &model, tick, 1, 0), expected);
            }
            assert!(cursor.observed.as_ref().unwrap().outgoing.is_none());
            // u64 wrap is a discontinuity, never a consecutive simulation tick.
            let f = frame(0, 0, 0);
            assert_eq!(
                observe(&mut cursor, &doc, &model, 0, 0, 0),
                sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap()
            );
        }
    }

    #[test]
    fn crossfade_render_lookup_rejects_unobserved_source_tick_and_state() {
        let model = Arc::new(model());
        let doc = crossfade_document(4);
        let mut cursor = PlaybackCursor::default();
        let original = frame(0, 0, 0);
        assert!(cursor
            .pose(&doc, &model, FrameView::of(&original), context())
            .is_err());
        let visible = observe(&mut cursor, &doc, &model, 0, 0, 0);
        for changed in [frame(1, 0, 0), frame(0, 1, 0)] {
            assert!(cursor
                .pose(&doc, &model, FrameView::of(&changed), context())
                .is_err());
        }
        let replacement = Arc::new((*model).clone());
        assert!(cursor
            .pose(&doc, &replacement, FrameView::of(&original), context())
            .is_err());
        let mut revision = context();
        revision.revision += 1;
        assert!(cursor
            .pose(&doc, &model, FrameView::of(&original), revision)
            .is_err());
        let mut revised = doc.clone();
        revised.crossfade_ticks = Some(3);
        assert!(cursor
            .pose(&revised, &model, FrameView::of(&original), context())
            .is_err());
        assert_eq!(
            cursor
                .pose(&doc, &model, FrameView::of(&original), context())
                .unwrap(),
            &visible
        );
    }

    #[test]
    fn crossfade_unknown_history_and_every_source_discontinuity_snap() {
        for trigger in 0..14 {
            let model = Arc::new(model());
            let mut doc = crossfade_document(10);
            let mut cursor = PlaybackCursor::default();
            observe(&mut cursor, &doc, &model, 10, 0, 0);
            observe(&mut cursor, &doc, &model, 11, 1, 0);
            let mut key = context();
            let mut tick = 12;
            let mut active_model = model.clone();
            match trigger {
                0 => tick = 9,
                1 => tick = 14,
                2 => cursor.reset(),
                3 => key.revision += 1,
                4 => key.entity.version += 1,
                5 => key.tick_rate = 30,
                6 => key.rest = true,
                7 => doc.speeds.as_mut().unwrap().carrying = PlaybackRate::Double,
                8 => active_model = Arc::new(AnimatedModel::new(model.source().clone()).unwrap()),
                9 => {
                    tick = 11;
                    doc.crossfade_ticks = Some(9);
                }
                10 => doc.player = "e_00000002".into(),
                11 => std::mem::swap(&mut doc.searching, &mut doc.carrying),
                12 => active_model = Arc::new((*model).clone()),
                13 => key.entity.index += 1,
                _ => unreachable!(),
            }
            let f = frame(tick, 1, 0);
            let expected = sample_pose(
                &doc,
                &active_model,
                FrameView::of(&f),
                key.tick_rate,
                key.rest,
            )
            .unwrap();
            assert_eq!(
                cursor
                    .observe(&doc, &active_model, FrameView::of(&f), key)
                    .unwrap(),
                &expected,
                "trigger {trigger}"
            );
            assert!(cursor.observed.as_ref().unwrap().outgoing.is_none());
        }
        // A same-tick correction is unknowable history too.
        let model = Arc::new(model());
        let doc = crossfade_document(10);
        let mut cursor = PlaybackCursor::default();
        observe(&mut cursor, &doc, &model, 10, 0, 0);
        let f = frame(10, 1, 0);
        assert_eq!(
            observe(&mut cursor, &doc, &model, 10, 1, 0),
            sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap()
        );
    }

    #[test]
    fn legacy_zero_crossfade_and_zero_length_clips_keep_exact_samples() {
        let model = Arc::new(model());
        for doc in [
            document(),
            Document {
                schema: 2,
                speeds: Some(PlaybackRates::default()),
                ..document()
            },
            crossfade_document(0),
        ] {
            let mut cursor = PlaybackCursor::default();
            for (tick, key, won) in [(0, 0, 0), (1, 1, 0), (2, 1, 1), (12, 0, 0)] {
                let f = frame(tick, key, won);
                assert_eq!(
                    observe(&mut cursor, &doc, &model, tick, key, won),
                    sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap()
                );
                assert!(cursor.observed.as_ref().unwrap().outgoing.is_none());
            }
        }
        let mut source = model.source().clone();
        for clip in &mut source.clips {
            for channel in &mut clip.channels {
                channel.times.truncate(1);
                if let ChannelValues::Translation(values) = &mut channel.values {
                    values.truncate(1);
                }
            }
        }
        let model = Arc::new(AnimatedModel::new(source).unwrap());
        let doc = crossfade_document(2);
        let mut cursor = PlaybackCursor::default();
        let before = observe(&mut cursor, &doc, &model, 0, 0, 0);
        assert_eq!(observe(&mut cursor, &doc, &model, 1, 1, 0), before);
        observe(&mut cursor, &doc, &model, 2, 1, 0);
        assert_eq!(
            observe(&mut cursor, &doc, &model, 3, 1, 0),
            model.sample_clip(doc.carrying, 0.0).unwrap()
        );
    }

    #[test]
    fn rejected_observation_keeps_previous_pose_and_crossfade_storage_is_bounded() {
        let model = Arc::new(model());
        let doc = crossfade_document(4);
        let mut cursor = PlaybackCursor::default();
        observe(&mut cursor, &doc, &model, 0, 0, 0);
        let before = observe(&mut cursor, &doc, &model, 1, 1, 0);
        let mut invalid = doc.clone();
        invalid.escaped = 31;
        let f = frame(2, 1, 0);
        assert!(cursor
            .observe(&invalid, &model, FrameView::of(&f), context())
            .is_err());
        assert_eq!(cursor.observed.as_ref().unwrap().current, before);
        assert_eq!(cursor.observed.as_ref().unwrap().tick, 1);
        let source = model.source();
        let one = source.nodes.len() * (size_of::<Trs>() + size_of::<Matrix4>())
            + source
                .skins
                .iter()
                .map(|skin| skin.joints.len() * size_of::<Matrix4>())
                .sum::<usize>();
        assert_eq!(crossfade_extra_pose_bytes(&model).unwrap(), one * 3);
        let none: Vec<&LoadedAsset> = Vec::new();
        assert!(
            crate::room_project::validate_decoded_model_budget_with_extra(
                none.iter().copied(),
                usize::MAX
            )
            .is_err()
        );
        assert!(
            crate::room_project::validate_decoded_model_budget_with_extra(
                none.iter().copied(),
                crate::room_project::MAX_DECODED_MODEL_BYTES
            )
            .is_ok()
        );
    }
}
