use super::*;
fn terrain() -> Terrain {
    super::super::tests::terrain(6, 5)
}
fn bytes() -> Vec<u8> {
    Clipboard::capture(&terrain(), [0, 0], [1, 0])
        .unwrap()
        .encode_asset()
        .unwrap()
}
#[test]
fn portable_asset_roundtrip_is_exact_and_deterministic() {
    let mut t = orr_terrain::TerrainDocument::new(terrain());
    t.apply(&[
        orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_raw(-1234567),
        },
        orr_terrain::Edit::SetHole {
            x: 1,
            z: 0,
            hole: true,
        },
    ])
    .unwrap();
    let patch = Clipboard::capture(t.terrain(), [0, 0], [1, 0]).unwrap();
    let bytes = patch.encode_asset().unwrap();
    assert_eq!(bytes, patch.encode_asset().unwrap());
    let decoded = Clipboard::decode_asset(&bytes, &terrain()).unwrap();
    assert_eq!(decoded.heights, patch.heights);
    assert_eq!(decoded.holes, patch.holes);
    assert_eq!(decoded.encode_asset().unwrap(), bytes);
}
#[test]
fn malformed_assets_are_rejected_before_rebinding() {
    let original: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("format", serde_json::json!("other")),
        ("cells", serde_json::json!([0, 1])),
        ("cells", serde_json::json!([128, 128])),
        ("cells", serde_json::json!([u32::MAX, 1])),
        ("spacing_raw", serde_json::json!(0)),
        ("spacing_raw", serde_json::json!(-1)),
        ("spacing_raw", serde_json::json!(i64::MAX)),
        ("spacing_raw", serde_json::json!(131072)),
        ("heights_raw", serde_json::json!([1])),
        ("holes", serde_json::json!([])),
        ("extra", serde_json::json!(1)),
        ("heights_raw", serde_json::json!([0.5, 0, 0, 0, 0, 0])),
        ("holes", serde_json::json!([0, 1])),
    ] {
        let mut bad = original.clone();
        bad[key] = value;
        assert!(
            Clipboard::decode_asset(&serde_json::to_vec(&bad).unwrap(), &terrain()).is_err(),
            "{key}: {bad}"
        );
    }
    for bad in [
        b"[]".to_vec(),
        b"{}".to_vec(),
        b"null".to_vec(),
        vec![b' '; MAX_ASSET_BYTES + 1],
        b"{\"format\":\"a\",\"format\":\"b\"}".to_vec(),
    ] {
        assert!(Clipboard::decode_asset(&bad, &terrain()).is_err());
    }
    for key in ["heights_raw", "holes"] {
        let mut bad = original.clone();
        bad[key] = if key == "holes" {
            serde_json::json!(vec![false; MAX_EDITS + 1])
        } else {
            serde_json::json!(vec![0; MAX_EDITS + 1])
        };
        assert!(Clipboard::decode_asset(&serde_json::to_vec(&bad).unwrap(), &terrain()).is_err());
    }
}
#[cfg(unix)]
#[test]
fn bounded_reader_rejects_unsafe_paths_and_files() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("patch.json");
    std::fs::write(&file, bytes()).unwrap();
    assert_eq!(read_patch(&file).unwrap(), bytes());
    assert!(read_patch(Path::new("relative.json")).is_err());
    assert!(read_patch(&temp.path().join("./patch.json")).is_err());
    assert!(read_patch(temp.path()).is_err());
    std::fs::write(&file, vec![0; MAX_ASSET_BYTES + 1]).unwrap();
    assert!(read_patch(&file).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = temp.path().join("link.json");
        symlink(&file, &link).unwrap();
        assert!(read_patch(&link).is_err());
        let dir_link = temp.path().join("alias");
        symlink(temp.path(), &dir_link).unwrap();
        assert!(read_patch(&dir_link.join("patch.json")).is_err());
        let fifo = temp.path().join("fifo.json");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR,
            0,
        )
        .unwrap();
        assert!(read_patch(&fifo).is_err());
    }
}

#[test]
fn legal_negative_origin_large_spacing_patch_roundtrips_without_origin_assumptions() {
    let source = Terrain::new(
        "wide".into(),
        3,
        2,
        [FP::from_raw(i64::MIN), FP::ZERO],
        FP::from_raw(i64::MAX),
        vec![FP::ONE; 6],
        vec![false, true],
    )
    .unwrap();
    let patch = Clipboard::capture(&source, [0, 0], [1, 0]).unwrap();
    let encoded = patch.encode_asset().unwrap();
    let decoded = Clipboard::decode_asset(&encoded, &source).unwrap();
    assert_eq!(decoded.heights, patch.heights);
    assert_eq!(decoded.holes, patch.holes);
    assert!(decoded.edits(&source, [0, 0], FP::ZERO).unwrap().is_empty());
    let mut impossible: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    impossible["cells"] = serde_json::json!([3, 1]);
    impossible["heights_raw"] = serde_json::json!(vec![0; 8]);
    impossible["holes"] = serde_json::json!(vec![false; 3]);
    assert!(Clipboard::decode_asset(&serde_json::to_vec(&impossible).unwrap(), &source).is_err());
}
