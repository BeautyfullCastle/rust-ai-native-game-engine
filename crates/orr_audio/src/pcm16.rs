//! Bounded presentation-only decoder for the existing `view.impact_pcm16/1`
//! payload: little-endian u32 sample rate/count, then mono little-endian i16.
//! This module performs no IO, opens no mixer and cannot modify simulation.

use crate::{AudioError, Clip};

pub const SAMPLE_RATE: u32 = 48_000;
pub const MAX_FRAMES: u32 = 48_000;
pub const MAX_PEAK: i16 = 8_192;
const HEADER_BYTES: usize = 8;
const SAMPLE_BYTES: usize = 2;

/// Validate the entire payload without allocating decoded samples, returning
/// its frame count. Rejects trailing bytes, empty clips and excessive peaks.
/// Successful validation allocates nothing, allowing whole-bank preflight.
pub fn validate(bytes: &[u8]) -> Result<u32, AudioError> {
    if !(HEADER_BYTES + SAMPLE_BYTES..=HEADER_BYTES + MAX_FRAMES as usize * SAMPLE_BYTES)
        .contains(&bytes.len())
    {
        return Err(AudioError("PCM16 payload length is out of bounds".into()));
    }
    let rate = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if rate != SAMPLE_RATE || !(1..=MAX_FRAMES).contains(&count) {
        return Err(AudioError(
            "PCM16 needs 48000 Hz and 1..=48000 mono frames".into(),
        ));
    }
    if bytes.len() != HEADER_BYTES + count as usize * SAMPLE_BYTES {
        return Err(AudioError(
            "PCM16 frame count and exact length differ".into(),
        ));
    }
    if bytes[HEADER_BYTES..]
        .chunks_exact(SAMPLE_BYTES)
        .any(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() > MAX_PEAK as u16)
    {
        return Err(AudioError("PCM16 sample peak exceeds 8192".into()));
    }
    Ok(count)
}

/// Validate before allocating and convert mono i16 into owned stereo f32 PCM.
/// One clip retains at most 48000 stereo frames; callers preflight their bank's
/// aggregate budget separately. No asset fallback, resampling or device IO.
pub fn decode(bytes: &[u8]) -> Result<Clip, AudioError> {
    validate(bytes)?;
    let frames = bytes[HEADER_BYTES..]
        .chunks_exact(SAMPLE_BYTES)
        .map(|sample| {
            let value = f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0;
            [value, value]
        })
        .collect();
    Clip::from_stereo(SAMPLE_RATE, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(rate: u32, count: u32, samples: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_BYTES + samples.len() * SAMPLE_BYTES);
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn accepts_exact_frame_and_peak_boundaries_and_owns_stereo() {
        let mut bytes = payload(SAMPLE_RATE, 3, &[-MAX_PEAK, 0, MAX_PEAK]);
        assert_eq!(validate(&bytes).unwrap(), 3);
        let clip = decode(&bytes).unwrap();
        bytes.fill(0);
        assert_eq!(clip.0.sample_rate, SAMPLE_RATE);
        for (frame, sample) in clip.0.frames.iter().zip([-0.25, 0.0, 0.25]) {
            assert_eq!(frame.left, sample);
            assert_eq!(frame.right, sample);
        }
        for count in [1, MAX_FRAMES] {
            let bytes = payload(SAMPLE_RATE, count, &vec![MAX_PEAK; count as usize]);
            assert_eq!(validate(&bytes).unwrap(), count);
            assert_eq!(decode(&bytes).unwrap().0.frames.len(), count as usize);
        }
    }

    #[test]
    fn rejects_malformed_headers_lengths_counts_and_peaks_without_panicking() {
        let valid = payload(SAMPLE_RATE, 2, &[0, 1]);
        for len in 0..valid.len() {
            assert!(validate(&valid[..len]).is_err());
            assert!(decode(&valid[..len]).is_err());
        }
        for rate in [0, 8_000, 44_100, 48_001, 192_000, u32::MAX] {
            assert!(decode(&payload(rate, 1, &[0])).is_err());
        }
        for count in [0, 2, MAX_FRAMES, MAX_FRAMES + 1, u32::MAX] {
            assert!(decode(&payload(SAMPLE_RATE, count, &[0])).is_err());
        }
        for peak in [i16::MIN, -MAX_PEAK - 1, MAX_PEAK + 1, i16::MAX] {
            assert!(decode(&payload(SAMPLE_RATE, 1, &[peak])).is_err());
        }
        for extra in [1, 2, 10] {
            let mut trailing = valid.clone();
            trailing.resize(trailing.len() + extra, 0);
            assert!(decode(&trailing).is_err());
        }
        let too_long = payload(
            SAMPLE_RATE,
            MAX_FRAMES + 1,
            &vec![0; MAX_FRAMES as usize + 1],
        );
        assert!(decode(&too_long).is_err());
    }
}
