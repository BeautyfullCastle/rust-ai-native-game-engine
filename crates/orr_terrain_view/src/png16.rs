//! Bounded, stored-sample grayscale PNG16 terrain authoring.
//!
//! Only non-interlaced Gray16 images matching the RAW16 recipe are accepted.
//! The conservative chunk allowlist is IHDR, consecutive IDAT, IEND, and unique
//! pre-IDAT gAMA/sRGB/sBIT. Metadata never changes samples. No color conversion,
//! gamma correction, resampling, transparency, palette, APNG or text is accepted.
#![deny(clippy::float_arithmetic)]

use crate::raw16::{self, Endian, Error, Recipe, Result};
use orr_terrain::Terrain;
use std::io::Cursor;

pub mod input;

#[cfg(test)]
mod tests;

/// Encoded source cap, independent of the 129 × 129 terrain sample cap.
pub const MAX_PNG_BYTES: usize = 1024 * 1024;
/// Includes IHDR, IEND, metadata, and even empty IDAT chunks.
pub const MAX_PNG_CHUNKS: usize = 1024;
const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn failure(context: &str, error: impl std::fmt::Display) -> Error {
    Error(format!("PNG16 {context}: {error}"))
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("checked four-byte PNG field"))
}

/// Check the whole chunk envelope before permitting allocation or decompression.
/// Returns the total IDAT size; chunk CRCs are checked by the strict PNG reader.
fn admit_chunks(bytes: &[u8], recipe: &Recipe) -> Result<usize> {
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(SIGNATURE) {
        return Err(Error(format!(
            "PNG16 requires a PNG signature and at most {MAX_PNG_BYTES} encoded bytes"
        )));
    }
    let mut offset = SIGNATURE.len();
    let mut count = 0;
    let mut data_bytes = 0usize;
    let mut have_data = false;
    let mut metadata = [false; 3];
    while offset < bytes.len() {
        count += 1;
        if count > MAX_PNG_CHUNKS {
            return Err(Error("PNG16 chunk count exceeds 1024".into()));
        }
        let header_end = offset
            .checked_add(8)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error("PNG16 truncated chunk header".into()))?;
        let length = usize::try_from(be_u32(&bytes[offset..offset + 4]))
            .map_err(|e| failure("chunk length", e))?;
        let data_end = header_end
            .checked_add(length)
            .ok_or_else(|| Error("PNG16 chunk length overflow".into()))?;
        let end = data_end
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error("PNG16 truncated chunk data or CRC".into()))?;
        let kind = &bytes[offset + 4..header_end];
        let data = &bytes[header_end..data_end];
        if count == 1 && kind != b"IHDR" {
            return Err(Error("PNG16 IHDR must be the first chunk".into()));
        }
        match kind {
            b"IHDR" => {
                if count != 1 || length != 13 {
                    return Err(Error("PNG16 requires one 13-byte IHDR".into()));
                }
                if be_u32(&data[..4]) != recipe.width || be_u32(&data[4..8]) != recipe.depth {
                    return Err(Error(
                        "PNG16 dimensions must exactly match the validated recipe".into(),
                    ));
                }
                if data[8..] != [16, 0, 0, 0, 0] {
                    return Err(Error(
                        "PNG16 requires grayscale 16-bit, non-interlaced, standard compression/filtering".into(),
                    ));
                }
            }
            b"IDAT" => {
                have_data = true;
                data_bytes = data_bytes
                    .checked_add(length)
                    .ok_or_else(|| Error("PNG16 IDAT length overflow".into()))?;
            }
            b"IEND" => {
                if length != 0 || !have_data || data_bytes == 0 || end != bytes.len() {
                    return Err(Error(
                        "PNG16 requires nonempty image data and a final empty IEND without trailing bytes".into(),
                    ));
                }
                return Ok(data_bytes);
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
                    _ => length == 1 && (1..=16).contains(&data[0]),
                };
                if have_data || metadata[index] || !valid {
                    return Err(Error(
                        "PNG16 gAMA/sRGB/sBIT must be valid, unique and before IDAT".into(),
                    ));
                }
                metadata[index] = true;
            }
            _ => {
                return Err(Error(
                    "PNG16 chunk is outside the IHDR/IDAT/IEND/gAMA/sRGB/sBIT allowlist".into(),
                ))
            }
        }
        offset = end;
    }
    Err(Error("PNG16 missing IEND".into()))
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
    // The 1 MiB encoded cap also bounds the number of calls in this loop.
    while read < compressed.len() && !inflater.is_done() {
        let end = read + 1;
        let (consumed, produced) = inflater
            .read(
                &compressed[read..end],
                &mut filtered,
                written,
                end == compressed.len(),
            )
            .map_err(|e| Error(format!("PNG16 zlib stream: {e:?}")))?;
        read += consumed;
        written += produced;
        if written > expected || (consumed == 0 && produced == 0) {
            return Err(Error("PNG16 excess or stalled image data".into()));
        }
    }
    if !inflater.is_done() || read != compressed.len() || written != expected {
        return Err(Error(
            "PNG16 requires exactly one complete zlib stream and the exact filtered sample length"
                .into(),
        ));
    }
    Ok(())
}

/// Import stored unsigned samples with RAW16's exact integer height mapping.
/// PNG byte order is always big endian; `recipe.endian` is intentionally ignored.
/// Recipe, dimensions, chunk layout and encoded/decoded limits precede decoding.
pub fn import_png16(bytes: &[u8], recipe: &Recipe) -> Result<Terrain> {
    let expected = recipe.validate()?;
    let compressed_len = admit_chunks(bytes, recipe)?;
    let filtered = expected
        .checked_add(recipe.depth as usize)
        .ok_or_else(|| Error("PNG16 filtered size overflow".into()))?;
    strict_image_data(bytes, compressed_len, filtered)?;

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
        return Err(Error("PNG16 decoded sample size mismatch".into()));
    }
    let mut samples = vec![0; expected];
    let output = reader
        .next_frame(&mut samples)
        .map_err(|e| failure("samples", e))?;
    if output.width != recipe.width
        || output.height != recipe.depth
        || output.color_type != png::ColorType::Grayscale
        || output.bit_depth != png::BitDepth::Sixteen
        || output.buffer_size() != expected
    {
        return Err(Error("PNG16 decoded format or sample size mismatch".into()));
    }
    reader.finish().map_err(|e| failure("finish", e))?;
    let mut recipe = recipe.clone();
    recipe.endian = Endian::Big;
    raw16::import_raw16(&samples, &recipe)
}
