use super::*;
use serde_json::json;

// Two original 2x1 RGBA PNGs with the same dimensions and different valid pixels.
// Each uses a stored DEFLATE block, PNG CRCs and a zlib Adler checksum.
const PNG_A: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 20, 73, 68, 65, 84, 120, 1, 1, 9, 0, 246, 255, 0, 255, 32,
    64, 255, 255, 32, 64, 255, 21, 55, 4, 189, 33, 106, 189, 26, 0, 0, 0, 0, 73, 69, 78, 68, 174,
    66, 96, 130,
];
const PNG_B: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 20, 73, 68, 65, 84, 120, 1, 1, 9, 0, 246, 255, 0, 21, 83,
    177, 113, 21, 83, 177, 113, 12, 113, 3, 21, 100, 211, 61, 49, 0, 0, 0, 0, 73, 69, 78, 68, 174,
    66, 96, 130,
];
const LICENSE: &[u8] = b"Original synthetic pixels; permission supplied by the caller.\n";

fn document() -> Vec<u8> {
    let mut bytes=serde_json::to_vec_pretty(&json!({"format":"orr_sprite","version":1,
        "atlas":{"image":"atlas.png","width":2,"height":1},
        "regions":[{"id":10,"x":0,"y":0,"width":1,"height":1},{"id":20,"x":1,"y":0,"width":1,"height":1}],
        "clips":[{"id":"walk","mode":"loop","frames":[{"region":10,"duration_ms":80},{"region":20,"duration_ms":120}]}]})).unwrap();
    bytes.push(b'\n');
    bytes
}
fn request(root: &Path) -> Request {
    let request = Request {
        png: root.join("input.png"),
        json: root.join("must-not-open.json"),
        license: root.join("license.txt"),
        name: "grid-sprites".into(),
        version: "1.2.3".into(),
        out: root.join("package"),
    };
    fs::write(&request.png, PNG_A).unwrap();
    fs::write(&request.license, LICENSE).unwrap();
    request
}

#[test]
fn png_inspection_fully_decodes_and_returns_exact_captured_hash_without_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("original.png");
    fs::write(&path, PNG_A).unwrap();
    assert_eq!(
        orr_sprite::png_import::decode_atlas(PNG_A, 2, 1).unwrap(),
        [255, 32, 64, 255, 255, 32, 64, 255]
    );
    assert_eq!(
        orr_sprite::png_import::decode_atlas(PNG_B, 2, 1).unwrap(),
        [21, 83, 177, 113, 21, 83, 177, 113]
    );
    let before = fs::read_dir(&root).unwrap().count();
    let inspection = inspect_png(&path).unwrap();
    assert_eq!(
        inspection,
        PngInspection {
            width: 2,
            height: 1,
            sha256: crate::hash(PNG_A)
        }
    );
    assert_eq!(inspection.clone(), inspection);
    assert_eq!(fs::read(&path).unwrap(), PNG_A);
    assert_eq!(fs::read_dir(&root).unwrap().count(), before);
    fs::write(&path, PNG_B).unwrap();
    let changed = inspect_png(&path).unwrap();
    assert_eq!(
        (changed.width, changed.height),
        (inspection.width, inspection.height)
    );
    assert_ne!(changed.sha256, inspection.sha256);
}

#[test]
fn png_inspection_rejects_header_only_corrupt_dimensions_nonregular_and_oversized_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("invalid.png");
    for bytes in [b"not PNG".as_slice(), &PNG_A[..24], &PNG_A[..44]] {
        fs::write(&path, bytes).unwrap();
        assert!(inspect_png(&path).is_err());
    }
    let mut corrupt = PNG_A.to_vec();
    corrupt[50] ^= 1;
    fs::write(&path, &corrupt).unwrap();
    assert!(inspect_png(&path).is_err());
    for (offset, side) in [(16, 0u32), (20, 0), (16, 2049), (20, 2049)] {
        let mut bytes = PNG_A.to_vec();
        bytes[offset..offset + 4].copy_from_slice(&side.to_be_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(inspect_png(&path).unwrap_err().contains("within 1..=2048"));
    }
    let directory = root.join("directory.png");
    fs::create_dir(&directory).unwrap();
    assert!(inspect_png(&directory).is_err());
    assert!(inspect_png(&root.join("missing.png")).is_err());
    File::create(&path)
        .unwrap()
        .set_len(MAX_PNG_BYTES as u64 + 1)
        .unwrap();
    assert!(inspect_png(&path)
        .unwrap_err()
        .contains("bounded regular file"));
    assert!(!root.join("package").exists());
}

#[test]
fn checked_png_hash_syntax_rejects_before_any_path_name_or_json_access() {
    let request = Request {
        png: "unopened.png".into(),
        json: "unopened.json".into(),
        license: "unopened-license".into(),
        name: "INVALID NAME".into(),
        version: "bad version".into(),
        out: "unopened-parent/output".into(),
    };
    for hash in [
        "".into(),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        "a ".repeat(32),
        "é".repeat(32),
    ] {
        assert!(import_package_bytes_checked(&request, b"bad JSON", &hash)
            .unwrap_err()
            .contains("64 hexadecimal"));
    }
}

#[test]
fn stale_valid_same_dimension_png_rejects_before_license_read_or_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let request = request(&root);
    let inspected = inspect_png(&request.png).unwrap();
    fs::write(&request.png, PNG_B).unwrap();
    fs::remove_file(&request.license).unwrap();
    let error = import_package_bytes_checked(&request, &document(), &inspected.sha256).unwrap_err();
    assert!(error.contains("inspected PNG changed"));
    assert!(!error.contains("license"));
    assert!(!request.out.exists());
    assert!(!request.json.exists());
    assert!(!request.license.exists());
    assert_eq!(fs::read(&request.png).unwrap(), PNG_B);
}

#[test]
fn checked_import_copies_exact_png_json_and_license_and_accepts_uppercase_hash() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let request = request(&root);
    let json = document();
    let inspected = inspect_png(&request.png).unwrap();
    let report =
        import_package_bytes_checked(&request, &json, &inspected.sha256.to_ascii_uppercase())
            .unwrap();
    assert_eq!(
        report,
        Report {
            width: 2,
            height: 1,
            rgba_bytes: 8,
            image: "atlas.png".into()
        }
    );
    assert_eq!(fs::read(request.out.join("atlas.png")).unwrap(), PNG_A);
    assert_eq!(fs::read(request.out.join("sprites.json")).unwrap(), json);
    assert_eq!(fs::read(request.out.join("LICENSE.txt")).unwrap(), LICENSE);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(request.out.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["files"],
        json!(["sprites.json", "atlas.png", "LICENSE.txt"])
    );
    assert_eq!(manifest["capabilities"], json!(["sprite"]));
    let parsed = SpriteDocument::from_json(std::str::from_utf8(&json).unwrap()).unwrap();
    assert_eq!(parsed.clip("walk").unwrap().duration_ms(), 200);
    assert_eq!(fs::read(&request.png).unwrap(), PNG_A);
    assert_eq!(fs::read(&request.license).unwrap(), LICENSE);
    assert!(!request.json.exists());
}

#[test]
fn legacy_in_memory_file_and_sprite_gen_entrypoints_keep_original_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut request = request(&root);
    let json = document();
    request.out = root.join("memory");
    let memory = import_package_bytes(&request, &json).unwrap();
    assert!(!request.json.exists());
    assert_eq!(fs::read(request.out.join("sprites.json")).unwrap(), json);
    request.json = root.join("provided.json");
    fs::write(&request.json, &json).unwrap();
    request.out = root.join("file");
    assert_eq!(import_package(&request).unwrap(), memory);
    assert_eq!(fs::read(request.out.join("sprites.json")).unwrap(), json);
    let manifest=json!({"engine":"component-row","degraded_static_fallback":false,
        "game_input":"never-open.png","sprite_sheet_alpha":"never-open.png",
        "frame_layout":{"sheetWidth":2,"sheetHeight":1,"rows":{"walk":[{"x":0,"y":0,"w":1,"h":1},{"x":1,"y":0,"w":1,"h":1}]}},
        "animation":{"columns":2,"rows":{"walk":{"row":0,"frames":2,"durations_ms":[80,120],"loop":true}}}}).to_string();
    fs::write(&request.json, &manifest).unwrap();
    request.out = root.join("sprite-gen");
    assert_eq!(import_sprite_gen_package(&request).unwrap(), memory);
    let converted = orr_sprite::sprite_gen::from_json(&manifest).unwrap();
    let mut expected = converted.to_json().unwrap().into_bytes();
    expected.push(b'\n');
    assert_eq!(
        fs::read(request.out.join("sprites.json")).unwrap(),
        expected
    );
    for name in ["memory", "file", "sprite-gen"] {
        assert_eq!(fs::read(root.join(name).join("atlas.png")).unwrap(), PNG_A);
        assert_eq!(
            fs::read(root.join(name).join("LICENSE.txt")).unwrap(),
            LICENSE
        );
    }
    assert!(!root.join("never-open.png").exists());
}

#[test]
fn checked_import_preserves_foreign_destinations_and_rejects_invalid_payloads_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let request = request(&root);
    let json = document();
    let hash = crate::hash(PNG_A);
    let foreign = request.out.join("foreign");
    fs::create_dir(&request.out).unwrap();
    fs::write(&foreign, b"keep").unwrap();
    assert!(import_package_bytes_checked(&request, &json, &hash).is_err());
    assert_eq!(fs::read(request.out.join("foreign")).unwrap(), b"keep");
    let mut request = request;
    request.out = root.join("invalid");
    assert!(import_package_bytes_checked(&request, b"bad JSON", &hash).is_err());
    assert!(!request.out.exists());
    let mut mismatch: serde_json::Value = serde_json::from_slice(&json).unwrap();
    mismatch["atlas"]["width"] = json!(3);
    assert!(
        import_package_bytes_checked(&request, &serde_json::to_vec(&mismatch).unwrap(), &hash)
            .is_err()
    );
    assert!(!request.out.exists());
    fs::write(&request.license, [0xff]).unwrap();
    assert!(import_package_bytes_checked(&request, &json, &hash)
        .unwrap_err()
        .contains("license must be UTF-8"));
    assert!(!request.out.exists());
    fs::write(&request.png, b"invalid PNG").unwrap();
    assert!(
        import_package_bytes_checked(&request, &json, &crate::hash(b"invalid PNG"))
            .unwrap_err()
            .contains("atlas PNG")
    );
    assert!(!request.out.exists());
    assert_eq!(fs::read(foreign).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn inspection_and_checked_import_reject_symlink_sources_ancestors_and_parent_traversal() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut request = request(&root);
    let original = request.png.clone();
    let link = root.join("linked.png");
    symlink(&original, &link).unwrap();
    assert!(inspect_png(&link).is_err());
    request.png = link;
    assert!(import_package_bytes_checked(&request, &document(), &crate::hash(PNG_A)).is_err());
    assert!(!request.out.exists());
    let directory = root.join("linked-parent");
    symlink(&root, &directory).unwrap();
    assert!(inspect_png(&directory.join("input.png")).is_err());
    request.png = directory.join("input.png");
    assert!(import_package_bytes_checked(&request, &document(), &crate::hash(PNG_A)).is_err());
    assert!(!request.out.exists());
    fs::create_dir(root.join("child")).unwrap();
    request.png = root.join("child/../input.png");
    assert!(inspect_png(&request.png)
        .unwrap_err()
        .contains("parent traversal"));
    assert!(import_package_bytes_checked(&request, &document(), &crate::hash(PNG_A)).is_err());
    assert_eq!(fs::read(&original).unwrap(), PNG_A);
    assert!(!request.out.exists());
}
