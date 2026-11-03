use super::*;
use orr_fp::FP;
use std::{fs, path::Path};

fn recipe() -> Recipe {
    Recipe {
        asset_id: "png16/test".into(),
        width: 3,
        depth: 2,
        endian: Endian::Little,
        height_offset: FP::from_raw(-100),
        height_scale: FP::from_raw(65535),
        origin: [FP::ZERO; 2],
        spacing: FP::ONE,
        flip_z: false,
    }
}

// Independent literal PNG scanlines: 0, 1, 255 / 256, 32768, 65535.
const ROWS: [u8; 14] = [0, 0, 0, 0, 1, 0, 255, 0, 1, 0, 128, 0, 255, 255];
const SAMPLES: [u16; 6] = [0, 1, 255, 256, 32768, 65535];

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
    let mut bytes = SIGNATURE.to_vec();
    for chunk in chunks {
        bytes.extend_from_slice(chunk);
    }
    bytes
}

fn fixture_data(data: &[u8]) -> Vec<u8> {
    png(&[
        header(3, 2, [16, 0, 0, 0, 0]),
        chunk(b"IDAT", data),
        chunk(b"IEND", &[]),
    ])
}

fn fixture() -> Vec<u8> {
    fixture_data(&stored_zlib(&ROWS))
}

#[test]
fn literal_samples_have_exact_signed_heights_and_canonical_raw_twins() {
    for endian in [Endian::Little, Endian::Big] {
        for flip_z in [false, true] {
            let mut recipe = recipe();
            recipe.endian = endian;
            recipe.flip_z = flip_z;
            let terrain = import_png16(&fixture(), &recipe).unwrap();
            let expected: Vec<_> = if flip_z {
                [156, 32668, 65435, -100, -99, 155]
            } else {
                [-100, -99, 155, 156, 32668, 65435]
            }
            .into_iter()
            .map(FP::from_raw)
            .collect();
            assert_eq!(terrain.heights(), expected);
            let raw: Vec<_> = SAMPLES
                .iter()
                .flat_map(|sample| match endian {
                    Endian::Little => sample.to_le_bytes(),
                    Endian::Big => sample.to_be_bytes(),
                })
                .collect();
            let twin = raw16::import_raw16(&raw, &recipe).unwrap();
            assert_eq!(terrain, twin);
            assert_eq!(terrain.cook(), twin.cook());
            assert_eq!(terrain.revision(), twin.revision());
            assert_eq!(Terrain::load(&terrain.cook()).unwrap(), terrain);
        }
    }
}

#[test]
fn integer_floor_scale_zero_scale_and_display_limits_reuse_raw_admission() {
    let mut recipe = recipe();
    recipe.height_offset = FP::from_raw(-131072);
    recipe.height_scale = FP::from_raw(98303);
    let terrain = import_png16(&fixture(), &recipe).unwrap();
    let expected: Vec<_> = SAMPLES
        .iter()
        .map(|sample| FP::from_raw(-131072 + i64::from(*sample) * 98303 / 65535))
        .collect();
    assert_eq!(terrain.heights(), expected);
    recipe.height_scale = FP::ZERO;
    assert_eq!(
        import_png16(&fixture(), &recipe).unwrap().heights(),
        [recipe.height_offset; 6]
    );
    recipe.height_offset = FP::from_raw(65_536_000_001);
    assert!(import_png16(&fixture(), &recipe).is_err());
}

#[test]
fn metadata_is_stored_sample_only_and_never_changes_revision() {
    let base = import_png16(&fixture(), &recipe()).unwrap();
    for gamma in [1, 45455, 100000, u32::MAX] {
        for significance in [1, 8, 16] {
            let bytes = png(&[
                header(3, 2, [16, 0, 0, 0, 0]),
                chunk(b"gAMA", &gamma.to_be_bytes()),
                chunk(b"sRGB", &[0]),
                chunk(b"sBIT", &[significance]),
                chunk(b"IDAT", &stored_zlib(&ROWS)),
                chunk(b"IEND", &[]),
            ]);
            let terrain = import_png16(&bytes, &recipe()).unwrap();
            assert_eq!(terrain, base);
            assert_eq!(terrain.cook(), base.cook());
            assert_eq!(terrain.revision(), base.revision());
        }
    }
}

#[test]
fn png_library_handles_all_standard_row_filters_without_conversion() {
    for filter in [
        png::Filter::NoFilter,
        png::Filter::Sub,
        png::Filter::Up,
        png::Filter::Avg,
        png::Filter::Paeth,
    ] {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 3, 2);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder.set_filter(filter);
        let raw: Vec<_> = SAMPLES
            .iter()
            .flat_map(|sample| sample.to_be_bytes())
            .collect();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&raw).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            import_png16(&bytes, &recipe()).unwrap(),
            import_png16(&fixture(), &recipe()).unwrap()
        );
    }
}

#[test]
fn rejects_every_truncation_crc_failure_and_external_trailer() {
    let bytes = fixture();
    for end in 0..bytes.len() {
        assert!(
            import_png16(&bytes[..end], &recipe()).is_err(),
            "length {end}"
        );
    }
    for trailer in [vec![0], b"trailer".to_vec(), chunk(b"IEND", &[])] {
        let mut changed = bytes.clone();
        changed.extend_from_slice(&trailer);
        assert!(import_png16(&changed, &recipe()).is_err());
    }
    let chunks = [
        header(3, 2, [16, 0, 0, 0, 0]),
        chunk(b"gAMA", &45455u32.to_be_bytes()),
        chunk(b"sRGB", &[0]),
        chunk(b"sBIT", &[16]),
        chunk(b"IDAT", &stored_zlib(&ROWS)),
        chunk(b"IEND", &[]),
    ];
    for index in 0..chunks.len() {
        let mut changed = chunks.clone();
        *changed[index].last_mut().unwrap() ^= 1;
        assert!(
            import_png16(&png(&changed), &recipe()).is_err(),
            "CRC {index}"
        );
    }
}

#[test]
fn rejects_missing_or_corrupt_adler_and_compressed_trailers() {
    let data = stored_zlib(&ROWS);
    for missing in 1..=4 {
        assert!(import_png16(&fixture_data(&data[..data.len() - missing]), &recipe()).is_err());
    }
    let mut bad_adler = data.clone();
    *bad_adler.last_mut().unwrap() ^= 1;
    assert!(import_png16(&fixture_data(&bad_adler), &recipe()).is_err());
    for length in 1..=16 {
        let mut changed = data.clone();
        changed.extend(std::iter::repeat_n(0, length));
        assert!(
            import_png16(&fixture_data(&changed), &recipe()).is_err(),
            "trailer {length}"
        );
    }
    let mut concatenated = data.clone();
    concatenated.extend_from_slice(&data);
    assert!(import_png16(&fixture_data(&concatenated), &recipe()).is_err());
}

#[test]
fn rejects_excess_inflate_short_rows_invalid_filter_and_bad_deflate() {
    for rows in [ROWS[..13].to_vec(), [ROWS.to_vec(), vec![0]].concat()] {
        assert!(import_png16(&fixture_data(&stored_zlib(&rows)), &recipe()).is_err());
    }
    let bomb = fdeflate::compress_to_vec(&vec![0; 2 * MAX_PNG_BYTES]);
    assert!(bomb.len() < MAX_PNG_BYTES);
    assert!(import_png16(&fixture_data(&bomb), &recipe()).is_err());
    let mut rows = ROWS;
    rows[0] = 5;
    assert!(import_png16(&fixture_data(&stored_zlib(&rows)), &recipe()).is_err());
    let mut data = stored_zlib(&ROWS);
    data[2] = 0b111; // Reserved DEFLATE block type.
    assert!(import_png16(&fixture_data(&data), &recipe()).is_err());
}

#[test]
fn accepts_consecutive_idat_splits_but_rejects_layout_and_chunk_count_errors() {
    let data = stored_zlib(&ROWS);
    for split in 0..=data.len() {
        let bytes = png(&[
            header(3, 2, [16, 0, 0, 0, 0]),
            chunk(b"IDAT", &data[..split]),
            chunk(b"IDAT", &[]),
            chunk(b"IDAT", &data[split..]),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe()).is_ok(), "split {split}");
    }
    let h = header(3, 2, [16, 0, 0, 0, 0]);
    let d = chunk(b"IDAT", &data);
    let e = chunk(b"IEND", &[]);
    for chunks in [
        vec![d.clone(), h.clone(), e.clone()],
        vec![h.clone(), h.clone(), d.clone(), e.clone()],
        vec![h.clone(), e.clone()],
        vec![h.clone(), chunk(b"IDAT", &[]), e.clone()],
        vec![
            h.clone(),
            d.clone(),
            chunk(b"gAMA", &1u32.to_be_bytes()),
            d.clone(),
            e.clone(),
        ],
        vec![h.clone(), d.clone(), chunk(b"IEND", &[0])],
    ] {
        assert!(import_png16(&png(&chunks), &recipe()).is_err());
    }
    let mut chunks = vec![h, d];
    chunks.extend(std::iter::repeat_n(chunk(b"IDAT", &[]), MAX_PNG_CHUNKS - 3));
    chunks.push(e.clone());
    assert!(import_png16(&png(&chunks), &recipe()).is_ok());
    chunks.insert(2, chunk(b"IDAT", &[]));
    assert!(import_png16(&png(&chunks), &recipe()).is_err());
}

#[test]
fn rejects_every_non_allowlisted_chunk_and_invalid_metadata() {
    for kind in [
        b"tRNS", b"PLTE", b"acTL", b"fcTL", b"fdAT", b"tEXt", b"zTXt", b"iTXt", b"iCCP", b"pHYs",
        b"eXIf", b"cHRM", b"ABCD", b"abCd",
    ] {
        let bytes = png(&[
            header(3, 2, [16, 0, 0, 0, 0]),
            chunk(kind, &[0; 32]),
            chunk(b"IDAT", &stored_zlib(&ROWS)),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe()).is_err(), "chunk {kind:?}");
    }
    for (kind, data) in [
        (b"gAMA", vec![]),
        (b"gAMA", vec![0; 4]),
        (b"gAMA", vec![1; 5]),
        (b"sRGB", vec![]),
        (b"sRGB", vec![4]),
        (b"sRGB", vec![0; 2]),
        (b"sBIT", vec![]),
        (b"sBIT", vec![0]),
        (b"sBIT", vec![17]),
        (b"sBIT", vec![16; 2]),
    ] {
        let bytes = png(&[
            header(3, 2, [16, 0, 0, 0, 0]),
            chunk(kind, &data),
            chunk(b"IDAT", &stored_zlib(&ROWS)),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe()).is_err());
    }
    for (kind, data) in [
        (b"gAMA", vec![0, 0, 0, 1]),
        (b"sRGB", vec![0]),
        (b"sBIT", vec![16]),
    ] {
        let bytes = png(&[
            header(3, 2, [16, 0, 0, 0, 0]),
            chunk(kind, &data),
            chunk(kind, &data),
            chunk(b"IDAT", &stored_zlib(&ROWS)),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe()).is_err());
    }
}

#[test]
fn dimensions_recipe_formats_and_encoded_limits_are_admitted_before_decode() {
    for (width, height) in [(2, 3), (0, 2), (1, 2), (130, 2), (u32::MAX, u32::MAX)] {
        let bytes = png(&[
            header(width, height, [16, 0, 0, 0, 0]),
            chunk(b"IDAT", &[0]),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe())
            .unwrap_err()
            .0
            .contains("dimensions"));
    }
    for dimension in [0, 1, 130, u32::MAX] {
        let mut changed = recipe();
        changed.width = dimension;
        assert!(import_png16(&[], &changed)
            .unwrap_err()
            .0
            .contains("width and depth"));
        let mut changed = recipe();
        changed.depth = dimension;
        assert!(import_png16(&[], &changed).is_err());
    }
    for format in [
        [8, 0, 0, 0, 0],
        [16, 2, 0, 0, 0],
        [8, 3, 0, 0, 0],
        [16, 4, 0, 0, 0],
        [16, 6, 0, 0, 0],
        [16, 0, 0, 0, 1],
        [16, 0, 1, 0, 0],
        [16, 0, 0, 1, 0],
    ] {
        let bytes = png(&[
            header(3, 2, format),
            chunk(b"IDAT", &[0]),
            chunk(b"IEND", &[]),
        ]);
        assert!(import_png16(&bytes, &recipe())
            .unwrap_err()
            .0
            .contains("requires grayscale"));
    }
    let mut bytes = fixture();
    bytes.resize(MAX_PNG_BYTES + 1, 0);
    assert!(import_png16(&bytes, &recipe()).is_err());
    let mut overflow = SIGNATURE.to_vec();
    overflow.extend_from_slice(&u32::MAX.to_be_bytes());
    overflow.extend_from_slice(b"IHDR");
    assert!(import_png16(&overflow, &recipe()).is_err());
}

#[test]
fn maximum_dimensions_and_encoded_size_have_fixed_bounded_output() {
    let mut max_recipe = recipe();
    max_recipe.width = 129;
    max_recipe.depth = 129;
    let rows = vec![0; (2 * 129 + 1) * 129];
    let bytes = png(&[
        header(129, 129, [16, 0, 0, 0, 0]),
        chunk(b"IDAT", &stored_zlib(&rows)),
        chunk(b"IEND", &[]),
    ]);
    assert_eq!(
        import_png16(&bytes, &max_recipe).unwrap().heights().len(),
        129 * 129
    );
    // Exactly the input cap is admitted, then rejected by strict zlib rather
    // than attempting unbounded decompression or allocation.
    let mut payload = stored_zlib(&ROWS);
    payload.resize(MAX_PNG_BYTES - 57, 0);
    let bytes = fixture_data(&payload);
    assert_eq!(bytes.len(), MAX_PNG_BYTES);
    assert!(import_png16(&bytes, &recipe())
        .unwrap_err()
        .0
        .contains("zlib stream"));

    // A valid near-cap file can spend encoded bytes on empty DEFLATE blocks.
    // This traverses almost the full bytewise input budget without extra rows.
    let source = stored_zlib(&ROWS);
    let mut payload = source[..2].to_vec();
    for _ in 0..(MAX_PNG_BYTES - 57 - source.len()) / 5 {
        payload.extend_from_slice(&[0, 0, 0, 255, 255]);
    }
    payload.extend_from_slice(&source[2..]);
    let bytes = fixture_data(&payload);
    assert!((MAX_PNG_BYTES - 4..=MAX_PNG_BYTES).contains(&bytes.len()));
    assert!(import_png16(&bytes, &recipe()).is_ok());
}

#[test]
fn input_retains_regular_file_bytes_aliases_and_rejects_invalid_paths_or_sizes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("height.png");
    fs::write(&path, fixture()).unwrap();
    let snapshot = input::read_png16(&path).unwrap();
    assert_eq!(snapshot, fixture());
    let alias = directory.path().join("alias.png");
    fs::hard_link(&path, &alias).unwrap();
    assert_eq!(input::read_png16(&alias).unwrap(), snapshot);
    fs::write(&path, b"not a PNG source").unwrap();
    assert!(input::read_png16(&path).is_err());
    assert!(import_png16(&snapshot, &recipe()).is_ok());
    assert!(input::read_png16(directory.path()).is_err());
    assert!(input::read_png16(Path::new("")).is_err());
    assert!(input::read_png16(&directory.path().join("../height.png")).is_err());
    fs::write(&path, []).unwrap();
    assert!(input::read_png16(&path).is_err());
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_PNG_BYTES as u64 + 1)
        .unwrap();
    assert!(input::read_png16(&path).is_err());
}

#[cfg(unix)]
#[test]
fn input_rejects_symlink_destination_and_ancestry() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("height.png");
    fs::write(&path, fixture()).unwrap();
    let alias = directory.path().join("alias.png");
    symlink(&path, &alias).unwrap();
    assert!(input::read_png16(&alias).is_err());
    let ancestry = directory.path().join("linked");
    symlink(directory.path(), &ancestry).unwrap();
    assert!(input::read_png16(&ancestry.join("height.png")).is_err());
}

#[cfg(unix)]
#[test]
#[ignore = "requires a host permitting local Unix socket creation; restricted-host EPERM is retained separately"]
fn input_rejects_socket_on_permitted_host() {
    use std::os::unix::net::UnixListener;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("socket.png");
    let _listener = UnixListener::bind(&socket).unwrap();
    assert!(input::read_png16(&socket).is_err());
}
