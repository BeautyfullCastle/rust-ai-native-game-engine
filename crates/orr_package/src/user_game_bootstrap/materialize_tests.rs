use super::*;
fn fixture(preview: bool) -> BTreeMap<String, Vec<u8>> {
    let mut cargo=String::from("[package]\nname = \"orr_static_starter\"\nversion.workspace = true\nedition.workspace = true\nlicense.workspace = true\n[lints]\nworkspace = true\n[dependencies]\norr_remote = { path = \"../../crates/orr_remote\", default-features = false }\n[features]\ndefault = [\"editor\"]\neditor = []\nstandalone-native-acceptance = [\"editor\"]\n");
    if preview {
        cargo.push_str("static-game-actions = [\"dep:orr_input\", \"orr_editor?/static-game-actions\"]\nmulti-action-standalone-acceptance = [\"editor\"]\nmulti-action-editor-acceptance = [\"editor\"]\n[[bin]]\nname = \"multi-action-fixture-player\"\npath = \"tests/support/multi_action_fixture_player.rs\"\n[[test]]\nname = \"multi_action_editor_x11\"\npath = \"tests/multi_action_editor_x11.rs\"\n");
    }
    cargo.push_str("[[test]]\nname = \"standalone_x11\"\npath = \"tests/standalone_x11.rs\"\nrequired-features = [\"standalone-native-acceptance\"]\n");
    let mut files = BTreeMap::from([
        ("Cargo.toml".into(), cargo.into_bytes()),
        ("README.md".into(), b"upstream\r\n".to_vec()),
        ("src/lib.rs".into(), b"// unchanged\n".to_vec()),
        ("tests/standalone_x11.rs".into(), b"omit".to_vec()),
    ]);
    if preview {
        files.insert(
            "tests/multi_action_contract.rs".into(),
            b"portable".to_vec(),
        );
        files.insert("tests/multi_action_editor_x11.rs".into(), b"omit".to_vec());
    }
    files
}
#[test]
fn legacy_preserves_sources_receipt_order_and_transformation() {
    let files = generated_files(
        fixture(false),
        Path::new("/engine with spaces"),
        "my-game",
        Profile::Legacy,
    )
    .unwrap();
    assert_eq!(files["src/lib.rs"], b"// unchanged\n");
    assert_eq!(files["UPSTREAM_README.md"], b"upstream\r\n");
    assert!(!files.contains_key("tests/standalone_x11.rs"));
    let cargo = std::str::from_utf8(&files["Cargo.toml"]).unwrap();
    let parsed: toml::Value = toml::from_str(cargo).unwrap();
    assert_eq!(parsed["package"]["name"].as_str(), Some("my-game"));
    assert_eq!(
        parsed["dependencies"]["orr_remote"]["path"].as_str(),
        Some("/engine with spaces/crates/orr_remote")
    );
    assert_eq!(
        parsed["dependencies"]["orr_remote"]["default-features"].as_bool(),
        Some(false)
    );
    assert_eq!(parsed["lib"]["name"].as_str(), Some("orr_static_starter"));
    let receipt = std::str::from_utf8(&files["provenance.json"]).unwrap();
    assert!(receipt.starts_with("{\n  \"schema\": 1,\n  \"engine_commit\":"));
    let receipt: serde_json::Value = serde_json::from_str(receipt).unwrap();
    assert!(receipt["user_source_commit"].is_null());
    assert!(receipt.get("template_profile").is_none());
    assert!(receipt["files_sha256"].get("provenance.json").is_none());
    for (path, hash) in receipt["files_sha256"].as_object().unwrap() {
        let actual: String = Sha256::digest(&files[path])
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hash.as_str(), Some(actual.as_str()));
    }
}
#[test]
fn preview_preserves_optional_feature_and_omits_native_harnesses() {
    let files = generated_files(
        fixture(true),
        Path::new("/engine"),
        "actions",
        Profile::MultiActionPreview,
    )
    .unwrap();
    let cargo = std::str::from_utf8(&files["Cargo.toml"]).unwrap();
    assert!(!cargo.contains("multi-action-fixture-player"));
    assert!(!cargo.contains("multi_action_editor_x11"));
    assert!(!cargo.contains("multi-action-editor-acceptance"));
    assert!(!files.contains_key("tests/multi_action_editor_x11.rs"));
    assert!(files.contains_key("tests/multi_action_contract.rs"));
    let readme = std::str::from_utf8(&files["README.md"]).unwrap();
    assert!(readme.contains("UNVERIFIED"));
    assert!(readme.contains("not a third playable game"));
    let receipt: serde_json::Value = serde_json::from_slice(&files["provenance.json"]).unwrap();
    assert_eq!(receipt["action_feature_default"], false);
    assert_eq!(receipt["template_profile"], "multi-action-preview");
    let mut missing = fixture(true);
    missing.remove("tests/multi_action_contract.rs");
    assert!(generated_files(
        missing,
        Path::new("/engine"),
        "actions",
        Profile::MultiActionPreview
    )
    .is_err());
}
#[test]
fn unicode_json_escaping_and_safe_generated_bounds() {
    assert_eq!(
        ascii_json(&"한😀\u{7f}", false).unwrap(),
        "\"\\ud55c\\ud83d\\ude00\\u007f\""
    );
    let files = generated_files(
        fixture(false),
        Path::new("/한 engine"),
        "game",
        Profile::Legacy,
    )
    .unwrap();
    assert!(std::str::from_utf8(&files["Cargo.toml"])
        .unwrap()
        .contains("\\ud55c"));
    for bad in [
        "../x",
        "/x",
        "a//b",
        "a\\b",
        "a/./b",
        ".git/config",
        "target/x",
    ] {
        let mut input = fixture(false);
        input.insert(bad.into(), vec![]);
        assert!(
            generated_files(input, Path::new("/engine"), "game", Profile::Legacy).is_err(),
            "{bad}"
        );
    }
    let mut collision = fixture(false);
    collision.insert("src".into(), vec![]);
    assert!(generated_files(collision, Path::new("/engine"), "game", Profile::Legacy).is_err());
    let mut many = fixture(false);
    for i in 0..129 {
        many.insert(format!("f{i}"), vec![]);
    }
    assert!(generated_files(many, Path::new("/engine"), "game", Profile::Legacy).is_err());
}
