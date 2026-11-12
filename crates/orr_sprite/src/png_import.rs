//! Optional PNG atlas decoding; no filesystem or renderer access.
pub fn decode_atlas(
    image: &[u8],
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, String> {
    if expected_width == 0
        || expected_height == 0
        || expected_width > crate::MAX_ATLAS_DIMENSION
        || expected_height > crate::MAX_ATLAS_DIMENSION
    {
        return Err("invalid atlas dimensions".into());
    }
    if image.len() < 24 || &image[..8] != b"\x89PNG\r\n\x1a\n" || &image[12..16] != b"IHDR" {
        return Err("atlas must be PNG".into());
    }
    let width = u32::from_be_bytes(image[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(image[20..24].try_into().unwrap());
    if width != expected_width || height != expected_height {
        return Err("atlas PNG dimensions differ from sprite document".into());
    }
    let mut decoder = png17::Decoder::new(std::io::Cursor::new(image));
    decoder.set_transformations(png17::Transformations::EXPAND | png17::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().animation_control.is_some() {
        return Err("animated PNG atlases are unsupported; use sprite clips".into());
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if info.width != width || info.height != height {
        return Err("decoded PNG frame dimensions differ from atlas".into());
    }
    let expected_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("atlas byte size overflow")?;
    let mut rgba = Vec::with_capacity(expected_bytes);
    match info.color_type {
        png17::ColorType::Rgba => rgba.extend_from_slice(&pixels[..info.buffer_size()]),
        png17::ColorType::Rgb => {
            for p in pixels[..info.buffer_size()].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png17::ColorType::Grayscale => {
            for p in &pixels[..info.buffer_size()] {
                rgba.extend_from_slice(&[*p, *p, *p, 255]);
            }
        }
        png17::ColorType::GrayscaleAlpha => {
            for p in pixels[..info.buffer_size()].chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        _ => return Err("unsupported PNG color type".into()),
    }
    if rgba.len() != expected_bytes {
        return Err("decoded PNG pixel count differs from atlas".into());
    }
    Ok(rgba)
}
