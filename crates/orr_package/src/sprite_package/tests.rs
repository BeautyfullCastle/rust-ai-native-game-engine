use super::*;
fn args() -> Vec<OsString> {
    [
        "--png",
        "a.png",
        "--license",
        "LICENSE",
        "--name",
        "sprites",
        "--version",
        "1.0.0",
        "--out",
        "new",
        "--atlas-size",
        "8",
        "4",
        "--region",
        "10",
        "1",
        "0",
        "2",
        "3",
        "--clip",
        "idle",
        "loop",
        "10:200",
    ]
    .into_iter()
    .map(Into::into)
    .collect()
}
#[test]
fn generated_bytes_match_python_indent_and_escape_contract() {
    let options = Options::parse(args()).unwrap();
    let expected="{\n  \"format\": \"orr_sprite\",\n  \"version\": 1,\n  \"atlas\": {\n    \"image\": \"atlas.png\",\n    \"width\": 8,\n    \"height\": 4\n  },\n  \"regions\": [\n    {\n      \"id\": 10,\n      \"x\": 1,\n      \"y\": 0,\n      \"width\": 2,\n      \"height\": 3\n    }\n  ],\n  \"clips\": [\n    {\n      \"id\": \"idle\",\n      \"mode\": \"loop\",\n      \"frames\": [\n        {\n          \"region\": 10,\n          \"duration_ms\": 200\n        }\n      ]\n    }\n  ]\n}\n";
    assert_eq!(options.generated.unwrap(), expected.as_bytes());
    let mut input = args();
    input[20] = "한글😀\u{1f}\n\t\"\\".into();
    let bytes = Options::parse(input).unwrap().generated.unwrap();
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains("한글😀\\u001f\\n\\t\\\"\\\\"));
}
#[test]
fn exclusive_modes_closed_flags_and_numeric_contract() {
    for flag in ["--packer", "--fps", "--pivot", "--shell", "--png"] {
        let mut a = args();
        a[10] = flag.into();
        assert!(Options::parse(a).is_err());
    }
    let mut a = args();
    a.extend([OsString::from("--json"), OsString::from("source.json")]);
    assert!(Options::parse(a).is_err());
    let mut a = args()[..10].to_vec();
    a.extend([OsString::from("--json"), OsString::from("original.json")]);
    let o = Options::parse(a).unwrap();
    assert!(o.generated.is_none());
    assert_eq!(o.request.json, PathBuf::from("original.json"));
    for v in ["+1", "--1", "1.0", "１", "", "18446744073709551616000"] {
        assert!(integer(v).is_err());
    }
    assert_eq!(integer("-0").unwrap(), 0);
    assert_eq!(integer("0001").unwrap(), 1);
    assert_eq!(integer("18446744073709551615").unwrap(), u64::MAX as i128);
    let mut a = args();
    a[22] = "10:0".into();
    let o = Options::parse(a).unwrap();
    assert!(orr_sprite::SpriteDocument::from_json(
        std::str::from_utf8(o.generated.as_ref().unwrap()).unwrap()
    )
    .is_err());
}
#[test]
fn allocation_and_collection_limits_fail_before_publication() {
    let mut a = args();
    a[1] = "x".repeat(MAX_ARGS).into();
    assert!(Options::parse(a).is_err());
    let mut a = args()[..13].to_vec();
    for _ in 0..=orr_sprite::MAX_REGIONS {
        a.extend(["--region", "0", "0", "0", "1", "1"].map(OsString::from));
    }
    assert!(Options::parse(a).is_err());
    let mut a = args();
    a[22] = std::iter::repeat_n("10:1", orr_sprite::MAX_FRAMES_PER_CLIP + 1)
        .collect::<Vec<_>>()
        .join(",")
        .into();
    assert!(Options::parse(a).is_err());
    let o = Options::parse(args()[..13].iter().cloned()).unwrap();
    assert!(orr_sprite::SpriteDocument::from_json(
        std::str::from_utf8(o.generated.as_ref().unwrap()).unwrap()
    )
    .is_ok());
}

#[test]
fn sprite_gen_input_is_explicit_and_exclusive() {
    let mut a = args()[..10].to_vec();
    a.extend([OsString::from("--sprite-gen"), OsString::from("manifest.json")]);
    let o = Options::parse(a.clone()).unwrap();
    assert!(o.sprite_gen && o.generated.is_none());
    assert_eq!(o.request.json, PathBuf::from("manifest.json"));
    for additional in [vec!["--json","source.json"],vec!["--sprite-gen","other.json"],vec!["--atlas-size","2","2"],vec!["--region","0","0","0","1","1"],vec!["--clip","idle","loop","0:50"]] {
        let mut invalid=a.clone(); invalid.extend(additional.into_iter().map(OsString::from));
        assert!(Options::parse(invalid).is_err());
    }
}
