use super::*;
use orr_sprite::{Atlas, ClipSource, Frame, PlaybackMode, Region, SpriteSource};
use std::fs;

fn png_bytes(variant: u8) -> Vec<u8> {
    let rgba = [
        255, variant, 0, 255, // region 0
        0, 255, variant, 255, // region 1
    ];
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    bytes
}

fn json_bytes(image: &str) -> Vec<u8> {
    sprite_json(
        image,
        vec![
            Region {
                id: 0,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            Region {
                id: 1,
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            },
        ],
        vec![ClipSource {
            id: "walk".into(),
            mode: PlaybackMode::Loop,
            frames: vec![Frame {
                region: 0,
                duration_ms: 45,
            }],
        }],
    )
}

fn sprite_json(image: &str, regions: Vec<Region>, clips: Vec<ClipSource>) -> Vec<u8> {
    let document = SpriteDocument::new(SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: image.into(),
            width: 2,
            height: 1,
        },
        regions,
        clips,
    })
    .unwrap();
    document.to_json().unwrap().into_bytes()
}

fn noncontiguous_regions() -> Vec<Region> {
    vec![
        Region {
            id: 2,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        Region {
            id: 9,
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        },
    ]
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Sources {
    _temp: tempfile::TempDir,
    root: PathBuf,
    json: PathBuf,
    png: PathBuf,
    license: PathBuf,
    json_bytes: Vec<u8>,
    png_bytes: Vec<u8>,
    license_bytes: Vec<u8>,
    inspection: Inspection,
}

fn sources() -> Sources {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let json = root.join("sprites.source.json");
    let png = root.join("actor_atlas.png");
    let license = root.join("LICENSE.txt");
    let json_bytes = json_bytes("actor_atlas.png");
    let png_bytes = png_bytes(19);
    let license_bytes = b"Redistribution rights reviewed\r\n".to_vec();
    fs::write(&json, &json_bytes).unwrap();
    fs::write(&png, &png_bytes).unwrap();
    fs::write(&license, &license_bytes).unwrap();
    let inspection = inspect_sources(&json, &png).unwrap();
    Sources {
        _temp: temp,
        root,
        json,
        png,
        license,
        json_bytes,
        png_bytes,
        license_bytes,
        inspection,
    }
}

fn request(sources: &Sources, destination: PathBuf) -> Request {
    Request {
        inspection: sources.inspection.clone(),
        license_path: sources.license.clone(),
        destination,
        name: "actor-sprites".into(),
        version: "1.2.3".into(),
        license_acknowledged: true,
    }
}

#[test]
fn inspection_is_read_only_and_reports_typed_document_and_exact_hashes() {
    let sources = sources();
    let inspected = &sources.inspection;
    assert_eq!(inspected.image_filename(), "actor_atlas.png");
    assert_eq!((inspected.width(), inspected.height()), (2, 1));
    assert_eq!((inspected.region_count(), inspected.clip_count()), (2, 1));
    assert_eq!(inspected.json_sha256(), hash(&sources.json_bytes));
    assert_eq!(inspected.png_sha256(), hash(&sources.png_bytes));
    assert_eq!(fs::read(&sources.json).unwrap(), sources.json_bytes);
    assert_eq!(fs::read(&sources.png).unwrap(), sources.png_bytes);
    assert_eq!(fs::read_dir(&sources.root).unwrap().count(), 3);
}

#[test]
fn package_creation_copies_original_json_png_and_license_bytes() {
    let sources = sources();
    let destination = sources.root.join("new-package");
    let report = create_package(request(&sources, destination.clone())).unwrap();
    assert_eq!(report.root, destination);
    assert_eq!(report.package, "actor-sprites");
    assert_eq!(report.document, "sprites.json");
    assert_eq!(report.image_filename, "actor_atlas.png");
    assert_eq!((report.width, report.height), (2, 1));
    assert_eq!((report.region_count, report.clip_count), (2, 1));
    assert_eq!(report.json_sha256, hash(&sources.json_bytes));
    assert_eq!(report.png_sha256, hash(&sources.png_bytes));
    assert_eq!(
        fs::read(report.root.join(&report.document)).unwrap(),
        sources.json_bytes
    );
    assert_eq!(
        fs::read(report.root.join(&report.image_filename)).unwrap(),
        sources.png_bytes
    );
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        sources.license_bytes
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(manifest["capabilities"], serde_json::json!(["sprite"]));
    assert_eq!(manifest["dependencies"], serde_json::json!({}));
    let mut output_files = fs::read_dir(&report.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    output_files.sort();
    assert_eq!(
        output_files,
        vec![
            "LICENSE.txt".to_owned(),
            "actor_atlas.png".to_owned(),
            "orr.package.json".to_owned(),
            "sprites.json".to_owned(),
        ]
    );
    assert_eq!(fs::read(&sources.json).unwrap(), sources.json_bytes);
    assert_eq!(fs::read(&sources.png).unwrap(), sources.png_bytes);
    assert_eq!(fs::read(&sources.license).unwrap(), sources.license_bytes);
}

#[test]
fn valid_region_only_document_packages_without_clips_and_preserves_source_bytes() {
    let sources = sources();
    let raw_json = sprite_json("actor_atlas.png", noncontiguous_regions(), Vec::new());
    fs::write(&sources.json, &raw_json).unwrap();
    let inspection = inspect_sources(&sources.json, &sources.png).unwrap();
    assert_eq!((inspection.region_count(), inspection.clip_count()), (2, 0));

    let destination = sources.root.join("region-only-package");
    let report = create_package(Request {
        inspection,
        license_path: sources.license.clone(),
        destination: destination.clone(),
        name: "region-only".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
    })
    .unwrap();
    assert_eq!(report.region_count, 2);
    assert_eq!(report.clip_count, 0);
    assert_eq!(
        fs::read(report.root.join("sprites.json")).unwrap(),
        raw_json
    );
    assert_eq!(
        fs::read(report.root.join("actor_atlas.png")).unwrap(),
        sources.png_bytes
    );
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        sources.license_bytes
    );
}

#[test]
fn multiple_clips_keep_repeated_noncontiguous_frames_modes_and_raw_json() {
    let sources = sources();
    let raw_json = sprite_json(
        "actor_atlas.png",
        noncontiguous_regions(),
        vec![
            ClipSource {
                id: "cycle".into(),
                mode: PlaybackMode::Loop,
                frames: vec![
                    Frame {
                        region: 9,
                        duration_ms: 3,
                    },
                    Frame {
                        region: 2,
                        duration_ms: 5,
                    },
                    Frame {
                        region: 9,
                        duration_ms: 7,
                    },
                ],
            },
            ClipSource {
                id: "hold".into(),
                mode: PlaybackMode::Once,
                frames: vec![
                    Frame {
                        region: 2,
                        duration_ms: 11,
                    },
                    Frame {
                        region: 9,
                        duration_ms: 13,
                    },
                    Frame {
                        region: 2,
                        duration_ms: 17,
                    },
                ],
            },
        ],
    );
    let mut raw_json_with_whitespace = raw_json;
    raw_json_with_whitespace.extend_from_slice(b"\r\n \t");
    fs::write(&sources.json, &raw_json_with_whitespace).unwrap();
    let inspection = inspect_sources(&sources.json, &sources.png).unwrap();
    assert_eq!((inspection.region_count(), inspection.clip_count()), (2, 2));

    let destination = sources.root.join("multiple-clips-package");
    let report = create_package(Request {
        inspection,
        license_path: sources.license.clone(),
        destination,
        name: "multi-clip".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
    })
    .unwrap();
    let packaged_bytes = fs::read(report.root.join("sprites.json")).unwrap();
    assert_eq!(packaged_bytes, raw_json_with_whitespace);
    let packaged =
        SpriteDocument::from_json(std::str::from_utf8(&packaged_bytes).unwrap()).unwrap();
    let cycle = packaged.clip("cycle").unwrap();
    assert_eq!(cycle.mode(), PlaybackMode::Loop);
    assert_eq!(
        cycle
            .frames()
            .iter()
            .map(|frame| frame.region)
            .collect::<Vec<_>>(),
        vec![9, 2, 9]
    );
    assert_eq!(cycle.duration_ms(), 15);
    assert_eq!(cycle.sample(0).region, 9);
    assert_eq!(cycle.sample(3).region, 2);
    assert_eq!(cycle.sample(8).region, 9);
    assert_eq!(cycle.sample(15).region, 9);
    let hold = packaged.clip("hold").unwrap();
    assert_eq!(hold.mode(), PlaybackMode::Once);
    assert_eq!(
        hold.frames()
            .iter()
            .map(|frame| frame.region)
            .collect::<Vec<_>>(),
        vec![2, 9, 2]
    );
    assert_eq!(hold.duration_ms(), 41);
    assert_eq!(hold.sample(0).region, 2);
    assert_eq!(hold.sample(11).region, 9);
    assert_eq!(hold.sample(24).region, 2);
    assert!(hold.sample(41).finished);
    assert_eq!(fs::read(&sources.json).unwrap(), raw_json_with_whitespace);
    assert_eq!(
        fs::read(report.root.join("actor_atlas.png")).unwrap(),
        sources.png_bytes
    );
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        sources.license_bytes
    );
}

#[test]
fn changed_json_bytes_are_rejected_even_when_the_typed_summary_is_unchanged() {
    let sources = sources();
    let mut changed = sources.json_bytes.clone();
    changed.extend_from_slice(b" \n");
    // Trailing whitespace remains a valid SpriteDocument with the same summary.
    assert!(SpriteDocument::from_json(std::str::from_utf8(&changed).unwrap()).is_ok());
    fs::write(&sources.json, &changed).unwrap();
    let destination = sources.root.join("new-package");
    let error = create_package(request(&sources, destination.clone())).unwrap_err();
    assert!(error.contains("JSON changed since inspection"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read(&sources.json).unwrap(), changed);
    assert_eq!(fs::read(&sources.png).unwrap(), sources.png_bytes);
}

#[test]
fn acknowledgement_and_store_alias_checks_precede_source_reads() {
    let sources = sources();
    let mut unacknowledged = request(&sources, sources.root.join("new-package"));
    unacknowledged.license_acknowledged = false;
    fs::remove_file(&sources.json).unwrap();
    assert!(
        create_package(unacknowledged)
            .unwrap_err()
            .contains("acknowledgement")
    );

    for alias in [".ORR", ".orr.", ".orr "] {
        let mut request = request(&sources, sources.root.join(alias).join("forbidden"));
        request.license_path = sources.root.join("missing-license");
        let error = create_package(request).unwrap_err();
        assert!(error.contains(".orr store"), "{alias}: {error}");
        assert!(!sources.root.join(alias).exists());
    }
}

#[test]
fn existing_destination_is_rejected_before_source_recapture() {
    let sources = sources();
    let destination = sources.root.join("already-there");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("keep.bin");
    fs::write(&sentinel, b"existing user data").unwrap();
    fs::remove_file(&sources.json).unwrap();
    let error = create_package(request(&sources, destination.clone())).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert!(destination.is_dir());
    assert_eq!(fs::read(&sentinel).unwrap(), b"existing user data");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
}

#[test]
fn inspection_rejects_invalid_json_oversize_and_png_dimension_mismatch() {
    let sources = sources();
    fs::write(&sources.json, [0xff, 0xfe]).unwrap();
    assert!(
        inspect_sources(&sources.json, &sources.png)
            .unwrap_err()
            .contains("UTF-8")
    );

    fs::write(&sources.json, vec![b' '; MAX_JSON_BYTES + 1]).unwrap();
    assert!(
        inspect_sources(&sources.json, &sources.png)
            .unwrap_err()
            .contains("byte limit")
    );

    fs::write(&sources.json, json_bytes("actor_atlas.png")).unwrap();
    let mut wide_png = Vec::new();
    {
        let rgba = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
        let mut encoder = png::Encoder::new(&mut wide_png, 3, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    fs::write(&sources.png, wide_png).unwrap();
    assert!(
        inspect_sources(&sources.json, &sources.png)
            .unwrap_err()
            .contains("do not match inspected PNG dimensions")
    );
}

#[test]
fn typed_document_rejects_unknown_fields_duplicate_regions_and_invalid_durations() {
    let sources = sources();
    let baseline: serde_json::Value = serde_json::from_slice(&sources.json_bytes).unwrap();
    let mut unknown_field = baseline.clone();
    unknown_field["unexpected"] = serde_json::json!(true);

    let mut duplicate_region = baseline.clone();
    let first_region = duplicate_region["regions"][0].clone();
    duplicate_region["regions"]
        .as_array_mut()
        .unwrap()
        .push(first_region);

    let mut zero_duration = baseline.clone();
    zero_duration["clips"][0]["frames"][0]["duration_ms"] = serde_json::json!(0);

    let mut overflowing_duration = baseline;
    overflowing_duration["clips"][0]["frames"] = serde_json::json!([
        { "region": 0, "duration_ms": u64::MAX },
        { "region": 1, "duration_ms": 1 }
    ]);

    let cases = [
        (unknown_field, "unknown field"),
        (duplicate_region, "duplicate region"),
        (zero_duration, "zero duration"),
        (overflowing_duration, "duration overflow"),
    ];
    for (value, expected_error) in cases {
        let json = serde_json::to_vec(&value).unwrap();
        fs::write(&sources.json, &json).unwrap();
        let error = inspect_sources(&sources.json, &sources.png).unwrap_err();
        assert!(
            error.contains(expected_error),
            "expected {expected_error}: {error}"
        );
    }
}

#[test]
fn changed_same_dimension_png_is_rejected_by_checked_core_before_publication() {
    let sources = sources();
    let destination = sources.root.join("new-package");
    let changed_png = png_bytes(31);
    fs::write(&sources.png, &changed_png).unwrap();
    let error = create_package(request(&sources, destination.clone())).unwrap_err();
    assert!(error.contains("inspected PNG changed"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read(&sources.png).unwrap(), changed_png);
    assert_eq!(fs::read(&sources.json).unwrap(), sources.json_bytes);
}

#[test]
fn opaque_typed_summary_is_rechecked_before_the_checked_writer() {
    let sources = sources();
    let mut request = request(&sources, sources.root.join("new-package"));
    request.inspection.summary.region_count += 1;
    let destination = request.destination.clone();
    let error = create_package(request).unwrap_err();
    assert!(
        error.contains("summary changed since inspection"),
        "{error}"
    );
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn unix_symlink_sources_and_destination_ancestry_are_rejected() {
    use std::os::unix::fs::symlink;

    let sources = sources();
    let json_link = sources.root.join("sprites-link.json");
    symlink(&sources.json, &json_link).unwrap();
    assert!(
        inspect_sources(&json_link, &sources.png)
            .unwrap_err()
            .contains("linked or reparse ancestry")
    );

    let png_link = sources.root.join("atlas-link.png");
    symlink(&sources.png, &png_link).unwrap();
    assert!(inspect_sources(&sources.json, &png_link).is_err());

    let real_parent = sources.root.join("real-parent");
    fs::create_dir(&real_parent).unwrap();
    let linked_parent = sources.root.join("linked-parent");
    symlink(&real_parent, &linked_parent).unwrap();
    let destination = linked_parent.join("output");
    let error = create_package(request(&sources, destination.clone())).unwrap_err();
    assert!(error.contains("linked or reparse ancestry"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read(&sources.json).unwrap(), sources.json_bytes);
    assert_eq!(fs::read(&sources.png).unwrap(), sources.png_bytes);
    assert_eq!(fs::read(&sources.license).unwrap(), sources.license_bytes);
}
