use super::*;
use crate::tilemap_panel::copy::Pattern;
use orr_tilemap::{CellCoord, Document, SpriteRef};

fn atlas() -> SpriteDocument {
    SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1}],"clips":[]}"#).unwrap()
}
fn document(width: u32, height: u32) -> ValidatedDocument {
    Document {
        version: 1,
        sprite: SpriteRef {
            package: "fixture".into(),
            document: "sprites.json".into(),
        },
        width,
        height,
        tile_size: [1, 1],
        origin: [0.0, 0.0],
        units_per_pixel: 1.0,
        cells: vec![Some(0); (width * height) as usize],
    }
    .validate(&atlas())
    .unwrap()
}
fn digest() -> String {
    "a".repeat(64)
}
fn bytes() -> Vec<u8> {
    let mut source = document(2, 1).document().clone();
    source.cells = vec![None, Some(1)];
    let source = source.validate(&atlas()).unwrap();
    Pattern::capture(
        &source,
        CellCoord { column: 0, row: 0 },
        CellCoord { column: 1, row: 0 },
        3,
        &atlas(),
    )
    .unwrap()
    .encode_asset(&source, 3, &atlas(), &digest())
    .unwrap()
}
fn decode(bytes: &[u8]) -> Result<Pattern, String> {
    Pattern::decode_asset(bytes, &document(2, 1), 8, &atlas(), &digest())
}
#[test]
fn portable_rebind_preserves_cells_and_has_no_geometry_or_epoch() {
    let original = bytes();
    let json: serde_json::Value = serde_json::from_slice(&original).unwrap();
    for absent in ["origin", "units_per_pixel", "epoch", "geometry"] {
        assert!(json.get(absent).is_none());
    }
    let mut target = document(4, 3).document().clone();
    target.origin = [-7.0, 8.0];
    target.units_per_pixel = 0.25;
    let target = target.validate(&atlas()).unwrap();
    let restored = Pattern::decode_asset(&original, &target, 99, &atlas(), &digest()).unwrap();
    assert!(restored.matches(&target, 99, &atlas()));
    assert!(!restored.matches(&target, 3, &atlas()));
    assert_eq!(
        restored
            .encode_asset(&target, 99, &atlas(), &digest())
            .unwrap(),
        original
    );
    let stamped = restored
        .candidate(&target, CellCoord { column: 1, row: 1 }, 99, &atlas())
        .unwrap();
    assert_eq!(stamped.document().origin, [-7.0, 8.0]);
    assert_eq!(stamped.document().cells[5..7], [None, Some(1)]);
    let small = document(1, 1);
    let large = Pattern::decode_asset(&original, &small, 10, &atlas(), &digest()).unwrap();
    assert!(large.end(&small, CellCoord { column: 0, row: 0 }).is_err());
}
#[test]
fn malformed_identity_bounds_and_unknown_fields_are_rejected() {
    let original: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    for (field, value) in [
        ("format", serde_json::json!("bad")),
        ("version", serde_json::json!(2)),
        ("width", serde_json::json!(0)),
        ("height", serde_json::json!(65)),
        ("width", serde_json::json!(4294967295u32)),
        ("cells", serde_json::json!([0])),
        ("cells", serde_json::json!([0, 9])),
        ("cells", serde_json::json!([0, -1])),
        ("cells", serde_json::json!([0, 0.0])),
        ("tile_size", serde_json::json!([2, 1])),
        ("sprite_package", serde_json::json!("other")),
        ("sprite_document", serde_json::json!("../../secret.json")),
        ("package_digest", serde_json::json!("b".repeat(64))),
        ("atlas_json", serde_json::json!("{}")),
        ("epoch", serde_json::json!(3)),
    ] {
        let mut bad = original.clone();
        bad[field] = value;
        assert!(
            decode(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "accepted {field}"
        );
    }
    let text = String::from_utf8(bytes()).unwrap();
    assert!(decode(text.replacen('{', "{\"version\":1,", 1).as_bytes()).is_err());
    assert!(
        decode(
            &serde_json::to_vec(&original.as_object().unwrap().values().collect::<Vec<_>>())
                .unwrap()
        )
        .is_err()
    );
    assert!(
        decode(&vec![b' '; MAX_ASSET_BYTES + 1])
            .unwrap_err()
            .contains("32 MiB")
    );
    let mut huge = original.clone();
    huge["cells"] = serde_json::json!(vec![None::<u32>; 4097]);
    assert!(decode(&serde_json::to_vec(&huge).unwrap()).is_err());
    huge = original;
    huge["atlas_json"] = serde_json::json!(" ".repeat(orr_sprite::MAX_JSON_BYTES + 1));
    assert!(
        decode(&serde_json::to_vec(&huge).unwrap())
            .unwrap_err()
            .contains("4 MiB")
    );
}
#[test]
fn empty_digest_and_stale_export_are_rejected() {
    let source = document(2, 1);
    assert!(Pattern::decode_asset(&bytes(), &source, 1, &atlas(), "").is_err());
    let pattern = decode(&bytes()).unwrap();
    assert!(
        pattern
            .encode_asset(&source, 9, &atlas(), &digest())
            .is_err()
    );
    assert!(pattern.encode_asset(&source, 8, &atlas(), "").is_err());
}
#[test]
fn canonical_atlas_is_required_even_when_semantically_identical() {
    let mut json: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    json["atlas_json"] = serde_json::json!(format!("{} ", atlas().to_json().unwrap()));
    assert!(decode(&serde_json::to_vec(&json).unwrap()).is_err());
}
#[cfg(unix)]
#[test]
fn bounded_regular_source_and_paths() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let file = root.join("pattern.json");
    fs::write(&file, bytes()).unwrap();
    assert_eq!(read_pattern(&file).unwrap(), bytes());
    assert!(read_pattern(Path::new("relative.json")).is_err());
    assert!(read_pattern(&root.join("./pattern.json")).is_err());
    assert!(read_pattern(&root.join("../pattern.json")).is_err());
    assert!(read_pattern(&root.join("pattern.txt")).is_err());
    assert!(read_pattern(&root.join("a".repeat(4097))).is_err());
    symlink(&file, root.join("link.json")).unwrap();
    assert!(read_pattern(&root.join("link.json")).is_err());
    symlink(&root, root.join("linked")).unwrap();
    assert!(read_pattern(&root.join("linked/pattern.json")).is_err());
    fs::create_dir(root.join("directory.json")).unwrap();
    assert!(read_pattern(&root.join("directory.json")).is_err());
    fs::File::create(root.join("huge.json"))
        .unwrap()
        .set_len(MAX_ASSET_BYTES as u64 + 1)
        .unwrap();
    assert!(read_pattern(&root.join("huge.json")).is_err());
}

#[test]
fn maximum_64_by_64_pattern_roundtrips() {
    let source = document(64, 64);
    let pattern = Pattern::capture(
        &source,
        CellCoord { column: 0, row: 0 },
        CellCoord {
            column: 63,
            row: 63,
        },
        1,
        &atlas(),
    )
    .unwrap();
    let encoded = pattern
        .encode_asset(&source, 1, &atlas(), &digest())
        .unwrap();
    let target = document(1, 1);
    let restored = Pattern::decode_asset(&encoded, &target, 2, &atlas(), &digest()).unwrap();
    assert_eq!((restored.width, restored.height), (64, 64));
    assert_eq!(
        restored
            .encode_asset(&target, 2, &atlas(), &digest())
            .unwrap(),
        encoded
    );
}
