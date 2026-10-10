use orr_asset::AssetRef;
use orr_asset_cook::authoring::{
    import_source, validate_payload, validate_source_path, AssetType, Entry, Index,
    MAX_INDEX_RECORDS, MAX_SOURCE_BYTES,
};

fn id(raw: u64) -> AssetRef {
    AssetRef::from_raw(raw)
}

fn doc(entries: &str) -> Vec<u8> {
    format!(r#"{{"format":"orr.asset-index/1","entries":[{entries}]}}"#).into_bytes()
}

#[test]
fn index_parses_sorts_and_roundtrips_full_width_ids() {
    let bytes = doc(
        r#"{"id":"a_0020000000000001","type":"sim.motion_profile","schema_version":1,"source":"source/β.json"},{"id":"a_0000000000000002","tombstone":true}"#,
    );
    let index = Index::parse(&bytes).unwrap();
    assert_eq!(index.entries[0].id(), id(2));
    assert_eq!(index.entries[1].id(), id(0x0020_0000_0000_0001));
    let encoded = index.encode().unwrap();
    assert!(encoded.ends_with(b"\n"));
    let reparsed = Index::parse(&encoded).unwrap();
    assert_eq!(reparsed, index);
    assert_eq!(reparsed.encode().unwrap(), encoded);
}

#[test]
fn index_rejects_duplicate_and_unknown_fields_and_bad_input() {
    assert!(Index::parse(
        br#"{"format":"orr.asset-index/1","format":"orr.asset-index/1","entries":[]}"#
    )
    .is_err());
    assert!(Index::parse(&doc(
        r#"{"id":"a_0000000000000001","id":"a_0000000000000002","type":"sim.motion_profile","schema_version":1,"source":"a.json"}"#
    )).is_err());
    assert!(Index::parse(br#"{"format":"orr.asset-index/1","entries":[],"extra":0}"#).is_err());
    assert!(Index::parse(&doc(
        r#"{"id":"a_0000000000000001","type":"sim.motion_profile","schema_version":1,"source":"a.json","extra":0}"#
    )).is_err());
    assert!(Index::parse(&doc(
        r#"{"id":"a_0000000000000001","type":"sim.motion_profile","schema_version":1,"source":"Source/A.json"},{"id":"a_0000000000000002","type":"sim.motion_profile","schema_version":1,"source":"source/a.JSON"}"#
    )).is_err());
    assert!(Index::parse(br#"{"format":"orr.asset-index/1","entries":["#).is_err());
    assert!(Index::parse(&[b'{', 0xff, b'}']).is_err());
    assert!(Index::parse(&doc(r#"{"id":"a_0000000000000000","tombstone":true}"#)).is_err());

    let nested = format!(
        r#"{{"format":"orr.asset-index/1","entries":[{}0{}]}}"#,
        "[".repeat(8),
        "]".repeat(8)
    );
    assert!(Index::parse(nested.as_bytes()).is_err());
}

#[test]
fn index_enforces_record_budgets_before_importing_sources() {
    let mut entries = Vec::new();
    for n in 1..=65 {
        entries.push(format!(
            r#"{{"id":"a_{n:016x}","type":"sim.motion_profile","schema_version":1,"source":"s/{n}.json"}}"#
        ));
    }
    assert!(Index::parse(&doc(&entries.join(","))).is_err());

    let mut entries = Vec::new();
    for n in 1..=17 {
        entries.push(format!(
            r#"{{"id":"a_{n:016x}","type":"view.impact_pcm16","schema_version":1,"source":"v/{n}.json"}}"#
        ));
    }
    assert!(Index::parse(&doc(&entries.join(","))).is_err());

    let many = std::iter::repeat_n(
        r#"{"id":"a_0000000000000001","tombstone":true}"#,
        MAX_INDEX_RECORDS + 1,
    )
    .collect::<Vec<_>>()
    .join(",");
    assert!(Index::parse(&doc(&many)).is_err());
}

#[test]
fn index_lifecycle_preserves_identity_and_never_reuses_tombstones() {
    let mut index = Index::default();
    assert!(index
        .register(AssetRef::NULL, AssetType::Motion, "m.json".into())
        .is_err());
    index
        .register(id(10), AssetType::Motion, "source/m.json".into())
        .unwrap();
    assert!(index
        .register(id(11), AssetType::Impact, "SOURCE/M.JSON".into())
        .is_err());
    index
        .clone_entry(id(10), id(20), "source/m-copy.json".into())
        .unwrap();
    assert!(index
        .clone_entry(id(99), id(21), "source/missing.json".into())
        .is_err());
    index
        .move_source(id(20), "renamed/m-copy.json".into())
        .unwrap();
    assert!(index.move_source(id(20), "source/m.json".into()).is_err());
    assert!(index.tombstone(id(10), &[id(10)]).is_err());
    index.tombstone(id(20), &[id(10)]).unwrap();
    assert!(matches!(
        index.entries.iter().find(|entry| entry.id() == id(20)),
        Some(Entry::Tombstone { .. })
    ));
    assert!(index
        .register(id(20), AssetType::Impact, "new.json".into())
        .is_err());
    assert!(index
        .clone_entry(id(20), id(30), "new.json".into())
        .is_err());
    assert!(index.validate_roots(&[id(10)]).is_ok());
    assert!(index.validate_roots(&[id(20)]).is_err());
    assert!(index.validate_roots(&[AssetRef::NULL]).is_err());
    assert!(index.validate_roots(&[id(999)]).is_err());
}

#[test]
fn source_paths_are_canonical_and_portable() {
    for valid in ["source/motion.json", "nested/β.json", "a_123/file-v1.json"] {
        validate_source_path(valid).unwrap();
    }
    for invalid in [
        "",
        "/absolute.json",
        "C:/drive.json",
        "a\\b.json",
        "../escape.json",
        "a/../escape.json",
        "./dot.json",
        "a//b.json",
        "a/",
        "a/.",
        "a/..",
        "CON",
        "con.txt",
        "folder/PRN.log",
        "aux.json",
        "COM1",
        "lpt9.data",
    ] {
        assert!(validate_source_path(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn motion_source_is_strict_and_uses_fp_rounding() {
    let payload = import_source(AssetType::Motion, br#"{"speed_per_tick":"0.0625"}"#).unwrap();
    assert_eq!(i64::from_le_bytes(payload.try_into().unwrap()), 4096);
    let half_raw = import_source(
        AssetType::Motion,
        br#"{"speed_per_tick":"0.00000762939453125"}"#,
    )
    .unwrap();
    assert_eq!(i64::from_le_bytes(half_raw.try_into().unwrap()), 1);
    let half_two = import_source(
        AssetType::Motion,
        br#"{"speed_per_tick":"0.00002288818359375"}"#,
    )
    .unwrap();
    assert_eq!(i64::from_le_bytes(half_two.try_into().unwrap()), 2);

    for input in [
        br#"{"speed_per_tick":"1e-2"}"#.as_slice(),
        br#"{"speed_per_tick":"0"}"#,
        br#"{"speed_per_tick":"-0.5"}"#,
        br#"{"speed_per_tick":"16.1"}"#,
        br#"{"speed_per_tick":"1","extra":0}"#,
        br#"{"speed_per_tick":"1","speed_per_tick":"2"}"#,
        br#"["0.0625"]"#,
    ] {
        assert!(import_source(AssetType::Motion, input).is_err());
    }
    assert!(import_source(AssetType::Motion, &vec![b' '; MAX_SOURCE_BYTES + 1]).is_err());
}

#[test]
fn impact_pcm_generation_and_validation_are_exact() {
    let source = br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":8,"period_frames":4,"peak_pcm16":2}"#;
    let payload = import_source(AssetType::Impact, source).unwrap();
    assert_eq!(&payload[..4], &48_000u32.to_le_bytes());
    assert_eq!(&payload[4..8], &8u32.to_le_bytes());
    let samples: Vec<i16> = payload[8..]
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    assert_eq!(samples, [-2, 0, 1, 0, -1, 0, 0, 0]);
    validate_payload(AssetType::Impact, &payload).unwrap();

    let mut trailing = payload.clone();
    trailing.push(0);
    assert!(validate_payload(AssetType::Impact, &trailing).is_err());
    let mut wrong_rate = payload.clone();
    wrong_rate[..4].copy_from_slice(&44_100u32.to_le_bytes());
    assert!(validate_payload(AssetType::Impact, &wrong_rate).is_err());
    let mut over_peak = payload.clone();
    over_peak[8..10].copy_from_slice(&8_193i16.to_le_bytes());
    assert!(validate_payload(AssetType::Impact, &over_peak).is_err());

    for bad in [
        br#"{"generator":"triangle_decay_v2","sample_rate":48000,"frames":8,"period_frames":4,"peak_pcm16":2}"#.as_slice(),
        br#"{"generator":"triangle_decay_v1","sample_rate":44100,"frames":8,"period_frames":4,"peak_pcm16":2}"#,
        br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":8,"period_frames":6,"peak_pcm16":2}"#,
        br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":8,"period_frames":4,"peak_pcm16":0}"#,
        br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":8,"period_frames":4,"peak_pcm16":2,"extra":true}"#,
        br#"["triangle_decay_v1",48000,8,4,2]"#,
    ] {
        assert!(import_source(AssetType::Impact, bad).is_err());
    }
}

#[test]
fn exact_record_and_source_limits_and_unknown_versions() {
    let tombstones = (1..=MAX_INDEX_RECORDS)
        .map(|n| format!(r#"{{"id":"a_{n:016x}","tombstone":true}}"#))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        Index::parse(&doc(&tombstones)).unwrap().entries.len(),
        MAX_INDEX_RECORDS
    );
    for entry in [
        r#"{"id":"a_0000000000000001","type":"sim.motion_profile","schema_version":2,"source":"m.json"}"#,
        r#"{"id":"a_0000000000000001","type":"unknown","schema_version":1,"source":"m.json"}"#,
        r#"{"id":"a_0000000000000001","tombstone":false}"#,
        r#"{"id":"a_0000000000000001","tombstone":true},{"id":"a_0000000000000001","tombstone":true}"#,
        r#"{"id":"a_10000000000000000","tombstone":true}"#,
        r#"{"id":"a_FFFFFFFFFFFFFFFF","tombstone":true}"#,
        r#"{"id":9007199254740993,"tombstone":true}"#,
        r#"{"id":"a_0000000000000001","tombstone":null}"#,
    ] {
        assert!(Index::parse(&doc(entry)).is_err(), "{entry}");
    }
    let mut source = br#"{"speed_per_tick":"16"}"#.to_vec();
    source.resize(MAX_SOURCE_BYTES, b' ');
    assert_eq!(
        i64::from_le_bytes(
            import_source(AssetType::Motion, &source)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        1048576
    );
    for value in [
        "18446744073709551616",
        "9223372036854775807",
        "0.00000000001",
        " 1",
        "1 ",
        "NaN",
        "inf",
    ] {
        let source = format!(r#"{{"speed_per_tick":"{value}"}}"#);
        assert!(import_source(AssetType::Motion, source.as_bytes()).is_err());
    }
}

#[test]
fn pcm_maximum_and_all_parameter_bounds_are_enforced() {
    let source = serde_json::json!({"generator":"triangle_decay_v1","sample_rate":48000,"frames":48000,"period_frames":48000,"peak_pcm16":8192});
    let payload = import_source(AssetType::Impact, &serde_json::to_vec(&source).unwrap()).unwrap();
    assert_eq!(payload.len(), 96008);
    validate_payload(AssetType::Impact, &payload).unwrap();
    for (field, value) in [
        ("frames", 0i64),
        ("frames", 48001),
        ("frames", -1),
        ("period_frames", 0),
        ("period_frames", 48004),
        ("peak_pcm16", 8193),
        ("sample_rate", 48001),
    ] {
        let mut bad = source.clone();
        bad[field] = value.into();
        assert!(
            import_source(AssetType::Impact, &serde_json::to_vec(&bad).unwrap()).is_err(),
            "{field}={value}"
        );
    }
    assert!(import_source(AssetType::Impact, br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":1,"frames":2,"period_frames":4,"peak_pcm16":1}"#).is_err());
    assert!(import_source(AssetType::Impact, br#"{"generator":"triangle_decay_v1","sample_rate":48000,"frames":1,"period_frames":4,"peak_pcm16":1,"dependencies":[]}"#).is_err());
    for length in 0..10 {
        assert!(validate_payload(AssetType::Impact, &vec![0; length]).is_err());
    }
}
