//! Fixed source-time inspection steps, independent of wall clock and binding speed.
use super::{PlaybackMode, PlaybackState, PreviewPlayers};
use orr_reflect::Guid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewStep {
    Previous,
    Next,
}

fn target_time(
    current: f32,
    duration: f32,
    mode: PlaybackMode,
    step: PreviewStep,
) -> Result<f32, String> {
    if !current.is_finite() || current < 0.0 || !duration.is_finite() || duration < 0.0 {
        return Err("Invalid animation preview step time".into());
    }
    let delta = match step {
        PreviewStep::Previous => -1.0 / 60.0,
        PreviewStep::Next => 1.0 / 60.0,
    };
    let target = (f64::from(current) + delta).max(0.0) as f32;
    if !target.is_finite() {
        return Err("Animation preview step cannot represent a finite time".into());
    }
    let legitimate_noop = duration == 0.0
        || (step == PreviewStep::Previous && current == 0.0)
        || (step == PreviewStep::Next && mode == PlaybackMode::Once && current >= duration);
    if target == current && !legitimate_noop {
        return Err("Animation preview time is too large to represent a 1/60-second step".into());
    }
    Ok(target)
}

impl PreviewPlayers {
    /// Inspect a ±1/60-second source-time step and stop automatic advancement.
    /// Previous clamps at zero; Next retains the existing Once clamp/Loop wrap.
    /// Stopped previews select their configured clip. Failures leave the original
    /// player and pose untouched; this does not edit a clip or host timeline.
    pub fn step(&mut self, guid: &Guid, direction: PreviewStep) -> Result<(), String> {
        let bound = self.get_mut(guid)?;
        let duration = bound
            .model
            .source()
            .clips
            .get(bound.clip_index as usize)
            .ok_or("Animation preview clip index is out of range")?
            .duration();
        let target = target_time(bound.player.time(), duration, bound.mode, direction)?;
        let mut candidate = bound.player.clone();
        if candidate.state() == PlaybackState::Stopped {
            candidate
                .play(&bound.model, bound.clip_index, bound.mode)
                .map_err(|e| e.to_string())?;
        }
        candidate
            .seek(&bound.model, target)
            .map_err(|e| e.to_string())?;
        candidate.pause(); // Finished Once is already stationary.
        candidate.pose(&bound.model).map_err(|e| e.to_string())?;
        bound.player = candidate;
        Ok(())
    }
}
#[cfg(test)]
#[path = "step/tests.rs"]
mod tests;
