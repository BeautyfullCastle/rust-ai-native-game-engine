//! Read-only, authored RoomEscape character state and absolute-tick poses.
//!
//! The model sidecar owns verified asset identity and local TRS. This small
//! schema-1 map only selects three distinct clips for its one PLAYER binding.
//! State is derived from the same immutable frame as body placement. There is
//! no transition clock, ECS animation state, or replay/checkpoint write-back.

use orr_bridge::FrameView;
use orr_games::room_escape_game::{RoomActor, RoomRun, PLAYER};
use orr_model::animation::{AnimatedModel, ChannelValues, Matrix4, Pose, Trs, MAX_CLIPS};
use orr_model_bindings::{
    animation_time::sample_clip_time,
    model_bindings::{self, AnimationDescriptor, LoadedAsset, ModelKind, PlaybackMode},
};
use orr_reflect::{Guid, SceneIndex};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, mem::size_of};

pub const MAX_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Document {
    pub schema: u32,
    pub player: String,
    pub searching: u32,
    pub carrying: u32,
    pub escaped: u32,
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
        }
        struct Object;
        impl<'de> serde::de::Visitor<'de> for Object {
            type Value = Document;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a room character JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Document, M::Error> {
                let wire = Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(Document {
                    schema: wire.schema,
                    player: wire.player,
                    searching: wire.searching,
                    carrying: wire.carrying,
                    escaped: wire.escaped,
                })
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
        if self.schema != 1 {
            return Err("unsupported room character schema".into());
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
        if model.source().skins.is_empty() || !model.source().primitives.iter().any(|primitive| primitive.skin.is_some()) {
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

/// Absolute frame tick at fixed 1x; state changes never reset or offset phase.
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
        let clip = document.clip(state(frame));
        let duration = model.source().clips[clip as usize].duration();
        let time = sample_clip_time(frame.tick(), tick_rate, duration, PlaybackMode::Loop)?;
        model.sample_clip(clip, time)
    }
    .map_err(|e| format!("room character pose: {e}"))?;
    validate_pose(model, &pose)?;
    Ok(pose)
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
}
