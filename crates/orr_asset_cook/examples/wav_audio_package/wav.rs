//! A deliberately narrow, bounded RIFF/WAVE PCM importer, not a general decoder.
//!
//! RIFF sizes exclude their eight-byte headers; odd chunk data has one pad byte.
//! See https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-
//! and https://learn.microsoft.com/en-us/windows/win32/api/mmeapi/ns-mmeapi-waveformatex.
//! Unknown chunks are opaque and skipped (including LIST contents); they cannot
//! supply format/audio data. Padding is required but its value is not interpreted.
//! The full RIFF must match the input, with one top-level fmt and data in either
//! order. This tool accepts only fmt sizes 16 or 18 (the latter with cbSize zero).

use orr_asset_cook::authoring::{validate_payload, AssetType};
use orr_asset_cook::{CookError, Result};

/// Includes headers, ancillary chunks and padding. Callers must bound file reads.
pub const MAX_WAV_BYTES: usize = 1_048_576;
const SAMPLE_RATE: u32 = 48_000;
const MAX_FRAMES: usize = 48_000;

fn invalid(message: &str) -> CookError {
    CookError::invalid(format!("WAV: {message}"))
}

fn u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn checked_end(start: usize, length: usize, limit: usize) -> Result<usize> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| invalid("chunk offset overflow"))?;
    if end > limit {
        return Err(invalid("truncated chunk or padding"));
    }
    Ok(end)
}

fn validate_format(format: &[u8]) -> Result<()> {
    if format.len() != 16 && format.len() != 18 {
        return Err(invalid(
            "fmt chunk must contain 16 bytes, or 18 with cbSize zero",
        ));
    }
    if format.len() == 18 && u16_le(&format[16..18]) != 0 {
        return Err(invalid(
            "extended fmt data is unsupported (cbSize must be zero)",
        ));
    }
    if u16_le(&format[0..2]) != 1 {
        return Err(invalid("only integer PCM format tag 1 is supported"));
    }
    if u16_le(&format[2..4]) != 1 {
        return Err(invalid(
            "only mono audio is supported; no downmixing is performed",
        ));
    }
    if u32_le(&format[4..8]) != SAMPLE_RATE {
        return Err(invalid(
            "sample rate must be exactly 48000 Hz; no resampling is performed",
        ));
    }
    if u16_le(&format[14..16]) != 16 {
        return Err(invalid("only signed PCM16 samples are supported"));
    }
    if u16_le(&format[12..14]) != 2 || u32_le(&format[8..12]) != 96_000 {
        return Err(invalid(
            "PCM block alignment must be 2 and byte rate must be 96000",
        ));
    }
    Ok(())
}

/// Return the existing Impact payload: u32LE rate, u32LE frames, i16LE samples.
/// No sample conversion, normalization, clipping or resampling is performed.
pub fn parse_wav(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_WAV_BYTES {
        return Err(invalid("file exceeds the 1048576-byte input limit"));
    }
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid("expected a little-endian RIFF/WAVE header"));
    }
    let riff_size =
        usize::try_from(u32_le(&bytes[4..8])).map_err(|_| invalid("RIFF size overflow"))?;
    let riff_end = checked_end(8, riff_size, bytes.len())?;
    if riff_end != bytes.len() || riff_size < 4 {
        return Err(invalid(
            "RIFF size must exactly match the file; trailing bytes are forbidden",
        ));
    }
    let mut offset = 12;
    let mut format_seen = false;
    let mut data = None;
    while offset < riff_end {
        let start = checked_end(offset, 8, riff_end)?;
        let header = &bytes[offset..start];
        let size =
            usize::try_from(u32_le(&header[4..8])).map_err(|_| invalid("chunk size overflow"))?;
        let end = checked_end(start, size, riff_end)?;
        let padded_end = checked_end(end, size & 1, riff_end)?;
        match &header[0..4] {
            b"fmt " => {
                if format_seen {
                    return Err(invalid("duplicate fmt chunk"));
                }
                validate_format(&bytes[start..end])?;
                format_seen = true;
            }
            b"data" => {
                if data.is_some() {
                    return Err(invalid("duplicate data chunk"));
                }
                if size % 2 != 0 {
                    return Err(invalid("PCM16 data must contain complete two-byte frames"));
                }
                if !(1..=MAX_FRAMES).contains(&(size / 2)) {
                    return Err(invalid("frame count must be in 1..=48000"));
                }
                data = Some(&bytes[start..end]);
            }
            _ => {} // Bounded opaque ancillary data; no metadata is imported.
        }
        offset = padded_end;
    }
    if !format_seen {
        return Err(invalid("missing fmt chunk"));
    }
    let data = data.ok_or_else(|| invalid("missing data chunk"))?;
    let frames = u32::try_from(data.len() / 2).map_err(|_| invalid("frame count overflow"))?;
    let payload_len = data
        .len()
        .checked_add(8)
        .ok_or_else(|| invalid("payload size overflow"))?;
    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    payload.extend_from_slice(&frames.to_le_bytes());
    payload.extend_from_slice(data);
    // Reuse the runtime authoring contract, including the inclusive ±8192 bound.
    validate_payload(AssetType::Impact, &payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&96_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes
    }

    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_vec();
        bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 != 0 {
            bytes.push(0);
        }
        bytes
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut body = b"WAVE".to_vec();
        for bytes in chunks {
            body.extend_from_slice(bytes);
        }
        chunk(b"RIFF", &body)
    }

    fn wav(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        riff(&[chunk(b"fmt ", &format()), chunk(b"data", &data)])
    }

    #[test]
    fn preserves_samples_and_header_exactly() {
        let samples = [-8192i16, -1, 0, 1, 8192];
        let payload = parse_wav(&wav(&samples)).unwrap();
        assert_eq!(&payload[..4], &48_000u32.to_le_bytes());
        assert_eq!(&payload[4..8], &5u32.to_le_bytes());
        let expected: Vec<u8> = samples
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(&payload[8..], expected);
        validate_payload(AssetType::Impact, &payload).unwrap();
    }

    #[test]
    fn accepts_frame_limits_and_silence() {
        assert!(parse_wav(&wav(&[0])).is_ok());
        assert!(parse_wav(&wav(&vec![8192; MAX_FRAMES])).is_ok());
        assert!(parse_wav(&wav(&[])).is_err());
        assert!(parse_wav(&wav(&vec![0; MAX_FRAMES + 1])).is_err());
    }

    #[test]
    fn rejects_out_of_range_samples_without_clipping() {
        for value in [-8193, 8193, i16::MIN, i16::MAX] {
            assert!(parse_wav(&wav(&[value])).is_err());
        }
    }

    #[test]
    fn accepts_extended_pcm_and_either_chunk_order() {
        let mut fmt = format();
        fmt.extend_from_slice(&0u16.to_le_bytes());
        let input = riff(&[chunk(b"data", &[0, 0]), chunk(b"fmt ", &fmt)]);
        assert_eq!(parse_wav(&input).unwrap(), parse_wav(&wav(&[0])).unwrap());
    }

    #[test]
    fn skips_opaque_ancillary_chunks_and_consumes_nonzero_pad() {
        let mut odd = chunk(b"JUNK", &[7]);
        *odd.last_mut().unwrap() = 255;
        let input = riff(&[
            odd,
            chunk(b"fmt ", &format()),
            chunk(b"LIST", &chunk(b"data", &[1, 0])),
            chunk(b"data", &[0, 0]),
            chunk(b"fact", &[9, 8, 7, 6]),
        ]);
        assert_eq!(parse_wav(&input).unwrap(), parse_wav(&wav(&[0])).unwrap());
    }

    #[test]
    fn rejects_every_truncated_prefix() {
        let input = wav(&[-8192, 0, 8192]);
        for end in 0..input.len() {
            assert!(parse_wav(&input[..end]).is_err(), "prefix {end}");
        }
    }

    #[test]
    fn rejects_non_riff_non_wave_and_big_endian_headers() {
        for id in [b"RIFX", b"RF64", b"BW64", b"riff"] {
            let mut input = wav(&[0]);
            input[..4].copy_from_slice(id);
            assert!(parse_wav(&input).is_err());
        }
        let mut input = wav(&[0]);
        input[8..12].copy_from_slice(b"AVI ");
        assert!(parse_wav(&input).is_err());
    }

    #[test]
    fn rejects_inexact_riff_sizes_and_trailing_bytes() {
        let valid = wav(&[0]);
        for size in [0u32, 3, 4, 37, 39, u32::MAX] {
            let mut input = valid.clone();
            input[4..8].copy_from_slice(&size.to_le_bytes());
            assert!(parse_wav(&input).is_err(), "size {size}");
        }
        let mut input = valid;
        input.push(0);
        assert!(parse_wav(&input).is_err());
    }

    #[test]
    fn rejects_duplicate_or_missing_required_chunks() {
        let fmt = chunk(b"fmt ", &format());
        let data = chunk(b"data", &[0, 0]);
        for chunks in [
            vec![],
            vec![fmt.clone()],
            vec![data.clone()],
            vec![fmt.clone(), fmt.clone(), data.clone()],
            vec![fmt.clone(), data.clone(), data.clone()],
            vec![fmt, chunk(b"LIST", &data)],
        ] {
            assert!(parse_wav(&riff(&chunks)).is_err());
        }
    }

    #[test]
    fn rejects_unsupported_format_fields() {
        for (offset, replacement) in [
            (0, 3u16.to_le_bytes().to_vec()),
            (0, 0xfffeu16.to_le_bytes().to_vec()),
            (2, 2u16.to_le_bytes().to_vec()),
            (4, 44_100u32.to_le_bytes().to_vec()),
            (8, 95_999u32.to_le_bytes().to_vec()),
            (12, 4u16.to_le_bytes().to_vec()),
            (14, 8u16.to_le_bytes().to_vec()),
            (14, 24u16.to_le_bytes().to_vec()),
        ] {
            let mut fmt = format();
            fmt[offset..offset + replacement.len()].copy_from_slice(&replacement);
            assert!(parse_wav(&riff(&[chunk(b"fmt ", &fmt), chunk(b"data", &[0, 0])])).is_err());
        }
    }

    #[test]
    fn rejects_unsupported_format_lengths_and_extensions() {
        for size in [0, 15, 17, 19, 40] {
            let mut fmt = format();
            fmt.resize(size, 0);
            assert!(parse_wav(&riff(&[chunk(b"fmt ", &fmt), chunk(b"data", &[0, 0])])).is_err());
        }
        let mut fmt = format();
        fmt.extend_from_slice(&1u16.to_le_bytes());
        assert!(parse_wav(&riff(&[chunk(b"fmt ", &fmt), chunk(b"data", &[0, 0])])).is_err());
    }

    #[test]
    fn rejects_partial_samples_and_missing_padding() {
        assert!(parse_wav(&riff(&[chunk(b"fmt ", &format()), chunk(b"data", &[0])])).is_err());
        let mut missing_pad = chunk(b"JUNK", &[0]);
        missing_pad.pop();
        // Build manually: riff() would add an outer padding byte.
        let mut input = wav(&[0]);
        input.extend_from_slice(&missing_pad);
        let size = u32::try_from(input.len() - 8).unwrap();
        input[4..8].copy_from_slice(&size.to_le_bytes());
        assert!(parse_wav(&input).is_err());
    }

    #[test]
    fn rejects_short_chunk_headers_and_impossible_chunk_lengths() {
        for length in 1..8 {
            let mut input = wav(&[0]);
            input.extend_from_slice(&vec![0; length]);
            let size = u32::try_from(input.len() - 8).unwrap();
            input[4..8].copy_from_slice(&size.to_le_bytes());
            assert!(parse_wav(&input).is_err());
        }
        for size in [u32::MAX, u32::MAX - 1, 1024] {
            let mut input = wav(&[0]);
            input[40..44].copy_from_slice(&size.to_le_bytes());
            assert!(parse_wav(&input).is_err());
        }
        assert!(checked_end(usize::MAX, 1, usize::MAX).is_err());
    }

    #[test]
    fn enforces_full_input_budget_including_ancillary_data() {
        let input = riff(&[
            chunk(b"fmt ", &format()),
            chunk(b"data", &[0, 0]),
            chunk(b"JUNK", &vec![0; MAX_WAV_BYTES - 54]),
        ]);
        assert_eq!(input.len(), MAX_WAV_BYTES);
        assert!(parse_wav(&input).is_ok());
        let mut oversized = input;
        oversized.push(0);
        assert!(parse_wav(&oversized).is_err());
    }
}
