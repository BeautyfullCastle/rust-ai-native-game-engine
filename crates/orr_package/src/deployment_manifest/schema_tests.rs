use super::*;
fn sample() -> Manifest {
    create("arena", "owner.game/v1", "123", None, vec![]).unwrap()
}
fn value() -> Value {
    serde_json::to_value(sample()).unwrap()
}
fn rejected(v: Value) {
    assert!(
        load(&serde_json::to_vec(&v).unwrap(), None, None).is_err(),
        "{v}"
    );
}
#[test]
fn core_identity_oracle_and_large_u64_roundtrip() {
    assert_eq!(orr_ecs::FRAME_FORMAT_VERSION, 2);
    for id in [1, 2, 123, (1u64 << 53) + 1, u64::MAX] {
        let m = create(
            "physics",
            "physics/custom",
            &format!("0X{id:x}"),
            Some("revision".into()),
            vec![],
        )
        .unwrap();
        assert_eq!(m.game_code_id, id.to_string());
        assert_eq!(m.build_id, orr_sim::frame_build_id(id).to_string());
        assert_eq!(
            load(
                &encode(&m).unwrap(),
                Some("physics"),
                Some("physics/custom")
            )
            .unwrap(),
            m
        );
        let e = export(&m).unwrap();
        assert_eq!(e["browser"]["opts"]["game"], "phys");
        assert_eq!(
            e["native_host"]["args"],
            json!(["--game", "physics", "--build-id", m.build_id])
        );
        assert_eq!(e["relay"]["args"], e["native_host"]["args"]);
        assert_eq!(e["native_client"]["program"], "physics");
    }
    for id in [
        "0",
        "00",
        "01",
        "+1",
        " 1",
        "1 ",
        "1.0",
        "1e0",
        "0x",
        "0xg",
        "0x0",
        "18446744073709551616",
        "0x10000000000000000",
        "١",
    ] {
        assert!(create("arena", "id", id, None, vec![]).is_err(), "{id}");
    }
}
#[test]
fn strict_shape_required_nullable_and_integer_fields() {
    for key in value().as_object().unwrap().keys() {
        let mut v = value();
        v.as_object_mut().unwrap().remove(key);
        rejected(v);
    }
    for key in ["source_revision", "binaries"] {
        let mut v = value();
        v["provenance"].as_object_mut().unwrap().remove(key);
        rejected(v);
    }
    for field in ["extra", "schema"] {
        let mut v = value();
        v[field] = json!(1);
        rejected(v);
    }
    let mut v = value();
    v["provenance"]["extra"] = json!(null);
    rejected(v);
    for invalid in [json!(true), json!(2.0), json!("2"), json!(null), json!(3)] {
        let mut v = value();
        v["frame_format_version"] = invalid;
        rejected(v);
    }
    for field in ["game_code_id", "build_id"] {
        for invalid in [
            json!(123),
            json!(true),
            json!("0123"),
            json!("0x7b"),
            json!("0"),
            json!(null),
        ] {
            let mut v = value();
            v[field] = invalid;
            rejected(v);
        }
    }
    let mut v = value();
    v["build_id"] = json!("1");
    rejected(v);
    for invalid in [json!(null), json!([]), json!(42)] {
        rejected(invalid);
    }
    let bytes = encode(&sample()).unwrap();
    assert!(load(&bytes, Some("physics"), None).is_err());
    assert!(load(&bytes, None, Some("other")).is_err());
    assert!(load(&vec![b' '; 1024 * 1024 + 1], None, None).is_err());
}
#[test]
fn duplicate_json_keys_rejected_at_every_object_level() {
    let original = String::from_utf8(encode(&sample()).unwrap()).unwrap();
    for duplicate in [
        original.replacen(
            "\"game\": \"arena\"",
            "\"game\": \"arena\", \"game\": \"arena\"",
            1,
        ),
        original.replacen(
            "\"source_revision\": null",
            "\"source_revision\": null, \"source_revision\": null",
            1,
        ),
        original.replacen(
            "\"binaries\": []",
            &format!(
                "\"binaries\": [{{\"label\":\"x\",\"label\":\"x\",\"sha256\":\"{}\"}}]",
                "a".repeat(64)
            ),
            1,
        ),
    ] {
        assert!(
            load(duplicate.as_bytes(), None, None).is_err(),
            "{duplicate}"
        );
    }
}
#[test]
fn provenance_bounds_trim_and_binary_order() {
    let record = |label: &str| BinaryRecord {
        label: label.into(),
        sha256: "a".repeat(64),
    };
    let m = create(
        "arena",
        "id",
        "1",
        Some("rev".into()),
        vec![record("z"), record("a")],
    )
    .unwrap();
    let loaded = load(&encode(&m).unwrap(), None, None).unwrap();
    assert_eq!(loaded.provenance.binaries[0].label, "z");
    assert!(create(
        "arena",
        "id",
        "1",
        None,
        vec![record("same"), record("same")]
    )
    .is_err());
    assert!(create(
        "arena",
        "id",
        "1",
        None,
        (0..33).map(|i| record(&i.to_string())).collect()
    )
    .is_err());
    for text in ["", " id", "id\n", "\u{1c}id", "id\u{1f}", "\u{a0}id"] {
        assert!(create("arena", text, "1", None, vec![]).is_err());
        assert!(create("arena", "id", "1", Some(text.into()), vec![]).is_err());
        assert!(create("arena", "id", "1", None, vec![record(text)]).is_err());
    }
    for digest in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
        assert!(create(
            "arena",
            "id",
            "1",
            None,
            vec![BinaryRecord {
                label: "x".into(),
                sha256: digest
            }]
        )
        .is_err());
    }
    for field in ["label", "sha256"] {
        let mut v = serde_json::to_value(&m).unwrap();
        v["provenance"]["binaries"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        rejected(v);
    }
    let mut v = serde_json::to_value(&m).unwrap();
    v["provenance"]["binaries"][0]["extra"] = json!(0);
    rejected(v);
    assert!(create("arena", &"x".repeat(4097), "1", None, vec![]).is_err());
}
#[test]
fn previous_identity_guard_and_data_only_launch_export() {
    let a = sample();
    check_previous(&a, &a).unwrap();
    let b = create(
        "physics",
        &a.game_code_identity,
        &a.game_code_id,
        None,
        vec![],
    )
    .unwrap();
    assert!(check_previous(&b, &a).is_err());
    let c = create("arena", "changed", &a.game_code_id, None, vec![]).unwrap();
    assert!(check_previous(&c, &a).is_err());
    let d = create("physics", "changed", "124", None, vec![]).unwrap();
    check_previous(&d, &a).unwrap();
    let exported = export(&a).unwrap();
    assert_eq!(
        exported["native_client"]["args"],
        json!(["--build-id", a.build_id])
    );
    assert!(exported.get("provenance").is_none());
    assert!(exported.get("game_code_identity").is_none());
}
#[test]
fn python_ascii_sorted_encoding_and_surrogate_pairs() {
    let v = json!({"z":["한😀\u{7f}\n\t\"\\",null],"a":{},"é":"ÿ"});
    assert_eq!(
        String::from_utf8(encode(&v).unwrap()).unwrap(),
        concat!(
            "{\n  \"a\": {},\n  \"z\": [\n",
            "    \"\\ud55c\\ud83d\\ude00\\u007f\\n\\t\\\"\\\\\",\n",
            "    null\n  ],\n  \"\\u00e9\": \"\\u00ff\"\n}\n"
        )
    );
    let m = create("arena", "한😀", "1", Some("révision".into()), vec![]).unwrap();
    assert_eq!(load(&encode(&m).unwrap(), None, None).unwrap(), m);
}
