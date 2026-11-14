//! Blending on top of clip sampling: bone masks, additive poses, a 1D blend
//! space, a state machine with cross-fades, clip events and root motion, and
//! an animator that stacks state machines as layers. Presentation only: none
//! of this enters simulation, and time is in seconds of `f32`.
use super::{lerp3, normalize_quaternion, slerp, AnimatedModel, Pose, Trs};
use crate::{invalid, Error};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_STATES: usize = 64;
pub const MAX_TRANSITIONS: usize = 256;
pub const MAX_EVENTS: usize = 1024;
pub const MAX_LAYERS: usize = 8;

const IDENTITY_ROTATION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Hamilton product: rotating by `b` first, then by `a`.
pub(super) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub(super) fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Weight of each node in a layer, 0..=1, in the asset's node order.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneMask {
    weights: Vec<f32>,
}

impl BoneMask {
    /// Every node at full weight.
    pub fn all(model: &AnimatedModel) -> Self {
        Self { weights: vec![1.0; model.source.nodes.len()] }
    }

    /// `root` and all of its descendants at full weight, every other node at zero.
    pub fn subtree(model: &AnimatedModel, root: &str) -> Result<Self, Error> {
        let start = model.node_index(root).ok_or_else(|| invalid("bone mask root not found"))?;
        let mut weights = vec![0.0; model.source.nodes.len()];
        let mut stack = vec![start as usize];
        while let Some(node) = stack.pop() {
            weights[node] = 1.0;
            stack.extend(model.source.nodes[node].children.iter().map(|&c| c as usize));
        }
        Ok(Self { weights })
    }

    pub fn weight(&self, node: usize) -> f32 {
        self.weights.get(node).copied().unwrap_or(0.0)
    }

    fn check(&self, model: &AnimatedModel) -> Result<(), Error> {
        if self.weights.len() != model.source.nodes.len() {
            return Err(invalid("bone mask belongs to a different hierarchy"));
        }
        Ok(())
    }
}

fn check_weight(weight: f32) -> Result<(), Error> {
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        return Err(invalid("animation blend weight must be finite and in [0, 1]"));
    }
    Ok(())
}

impl AnimatedModel {
    /// Index of the node named `name`, the first one when names repeat.
    pub fn node_index(&self, name: &str) -> Option<u32> {
        self.source.nodes.iter().position(|n| n.name == name).map(|i| i as u32)
    }

    /// The pose with these local transforms, one per node in node order.
    pub fn pose_from_trs(&self, local: Vec<Trs>) -> Result<Pose, Error> {
        if local.len() != self.source.nodes.len() {
            return Err(invalid("pose needs one transform per node"));
        }
        self.pose_from_local(local)
    }

    /// `over` blended onto `base` with `weight` scaled per node by `mask`.
    pub fn blend_masked(&self, base: &Pose, over: &Pose, weight: f32, mask: &BoneMask) -> Result<Pose, Error> {
        check_weight(weight)?;
        mask.check(self)?;
        self.validate_pose(base)?;
        self.validate_pose(over)?;
        let local = base
            .local
            .iter()
            .zip(&over.local)
            .enumerate()
            .map(|(i, (from, to))| {
                let w = weight * mask.weight(i);
                Trs { translation: lerp3(from.translation, to.translation, w), rotation: slerp(from.rotation, to.rotation, w), scale: lerp3(from.scale, to.scale, w) }
            })
            .collect();
        self.pose_from_local(local)
    }

    /// Adds the difference between `additive` and `reference` to `base`, scaled
    /// by `weight` and per node by `mask`. A breathing or recoil clip made over
    /// any stance plays on top of whatever stance `base` has.
    pub fn add_pose(&self, base: &Pose, additive: &Pose, reference: &Pose, weight: f32, mask: &BoneMask) -> Result<Pose, Error> {
        check_weight(weight)?;
        mask.check(self)?;
        for pose in [base, additive, reference] {
            self.validate_pose(pose)?;
        }
        let local = base
            .local
            .iter()
            .zip(additive.local.iter().zip(&reference.local))
            .enumerate()
            .map(|(i, (base, (add, reference)))| {
                let w = weight * mask.weight(i);
                let delta = quat_mul(quat_conjugate(reference.rotation), add.rotation);
                Trs {
                    translation: std::array::from_fn(|k| base.translation[k] + (add.translation[k] - reference.translation[k]) * w),
                    rotation: normalize_quaternion(quat_mul(base.rotation, slerp(IDENTITY_ROTATION, delta, w))),
                    scale: std::array::from_fn(|k| base.scale[k] * (1.0 + (add.scale[k] / reference.scale[k] - 1.0) * w)),
                }
            })
            .collect();
        self.pose_from_local(local)
    }

    fn clip_duration(&self, clip: u32) -> Result<f32, Error> {
        self.source.clips.get(clip as usize).map(super::AnimationClip::duration).ok_or_else(|| invalid("animation clip out of range"))
    }
}

/// Clips placed along one parameter, such as idle at 0, walk at 1.5 and run at
/// 4 meters per second. Neighboring clips are sampled at the same phase and
/// blended, so footsteps stay in step while the parameter changes.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace1D {
    points: Vec<(f32, u32)>,
}

impl BlendSpace1D {
    pub fn new(model: &AnimatedModel, mut points: Vec<(f32, u32)>) -> Result<Self, Error> {
        if points.is_empty() || points.len() > super::MAX_CLIPS {
            return Err(invalid("a blend space needs 1 to 32 points"));
        }
        for &(at, clip) in &points {
            if !at.is_finite() {
                return Err(invalid("nonfinite blend space position"));
            }
            model.clip_duration(clip)?;
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        if points.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(invalid("two blend space points share a position"));
        }
        Ok(Self { points })
    }

    /// The two clips around `at` and how far `at` is from the first toward the second.
    fn around(&self, at: f32) -> ((f32, u32), (f32, u32), f32) {
        let next = self.points.partition_point(|p| p.0 <= at);
        if next == 0 {
            return (self.points[0], self.points[0], 0.0);
        }
        if next == self.points.len() {
            let last = self.points[next - 1];
            return (last, last, 0.0);
        }
        let (a, b) = (self.points[next - 1], self.points[next]);
        (a, b, (at - a.0) / (b.0 - a.0))
    }

    /// Length of one cycle at `at`, between the lengths of its two clips.
    pub fn duration(&self, model: &AnimatedModel, at: f32) -> Result<f32, Error> {
        let (a, b, t) = self.around(at);
        Ok(model.clip_duration(a.1)? * (1.0 - t) + model.clip_duration(b.1)? * t)
    }

    /// The pose at `at`, both clips sampled at `phase` (0..=1) of their length.
    pub fn sample(&self, model: &AnimatedModel, at: f32, phase: f32) -> Result<Pose, Error> {
        let (a, b, t) = self.around(at);
        let first = model.sample_clip(a.1, phase * model.clip_duration(a.1)?)?;
        if a.1 == b.1 || t == 0.0 {
            return Ok(first);
        }
        let second = model.sample_clip(b.1, phase * model.clip_duration(b.1)?)?;
        model.blend_poses(&first, &second, t.clamp(0.0, 1.0))
    }
}

/// Clips placed on a plane of two parameters, such as strafing speed and
/// forward speed. Each clip's weight falls off toward every other clip
/// (gradient band interpolation), so any point between them mixes the nearby
/// clips and a point on a clip plays that clip alone.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace2D {
    points: Vec<([f32; 2], u32)>,
}

impl BlendSpace2D {
    pub fn new(model: &AnimatedModel, points: Vec<([f32; 2], u32)>) -> Result<Self, Error> {
        if points.is_empty() || points.len() > super::MAX_CLIPS {
            return Err(invalid("a blend space needs 1 to 32 points"));
        }
        for (i, &(at, clip)) in points.iter().enumerate() {
            if !at.iter().all(|v| v.is_finite()) {
                return Err(invalid("nonfinite blend space position"));
            }
            if points[..i].iter().any(|(other, _)| *other == at) {
                return Err(invalid("two blend space points share a position"));
            }
            model.clip_duration(clip)?;
        }
        Ok(Self { points })
    }

    /// Weight of each point at `at`, summing to 1.
    pub fn weights(&self, at: [f32; 2]) -> Vec<f32> {
        let mut weights: Vec<f32> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, (p, _))| {
                let toward = [at[0] - p[0], at[1] - p[1]];
                self.points.iter().enumerate().filter(|(j, _)| *j != i).fold(1.0f32, |weight, (_, (q, _))| {
                    let edge = [q[0] - p[0], q[1] - p[1]];
                    let along = (toward[0] * edge[0] + toward[1] * edge[1]) / (edge[0] * edge[0] + edge[1] * edge[1]);
                    weight.min(1.0 - along)
                })
                .max(0.0)
            })
            .collect();
        let total: f32 = weights.iter().sum();
        if total > 0.0 {
            weights.iter_mut().for_each(|w| *w /= total);
        } else {
            // Outside every band: the nearest point plays alone.
            let nearest = self.nearest(at);
            weights.iter_mut().enumerate().for_each(|(i, w)| *w = if i == nearest { 1.0 } else { 0.0 });
        }
        weights
    }

    fn nearest(&self, at: [f32; 2]) -> usize {
        let distance = |p: [f32; 2]| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2);
        (0..self.points.len()).min_by(|&a, &b| distance(self.points[a].0).total_cmp(&distance(self.points[b].0))).expect("at least one point")
    }

    /// Length of one cycle at `at`: the clips' lengths, weighted.
    pub fn duration(&self, model: &AnimatedModel, at: [f32; 2]) -> Result<f32, Error> {
        self.weights(at).iter().zip(&self.points).try_fold(0.0, |sum, (w, (_, clip))| Ok(sum + w * model.clip_duration(*clip)?))
    }

    /// The pose at `at`, every weighted clip sampled at `phase` (0..=1) of its length.
    pub fn sample(&self, model: &AnimatedModel, at: [f32; 2], phase: f32) -> Result<Pose, Error> {
        let mut blended: Option<(Pose, f32)> = None;
        for (w, (_, clip)) in self.weights(at).into_iter().zip(&self.points).filter(|(w, _)| *w > 0.0) {
            let pose = model.sample_clip(*clip, phase * model.clip_duration(*clip)?)?;
            blended = Some(match blended {
                None => (pose, w),
                Some((so_far, total)) => (model.blend_poses(&so_far, &pose, (w / (total + w)).clamp(0.0, 1.0))?, total + w),
            });
        }
        Ok(blended.expect("weights sum to one").0)
    }
}

/// What a state plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    Clip(u32),
    /// A blend space driven by the float parameter `parameter`.
    Blend { space: BlendSpace1D, parameter: String },
    /// A 2D blend space driven by the float parameters `x` and `y`.
    Blend2D { space: BlendSpace2D, x: String, y: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationState {
    pub name: String,
    pub motion: Motion,
    pub looping: bool,
    /// Playback rate, 0..=16; 1 is the clip's own speed.
    pub speed: f32,
}

/// One test a transition makes on the machine's parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// The float parameter is above the value.
    Above(String, f32),
    /// The float parameter is below the value.
    Below(String, f32),
    /// The bool parameter has this value.
    Is(String, bool),
    /// The trigger was set; taking the transition clears it.
    Trigger(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// The state it leaves, or `None` for any state other than `to`.
    pub from: Option<usize>,
    pub to: usize,
    /// All must hold. An empty list leaves as soon as `exit_time` allows.
    pub conditions: Vec<Condition>,
    /// Cross-fade length in seconds; 0 switches at once.
    pub fade: f32,
    /// Leaves only once this share (0..=1) of the current cycle has played.
    pub exit_time: Option<f32>,
}

/// A named moment in a clip, such as a footstep, reported when playback passes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEvent {
    pub clip: u32,
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fade {
    from: usize,
    phase: f32,
    elapsed: f32,
    length: f32,
}

/// Where playback moved during one update: the phase before and after and
/// whether it wrapped past the end of a looping cycle.
#[derive(Clone, Copy)]
struct Step {
    before: f32,
    after: f32,
    wrapped: bool,
}

/// States, the transitions between them, and the playback of the current one.
#[derive(Clone, Debug, PartialEq)]
pub struct StateMachine {
    states: Vec<AnimationState>,
    transitions: Vec<Transition>,
    events: Vec<ClipEvent>,
    floats: BTreeMap<String, f32>,
    bools: BTreeMap<String, bool>,
    triggers: BTreeSet<String>,
    current: usize,
    phase: f32,
    fade: Option<Fade>,
    root: Option<usize>,
    root_motion: [f32; 3],
}

impl StateMachine {
    /// Starts in `states[0]` at the beginning of its motion.
    pub fn new(model: &AnimatedModel, states: Vec<AnimationState>, transitions: Vec<Transition>, events: Vec<ClipEvent>) -> Result<Self, Error> {
        if states.is_empty() || states.len() > MAX_STATES || transitions.len() > MAX_TRANSITIONS || events.len() > MAX_EVENTS {
            return Err(invalid("state machine size out of range"));
        }
        for state in &states {
            if !state.speed.is_finite() || !(0.0..=16.0).contains(&state.speed) {
                return Err(invalid("state speed must be finite and in [0, 16]"));
            }
            if let Motion::Clip(clip) = state.motion {
                model.clip_duration(clip)?;
            }
        }
        for t in &transitions {
            let exit_ok = t.exit_time.is_none_or(|e| e.is_finite() && (0.0..=1.0).contains(&e));
            let thresholds_ok = t.conditions.iter().all(|c| match c {
                Condition::Above(_, v) | Condition::Below(_, v) => v.is_finite(),
                _ => true,
            });
            if t.to >= states.len() || t.from.is_some_and(|f| f >= states.len()) || !t.fade.is_finite() || t.fade < 0.0 || !exit_ok || !thresholds_ok {
                return Err(invalid("invalid state machine transition"));
            }
        }
        for event in &events {
            if !event.time.is_finite() || event.time < 0.0 || event.time > model.clip_duration(event.clip)? {
                return Err(invalid("clip event outside its clip"));
            }
        }
        Ok(Self {
            states,
            transitions,
            events,
            floats: BTreeMap::new(),
            bools: BTreeMap::new(),
            triggers: BTreeSet::new(),
            current: 0,
            phase: 0.0,
            fade: None,
            root: None,
            root_motion: [0.0; 3],
        })
    }

    pub fn set_float(&mut self, name: &str, value: f32) {
        self.floats.insert(name.into(), value);
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.bools.insert(name.into(), value);
    }

    /// Sets a trigger until a transition that tests it is taken.
    pub fn trigger(&mut self, name: &str) {
        self.triggers.insert(name.into());
    }

    /// Index of the current state; during a cross-fade, the state being faded in.
    pub fn current(&self) -> usize {
        self.current
    }

    pub fn state(&self) -> &AnimationState {
        &self.states[self.current]
    }

    /// Share of the current cycle played, 0..=1.
    pub fn phase(&self) -> f32 {
        self.phase
    }

    pub fn is_fading(&self) -> bool {
        self.fade.is_some()
    }

    /// Moves the horizontal translation of `node` out of the pose: [`StateMachine::pose`]
    /// keeps the node over its rest position and [`StateMachine::take_root_motion`]
    /// returns how far it moved, for the caller to move the character instead.
    pub fn extract_root_motion(&mut self, model: &AnimatedModel, node: &str) -> Result<(), Error> {
        self.root = Some(model.node_index(node).ok_or_else(|| invalid("root motion node not found"))? as usize);
        Ok(())
    }

    /// Root translation gathered since the last call, in the root's parent space.
    pub fn take_root_motion(&mut self) -> [f32; 3] {
        std::mem::take(&mut self.root_motion)
    }

    fn duration(&self, model: &AnimatedModel, state: usize) -> Result<f32, Error> {
        match &self.states[state].motion {
            Motion::Clip(clip) => model.clip_duration(*clip),
            Motion::Blend { space, parameter } => space.duration(model, self.float(parameter)),
            Motion::Blend2D { space, x, y } => space.duration(model, [self.float(x), self.float(y)]),
        }
    }

    fn float(&self, name: &str) -> f32 {
        self.floats.get(name).copied().unwrap_or(0.0)
    }

    fn advance(&self, model: &AnimatedModel, state: usize, phase: f32, delta: f32) -> Result<Step, Error> {
        let duration = self.duration(model, state)?;
        let moved = if duration > 0.0 { delta * self.states[state].speed / duration } else { 0.0 };
        let raw = phase + moved;
        if self.states[state].looping {
            Ok(Step { before: phase, after: raw.rem_euclid(1.0), wrapped: raw >= 1.0 })
        } else {
            Ok(Step { before: phase, after: raw.min(1.0), wrapped: false })
        }
    }

    fn holds(&self, condition: &Condition) -> bool {
        match condition {
            Condition::Above(name, value) => self.float(name) > *value,
            Condition::Below(name, value) => self.float(name) < *value,
            Condition::Is(name, value) => self.bools.get(name).copied().unwrap_or(false) == *value,
            Condition::Trigger(name) => self.triggers.contains(name),
        }
    }

    /// Advances playback by `delta` seconds, then takes the first transition
    /// whose source, exit time and conditions allow it. Returns the names of
    /// the clip events passed, in time order.
    pub fn update(&mut self, model: &AnimatedModel, delta: f32) -> Result<Vec<String>, Error> {
        if !delta.is_finite() || delta < 0.0 {
            return Err(invalid("invalid animation delta"));
        }
        let step = self.advance(model, self.current, self.phase, delta)?;
        let passed = self.passed_events(model, step)?;
        self.gather_root_motion(model, step)?;
        self.phase = step.after;
        if let Some(fade) = self.fade {
            let from = self.advance(model, fade.from, fade.phase, delta)?;
            let elapsed = fade.elapsed + delta;
            self.fade = (elapsed < fade.length).then_some(Fade { phase: from.after, elapsed, ..fade });
        }
        self.take_transition(model)?;
        Ok(passed)
    }

    fn take_transition(&mut self, model: &AnimatedModel) -> Result<(), Error> {
        let current = self.current;
        let chosen = self.transitions.iter().find(|t| {
            let source = t.from.map_or(t.to != current, |f| f == current);
            source && t.exit_time.is_none_or(|e| self.phase >= e) && t.conditions.iter().all(|c| self.holds(c))
        });
        let Some(transition) = chosen.cloned() else { return Ok(()) };
        for condition in &transition.conditions {
            if let Condition::Trigger(name) = condition {
                self.triggers.remove(name);
            }
        }
        self.duration(model, transition.to)?;
        self.fade = (transition.fade > 0.0).then_some(Fade { from: current, phase: self.phase, elapsed: 0.0, length: transition.fade });
        self.current = transition.to;
        self.phase = 0.0;
        Ok(())
    }

    fn passed_events(&self, model: &AnimatedModel, step: Step) -> Result<Vec<String>, Error> {
        let Motion::Clip(clip) = self.states[self.current].motion else { return Ok(Vec::new()) };
        let duration = model.clip_duration(clip)?;
        let (before, after) = (step.before * duration, step.after * duration);
        let crossed = |time: f32| if step.wrapped { time > before || time <= after } else { time > before && time <= after };
        let mut passed: Vec<&ClipEvent> = self.events.iter().filter(|e| e.clip == clip && crossed(e.time)).collect();
        // Events after the wrap come after those before it.
        passed.sort_by(|a, b| {
            let key = |e: &ClipEvent| if step.wrapped && e.time <= after { e.time + duration } else { e.time };
            key(a).total_cmp(&key(b))
        });
        Ok(passed.into_iter().map(|e| e.name.clone()).collect())
    }

    fn root_at(&self, model: &AnimatedModel, state: usize, phase: f32) -> Result<[f32; 3], Error> {
        let root = self.root.expect("called with root motion on");
        Ok(self.motion_pose(model, state, phase)?.local[root].translation)
    }

    fn gather_root_motion(&mut self, model: &AnimatedModel, step: Step) -> Result<(), Error> {
        if self.root.is_none() {
            return Ok(());
        }
        let after = self.root_at(model, self.current, step.after)?;
        let before = self.root_at(model, self.current, step.before)?;
        let moved: [f32; 3] = if step.wrapped {
            let end = self.root_at(model, self.current, 1.0)?;
            let start = self.root_at(model, self.current, 0.0)?;
            std::array::from_fn(|k| (end[k] - before[k]) + (after[k] - start[k]))
        } else {
            std::array::from_fn(|k| after[k] - before[k])
        };
        // Vertical motion (jumps, bobbing) stays in the pose.
        self.root_motion[0] += moved[0];
        self.root_motion[2] += moved[2];
        Ok(())
    }

    fn motion_pose(&self, model: &AnimatedModel, state: usize, phase: f32) -> Result<Pose, Error> {
        match &self.states[state].motion {
            Motion::Clip(clip) => model.sample_clip(*clip, phase * model.clip_duration(*clip)?),
            Motion::Blend { space, parameter } => space.sample(model, self.float(parameter), phase),
            Motion::Blend2D { space, x, y } => space.sample(model, [self.float(x), self.float(y)], phase),
        }
    }

    /// The current pose, cross-faded from the previous state while a fade runs.
    pub fn pose(&self, model: &AnimatedModel) -> Result<Pose, Error> {
        let current = self.motion_pose(model, self.current, self.phase)?;
        let pose = match self.fade {
            Some(fade) => {
                let previous = self.motion_pose(model, fade.from, fade.phase)?;
                model.blend_poses(&previous, &current, (fade.elapsed / fade.length).clamp(0.0, 1.0))?
            }
            None => current,
        };
        let Some(root) = self.root else { return Ok(pose) };
        let rest = model.source.nodes[root].rest.translation;
        let mut local = pose.local;
        local[root].translation[0] = rest[0];
        local[root].translation[2] = rest[2];
        model.pose_from_local(local)
    }

    /// The first frame of the current motion: the reference an additive layer subtracts.
    fn reference_pose(&self, model: &AnimatedModel) -> Result<Pose, Error> {
        self.motion_pose(model, self.current, 0.0)
    }
}

/// How a layer combines with the layers under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerBlend {
    /// Replaces the pose below, by weight and mask.
    Override,
    /// Adds its change from its motion's first frame.
    Additive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationLayer {
    pub machine: StateMachine,
    pub mask: BoneMask,
    /// 0..=1. The first layer is the base and always plays at full weight.
    pub weight: f32,
    pub blend: LayerBlend,
}

/// State machines stacked as layers, such as legs walking in the base layer
/// and an upper-body wave masked on top.
#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    pub layers: Vec<AnimationLayer>,
}

impl Animator {
    pub fn new(layers: Vec<AnimationLayer>) -> Result<Self, Error> {
        if layers.is_empty() || layers.len() > MAX_LAYERS {
            return Err(invalid("an animator needs 1 to 8 layers"));
        }
        for layer in &layers {
            check_weight(layer.weight)?;
        }
        Ok(Self { layers })
    }

    /// Advances every layer; returns the events each layer passed, base layer first.
    pub fn update(&mut self, model: &AnimatedModel, delta: f32) -> Result<Vec<String>, Error> {
        let mut passed = Vec::new();
        for layer in &mut self.layers {
            passed.extend(layer.machine.update(model, delta)?);
        }
        Ok(passed)
    }

    pub fn pose(&self, model: &AnimatedModel) -> Result<Pose, Error> {
        let mut pose = self.layers[0].machine.pose(model)?;
        for layer in &self.layers[1..] {
            let over = layer.machine.pose(model)?;
            pose = match layer.blend {
                LayerBlend::Override => model.blend_masked(&pose, &over, layer.weight, &layer.mask)?,
                LayerBlend::Additive => model.add_pose(&pose, &over, &layer.machine.reference_pose(model)?, layer.weight, &layer.mask)?,
            };
        }
        Ok(pose)
    }
}
