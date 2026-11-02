//! Bounded web-atlas PNG admission. Decode pixels, but retain no converted image.
//! This uses the existing PNG codec directly: the sprite document's 2048-pixel
//! policy is intentionally not the web author's 4096-pixel policy.
const MAX_IMAGE: usize = 8 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;
const MAX_DECODED: usize = 64 * 1024 * 1024;

pub(super) fn validate(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() > MAX_IMAGE || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid PNG signature or byte limit".into());
    }
    let mut position = 8usize;
    let mut chunks = 0usize;
    let mut dimensions = None;
    let mut idat = false;
    let mut ended = false;
    while position < bytes.len() {
        let header = bytes
            .get(position..position.saturating_add(8))
            .ok_or("truncated PNG chunk")?;
        let length =
            u32::from_be_bytes(header[..4].try_into().map_err(|_| "invalid PNG length")?) as usize;
        let kind = &header[4..8];
        let end = position
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or("PNG chunk length overflow")?;
        if end > bytes.len() || !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err("invalid PNG chunk".into());
        }
        let expected = u32::from_be_bytes(
            bytes[end - 4..end]
                .try_into()
                .map_err(|_| "invalid PNG CRC")?,
        );
        if crc32fast::hash(&bytes[position + 4..end - 4]) != expected {
            return Err("invalid PNG CRC".into());
        }
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
            return Err("animated PNG is not a sprite atlas".into());
        }
        if chunks == 0 && kind != b"IHDR" {
            return Err("PNG must begin with IHDR".into());
        }
        match kind {
            b"IHDR" => {
                if dimensions.is_some() || length != 13 {
                    return Err("invalid or duplicate PNG IHDR".into());
                }
                let data = &bytes[position + 8..end - 4];
                let width =
                    u32::from_be_bytes(data[..4].try_into().map_err(|_| "invalid PNG width")?);
                let height =
                    u32::from_be_bytes(data[4..8].try_into().map_err(|_| "invalid PNG height")?);
                if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
                    return Err("PNG dimensions exceed 4096x4096".into());
                }
                dimensions = Some((width, height));
            }
            b"IDAT" => idat = true,
            b"IEND" => {
                if length != 0 || end != bytes.len() {
                    return Err("invalid PNG end or trailing bytes".into());
                }
                ended = true;
            }
            b"PLTE" => {}
            _ if kind[0] & 32 == 0 => return Err("unknown critical PNG chunk".into()),
            _ => {}
        }
        position = end;
        chunks += 1;
    }
    let dimensions = dimensions.ok_or("missing PNG IHDR")?;
    if !ended || !idat {
        return Err("incomplete PNG".into());
    }

    let mut decoder = png17::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png17::Limits { bytes: MAX_DECODED });
    decoder.set_transformations(png17::Transformations::EXPAND | png17::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().animation_control.is_some() {
        return Err("animated PNG is unsupported".into());
    }
    let size = reader.output_buffer_size();
    if size == 0 || size > MAX_DECODED {
        return Err("PNG decoded pixel budget exceeded".into());
    }
    let mut pixels = vec![0; size];
    let frame = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if (frame.width, frame.height) != dimensions || frame.buffer_size() != size {
        return Err("decoded PNG dimensions or pixel count mismatch".into());
    }
    reader.finish().map_err(|e| e.to_string())?;
    Ok(dimensions)
}

#[cfg(test)]
#[path = "png_tests.rs"]
mod tests;
