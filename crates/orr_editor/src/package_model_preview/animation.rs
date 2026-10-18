//! Explicit indexed, time-in-seconds pose inspection. No player or clock is kept.
use super::PreviewBounds;
use orr_model::animation::{AnimatedModel, Pose};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedPose {
    pub clip_index: u32,
    pub seconds: f32,
    pub pose: Pose,
    pub bounds: PreviewBounds,
}

pub(super) fn sample(
    model: &AnimatedModel,
    clip_index: u32,
    seconds: f32,
) -> Result<AcceptedPose, String> {
    // The index is identity even when two display names are equal or empty.
    let clip = model
        .source()
        .clips
        .get(clip_index as usize)
        .ok_or("animated preview clip index is out of range or no clips are available")?;
    let duration = clip.duration();
    if !seconds.is_finite() || seconds < 0.0 || seconds > duration {
        return Err(format!(
            "sample time must be finite and between 0 and {duration} seconds"
        ));
    }
    let pose = model
        .sample_clip(clip_index, seconds)
        .map_err(|error| error.to_string())?;
    let bounds =
        PreviewBounds::from_animation(model.bounds(&pose).map_err(|error| error.to_string())?)?;
    Ok(AcceptedPose {
        clip_index,
        seconds,
        pose,
        bounds,
    })
}

#[cfg(test)]
mod tests;
