//! Read-only, authored RoomEscape character state and absolute-tick poses.
//!
//! The model sidecar owns verified asset identity and local TRS. This small
//! map selects three distinct clips for its one PLAYER binding. Schema 1 keeps
//! fixed 1x; schema 2 explicitly authors a closed playback rate for every state.
//! Schema 3 adds a bounded view-only crossfade observed on simulation ticks.
//! Schema 4 additionally plays the escaped clip once from its observed entry
//! tick, then holds its endpoint; earlier schemas keep absolute looping.
//! Schema 6 adds an optional motion.pickup clip on a known consecutive key-entry.
//! Its once phase resumes Carrying at elapsed >= clip duration (no held endpoint).
//! State is derived from the same immutable frame as body placement. No animation
//! state is written into ECS, replay, input, or checkpoints.

use orr_bridge::FrameView;
use orr_games::room_escape_game::{RoomActor, RoomRun, PLAYER};
use orr_model::animation::{AnimatedModel, Pose, MAX_CLIPS};
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

/// Schema-5 policy for observed idle/moving clips and escaped completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Motion {
    pub searching_idle: u32,
    pub carrying_idle: u32,
    pub escaped_once: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pickup: Option<Pickup>,
}
impl<'de> Deserialize<'de> for Motion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire { searching_idle: u32, carrying_idle: u32, escaped_once: bool, #[serde(default,deserialize_with="present_pickup")] pickup:Option<Pickup> }
        struct Object;
        impl<'de> serde::de::Visitor<'de> for Object {
            type Value = Motion;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a complete room motion JSON object") }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Motion, M::Error> {
                let wire = Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(Motion { searching_idle: wire.searching_idle, carrying_idle: wire.carrying_idle, escaped_once: wire.escaped_once, pickup:wire.pickup })
            }
        }
        deserializer.deserialize_map(Object)
    }
}
fn present_motion<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Motion>, D::Error> {
    Motion::deserialize(deserializer).map(Some)
}

/// Optional observed key-pickup one-shot. Shared clips are permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Pickup { pub clip:u32, pub speed:PlaybackRate }
impl<'de> Deserialize<'de> for Pickup {
 fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
  #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Wire {clip:u32,speed:PlaybackRate}
  struct Object;
  impl<'de> serde::de::Visitor<'de> for Object {
   type Value=Pickup;
   fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str("a pickup animation object")}
   fn visit_map<M:serde::de::MapAccess<'de>>(self,map:M)->Result<Pickup,M::Error> {let w=Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;Ok(Pickup{clip:w.clip,speed:w.speed})}
  }
  d.deserialize_map(Object)
 }
}
fn present_pickup<'de,D:serde::Deserializer<'de>>(d:D)->Result<Option<Pickup>,D::Error> {Pickup::deserialize(d).map(Some)}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
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
            #[serde(default, deserialize_with = "present_motion")]
            motion: Option<Motion>,
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
                    motion: wire.motion,
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
        match (self.schema, self.speeds, self.crossfade_ticks, self.motion, self.motion.and_then(|m|m.pickup)) {
            (1, None, None, None, None) | (2, Some(_), None, None, None) => {}
            (3 | 4, Some(_), Some(ticks), None, None) | (5, Some(_), Some(ticks), Some(_), None) | (6, Some(_), Some(ticks), Some(_), Some(_)) if ticks <= MAX_CROSSFADE_TICKS => {}
            _ => return Err("room character schema 1 requires legacy fields, schema 2 requires speeds, and schemas 3/4 require speeds plus crossfade_ticks 0..120, and schema 5 additionally requires motion; schema 6 requires motion and pickup".into()),
        }
        let player =
            Guid::parse(&self.player).map_err(|e| format!("room character player GUID: {e}"))?;
        if player.to_string() != self.player {
            return Err("room character player GUID must be canonical".into());
        }
        if self.mapped_clips().any(|clip| clip as usize >= MAX_CLIPS) {
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

    /// Schema 4 explicitly opts into an observed-entry escaped one-shot.
    pub fn escaped_once(&self) -> bool { self.schema == 4 || self.motion.is_some_and(|motion| motion.escaped_once) }

    pub fn mapped_clips(&self) -> impl Iterator<Item = u32> + '_ {
        [self.searching, self.carrying, self.escaped].into_iter().chain(self.motion.into_iter().flat_map(|motion| [motion.searching_idle, motion.carrying_idle])).chain(self.motion.and_then(|m|m.pickup).map(|pickup|pickup.clip))
    }
    fn selected_clip(&self, state: State, moving: bool) -> u32 {
        match (self.motion, state, moving) {
            (Some(motion), State::Searching, false) => motion.searching_idle,
            (Some(motion), State::Carrying, false) => motion.carrying_idle,
            _ => self.clip(state),
        }
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
        for clip in self.mapped_clips() {
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
    pub fn validate_current_bindings(
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
        // Every selectable state must fit the current generic Room per-frame budget,
        // including clips not initially active. Only one is sampled per frame.
        for clip_index in self.mapped_clips() {
            let mut selected = binding.clone();
            selected.animation = Some(AnimationDescriptor {
                clip_index,
                playback: PlaybackMode::Loop,
            });
            crate::room_project::validate_animation_workload([(&selected, loaded)])?;
        }
        Ok(())
    }

    /// Full candidate admission; probing belongs here, never on each draw.
    pub fn validate_bindings(
        &self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &model_bindings::Document,
        assets: &BTreeMap<(String, String), LoadedAsset>,
    ) -> Result<(), String> {
        self.validate_current_bindings(frame, index, models, assets)?;
        let binding = &models.bindings[&self.player];
        let loaded = &assets[&(binding.package.clone(), binding.asset.clone())];
        let model = loaded
            .animated_model()
            .ok_or("room character PLAYER model is not animated")?;
        validate_pose(model, &model.rest_pose().map_err(|e| e.to_string())?)?;
        // Admission probes all mapped states, not only the initial searching
        // clip. Each runtime sample is independently validated as well.
        for clip in self.mapped_clips() {
            let duration = model.source().clips[clip as usize].duration();
            for time in [0.0, duration * 0.5, duration] {
                let pose = model.sample_clip(clip, time).map_err(|e| e.to_string())?;
                validate_pose(model, &pose)?;
            }
        }
        Ok(())
    }
}

/// Stateless absolute looping at the authored rate (legacy 1x). Schema-4
/// escaped playback requires PlaybackCursor to establish its entry tick.
/// `rest` explicitly distinguishes Edit/Stop from Play at tick zero. No call
/// mutates the frame, descriptor, asset, or any prior presentation state.
pub fn sample_pose(
    document: &Document,
    model: &AnimatedModel,
    frame: FrameView<'_>,
    tick_rate: u32,
    rest: bool,
) -> Result<Pose, String> {
    if document.escaped_once() && !rest && state(frame) == State::Escaped {
        return Err("escaped one-shot requires an observed playback cursor".into());
    }
    if document.motion.is_some() && !rest && frame.tick() != 0 {
        return Err("idle/moving animation requires an observed playback cursor".into());
    }
    sample_pose_at_tick(document, model, frame, tick_rate, rest, frame.tick(), document.selected_clip(if rest { State::Searching } else { state(frame) }, false))
}
fn sample_pose_at_tick(
    document: &Document,
    model: &AnimatedModel,
    frame: FrameView<'_>,
    tick_rate: u32,
    rest: bool,
    phase_tick: u64,
    clip: u32,
) -> Result<Pose, String> {
    if tick_rate == 0 {
        return Err("room character cannot sample a zero tick rate".into());
    }
    document.validate_clips(model)?;
    let pose = if rest {
        model.rest_pose()
    } else {
        let state = state(frame);
        let duration = model.source().clips[clip as usize].duration();
        let time = sample_clip_time_at_rate(
            phase_tick,
            tick_rate,
            duration,
            if document.escaped_once() && state == State::Escaped { PlaybackMode::Once } else { PlaybackMode::Loop },
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
    escaped_start: u64,
    pickup_start: Option<u64>,
    pickup_active: bool,
    position: Option<orr_fp::FPVec3>,
    clip: u32,
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
fn observed_position(document: &Document, frame: FrameView<'_>, context: PlaybackContext) -> Result<Option<orr_fp::FPVec3>, String> {
    if document.motion.is_none() { return Ok(None); }
    if !frame.get::<RoomActor>(context.entity).is_some_and(|actor| actor.kind == PLAYER) {
        return Err("idle/moving animation context must name the current PLAYER role".into());
    }
    frame.get::<orr_physics3d::Body>(context.entity).map(|body| Some(body.pos))
        .ok_or_else(|| "idle/moving animation requires the current PLAYER body generation".into())
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
        let position = observed_position(document, frame, context)?;
        let matching = self.observed.as_ref().filter(|old| {
            old.document == *document && Arc::ptr_eq(&old.model, model) && old.context == context
        });
        // Repeated draws do not even resample a pose or mutate transition state.
        if matching.is_some_and(|old| old.tick == tick && old.state == state && old.position == position) {
            return Ok(&self.observed.as_ref().unwrap().current);
        }
        // A forward gap in the monotonic Escaped state keeps a known entry.
        // New source/context, backward time, or first observed win starts here;
        // unknown transition history is never inferred from absolute run time.
        let escaped_start = matching.filter(|old| old.state == State::Escaped && old.tick <= tick)
            .map_or(tick, |old| old.escaped_start);
        let phase_tick = if document.escaped_once() && state == State::Escaped {
            tick - escaped_start
        } else { tick };
        let moving = matching.filter(|old| old.tick < tick).and_then(|old| old.position)
            .zip(position).is_some_and(|(old, current)| old.x != current.x || old.z != current.z);
        let pickup_start = if document.motion.and_then(|m|m.pickup).is_some() && !context.rest && state == State::Carrying {
            matching.filter(|old| !old.context.rest && old.tick < tick).and_then(|old| {
                if old.state == State::Searching && old.tick.checked_add(1) == Some(tick) { Some(tick) }
                else if old.state == State::Carrying { old.pickup_start } else { None }
            })
        } else { None };
        let pickup_sample = document.motion.and_then(|m|m.pickup).zip(pickup_start).map(|(pickup,start)| {
            let duration=model.source().clips[pickup.clip as usize].duration();
            sample_clip_time_at_rate(tick-start,context.tick_rate,duration,PlaybackMode::Once,pickup.speed)
                .map(|time|(pickup.clip,time,duration))
        }).transpose()?;
        // Completion, including zero-duration clips, immediately resumes the role loop.
        let active_pickup=pickup_sample.filter(|(_,time,duration)|time<duration);
        let clip = active_pickup.map_or_else(||document.selected_clip(state,moving),|(clip,_,_)|clip);
        let target = if let Some((clip,time,_))=active_pickup {
            let pose=model.sample_clip(clip,time).map_err(|e|format!("room pickup pose: {e}"))?;
            validate_pose(model,&pose)?;pose
        } else {sample_pose_at_tick(document, model, frame, context.tick_rate, context.rest, phase_tick, clip)?};
        let duration = document.crossfade_ticks.unwrap_or(0);
        let continuous = matching
            .filter(|old| !context.rest && duration != 0 && old.tick.checked_add(1) == Some(tick));
        let (current, outgoing) = if let Some(old) = continuous {
            if old.state != state || old.clip != clip || old.pickup_active != active_pickup.is_some() {
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
            escaped_start,
            pickup_start,
            pickup_active: active_pickup.is_some(),
            position,
            clip,
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
        let position = observed_position(document, frame, context)?;
        self.observed
            .as_ref()
            .filter(|old| {
                old.document == *document
                    && Arc::ptr_eq(&old.model, model)
                    && old.context == context
                    && old.tick == frame.tick()
                    && old.state == state(frame)
                    && old.position == position
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

/// Full positive-crossfade peak: retained current/outgoing, sampled target,
/// blend result and atomic outgoing replacement. The base source budget has no
/// pose allowance. Legacy/zero-duration callers do not pay this surcharge.
pub fn crossfade_extra_pose_bytes(model: &AnimatedModel) -> Result<usize, String> {
    let source = model.source();
    let mut bytes = size_of::<Pose>();
    for (count, stride) in [
        (
            source.nodes.len(),
            size_of::<orr_model::animation::Trs>() + size_of::<orr_model::animation::Matrix4>(),
        ),
        (
            source.skins.len(),
            size_of::<Vec<orr_model::animation::Matrix4>>(),
        ),
    ] {
        bytes = bytes
            .checked_add(
                count
                    .checked_mul(stride)
                    .ok_or("crossfade pose budget overflow")?,
            )
            .ok_or("crossfade pose budget overflow")?;
    }
    for skin in &source.skins {
        bytes = bytes
            .checked_add(
                skin.joints
                    .len()
                    .checked_mul(size_of::<orr_model::animation::Matrix4>())
                    .ok_or("crossfade pose budget overflow")?,
            )
            .ok_or("crossfade pose budget overflow")?;
    }
    bytes
        .checked_mul(5)
        .ok_or_else(|| "crossfade pose budget overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_model::animation::{
        AnimatedNode, AnimatedPrimitive, AnimatedSource, AnimationChannel, AnimationClip,
        ChannelValues, Interpolation, SkinnedVertex, Trs,
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
            motion: None,
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
        for schema in [1, 2, 6] {
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
        let one = source.nodes.len()
            * (size_of::<Trs>() + size_of::<orr_model::animation::Matrix4>())
            + source
                .skins
                .iter()
                .map(|skin| skin.joints.len() * size_of::<orr_model::animation::Matrix4>())
                .sum::<usize>();
        assert_eq!(
            crossfade_extra_pose_bytes(&model).unwrap(),
            one * 5
                + 5 * (size_of::<Pose>()
                    + source.skins.len() * size_of::<Vec<orr_model::animation::Matrix4>>())
        );
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
    mod once {
        use super::*;
        fn enabled(ticks: u32) -> Document {
            let mut doc = crossfade_document(ticks); doc.schema = 4; doc
        }
        #[test]
        fn schema4_round_trip_is_strict_and_legacy_bytes_stay_unchanged() {
            let doc = enabled(12);
            assert!(Document::parse(&doc.to_bytes().unwrap()).unwrap().escaped_once());
            for field in ["speeds", "crossfade_ticks"] {
                let mut bad = serde_json::to_value(&doc).unwrap();
                bad.as_object_mut().unwrap().remove(field);
                assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
            }
            let mut bad = serde_json::to_value(&doc).unwrap(); bad["schema"] = 6.into();
            assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
            for legacy in [document(), Document { schema: 2, speeds: Some(Default::default()), ..document() }, crossfade_document(12)] {
                let bytes = legacy.to_bytes().unwrap();
                assert!(!legacy.escaped_once());
                assert_eq!(Document::parse(&bytes).unwrap().to_bytes().unwrap(), bytes);
            }
        }
        #[test]
        fn late_escape_starts_at_zero_advances_and_holds_endpoint_without_frame_writes() {
            let doc = enabled(0); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            observe(&mut cursor, &doc, &model, 7000, 1, 0);
            assert_eq!(observe(&mut cursor, &doc, &model, 7001, 1, 1), model.sample_clip(2, 0.0).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, 7031, 1, 1), model.sample_clip(2, 0.5).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, 9000, 1, 1), model.sample_clip(2, 6.0).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, u64::MAX, 1, 1), model.sample_clip(2, 6.0).unwrap());
            let f = frame(u64::MAX, 1, 1);
            assert_eq!(cursor.pose(&doc, &model, FrameView::of(&f), context()).unwrap(), &model.sample_clip(2, 6.0).unwrap());
        }
        #[test]
        fn authored_rate_applies_to_entry_elapsed_ticks_and_legacy_keeps_looping() {
            let mut doc = enabled(0); doc.speeds.as_mut().unwrap().escaped = PlaybackRate::Double;
            let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            observe(&mut cursor, &doc, &model, 1000, 1, 1);
            assert_eq!(observe(&mut cursor, &doc, &model, 1030, 1, 1), model.sample_clip(2, 1.0).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, 1180, 1, 1), model.sample_clip(2, 6.0).unwrap());
            doc.schema = 3;
            let f = frame(1180, 1, 1);
            assert_eq!(observe(&mut cursor, &doc, &model, 1180, 1, 1), sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap());
        }
        #[test]
        fn first_unknown_escape_backward_time_reset_and_revision_restart_the_once_clock() {
            let doc = enabled(0); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            observe(&mut cursor, &doc, &model, 40, 1, 0);
            assert_eq!(observe(&mut cursor, &doc, &model, 1000, 1, 1), model.sample_clip(2, 0.0).unwrap());
            observe(&mut cursor, &doc, &model, 1060, 1, 1);
            assert_eq!(observe(&mut cursor, &doc, &model, 1001, 1, 1), model.sample_clip(2, 0.0).unwrap());
            let f = frame(1002, 1, 1); let mut revision = context(); revision.revision = 1;
            assert_eq!(cursor.observe(&doc, &model, FrameView::of(&f), revision).unwrap(), &model.sample_clip(2, 0.0).unwrap());
            cursor.reset();
            assert_eq!(observe(&mut cursor, &doc, &model, 5000, 1, 1), model.sample_clip(2, 0.0).unwrap());
        }
        #[test]
        fn crossfade_targets_the_entry_clock_and_then_holds_the_final_pose() {
            let doc = enabled(2); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            let old = observe(&mut cursor, &doc, &model, 1000, 1, 0);
            assert_eq!(observe(&mut cursor, &doc, &model, 1001, 1, 1), old);
            let target = model.sample_clip(2, 1.0 / 60.0).unwrap();
            assert_eq!(observe(&mut cursor, &doc, &model, 1002, 1, 1), model.blend_poses(&old, &target, 0.5).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, 1003, 1, 1), model.sample_clip(2, 2.0 / 60.0).unwrap());
            assert_eq!(observe(&mut cursor, &doc, &model, 2000, 1, 1), model.sample_clip(2, 6.0).unwrap());
        }
        #[test]
        fn stateless_escape_fails_closed_rest_and_repeated_draws_keep_their_contract() {
            let doc = enabled(0); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            let f = frame(1000, 1, 1);
            assert!(sample_pose(&doc, &model, FrameView::of(&f), 60, false).unwrap_err().contains("observed playback cursor"));
            assert_eq!(sample_pose(&doc, &model, FrameView::of(&f), 60, true).unwrap(), model.rest_pose().unwrap());
            assert!(cursor.pose(&doc, &model, FrameView::of(&f), context()).is_err());
            let first = observe(&mut cursor, &doc, &model, 1000, 1, 1);
            assert_eq!(observe(&mut cursor, &doc, &model, 1000, 1, 1), first);
            let mut invalid = doc.clone(); invalid.escaped = 31;
            assert!(cursor.observe(&invalid, &model, FrameView::of(&f), context()).is_err());
            assert_eq!(cursor.pose(&doc, &model, FrameView::of(&f), context()).unwrap(), &first);
        }
    }

    mod motion_tests {
        use super::*;
        fn doc(ticks: u32) -> Document {
            Document { schema: 5, motion: Some(Motion { searching_idle: 2, carrying_idle: 0, escaped_once: true, pickup:None }), ..crossfade_document(ticks) }
        }
        fn observed_frame(tick: u64, x: i32, key: u32, won: u32) -> (Frame, PlaybackContext) {
            use crate::room_game::{RoomConfig, RoomEscapeV1};
            let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
            let mut frame = sim.frame().clone();
            let entity = frame.entities().find(|&entity| frame.get::<RoomActor>(entity).is_some_and(|actor| actor.kind == PLAYER)).unwrap();
            frame.get_mut::<orr_physics3d::Body>(entity).unwrap().pos.x = orr_fp::FP::from_int(x);
            frame.set_tick(tick); frame.set_singleton(RoomRun { key_collected: key, won, ..Default::default() });
            (frame, PlaybackContext { entity, ..context() })
        }

        fn observe_motion(cursor: &mut PlaybackCursor, doc: &Document, model: &Arc<AnimatedModel>, tick: u64, x: i32, key: u32, won: u32) -> Pose {
            let (frame, context) = observed_frame(tick, x, key, won); let before = frame.to_bytes();
            let pose = cursor.observe(doc, model, FrameView::of(&frame), context).unwrap().clone();
            assert_eq!(frame.to_bytes(), before); pose
        }
        #[test]
        fn schema5_requires_strict_complete_motion_and_preserves_legacy_serialization() {
            let document = doc(0); let bytes = document.to_bytes().unwrap();
            assert_eq!(Document::parse(&bytes).unwrap(), document);
            for value in [serde_json::json!(null), serde_json::json!([]), serde_json::json!({"searching_idle":2,"carrying_idle":0}), serde_json::json!({"searching_idle":2,"carrying_idle":0,"escaped_once":null}), serde_json::json!({"searching_idle":32,"carrying_idle":0,"escaped_once":true}), serde_json::json!({"searching_idle":2,"carrying_idle":0,"escaped_once":true,"other":0})] {
                let mut bad = serde_json::to_value(&document).unwrap(); bad["motion"] = value;
                assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
            }
            let text = String::from_utf8(bytes).unwrap();
            assert!(Document::parse(text.replace("\"escaped_once\": true", "\"escaped_once\": true, \"escaped_once\": false").as_bytes()).is_err());
            for schema in [1,2,3,4] {
                let mut legacy = if schema == 1 { super::document() } else if schema == 2 { Document { schema: 2, speeds: Some(Default::default()), ..super::document() } } else { crossfade_document(0) };
                legacy.schema = schema; let old = legacy.to_bytes().unwrap();
                assert!(!String::from_utf8(old.clone()).unwrap().contains("motion"));
                assert_eq!(Document::parse(&old).unwrap().to_bytes().unwrap(), old);
                legacy.motion = document.motion; assert!(legacy.validate().is_err());
            }
            let mut missing = serde_json::to_value(&document).unwrap(); missing.as_object_mut().unwrap().remove("motion");
            assert!(Document::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
        }
        #[test]
        fn observed_displacement_and_stop_select_both_role_clips_and_repeated_draw_keeps_moving_pose() {
            let doc = doc(0); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            assert_eq!(observe_motion(&mut cursor,&doc,&model,0,0,0,0),model.sample_clip(2,0.0).unwrap());
            let moving = observe_motion(&mut cursor,&doc,&model,1,1,0,0);
            assert_eq!(moving,model.sample_clip(0,1.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,1,1,0,0),moving);
            assert_eq!(observe_motion(&mut cursor,&doc,&model,2,1,0,0),model.sample_clip(2,2.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,3,2,1,0),model.sample_clip(1,3.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,4,2,1,0),model.sample_clip(0,4.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,5,2,1,1),model.sample_clip(2,0.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,500,2,1,1),model.sample_clip(2,6.0).unwrap());
        }
        #[test]
        fn crossfade_triggers_on_motion_clip_change_without_a_game_state_change() {
            let doc = doc(2); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            let idle = observe_motion(&mut cursor,&doc,&model,0,0,0,0);
            assert_eq!(observe_motion(&mut cursor,&doc,&model,1,1,0,0),idle);
            let target = model.sample_clip(0,2.0/60.0).unwrap();
            assert_eq!(observe_motion(&mut cursor,&doc,&model,2,2,0,0),model.blend_poses(&idle,&target,0.5).unwrap());
            let moving = observe_motion(&mut cursor,&doc,&model,3,3,0,0);
            assert_eq!(moving,model.sample_clip(0,3.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,4,3,0,0),moving);
        }
        #[test]
        fn forward_gaps_use_net_displacement_backwards_or_new_context_starts_idle() {
            let doc = doc(0); let model = Arc::new(model()); let mut cursor = PlaybackCursor::default();
            observe_motion(&mut cursor,&doc,&model,10,0,0,0);
            assert_eq!(observe_motion(&mut cursor,&doc,&model,100,1,0,0),model.sample_clip(0,100.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,101,1,0,0),model.sample_clip(2,101.0/60.0).unwrap());
            assert_eq!(observe_motion(&mut cursor,&doc,&model,50,2,0,0),model.sample_clip(2,50.0/60.0).unwrap());
            let (frame,mut context) = observed_frame(51,3,0,0); context.revision=1;
            assert_eq!(cursor.observe(&doc,&model,FrameView::of(&frame),context).unwrap(),&model.sample_clip(2,51.0/60.0).unwrap());
        }
        #[test]
        fn missing_or_stale_body_and_unavailable_idle_clip_keep_previous_cursor() {
            let doc = doc(0); let model=Arc::new(model()); let mut cursor=PlaybackCursor::default();
            let old=observe_motion(&mut cursor,&doc,&model,0,0,0,0);
            let (frame,context)=observed_frame(1,1,0,0);
            let mut stale=context; stale.entity.version+=1;
            assert!(cursor.observe(&doc,&model,FrameView::of(&frame),stale).is_err());
            assert_eq!(cursor.observed.as_ref().unwrap().current,old);
            let mut wrong=context; wrong.entity=frame.entities().find(|&entity| frame.get::<RoomActor>(entity).is_some_and(|actor|actor.kind==orr_games::room_escape_game::KEY)).unwrap();
            assert!(cursor.observe(&doc,&model,FrameView::of(&frame),wrong).is_err());
            assert_eq!(cursor.observed.as_ref().unwrap().current,old);
            let mut invalid=doc.clone(); invalid.motion.as_mut().unwrap().searching_idle=31;
            assert!(invalid.validate_clips(&model).is_err());
            assert!(cursor.observe(&invalid,&model,FrameView::of(&frame),context).is_err());
            assert_eq!(cursor.observed.as_ref().unwrap().current,old);
            assert_eq!(doc.mapped_clips().collect::<Vec<_>>(),vec![0,1,2,2,0]);
        }
        #[test]
        fn stateless_initial_idle_rest_and_exact_observed_position_are_required() {
            let doc=doc(0); let model=Arc::new(model()); let mut cursor=PlaybackCursor::default();
            let (initial,_)=observed_frame(0,0,0,0);
            assert_eq!(sample_pose(&doc,&model,FrameView::of(&initial),60,false).unwrap(),model.sample_clip(2,0.0).unwrap());
            observe_motion(&mut cursor,&doc,&model,1,1,0,0);
            let (changed,context)=observed_frame(1,2,0,0);
            assert!(cursor.pose(&doc,&model,FrameView::of(&changed),context).is_err());
            assert!(sample_pose(&doc,&model,FrameView::of(&changed),60,false).is_err());
            assert_eq!(sample_pose(&doc,&model,FrameView::of(&changed),60,true).unwrap(),model.rest_pose().unwrap());
        }
    }

    mod pickup_tests { include!("room_character/pickup_tests.rs"); }

}
