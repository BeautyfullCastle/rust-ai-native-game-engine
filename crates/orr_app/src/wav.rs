//! WAV files as sounds: [`clip_from_wav`] and `App::sound_file`.
//!
//! Supported: RIFF/WAVE with 16-bit integer PCM or 32-bit float samples, one channel (played on
//! both sides) or two channels, 8000 to 192000 Hz, at most 10 seconds (the limit of a `Clip`).
//! Chunks other than `fmt ` and `data` are skipped. Everything else is refused with a message
//! that says why. The clip keeps the sample rate of the file: the mixer resamples it to the rate of
//! the output when it plays, so a 22050 Hz file lasts as long on a 48000 Hz device as on any other.

use std::fmt;

use orr_audio::Clip;

/// The most bytes of a WAV file that is read (10 seconds of 192 kHz stereo float fit in it).
pub const MAX_WAV_BYTES: usize = 16 << 20;

/// Why a WAV file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavError(pub String);

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WavError {}

fn bad(message: impl Into<String>) -> WavError {
    WavError(message.into())
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Int16,
    Float32,
}

struct Format {
    encoding: Encoding,
    channels: usize,
    rate: u32,
}

const TAG_PCM: u16 = 1;
const TAG_FLOAT: u16 = 3;
const TAG_EXTENSIBLE: u16 = 0xFFFE;

fn parse_format(chunk: &[u8]) -> Result<Format, WavError> {
    if chunk.len() < 16 {
        return Err(bad("the fmt chunk is too short"));
    }
    let mut tag = u16_at(chunk, 0);
    let channels = usize::from(u16_at(chunk, 2));
    let rate = u32_at(chunk, 4);
    let block_align = usize::from(u16_at(chunk, 12));
    let bits = u16_at(chunk, 14);
    if tag == TAG_EXTENSIBLE {
        // 2 bytes cbSize, 2 valid bits, 4 channel mask, then the 16-byte sub format: its first two bytes are the tag.
        if chunk.len() < 40 {
            return Err(bad("the extensible fmt chunk is too short"));
        }
        tag = u16_at(chunk, 24);
    }
    let encoding = match (tag, bits) {
        (TAG_PCM, 16) => Encoding::Int16,
        (TAG_FLOAT, 32) => Encoding::Float32,
        (TAG_PCM, other) => return Err(bad(format!("{other}-bit integer WAV is not supported (use 16-bit PCM or 32-bit float)"))),
        (TAG_FLOAT, other) => return Err(bad(format!("{other}-bit float WAV is not supported (use 32-bit float or 16-bit PCM)"))),
        (other, _) => return Err(bad(format!("WAV format tag {other} is not supported (use 16-bit PCM or 32-bit float)"))),
    };
    if !(1..=2).contains(&channels) {
        return Err(bad(format!("{channels} channels: only mono and stereo are supported")));
    }
    if !(8_000..=192_000).contains(&rate) {
        return Err(bad(format!("{rate} Hz: the sample rate must be 8000 to 192000 Hz")));
    }
    let width = if encoding == Encoding::Int16 { 2 } else { 4 };
    if block_align != channels * width {
        return Err(bad("the block alignment of the fmt chunk does not match its channels and bits"));
    }
    Ok(Format { encoding, channels, rate })
}

/// Reads a WAV file into a [`Clip`] (see the module docs for what is supported).
pub fn clip_from_wav(bytes: &[u8]) -> Result<Clip, WavError> {
    if bytes.len() > MAX_WAV_BYTES {
        return Err(bad(format!("the WAV file is {} bytes, more than the {MAX_WAV_BYTES} bytes allowed", bytes.len())));
    }
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file: the RIFF/WAVE header is missing"));
    }
    let riff_end = u32_at(bytes, 4) as usize;
    if riff_end.saturating_add(8) > bytes.len() {
        return Err(bad("the WAV file is cut off: the RIFF size is larger than the file"));
    }
    if riff_end < 4 {
        return Err(bad("the RIFF size is too small"));
    }
    let body = &bytes[12..riff_end + 8];
    let (mut format, mut data) = (None, None);
    let mut at = 0usize;
    while at < body.len() {
        if body.len() - at < 8 {
            return Err(bad("the WAV file is cut off inside a chunk header"));
        }
        let id = &body[at..at + 4];
        let length = u32_at(body, at + 4) as usize;
        let start = at + 8;
        let end = start.checked_add(length).filter(|&end| end <= body.len()).ok_or_else(|| bad("the WAV file is cut off inside a chunk"))?;
        let chunk = &body[start..end];
        match id {
            b"fmt " => {
                if format.is_some() {
                    return Err(bad("the WAV file has two fmt chunks"));
                }
                format = Some(parse_format(chunk)?);
            }
            b"data" => {
                if data.is_some() {
                    return Err(bad("the WAV file has two data chunks"));
                }
                data = Some(chunk);
            }
            _ => {}
        }
        at = end + (length & 1); // chunks are padded to an even length
    }
    let format = format.ok_or_else(|| bad("the WAV file has no fmt chunk"))?;
    let data = data.ok_or_else(|| bad("the WAV file has no data chunk"))?;
    let width = if format.encoding == Encoding::Int16 { 2 } else { 4 };
    let frame_bytes = width * format.channels;
    if data.is_empty() {
        return Err(bad("the WAV file has no sound data"));
    }
    if data.len() % frame_bytes != 0 {
        return Err(bad("the data chunk ends inside a sample frame"));
    }
    let sample = |at: usize| -> Result<f32, WavError> {
        match format.encoding {
            Encoding::Int16 => Ok(f32::from(i16::from_le_bytes([data[at], data[at + 1]])) / 32768.0),
            Encoding::Float32 => {
                let v = f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
                if !v.is_finite() || v.abs() > 1.0 {
                    return Err(bad("a float sample is not a number or is outside -1..1"));
                }
                Ok(v)
            }
        }
    };
    let mut frames = Vec::with_capacity(data.len() / frame_bytes);
    for at in (0..data.len()).step_by(frame_bytes) {
        let left = sample(at)?;
        let right = if format.channels == 2 { sample(at + width)? } else { left };
        frames.push([left, right]);
    }
    Clip::from_stereo(format.rate, frames).map_err(|e| bad(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt_chunk(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let width = bits / 8;
        let mut v = Vec::new();
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * u32::from(channels * width)).to_le_bytes());
        v.extend_from_slice(&(channels * width).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v
    }

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.iter().flatten().copied().collect();
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(&body);
        v
    }

    fn pcm16(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn refused(bytes: &[u8]) -> String {
        match clip_from_wav(bytes) {
            Err(error) => error.0,
            Ok(_) => panic!("the file was accepted"),
        }
    }

    #[test]
    fn a_16_bit_mono_file_becomes_a_clip() {
        let wav = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 22_050, 16)), chunk(b"data", &pcm16(&[0, 16384, -16384, 32767]))]);
        assert!(clip_from_wav(&wav).is_ok());
    }

    #[test]
    fn a_stereo_float_file_with_other_chunks_becomes_a_clip() {
        let data: Vec<u8> = [0.0f32, 0.5, -0.5, 1.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        let wav = riff(&[chunk(b"LIST", b"abc"), chunk(b"fmt ", &fmt_chunk(3, 2, 48_000, 32)), chunk(b"data", &data)]);
        assert!(clip_from_wav(&wav).is_ok());
    }

    #[test]
    fn bad_headers_and_unsupported_formats_are_refused_with_a_reason() {
        let ok = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 22_050, 16)), chunk(b"data", &pcm16(&[1, 2, 3, 4]))]);
        let mut not_riff = ok.clone();
        not_riff[0] = b'X';
        assert!(refused(&not_riff).contains("RIFF"));
        assert!(refused(&ok[..ok.len() - 3]).contains("cut off"));
        assert!(clip_from_wav(&[]).is_err());
        let eight = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 22_050, 8)), chunk(b"data", &[1, 2])]);
        assert!(refused(&eight).contains("8-bit"));
        let six = riff(&[chunk(b"fmt ", &fmt_chunk(1, 6, 22_050, 16)), chunk(b"data", &pcm16(&[0; 6]))]);
        assert!(refused(&six).contains("channels"));
        let slow = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 4_000, 16)), chunk(b"data", &pcm16(&[0; 4]))]);
        assert!(refused(&slow).contains("Hz"));
        let no_data = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 22_050, 16))]);
        assert!(refused(&no_data).contains("no data"));
        let no_fmt = riff(&[chunk(b"data", &pcm16(&[0; 4]))]);
        assert!(refused(&no_fmt).contains("no fmt"));
        let odd = riff(&[chunk(b"fmt ", &fmt_chunk(1, 2, 22_050, 16)), chunk(b"data", &pcm16(&[0; 3]))]);
        assert!(refused(&odd).contains("sample frame"));
        let nan: Vec<u8> = f32::NAN.to_le_bytes().to_vec();
        let float = riff(&[chunk(b"fmt ", &fmt_chunk(3, 1, 22_050, 32)), chunk(b"data", &nan)]);
        assert!(refused(&float).contains("not a number"));
    }

    #[test]
    fn a_clip_longer_than_ten_seconds_is_refused() {
        let wav = riff(&[chunk(b"fmt ", &fmt_chunk(1, 1, 8_000, 16)), chunk(b"data", &pcm16(&vec![0i16; 80_001]))]);
        assert!(refused(&wav).contains("10 seconds"));
    }
}
