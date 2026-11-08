use super::*;
use sha2::{Digest, Sha256};
use std::fs;
fn spec() -> GridSpec {
    GridSpec {
        cell_width: 2,
        cell_height: 2,
        clip: "walk".into(),
        mode: PlaybackMode::Loop,
        duration_ms: 17,
    }
}
fn image(width: u32, height: u32) -> PngInspection {
    PngInspection {
        width,
        height,
        sha256: "a".repeat(64),
    }
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
        name: "grid-actor".into(),
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
        name: "grid-actor".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        inspection: image(6, 4),
        spec: spec(),
    }
}
#[test]
fn grid_row_major_rectangles_uvs_and_pixel_clip_timing_match_independent_oracle() {
    let png = png_bytes(19);
    let rgba = orr_sprite::png_import::decode_atlas(&png, 6, 4).unwrap();
    for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
        let mut s = spec();
        s.mode = mode;
        let doc = validate_grid_spec(&s, &image(6, 4)).unwrap();
        assert_eq!(doc.regions().len(), 6);
        assert_eq!(doc.clips().len(), 1);
        for id in 0..6 {
            let col = id % 3;
            let row = id / 3;
            assert_eq!(
                *doc.region(id).unwrap(),
                Region {
                    id,
                    x: col * 2,
                    y: row * 2,
                    width: 2,
                    height: 2
                }
            );
            assert_eq!(
                doc.uv_rect(id).unwrap(),
                [
                    col as f32 / 3.0,
                    row as f32 / 2.0,
                    (col + 1) as f32 / 3.0,
                    (row + 1) as f32 / 2.0
                ]
            );
        }
        let clip = doc.clip("walk").unwrap();
        assert_eq!(clip.duration_ms(), 102);
        for time in 0..=220u64 {
            let sample = clip.sample(time);
            let expected = if mode == PlaybackMode::Loop {
                (time / 17) % 6
            } else {
                (time / 17).min(5)
            };
            assert_eq!(sample.region, u32::try_from(expected).unwrap());
            assert_eq!(sample.finished, mode == PlaybackMode::Once && time >= 102);
            let x = (expected as u32 % 3) * 2 + 1;
            let y = (expected as u32 / 3) * 2 + 1;
            let offset = ((y * 6 + x) * 4) as usize;
            assert_eq!(&rgba[offset..offset + 4], &pixel(x, y, 19));
        }
    }
}
#[test]
fn grid_collection_clip_name_and_duration_limits_are_checked_before_allocation_or_io() {
    let mut s = spec();
    s.cell_width = 1;
    s.cell_height = 1;
    let doc = validate_grid_spec(&s, &image(16, 16)).unwrap();
    assert_eq!(doc.regions().len(), 256);
    assert!(validate_grid_spec(&s, &image(17, 16)).is_err());
    assert!(validate_grid_spec(&s, &image(2048, 2048)).is_err());
    for (width, height) in [(0, 4), (6, 0), (2049, 4), (6, 2049), (u32::MAX, u32::MAX)] {
        assert!(validate_grid_spec(&spec(), &image(width, height)).is_err());
    }
    for (width, height) in [(0, 2), (2, 0), (4, 2), (2, 3), (u32::MAX, 2)] {
        let mut s = spec();
        s.cell_width = width;
        s.cell_height = height;
        assert!(validate_grid_spec(&s, &image(6, 4)).is_err());
    }
    for clip in [
        "".into(),
        "   ".into(),
        "walk\n".into(),
        "walk\0".into(),
        "x".repeat(129),
        "가".repeat(43),
    ] {
        let mut s = spec();
        s.clip = clip;
        assert!(validate_grid_spec(&s, &image(6, 4)).is_err());
    }
    let mut s = spec();
    s.clip = "가".repeat(42);
    validate_grid_spec(&s, &image(6, 4)).unwrap();
    for duration_ms in [0, u64::MAX, u64::MAX / 6 + 1] {
        let mut s = spec();
        s.duration_ms = duration_ms;
        assert!(validate_grid_spec(&s, &image(6, 4)).is_err());
    }
    let mut s = spec();
    s.cell_width = 6;
    s.cell_height = 4;
    s.duration_ms = u64::MAX;
    assert_eq!(
        validate_grid_spec(&s, &image(6, 4))
            .unwrap()
            .clip("walk")
            .unwrap()
            .duration_ms(),
        u64::MAX
    );
}
#[test]
fn grid_rights_hash_spec_and_absolute_path_admission_precede_filesystem() {
    let mut req = unopened();
    req.license_acknowledged = false;
    assert!(create_package(req).unwrap_err().contains("acknowledgement"));
    let mut req = unopened();
    req.spec.cell_width = 0;
    assert!(create_package(req).unwrap_err().contains("nonzero cell"));
    for bad in ["".into(), "x".repeat(64), "a".repeat(63), "a".repeat(65)] {
        let mut req = unopened();
        req.inspection.sha256 = bad;
        assert!(create_package(req).unwrap_err().contains("64 hexadecimal"));
    }
    assert!(create_package(unopened()).unwrap_err().contains("absolute"));
    assert!(inspect_png(Path::new("relative.png"))
        .unwrap_err()
        .contains("absolute"));
}
#[test]
fn grid_png_inspection_is_read_only_and_fully_decodes_the_captured_image() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("image.png");
    let bytes = png_bytes(19);
    fs::write(&path, &bytes).unwrap();
    let inspected = inspect_png(&path).unwrap();
    assert_eq!(inspected, image_with_hash(6, 4, hash(&bytes)));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::write(&path, &bytes[..33]).unwrap();
    assert!(inspect_png(&path).is_err());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}
fn image_with_hash(width: u32, height: u32, sha256: String) -> PngInspection {
    PngInspection {
        width,
        height,
        sha256,
    }
}
#[test]
fn grid_publication_uses_generated_schema_and_exact_original_png_license_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    let png = req.png.clone();
    let license = req.license.clone();
    let before = fs::read(&png).unwrap();
    let expected = validate_grid_spec(&req.spec, &req.inspection)
        .unwrap()
        .to_json()
        .unwrap()
        + "\n";
    req.inspection.sha256 = req.inspection.sha256.to_uppercase();
    let report = create_package(req).unwrap();
    assert_eq!(report.root, root.join("new-package"));
    assert_eq!(report.package, "grid-actor");
    assert_eq!(report.document, "sprites.json");
    assert_eq!((report.width, report.height, report.cells), (6, 4, 6));
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
    assert_eq!(doc.atlas().image, "atlas.png");
    assert_eq!(doc.clip("walk").unwrap().sample(17).region, 1);
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
fn grid_same_dimension_png_change_rejects_before_license_payload_and_publication() {
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
fn grid_forged_inspection_dimensions_and_bad_inputs_never_publish() {
    for case in 0..6 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root);
        let destination = req.destination.clone();
        match case {
            0 => req.inspection.width = 4,
            1 => fs::write(&req.png, b"not PNG").unwrap(),
            2 => fs::write(&req.license, [0xff]).unwrap(),
            3 => fs::write(&req.license, b"  ").unwrap(),
            4 => req.version = "not-semver".into(),
            _ => req.name = "../bad".into(),
        }
        let png = fs::read(&req.png).unwrap();
        let license = fs::read(&req.license).unwrap();
        assert!(create_package(req).is_err());
        assert!(!destination.exists());
        assert_eq!(fs::read(root.join("source.png")).unwrap(), png);
        assert_eq!(fs::read(root.join("LICENSE.txt")).unwrap(), license);
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
}
#[test]
fn grid_existing_partial_output_is_retained_and_error_reports_destination() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root);
    let destination = req.destination.clone();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sprites.json"), b"partial earlier write").unwrap();
    fs::write(destination.join("foreign"), b"keep").unwrap();
    let error = create_package(req).unwrap_err();
    assert!(error.contains(destination.to_str().unwrap()), "{error}");
    assert!(error.contains("retain"), "{error}");
    assert_eq!(
        fs::read(destination.join("sprites.json")).unwrap(),
        b"partial earlier write"
    );
    assert_eq!(fs::read(destination.join("foreign")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 2);
}
#[cfg(unix)]
#[test]
fn grid_linked_png_license_and_destination_parent_are_rejected_without_mutation() {
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

#[test]
fn grid_store_destination_policy_rejects_before_missing_png_or_license_io() {
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
#[test]
fn grid_png_and_license_byte_limits_reject_without_output_or_input_cleanup() {
    for oversized_png in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root);
        let destination = req.destination.clone();
        let target = if oversized_png {
            &req.png
        } else {
            &req.license
        };
        let limit = if oversized_png {
            authoring::MAX_PNG_BYTES
        } else {
            authoring::MAX_LICENSE_BYTES
        };
        fs::write(target, vec![b'x'; limit + 1]).unwrap();
        assert!(create_package(req).is_err());
        assert!(!destination.exists());
        assert_eq!(
            fs::metadata(root.join(if oversized_png {
                "source.png"
            } else {
                "LICENSE.txt"
            }))
            .unwrap()
            .len(),
            (limit + 1) as u64
        );
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
}
