use super::*;

fn encoded(width: u32, height: u32, color: png17::ColorType, depth: png17::BitDepth) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if color == png17::ColorType::Indexed {
            encoder.set_palette(vec![0, 0, 0, 255, 0, 0]);
        }
        let mut writer = encoder.write_header().unwrap();
        let bits = width as usize * color.samples() * depth as usize;
        writer
            .write_image_data(&vec![0; bits.div_ceil(8) * height as usize])
            .unwrap();
    }
    bytes
}
fn chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut position = 8;
    let mut result = Vec::new();
    while position < bytes.len() {
        let len = u32::from_be_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
        result.push((
            bytes[position + 4..position + 8].try_into().unwrap(),
            bytes[position + 8..position + 8 + len].to_vec(),
        ));
        position += len + 12;
    }
    result
}
fn assemble(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in chunks {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc32fast::hash(&out[start..]).to_be_bytes());
    }
    out
}
#[test]
fn valid_color_variants_fully_decode_without_changing_source() {
    for color in [
        png17::ColorType::Grayscale,
        png17::ColorType::GrayscaleAlpha,
        png17::ColorType::Rgb,
        png17::ColorType::Rgba,
        png17::ColorType::Indexed,
    ] {
        let bytes = encoded(7, 3, color, png17::BitDepth::Eight);
        let original = bytes.clone();
        assert_eq!(validate(&bytes).unwrap(), (7, 3));
        assert_eq!(bytes, original);
    }
    for depth in [
        png17::BitDepth::One,
        png17::BitDepth::Two,
        png17::BitDepth::Four,
        png17::BitDepth::Sixteen,
    ] {
        assert_eq!(
            validate(&encoded(7, 3, png17::ColorType::Grayscale, depth)).unwrap(),
            (7, 3)
        );
    }
    assert!(validate(&encoded(
        2,
        2,
        png17::ColorType::Rgba,
        png17::BitDepth::Sixteen
    ))
    .is_ok());
}
#[test]
fn web_dimensions_do_not_inherit_the_sprite_policy() {
    for width in [2049, 4096] {
        let bytes = encoded(width, 1, png17::ColorType::Rgba, png17::BitDepth::Eight);
        assert_eq!(validate(&bytes).unwrap(), (width, 1));
        assert!(orr_sprite::png_import::decode_atlas(&bytes, width, 1).is_err());
    }
    assert_eq!(
        validate(&encoded(
            1,
            4096,
            png17::ColorType::Grayscale,
            png17::BitDepth::Eight
        ))
        .unwrap(),
        (1, 4096)
    );
    for (width, height) in [(4097, 1), (1, 4097)] {
        assert!(validate(&encoded(
            width,
            height,
            png17::ColorType::Grayscale,
            png17::BitDepth::Eight
        ))
        .is_err());
    }
    let mut parts = chunks(&encoded(
        1,
        1,
        png17::ColorType::Grayscale,
        png17::BitDepth::Eight,
    ));
    parts[0].1[..4].copy_from_slice(&0u32.to_be_bytes());
    assert!(validate(&assemble(&parts)).is_err());
}
#[test]
fn envelope_rejects_apng_crc_trailing_unknown_critical_and_bad_order() {
    let valid = encoded(2, 2, png17::ColorType::Rgba, png17::BitDepth::Eight);
    for name in [*b"acTL", *b"fcTL", *b"fdAT", *b"ABCD", *b"a1Cd"] {
        let mut parts = chunks(&valid);
        parts.insert(1, (name, vec![0; 8]));
        assert!(validate(&assemble(&parts)).is_err(), "{name:?}");
    }
    let mut corrupt = valid.clone();
    corrupt[29] ^= 1;
    assert!(validate(&corrupt).is_err());
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(validate(&trailing).is_err());
    let mut parts = chunks(&valid);
    parts.swap(0, 1);
    assert!(validate(&assemble(&parts)).is_err());
    let mut parts = chunks(&valid);
    parts.insert(1, parts[0].clone());
    assert!(validate(&assemble(&parts)).is_err());
    let mut parts = chunks(&valid);
    parts.last_mut().unwrap().1.push(0);
    assert!(validate(&assemble(&parts)).is_err());
    let mut parts = chunks(&valid);
    parts.retain(|(kind, _)| kind != b"IDAT");
    assert!(validate(&assemble(&parts)).is_err());
    let mut parts = chunks(&valid);
    parts.pop();
    assert!(validate(&assemble(&parts)).is_err());
    for n in 0..valid.len() {
        assert!(validate(&valid[..n]).is_err(), "truncation {n}");
    }
}
#[test]
fn valid_chunk_crc_does_not_substitute_for_pixel_decode() {
    let mut parts = chunks(&encoded(
        2,
        2,
        png17::ColorType::Rgba,
        png17::BitDepth::Eight,
    ));
    for (kind, data) in &mut parts {
        if kind == b"IDAT" {
            *data = vec![1, 2, 3, 4, 5];
        }
    }
    // assemble recomputes every CRC; rejection must still reach the PNG codec.
    assert!(validate(&assemble(&parts)).is_err());
}
#[test]
fn bounded_bytes_and_unknown_ancillary_contract() {
    let valid = encoded(1, 1, png17::ColorType::Rgb, png17::BitDepth::Eight);
    let mut parts = chunks(&valid);
    parts.insert(1, (*b"vpAg", vec![1, 2, 3]));
    assert_eq!(validate(&assemble(&parts)).unwrap(), (1, 1));
    let mut huge = valid;
    huge.resize(MAX_IMAGE + 1, 0);
    assert!(validate(&huge).is_err());
    assert!(validate(b"not PNG").is_err());
    let mut bad = b"\x89PNG\r\n\x1a\n".to_vec();
    bad.extend_from_slice(&u32::MAX.to_be_bytes());
    bad.extend_from_slice(b"IHDR");
    assert!(validate(&bad).is_err());
}
