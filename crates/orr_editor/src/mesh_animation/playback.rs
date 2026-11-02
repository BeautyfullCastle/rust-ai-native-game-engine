//! Presentation-only, bounded idle/move crossfade. No simulation state is written.
//! Callers must supply a new discontinuity revision for explicit seek/restart.
//! Missing intermediate ticks snap: this cursor does not invent unseen history.
use super::{Settings, Snapshot};
use crate::viewport3d::AnimatedPlacement;
use orr_model::animation::{AnimatedModel, Matrix4, Pose, Trs};
use orr_remote::mesh_navigation::MeshView;
use std::{mem::size_of, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaybackContext {
    pub(crate) scene_epoch: u64,
    pub(crate) sim_epoch: u64,
    pub(crate) discontinuity: u64,
    /// Edit/rest sampling has no transition history.
    pub(crate) rest: bool,
}

struct Observed {
    model: Arc<AnimatedModel>,
    settings: Settings,
    graph: [u8; 32],
    entity: orr_ecs::Entity,
    context: PlaybackContext,
    tick: u64,
    clip: u32,
    instance: Matrix4,
    current: Pose,
    outgoing: Option<(u64, Pose)>,
}

#[derive(Default)]
pub(crate) struct PlaybackCursor {
    observed: Option<Observed>,
}
impl PlaybackCursor {
    pub(crate) fn reset(&mut self) {
        self.observed = None;
    }

    pub(crate) fn observe(
        &mut self,
        snapshot: &Snapshot,
        view: &MeshView,
        tick: u64,
        context: PlaybackContext,
    ) -> Result<AnimatedPlacement, String> {
        let result = self.observe_inner(snapshot, view, tick, context);
        if result.is_err() {
            // An invalid observation breaks continuity, never bridges a failure.
            self.reset();
        }
        result
    }

    fn observe_inner(
        &mut self,
        snapshot: &Snapshot,
        view: &MeshView,
        tick: u64,
        context: PlaybackContext,
    ) -> Result<AnimatedPlacement, String> {
        let mut target = snapshot.placement_for_view(view, tick)?;
        let duration = snapshot.settings.crossfade_ticks.unwrap_or(0);
        if duration == 0 || context.rest {
            self.reset();
            return Ok(target);
        }
        validate_pose_budget(&snapshot.model)?;
        let (_, clip) = snapshot.selected_clip_for_view(view);
        let matching = self.observed.as_ref().filter(|old| {
            Arc::ptr_eq(&old.model, &snapshot.model)
                && old.settings == snapshot.settings
                && old.graph == snapshot.graph_revision
                && old.entity == view.agent.entity
                && old.context == context
        });
        if let Some(old) = matching
            .filter(|old| old.tick == tick && old.clip == clip && old.instance == target.instance)
        {
            target.pose = old.current.clone();
            return Ok(target);
        }
        let continuous = matching.filter(|old| old.tick.checked_add(1) == Some(tick));
        let (pose, outgoing) = if let Some(old) = continuous {
            if old.clip != clip {
                (old.current.clone(), Some((tick, old.current.clone())))
            } else if let Some((start, outgoing)) = &old.outgoing {
                let elapsed = tick - start;
                if elapsed >= u64::from(duration) {
                    (target.pose, None)
                } else {
                    let pose = snapshot
                        .model
                        .blend_poses(outgoing, &target.pose, elapsed as f32 / duration as f32)
                        .map_err(|error| format!("blend mesh animation: {error}"))?;
                    (pose, Some((*start, outgoing.clone())))
                }
            } else {
                (target.pose, None)
            }
        } else {
            (target.pose, None)
        };
        snapshot
            .model
            .validate_pose(&pose)
            .map_err(|error| format!("validate blended mesh animation: {error}"))?;
        super::validate_pose_bounds(&snapshot.model, &pose)?;
        let bounds = snapshot
            .model
            .bounds(&pose)
            .map_err(|error| format!("bound blended mesh animation: {error}"))?;
        super::validate_instance_bounds(bounds, target.instance)?;
        self.observed = Some(Observed {
            model: snapshot.model.clone(),
            settings: snapshot.settings,
            graph: snapshot.graph_revision,
            entity: view.agent.entity,
            context,
            tick,
            clip,
            instance: target.instance,
            current: pose.clone(),
            outgoing,
        });
        target.pose = pose;
        Ok(target)
    }
}

/// Six simultaneous poses cover retained history, target, result and atomic
/// history/returned placement clones. Independent of geometry/texture budget.
/// Checked arithmetic rejects pathological skins before cursor allocations.
pub(super) fn validate_pose_budget(model: &AnimatedModel) -> Result<(), String> {
    let source = model.source();
    let mut bytes = size_of::<Pose>();
    for (count, stride) in [
        (source.nodes.len(), size_of::<Trs>() + size_of::<Matrix4>()),
        (source.skins.len(), size_of::<Vec<Matrix4>>()),
    ] {
        bytes = bytes
            .checked_add(
                count
                    .checked_mul(stride)
                    .ok_or("mesh crossfade pose budget overflow")?,
            )
            .ok_or("mesh crossfade pose budget overflow")?;
    }
    for skin in &source.skins {
        bytes = bytes
            .checked_add(
                skin.joints
                    .len()
                    .checked_mul(size_of::<Matrix4>())
                    .ok_or("mesh crossfade pose budget overflow")?,
            )
            .ok_or("mesh crossfade pose budget overflow")?;
    }
    if bytes.checked_mul(6).is_none_or(|peak| peak > 1024 * 1024) {
        return Err("mesh crossfade exceeds 1 MiB presentation pose budget".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
