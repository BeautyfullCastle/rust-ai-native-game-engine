//! Bounded frame-sequence authoring using the existing version-1 clip schema.
use crate::{
    invalid, ClipSource, Error, Frame, PlaybackMode, MAX_CLIP_ID_BYTES, MAX_FRAMES_PER_CLIP,
};

/// Expand forward frames into a looping ping-pong sequence: positions
/// `0, 1, ..., n-1, n-2, ..., 1`. Endpoints occur once per cycle; one or two
/// input frames are unchanged. Each visited frame keeps its region and duration,
/// including repeated region IDs. The input is never modified.
///
/// Checks the nonblank ID's UTF-8 byte length, expanded frame count, positive
/// durations and cycle-duration overflow before allocating generated frames.
/// This returns authoring data: [`crate::SpriteDocument::new`] must still check
/// region references, duplicate clip IDs and document-wide limits. A sampled
/// `frame_index` is the expanded sequence index, not the forward input index.
pub fn ping_pong_clip(id: String, forward_frames: &[Frame]) -> Result<ClipSource, Error> {
    if id.trim().is_empty() || id.len() > MAX_CLIP_ID_BYTES {
        return Err(invalid("ping-pong clip ID is empty or exceeds byte limit"));
    }
    if forward_frames.is_empty() {
        return Err(invalid("ping-pong clip requires forward frames"));
    }
    let interior_count = forward_frames.len().saturating_sub(2);
    let expanded_count = forward_frames
        .len()
        .checked_add(interior_count)
        .filter(|&count| count <= MAX_FRAMES_PER_CLIP)
        .ok_or_else(|| invalid("ping-pong frame count exceeds schema limit or overflows"))?;
    let forward_duration_ms = forward_frames.iter().try_fold(0u64, |total, frame| {
        if frame.duration_ms == 0 {
            return Err(invalid("ping-pong frame duration must be positive"));
        }
        total
            .checked_add(frame.duration_ms)
            .ok_or_else(|| invalid("ping-pong cycle duration overflow"))
    })?;
    // Add only the return-leg interior, not twice the whole forward duration.
    // In particular, singleton and two-frame cycles may total u64::MAX.
    let return_frames = forward_frames.iter().rev().skip(1).take(interior_count);
    return_frames
        .clone()
        .try_fold(forward_duration_ms, |total, frame| {
            total
                .checked_add(frame.duration_ms)
                .ok_or_else(|| invalid("ping-pong cycle duration overflow"))
        })?;

    let mut frames = Vec::with_capacity(expanded_count);
    frames.extend_from_slice(forward_frames);
    frames.extend(return_frames.copied());
    Ok(ClipSource {
        id,
        mode: PlaybackMode::Loop,
        frames,
    })
}
