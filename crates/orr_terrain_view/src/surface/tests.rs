use super::*;
use ::png as png_codec;
use orr_fp::FP;
use orr_terrain::{Edit, TerrainDocument};

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    let mut crc = u32::MAX;
    for byte in &bytes[4..] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes.extend_from_slice(&(!crc).to_be_bytes());
    bytes
}

fn header(width: u32, height: u32, format: [u8; 5]) -> Vec<u8> {
    let mut data = width.to_be_bytes().to_vec();
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&format);
    chunk(b"IHDR", &data)
}

// Independent zlib fixture generator: only uncompressed RFC1951 blocks.
fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x78, 0x01];
    let blocks = data.chunks(65535);
    let total = blocks.len();
    for (index, block) in blocks.enumerate() {
        bytes.push(u8::from(index + 1 == total));
        let length = block.len() as u16;
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&(!length).to_le_bytes());
        bytes.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    bytes.extend_from_slice(&((b << 16) | a).to_be_bytes());
    bytes
}

fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    for chunk in chunks {
        bytes.extend_from_slice(chunk);
    }
    bytes
}

const RGB_ROWS: [u8; 14] = [0, 0, 1, 2, 253, 254, 255, 0, 30, 40, 50, 60, 70, 80];
const RGBA: [u8; 16] = [
    0, 1, 2, 255, 253, 254, 255, 255, 30, 40, 50, 255, 60, 70, 80, 255,
];
fn fixture_data(data: &[u8]) -> Vec<u8> {
    png(&[
        header(2, 2, [8, 2, 0, 0, 0]),
        chunk(b"IDAT", data),
        chunk(b"IEND", &[]),
    ])
}
fn fixture() -> Vec<u8> {
    fixture_data(&stored_zlib(&RGB_ROWS))
}
fn load(bytes: &[u8]) -> Result<Surface, String> {
    Surface::from_png("terrain/lab.orrt", "terrain/lab.orrt", bytes)
}

#[test]
fn literal_rgb_rgba_and_metadata_keep_exact_stored_pixels() {
    let base = load(&fixture()).unwrap();
    assert_eq!(
        base.image(),
        &Image {
            width: 2,
            height: 2,
            rgba8: RGBA.to_vec()
        }
    );
    let rows: Vec<u8> = RGBA
        .chunks_exact(8)
        .flat_map(|row| [vec![0], row.to_vec()].concat())
        .collect();
    let rgba = png(&[
        header(2, 2, [8, 6, 0, 0, 0]),
        chunk(b"IDAT", &stored_zlib(&rows)),
        chunk(b"IEND", &[]),
    ]);
    assert_eq!(load(&rgba).unwrap(), base);
    for color in [2, 6] {
        let rows = if color == 2 {
            RGB_ROWS.to_vec()
        } else {
            rows.clone()
        };
        let metadata = png(&[
            header(2, 2, [8, color, 0, 0, 0]),
            chunk(b"gAMA", &1u32.to_be_bytes()),
            chunk(b"sRGB", &[0]),
            chunk(b"sBIT", &vec![1; if color == 2 { 3 } else { 4 }]),
            chunk(b"IDAT", &stored_zlib(&rows)),
            chunk(b"IEND", &[]),
        ]);
        assert_eq!(load(&metadata).unwrap().digest(), base.digest());
    }
    let mut transparent = rows;
    transparent[4] = 254;
    assert!(load(&png(&[
        header(2, 2, [8, 6, 0, 0, 0]),
        chunk(b"IDAT", &stored_zlib(&transparent)),
        chunk(b"IEND", &[])
    ]))
    .is_err());
}

#[test]
fn json_is_strict_versioned_and_canonical() {
    let surface = load(&fixture()).unwrap();
    let bytes = surface.to_bytes().unwrap();
    assert_eq!(bytes, br#"{"format":"orr_terrain_surface","version":1,"terrain_path":"terrain/lab.orrt","asset_id":"terrain/lab.orrt","image":{"width":2,"height":2,"rgba8":[0,1,2,255,253,254,255,255,30,40,50,255,60,70,80,255]}}"#);
    assert_eq!(Surface::from_bytes(&bytes).unwrap(), surface);
    assert_eq!(surface.digest(), <[u8; 32]>::from(Sha256::digest(&bytes)));
    let text = String::from_utf8(bytes.clone()).unwrap();
    for invalid in [
        format!("{text}\n"),
        text.replacen('{', "{\"version\":1,", 1),
        text.replacen('{', "{\"unknown\":1,", 1),
        text.replace("\"version\":1", "\"version\":2"),
        text.replace("\"image\":{", "\"image\":null,\"unused\":{"),
    ] {
        assert!(
            Surface::from_bytes(invalid.as_bytes()).is_err(),
            "{invalid}"
        );
    }
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (key, value) in [
        ("width", serde_json::json!(0)),
        ("height", serde_json::json!(513)),
        ("rgba8", serde_json::json!([0, 0, 0, 0])),
        ("width", serde_json::Value::Null),
    ] {
        let mut changed = original.clone();
        changed["image"][key] = value;
        assert!(Surface::from_bytes(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    assert!(Surface::from_bytes(&vec![b' '; MAX_SIDECAR_BYTES + 1]).is_err());
}

#[test]
fn confined_paths_and_stable_identity_survive_height_and_hole_changes() {
    for path in [
        "",
        "/a.orrt",
        "../a.orrt",
        "a/../a.orrt",
        "a//b.orrt",
        "./a.orrt",
        "a\\b.orrt",
        "C:/a.orrt",
        "a.png",
        ".orrt",
        "a/.orrt",
        "a/./b.orrt",
    ] {
        assert!(sidecar_relative(path).is_err(), "{path}");
    }
    assert!(sidecar_relative(&format!("{}.orrt", "a".repeat(256))).is_err());
    let surface = load(&fixture()).unwrap();
    assert_eq!(
        sidecar_relative(surface.terrain_path()).unwrap(),
        "terrain/lab.orrt.surface.json"
    );
    let mut document = TerrainDocument::new(crate::fixture());
    document
        .apply(&[
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::ONE,
            },
            Edit::SetHole {
                x: 1,
                z: 1,
                hole: true,
            },
        ])
        .unwrap();
    surface
        .validate_terrain("terrain/lab.orrt", document.terrain())
        .unwrap();
    assert!(surface
        .validate_terrain("other.orrt", document.terrain())
        .is_err());
    let wrong = Surface::from_png("terrain/lab.orrt", "wrong", &fixture()).unwrap();
    assert!(to_static_model(document.terrain(), &[], &wrong).is_err());
}

#[test]
fn model_changes_only_ground_image_and_adds_truthful_provenance() {
    let terrain = crate::fixture();
    let marker = QueryMarker::sample(&terrain, FP::ZERO, FP::ZERO).unwrap();
    let base = crate::to_static_model(&terrain, &[marker])
        .unwrap()
        .unwrap();
    let surface = load(&fixture()).unwrap();
    let model = to_static_model(&terrain, &[marker], &surface)
        .unwrap()
        .unwrap();
    assert_eq!(model.source().primitives, base.source().primitives);
    assert_eq!(model.source().materials, base.source().materials);
    assert_eq!(model.source().images[1], base.source().images[1]);
    assert_eq!(model.source().images[0], *surface.image());
    assert_eq!(
        model.source().dependencies[0],
        base.source().dependencies[0]
    );
    assert_eq!(
        model.source().dependencies[1].uri,
        "terrain/lab.orrt.surface.json"
    );
    let empty = Terrain::new(
        "terrain/lab.orrt".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![true],
    )
    .unwrap();
    assert!(to_static_model(&empty, &[], &surface).unwrap().is_none());
}

#[test]
fn rejects_truncation_crc_and_both_external_and_compressed_trailers() {
    let original = fixture();
    for end in 0..original.len() {
        assert!(load(&original[..end]).is_err(), "end {end}");
    }
    let mut trailer = original.clone();
    trailer.push(0);
    assert!(load(&trailer).is_err());
    let chunks = [
        header(2, 2, [8, 2, 0, 0, 0]),
        chunk(b"gAMA", &1u32.to_be_bytes()),
        chunk(b"sRGB", &[0]),
        chunk(b"sBIT", &[8; 3]),
        chunk(b"IDAT", &stored_zlib(&RGB_ROWS)),
        chunk(b"IEND", &[]),
    ];
    for index in 0..chunks.len() {
        let mut bad = chunks.clone();
        *bad[index].last_mut().unwrap() ^= 1;
        assert!(load(&png(&bad)).is_err(), "crc {index}");
    }
    let data = stored_zlib(&RGB_ROWS);
    for missing in 1..=4 {
        assert!(load(&fixture_data(&data[..data.len() - missing])).is_err());
    }
    for length in 1..=16 {
        let mut bad = data.clone();
        bad.extend(vec![0; length]);
        assert!(load(&fixture_data(&bad)).is_err());
    }
    let mut bad = data.clone();
    bad.extend_from_slice(&data);
    assert!(load(&fixture_data(&bad)).is_err());
    let mut bad = data;
    *bad.last_mut().unwrap() ^= 1;
    assert!(load(&fixture_data(&bad)).is_err());
}

#[test]
fn rejects_bad_layout_formats_metadata_inflate_and_limits() {
    for format in [
        [16, 2, 0, 0, 0],
        [8, 0, 0, 0, 0],
        [8, 3, 0, 0, 0],
        [8, 4, 0, 0, 0],
        [8, 2, 0, 0, 1],
        [8, 2, 1, 0, 0],
        [8, 2, 0, 1, 0],
    ] {
        assert!(load(&png(&[
            header(2, 2, format),
            chunk(b"IDAT", &stored_zlib(&RGB_ROWS)),
            chunk(b"IEND", &[])
        ]))
        .is_err());
    }
    for kind in [
        b"tRNS", b"PLTE", b"acTL", b"fcTL", b"fdAT", b"tEXt", b"zTXt", b"iTXt", b"iCCP", b"pHYs",
        b"ABCD", b"abCd",
    ] {
        assert!(load(&png(&[
            header(2, 2, [8, 2, 0, 0, 0]),
            chunk(kind, &[0; 32]),
            chunk(b"IDAT", &stored_zlib(&RGB_ROWS)),
            chunk(b"IEND", &[])
        ]))
        .is_err());
    }
    for (kind, data) in [
        (b"gAMA", vec![0; 4]),
        (b"sRGB", vec![4]),
        (b"sBIT", vec![8]),
        (b"sBIT", vec![9; 3]),
    ] {
        assert!(load(&png(&[
            header(2, 2, [8, 2, 0, 0, 0]),
            chunk(kind, &data),
            chunk(b"IDAT", &stored_zlib(&RGB_ROWS)),
            chunk(b"IEND", &[])
        ]))
        .is_err());
    }
    let h = header(2, 2, [8, 2, 0, 0, 0]);
    let d = chunk(b"IDAT", &stored_zlib(&RGB_ROWS));
    let e = chunk(b"IEND", &[]);
    for chunks in [
        vec![d.clone(), h.clone(), e.clone()],
        vec![h.clone(), h.clone(), d.clone(), e.clone()],
        vec![h.clone(), e.clone()],
        vec![
            h.clone(),
            d.clone(),
            chunk(b"sRGB", &[0]),
            d.clone(),
            e.clone(),
        ],
        vec![
            h.clone(),
            chunk(b"sRGB", &[0]),
            chunk(b"sRGB", &[0]),
            d.clone(),
            e.clone(),
        ],
        vec![h.clone(), d.clone(), chunk(b"IEND", &[0])],
    ] {
        assert!(load(&png(&chunks)).is_err());
    }
    let mut chunks = vec![h, d];
    chunks.extend(std::iter::repeat_n(chunk(b"IDAT", &[]), 1022));
    chunks.push(e);
    assert!(load(&png(&chunks)).is_err());
    for (width, height) in [(0, 1), (1, 0), (513, 1), (1, 513), (u32::MAX, u32::MAX)] {
        assert!(load(&png(&[
            header(width, height, [8, 2, 0, 0, 0]),
            chunk(b"IDAT", &[0]),
            chunk(b"IEND", &[])
        ]))
        .is_err());
    }
    for rows in [
        RGB_ROWS[..13].to_vec(),
        [RGB_ROWS.to_vec(), vec![0]].concat(),
        vec![0; 2 * 1024 * 1024],
    ] {
        assert!(load(&fixture_data(&fdeflate::compress_to_vec(&rows))).is_err());
    }
    let mut rows = RGB_ROWS;
    rows[0] = 5;
    assert!(load(&fixture_data(&stored_zlib(&rows))).is_err());
    assert!(load(&vec![0; MAX_PNG_BYTES + 1]).is_err());
}

#[test]
fn maximum_size_and_all_filters_are_supported_without_gpu() {
    let rows = vec![1; 512 * 512 * 3];
    for filter in [
        png_codec::Filter::NoFilter,
        png_codec::Filter::Sub,
        png_codec::Filter::Up,
        png_codec::Filter::Avg,
        png_codec::Filter::Paeth,
    ] {
        let mut bytes = Vec::new();
        let mut encoder = png_codec::Encoder::new(&mut bytes, 512, 512);
        encoder.set_color(png_codec::ColorType::Rgb);
        encoder.set_depth(png_codec::BitDepth::Eight);
        encoder.set_filter(filter);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rows).unwrap();
        writer.finish().unwrap();
        let surface = load(&bytes).unwrap();
        assert_eq!(surface.image().rgba8.len(), 512 * 512 * 4);
        assert!(surface
            .image()
            .rgba8
            .chunks_exact(4)
            .all(|p| p == [1, 1, 1, 255]));
        let canonical = surface.to_bytes().unwrap();
        assert_eq!(Surface::from_bytes(&canonical).unwrap(), surface);
    }
}

#[test]
fn consecutive_split_idats_and_minimum_image_are_accepted() {
    let data = stored_zlib(&RGB_ROWS);
    for split in 0..=data.len() {
        let bytes = png(&[
            header(2, 2, [8, 2, 0, 0, 0]),
            chunk(b"IDAT", &data[..split]),
            chunk(b"IDAT", &[]),
            chunk(b"IDAT", &data[split..]),
            chunk(b"IEND", &[]),
        ]);
        assert_eq!(load(&bytes).unwrap().image().rgba8, RGBA);
    }
    let bytes = png(&[
        header(1, 1, [8, 2, 0, 0, 0]),
        chunk(b"IDAT", &stored_zlib(&[0, 7, 8, 9])),
        chunk(b"IEND", &[]),
    ]);
    assert_eq!(load(&bytes).unwrap().image().rgba8, [7, 8, 9, 255]);
}

#[test]
fn maximum_identity_path_renders_with_a_valid_dependency_uri() {
    // Terrain identity bounds are 256; model dependency URIs allow 1024.
    let path = format!("{}.orrt", "a".repeat(251));
    let sidecar = sidecar_relative(&path).unwrap();
    assert_eq!(path.len(), 256);
    assert_eq!(sidecar.len(), 269);
    let surface = Surface::from_png(&path, &path, &fixture()).unwrap();
    let terrain = Terrain::new(
        path,
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    assert_eq!(
        to_static_model(&terrain, &[], &surface)
            .unwrap()
            .unwrap()
            .source()
            .dependencies[1]
            .uri,
        sidecar
    );
}
