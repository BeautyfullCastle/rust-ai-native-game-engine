//! Strict stored-sample PNG admission. No gamma or color transformations.
use super::{MAX_PNG_BYTES, MAX_SIDE};
use orr_model::Image;
use std::io::Cursor;
#[derive(Debug)]
struct Error(String);
type Result<T> = std::result::Result<T, Error>;
const MAX_PNG_CHUNKS: usize = 1024;
const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn failure(context: &str, error: impl std::fmt::Display) -> Error {
    Error(format!("surface PNG {context}: {error}"))
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("checked four-byte PNG field"))
}

/// Check the whole chunk envelope before permitting allocation or decompression.
/// Returns the total IDAT size; chunk CRCs are checked by the strict PNG reader.
fn admit_chunks(bytes: &[u8]) -> Result<(usize, u32, u32, usize)> {
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(SIGNATURE) {
        return Err(Error(format!(
            "surface PNG requires a PNG signature and at most {MAX_PNG_BYTES} encoded bytes"
        )));
    }
    let mut offset = SIGNATURE.len();
    let mut count = 0;
    let mut data_bytes = 0usize;
    let mut have_data = false;
    let mut metadata = [false; 3];
    let (mut width, mut height, mut channels) = (0, 0, 0);
    while offset < bytes.len() {
        count += 1;
        if count > MAX_PNG_CHUNKS {
            return Err(Error("surface PNG chunk count exceeds 1024".into()));
        }
        let header_end = offset
            .checked_add(8)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error("surface PNG truncated chunk header".into()))?;
        let length = usize::try_from(be_u32(&bytes[offset..offset + 4]))
            .map_err(|e| failure("chunk length", e))?;
        let data_end = header_end
            .checked_add(length)
            .ok_or_else(|| Error("surface PNG chunk length overflow".into()))?;
        let end = data_end
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error("surface PNG truncated chunk data or CRC".into()))?;
        let kind = &bytes[offset + 4..header_end];
        let data = &bytes[header_end..data_end];
        if count == 1 && kind != b"IHDR" {
            return Err(Error("surface PNG IHDR must be the first chunk".into()));
        }
        match kind {
            b"IHDR" => {
                if count != 1 || length != 13 {
                    return Err(Error("surface PNG requires one 13-byte IHDR".into()));
                }
                width = be_u32(&data[..4]);
                height = be_u32(&data[4..8]);
                if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
                    return Err(Error("surface PNG dimensions must be 1..=512".into()));
                }
                if data[8] != 8 || ![2, 6].contains(&data[9]) || data[10..] != [0, 0, 0] {
                    return Err(Error(
                        "surface PNG requires original RGB8 or RGBA8, non-interlaced".into(),
                    ));
                }
                channels = if data[9] == 2 { 3 } else { 4 };
            }
            b"IDAT" => {
                have_data = true;
                data_bytes = data_bytes
                    .checked_add(length)
                    .ok_or_else(|| Error("surface PNG IDAT length overflow".into()))?;
            }
            b"IEND" => {
                if length != 0 || !have_data || data_bytes == 0 || end != bytes.len() {
                    return Err(Error(
                        "surface PNG requires nonempty image data and a final empty IEND without trailing bytes".into(),
                    ));
                }
                return Ok((data_bytes, width, height, channels));
            }
            b"gAMA" | b"sRGB" | b"sBIT" => {
                let index = match kind {
                    b"gAMA" => 0,
                    b"sRGB" => 1,
                    _ => 2,
                };
                let valid = match index {
                    0 => length == 4 && be_u32(data) != 0,
                    1 => length == 1 && data[0] <= 3,
                    _ => length == channels && data.iter().all(|v| (1..=8).contains(v)),
                };
                if have_data || metadata[index] || !valid {
                    return Err(Error(
                        "surface PNG gAMA/sRGB/sBIT must be valid, unique and before IDAT".into(),
                    ));
                }
                metadata[index] = true;
            }
            _ => {
                return Err(Error(
                    "surface PNG chunk is outside the IHDR/IDAT/IEND/gAMA/sRGB/sBIT allowlist"
                        .into(),
                ))
            }
        }
        offset = end;
    }
    Err(Error("surface PNG missing IEND".into()))
}

fn strict_image_data(bytes: &[u8], compressed_len: usize, expected: usize) -> Result<()> {
    // The envelope has already checked every offset and the total encoded cap.
    let mut compressed = Vec::with_capacity(compressed_len);
    let mut offset = SIGNATURE.len();
    while offset < bytes.len() {
        let length = be_u32(&bytes[offset..offset + 4]) as usize;
        if &bytes[offset + 4..offset + 8] == b"IDAT" {
            compressed.extend_from_slice(&bytes[offset + 8..offset + 8 + length]);
        }
        offset += length + 12;
    }
    // png::Reader deliberately tolerates excess output, incomplete checksums
    // and compressed trailers. Validate its zlib input independently with the
    // same already-locked inflater's supported completion/byte-count APIs.
    let mut inflater = fdeflate::Decompressor::new(); // Adler-32 is enabled.
    let mut filtered = vec![0; expected + 1];
    let mut read = 0;
    let mut written = 0;
    // Bulk reads may prefetch beyond the zlib checksum. Feed one byte at a time
    // so trailers cannot hide in the bit buffer, without assuming its size.
    // The 4 MiB encoded cap also bounds the number of calls in this loop.
    while read < compressed.len() && !inflater.is_done() {
        let end = read + 1;
        let (consumed, produced) = inflater
            .read(
                &compressed[read..end],
                &mut filtered,
                written,
                end == compressed.len(),
            )
            .map_err(|e| Error(format!("surface PNG zlib stream: {e:?}")))?;
        read += consumed;
        written += produced;
        if written > expected || (consumed == 0 && produced == 0) {
            return Err(Error("surface PNG excess or stalled image data".into()));
        }
    }
    if !inflater.is_done() || read != compressed.len() || written != expected {
        return Err(Error(
            "surface PNG requires exactly one complete zlib stream and the exact filtered sample length"
                .into(),
        ));
    }
    Ok(())
}

pub(super) fn decode(bytes: &[u8]) -> std::result::Result<Image, String> {
    decode_inner(bytes).map_err(|error| error.0)
}
fn decode_inner(bytes: &[u8]) -> Result<Image> {
    let (compressed, width, height, channels) = admit_chunks(bytes)?;
    let expected = width as usize * height as usize * channels;
    strict_image_data(bytes, compressed, expected + height as usize)?;
    let mut options = png::DecodeOptions::default();
    options.set_ignore_checksums(false);
    options.set_skip_ancillary_crc_failures(false);
    options.set_ignore_text_chunk(true);
    options.set_ignore_iccp_chunk(true);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), options);
    decoder.set_transformations(png::Transformations::IDENTITY);
    decoder.set_limits(png::Limits {
        bytes: MAX_PNG_BYTES,
    });
    let mut reader = decoder.read_info().map_err(|e| failure("header", e))?;
    if reader.output_buffer_size() != Some(expected) {
        return Err(Error("surface PNG output size mismatch".into()));
    }
    let mut samples = vec![0; expected];
    let output = reader
        .next_frame(&mut samples)
        .map_err(|e| failure("samples", e))?;
    let color = if channels == 3 {
        png::ColorType::Rgb
    } else {
        png::ColorType::Rgba
    };
    if output.width != width
        || output.height != height
        || output.color_type != color
        || output.bit_depth != png::BitDepth::Eight
        || output.buffer_size() != expected
    {
        return Err(Error("surface PNG decoded format mismatch".into()));
    }
    reader.finish().map_err(|e| failure("finish", e))?;
    let rgba8 = if channels == 3 {
        samples
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    } else {
        if samples.chunks_exact(4).any(|p| p[3] != 255) {
            return Err(Error("surface PNG must be fully opaque".into()));
        }
        samples
    };
    Ok(Image {
        width,
        height,
        rgba8,
    })
}
