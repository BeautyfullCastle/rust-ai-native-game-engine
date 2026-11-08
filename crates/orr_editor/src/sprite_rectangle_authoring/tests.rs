use super::*;
use sha2::{Digest, Sha256};
use std::fs;

fn image(width: u32, height: u32) -> PngInspection {
    PngInspection {
        width,
        height,
        sha256: "a".repeat(64),
    }
}

fn regions() -> Vec<orr_sprite::Region> {
    vec![
        orr_sprite::Region {
            id: 17,
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        },
        orr_sprite::Region {
            id: 0,
            x: 2,
            y: 1,
            width: 4,
            height: 3,
        },
        orr_sprite::Region {
            id: 4,
            x: 4,
            y: 0,
            width: 2,
            height: 2,
        },
    ]
}

fn spec() -> RectangleSpec {
    RectangleSpec { regions: regions() }
}

fn pixel(x: u32, y: u32, variant: u8) -> [u8; 4] {
    [(x * 31) as u8, (y * 47) as u8, variant, 255]
}

fn png_bytes(variant: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    let rgba: Vec<_> = (0..4)
        .flat_map(|y| (0..6).flat_map(move |x| pixel(x, y, variant)))
        .collect();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 6, 4);
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

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn request(root: &Path) -> Request {
    let png = root.join("source.png");
    let license = root.join("LICENSE.txt");
    fs::write(&png, png_bytes(19)).unwrap();
    fs::write(&license, b"Rights reviewed\n").unwrap();
    let inspection = inspect_png(&png).unwrap();
    Request {
        png,
        license,
        destination: root.join("new-package"),
        name: "rectangle-actor".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        inspection,
        spec: spec(),
    }
}

fn unopened() -> Request {
    Request {
        png: PathBuf::new(),
        license: PathBuf::new(),
        destination: PathBuf::new(),
        name: "rectangle-actor".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        inspection: image(6, 4),
        spec: spec(),
    }
}

#[test]
fn rectangles_preserve_order_ids_overlap_zero_origin_and_exact_edges() {
    let input = vec![
        orr_sprite::Region {
            id: 91,
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        },
        orr_sprite::Region {
            id: 0,
            x: 2,
            y: 1,
            width: 4,
            height: 3,
        },
        orr_sprite::Region {
            id: 7,
            x: 4,
            y: 0,
            width: 2,
            height: 4,
        },
    ];
    let doc = validate_rectangle_spec(
        &RectangleSpec {
            regions: input.clone(),
        },
        &image(6, 4),
    )
    .unwrap();
    assert_eq!(doc.regions(), input);
    assert!(doc.clips().is_empty());
    assert_eq!(doc.region(0), Some(&input[1]));
    assert_eq!(doc.uv_rect(7), Some([4.0 / 6.0, 0.0, 1.0, 1.0]));
}

#[test]
fn rectangle_count_limits_are_1_through_256() {
    let mut empty = RectangleSpec { regions: vec![] };
    assert!(validate_rectangle_spec(&empty, &image(6, 4))
        .unwrap_err()
        .contains("1..=256"));
    empty.regions = (0..256)
        .map(|id| orr_sprite::Region {
            id,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        })
        .collect();
    assert_eq!(
        validate_rectangle_spec(&empty, &image(6, 4))
            .unwrap()
            .regions()
            .len(),
        256
    );
    empty.regions.push(orr_sprite::Region {
        id: 256,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    assert!(validate_rectangle_spec(&empty, &image(6, 4))
        .unwrap_err()
        .contains("1..=256"));
}

#[test]
fn constructor_rejects_duplicate_ids_empty_rectangles_overflow_and_out_of_bounds() {
    let mut duplicate = spec();
    duplicate.regions[1].id = duplicate.regions[0].id;
    assert!(validate_rectangle_spec(&duplicate, &image(6, 4))
        .unwrap_err()
        .contains("duplicate region"));

    for mutate in [
        |r: &mut Vec<orr_sprite::Region>| r[0].width = 0,
        |r: &mut Vec<orr_sprite::Region>| r[0].height = 0,
        |r: &mut Vec<orr_sprite::Region>| r[0].x = u32::MAX,
        |r: &mut Vec<orr_sprite::Region>| r[0].y = u32::MAX,
        |r: &mut Vec<orr_sprite::Region>| r[0].x = 4,
        |r: &mut Vec<orr_sprite::Region>| r[0].y = 3,
    ] {
        let mut malformed = spec();
        mutate(&mut malformed.regions);
        assert!(validate_rectangle_spec(&malformed, &image(6, 4)).is_err());
    }
}

#[test]
fn inspection_hash_and_dimensions_are_bounded() {
    for bad_hash in ["".into(), "z".repeat(64), "a".repeat(63), "a".repeat(65)] {
        let mut inspected = image(6, 4);
        inspected.sha256 = bad_hash;
        assert!(validate_rectangle_spec(&spec(), &inspected)
            .unwrap_err()
            .contains("64 hexadecimal"));
    }
    for (width, height) in [(0, 4), (6, 0), (2049, 4), (6, 2049), (u32::MAX, u32::MAX)] {
        assert!(validate_rectangle_spec(&spec(), &image(width, height))
            .unwrap_err()
            .contains("1..=2048"));
    }
    let mut edge = RectangleSpec {
        regions: vec![orr_sprite::Region {
            id: 0,
            x: 2047,
            y: 2047,
            width: 1,
            height: 1,
        }],
    };
    assert!(validate_rectangle_spec(&edge, &image(2048, 2048)).is_ok());
    edge.regions[0].width = 2;
    assert!(validate_rectangle_spec(&edge, &image(2048, 2048)).is_err());
}

#[test]
fn rectangle_png_inspection_is_read_only_and_fully_decodes_the_captured_image() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("image.png");
    let bytes = png_bytes(19);
    fs::write(&path, &bytes).unwrap();
    let inspected = inspect_png(&path).unwrap();
    assert_eq!(
        inspected,
        PngInspection {
            width: 6,
            height: 4,
            sha256: hash(&bytes),
        }
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::write(&path, &bytes[..33]).unwrap();
    assert!(inspect_png(&path).is_err());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn publication_copies_exact_inputs_and_emits_installable_empty_clip_package() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    let png = req.png.clone();
    let license = req.license.clone();
    let before = fs::read(&png).unwrap();
    let expected = validate_rectangle_spec(&req.spec, &req.inspection)
        .unwrap()
        .to_json()
        .unwrap()
        + "\n";
    req.inspection.sha256 = req.inspection.sha256.to_uppercase();
    let report = create_package(req).unwrap();

    assert_eq!(report.root, root.join("new-package"));
    assert_eq!(report.package, "rectangle-actor");
    assert_eq!(report.document, "sprites.json");
    assert_eq!(
        (report.width, report.height, report.region_count),
        (6, 4, 3)
    );
    assert_eq!(report.source_sha256, hash(&before));
    assert_eq!(fs::read(report.root.join("atlas.png")).unwrap(), before);
    assert_eq!(fs::read(&png).unwrap(), before);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(
        fs::read_to_string(report.root.join(&report.document)).unwrap(),
        expected
    );
    let doc = SpriteDocument::from_json(&expected).unwrap();
    assert!(doc.clips().is_empty());
    assert_eq!(doc.regions(), regions());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(manifest["capabilities"], serde_json::json!(["sprite"]));
    assert_eq!(
        manifest["files"],
        serde_json::json!(["sprites.json", "atlas.png", "LICENSE.txt"])
    );
    assert_eq!(fs::read_dir(&report.root).unwrap().count(), 4);
    assert_eq!(fs::read_dir(root).unwrap().count(), 3);
}

#[test]
fn same_dimension_png_change_rejects_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root);
    let destination = req.destination.clone();
    let changed = png_bytes(23);
    fs::write(&req.png, &changed).unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let error = create_package(req).unwrap_err();
    assert!(
        error.contains("PNG") && error.contains("changed"),
        "{error}"
    );
    assert!(!destination.exists());
    assert_eq!(fs::read(root.join("source.png")).unwrap(), changed);
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
fn acknowledgement_path_metadata_and_license_errors_never_publish() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();

    let mut req = unopened();
    req.license_acknowledged = false;
    assert!(create_package(req).unwrap_err().contains("acknowledgement"));
    for bad in [
        PathBuf::from("relative.png"),
        root.join("..").join("outside.png"),
    ] {
        let mut req = request(&root);
        req.png = bad;
        assert!(create_package(req).unwrap_err().contains("PNG path"));
    }

    for (name, version, expected) in [
        ("../bad", "1.0.0", "package name"),
        ("Rectangle", "1.0.0", "package name"),
        ("rectangle-actor", "not-semver", "semantic version"),
        ("rectangle-actor", "01.0.0", "semantic version"),
    ] {
        let mut req = request(&root);
        req.name = name.into();
        req.version = version.into();
        assert!(create_package(req).unwrap_err().contains(expected));
        assert!(!root.join("new-package").exists());
    }

    for invalid_license in [vec![], b"  \r\n".to_vec(), vec![0xff]] {
        let req = request(&root);
        fs::write(&req.license, invalid_license).unwrap();
        assert!(create_package(req).is_err());
        assert!(!root.join("new-package").exists());
    }
}

#[test]
fn existing_destination_is_preserved_and_partial_output_is_never_retried() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root);
    let destination = req.destination.clone();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep me").unwrap();
    let error = create_package(req).unwrap_err();
    assert!(error.contains(destination.to_str().unwrap()), "{error}");
    assert!(error.contains("retain"), "{error}");
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
}

#[test]
fn package_store_destination_is_rejected_before_source_or_license_reads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for store in [".orr", ".ORR", ".OrR"] {
        let req = Request {
            png: root.join("missing.png"),
            license: root.join("missing-license"),
            destination: root.join(store).join("forbidden"),
            ..unopened()
        };
        assert!(create_package(req).unwrap_err().contains(".orr store"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
}

#[cfg(windows)]
#[test]
fn win32_trailing_dot_or_space_cannot_alias_package_store() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for alias in [".orr.", ".orr ", ".orr...   "] {
        let req = Request {
            png: root.join("missing.png"),
            license: root.join("missing-license"),
            destination: root.join(alias).join("forbidden"),
            ..unopened()
        };
        assert!(create_package(req).unwrap_err().contains(".orr store"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
}

#[test]
fn core_png_and_license_byte_limits_reject_without_output_or_input_cleanup() {
    for oversized_png in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root);
        let destination = req.destination.clone();
        let target = if oversized_png {
            req.png.clone()
        } else {
            req.license.clone()
        };
        let limit = if oversized_png {
            authoring::MAX_PNG_BYTES
        } else {
            authoring::MAX_LICENSE_BYTES
        };
        fs::write(&target, vec![b'x'; limit + 1]).unwrap();
        assert!(create_package(req).is_err());
        assert!(!destination.exists());
        assert_eq!(fs::metadata(&target).unwrap().len(), (limit + 1) as u64);
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
}

#[cfg(unix)]
#[test]
fn linked_png_license_and_destination_parent_are_rejected() {
    for case in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root);
        let link = root.join("link");
        match case {
            0 => {
                std::os::unix::fs::symlink(&req.png, &link).unwrap();
                assert!(inspect_png(&link).is_err());
                req.png = link;
            }
            1 => {
                std::os::unix::fs::symlink(&req.license, &link).unwrap();
                req.license = link;
            }
            _ => {
                let parent = root.join("real-parent");
                fs::create_dir(&parent).unwrap();
                std::os::unix::fs::symlink(parent, &link).unwrap();
                req.destination = link.join("output");
            }
        }
        let destination = req.destination.clone();
        assert!(create_package(req).is_err());
        assert!(!destination.exists());
        assert_eq!(fs::read(root.join("source.png")).unwrap(), png_bytes(19));
        assert_eq!(
            fs::read(root.join("LICENSE.txt")).unwrap(),
            b"Rights reviewed\n"
        );
    }
}
