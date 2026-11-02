use super::*;
use serde_json::json;
fn base() -> Value {
    json!({"unit:0":{"image":"frames/a.png","clips":{"idle":{"frames":[[0,0,2,3]]}}}})
}
#[test]
fn duplicate_keys_and_nonfinite_json_fail_before_admission() {
    for bytes in [
        br#"{"unit:0":{},"unit:0":{}}"#.as_slice(),
        br#"{"a":{"x":1,"x":2}}"#,
        br#"{"a":[{"x":1,"x":2}]}"#,
        b"NaN",
        b"Infinity",
        b"1e999",
        b"\xff",
        b"{} trailing",
    ] {
        assert!(parse(bytes).is_err(), "{bytes:?}");
    }
    assert!(parse(&vec![b' '; 65537]).is_err());
    assert!(parse(format!("{}0{}", "[".repeat(200), "]".repeat(200)).as_bytes()).is_err());
}
#[test]
fn closed_role_fields_paths_and_boolean_numbers() {
    assert_eq!(names(&base()).unwrap(), vec!["frames/a.png"]);
    for role in ["unit:6", "unit:00", "enemy:5", "enemy:-1", "unit:0\n"] {
        let v = json!({role:base()["unit:0"].clone()});
        assert!(names(&v).is_err());
    }
    for path in [
        "../a.png",
        "/a.png",
        "a//b.png",
        "a\\b.png",
        "a.PNG",
        ".hidden/a.png",
        "한.png",
    ] {
        let mut v = base();
        v["unit:0"]["image"] = path.into();
        assert!(names(&v).is_err());
    }
    let mut v = base();
    v["unit:0"]["extra"] = json!(1);
    assert!(names(&v).is_err());
    for field in ["image", "clips"] {
        let mut v = base();
        v["unit:0"].as_object_mut().unwrap().remove(field);
        assert!(names(&v).is_err());
    }
    for field in ["fps", "loop", "pivots", "windupMs"] {
        let mut v = base();
        v["unit:0"]["clips"]["idle"][field] = if field == "fps" {
            json!(true)
        } else {
            json!(1)
        };
        assert!(names(&v).is_err(), "{field}");
    }
    let mut v = base();
    v["unit:0"]["clips"]["idle"]["frames"][0][0] = json!(true);
    assert!(names(&v).is_err());
}
#[test]
fn fractional_bounds_attack_timing_and_per_frame_pivots() {
    let mut v = base();
    let d = v["unit:0"].as_object_mut().unwrap();
    d.insert("anchor".into(), json!([-16, 16]));
    d.insert("pivot".into(), json!([-4096, 4096]));
    d.insert("logicalSize".into(), json!([0.25, 4096]));
    d.insert("clips".into(),json!({"attack":{"frames":[[-0.0,0.5,0.25,4096]],"fps":1.5,"loop":false,"pivots":[[-4096,4096]],"windupMs":1,"releaseMs":1.25,"recoveryMs":2000}}));
    assert!(names(&v).is_ok());
    for (field, bad) in [
        ("fps", json!(120.001)),
        ("loop", json!(0)),
        ("pivots", json!([])),
        ("windupMs", json!(0)),
        ("releaseMs", json!(true)),
        ("recoveryMs", json!(2000.1)),
    ] {
        let mut badv = v.clone();
        badv["unit:0"]["clips"]["attack"][field] = bad;
        assert!(names(&badv).is_err());
    }
    for rect in [
        json!([0, 0, 0, 1]),
        json!([-0.1, 0, 1, 1]),
        json!([4096.1, 0, 1, 1]),
        json!([0, 0, 1]),
    ] {
        let mut badv = v.clone();
        badv["unit:0"]["clips"]["attack"]["frames"] = json!([rect]);
        assert!(names(&badv).is_err());
    }
    let mut badv = v.clone();
    badv["unit:0"]["logicalSize"] = json!([0, 1]);
    assert!(names(&badv).is_err());
    let mut badv = base();
    badv["unit:0"]["clips"]["idle"]["frames"] = json!(vec![[0, 0, 1, 1]; 257]);
    assert!(names(&badv).is_err());
}
#[test]
fn create_preserves_omitted_defaults_and_configured_frame_order() {
    let frames = vec![
        ("idle".into(), [-0.0, 0.0, 1.0, 2.0]),
        ("idle".into(), [2.0, 0.0, 1.0, 2.0]),
    ];
    let bytes = create("unit:0", "a.png", &frames, &[], None).unwrap();
    let v = parse(&bytes).unwrap();
    let c = &v["unit:0"]["clips"]["idle"];
    assert!(c.get("fps").is_none());
    assert!(c.get("loop").is_none());
    assert!(v["unit:0"].get("pivot").is_none());
    assert_eq!(c["frames"][1][0].as_f64(), Some(2.0));
    assert!(c["frames"][0][0].as_f64().unwrap().is_sign_negative());
    assert!(create(
        "unit:0",
        "a.png",
        &frames,
        &[("move".into(), 12.0, true)],
        None
    )
    .is_err());
    assert!(create(
        "unit:0",
        "a.png",
        &frames,
        &[("idle".into(), 12.0, true), ("idle".into(), 12.0, false)],
        None
    )
    .is_err());
    assert!(create(
        "unit:0",
        "a.png",
        &[("idle".into(), [0.0, 0.0, f64::NAN, 1.0])],
        &[],
        None
    )
    .is_err());
}
#[test]
fn compose_receipts_preserve_input_order_and_refuse_duplicate_roles() {
    let a = encode(&base(), true).unwrap();
    let mut v = Map::new();
    v.insert("enemy:0".into(), base()["unit:0"].clone());
    let b = encode(&Value::Object(v), false).unwrap();
    let (bytes, receipts) = compose(&[b.clone(), a.clone()]).unwrap();
    assert_eq!(receipts[0].bytes, b.len());
    assert_eq!(receipts[1].bytes, a.len());
    assert_eq!(
        receipts[0].sha256,
        Sha256::digest(&b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        names(&parse(&bytes).unwrap()).unwrap(),
        vec!["frames/a.png"]
    );
    assert!(bytes.ends_with(b"\n"));
    assert!(compose(&[a.clone(), a]).is_err());
    assert!(compose(&[]).is_err());
    assert!(compose(&vec![b; 12]).is_err());
}
#[test]
fn ascii_sorted_encoding_and_numeric_semantics() {
    let v = parse(br#"{"z":-0.0,"a":1e-3,"unicode":"\ud55c\ud83d\ude00"}"#).unwrap();
    let encoded = encode(&v, false).unwrap();
    assert!(encoded.starts_with(b"{\"a\":"));
    assert!(String::from_utf8(encoded.clone())
        .unwrap()
        .contains("\\ud55c\\ud83d\\ude00"));
    let round = parse(&encoded).unwrap();
    assert_eq!(round["a"].as_f64(), Some(0.001));
    assert!(round["z"].as_f64().unwrap().is_sign_negative());
}
