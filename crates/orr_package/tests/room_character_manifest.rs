use orr_package::ProjectManifest;
fn base() -> serde_json::Value {
    serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.yaml","models":"room.models.json","camera":"room.camera.json","character":"room.character.json","lighting":"room.lighting.json"}})
}
fn valid(value: serde_json::Value) -> bool {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    orr_package::Project::open(&root, orr_package::Runtime::content_only()).is_ok()
}
#[test]
fn character_is_strict_room_only_and_cannot_alias_other_owned_documents() {
    assert!(valid(base()));
    for path in [
        "room.yaml",
        "ROOM.MODELS.JSON",
        "room.camera.json",
        "room.lighting.json",
        "orr.project.json",
        "orr.packages.lock.json",
        ".hidden",
        "../escape",
        "dir/character.json",
    ] {
        let mut value = base();
        value["entry"]["character"] = serde_json::json!(path);
        assert!(!valid(value), "{path}");
    }
    for game in ["arena", "collect-dodge-v1", "terrain-point-route-3d-v1"] {
        let mut value = base();
        value["entry"]["game"] = serde_json::json!(game);
        assert!(!valid(value));
    }
    for value in [
        serde_json::json!(null),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!(3),
    ] {
        let mut document = base();
        document["entry"]["character"] = value;
        assert!(!valid(document));
    }
    let duplicate = r#"{"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.yaml","models":"room.models.json","character":"a.json","character":"b.json"}}"#;
    assert!(serde_json::from_str::<ProjectManifest>(duplicate).is_err());
}

#[test]
fn character_metadata_requires_objects_and_preserves_legacy_slots() {
    let ui = serde_json::json!({"profile":"arena-korean-v1","font":{"package":"font","asset":"font.otf"}});
    let entry = serde_json::json!([
        "tile.json",
        "camera.json",
        "room-escape-v1",
        "room.yaml",
        "sprites.json",
        "room.models.json",
        ui,
        "lighting.json",
        "room.character.json"
    ]);
    let mut doc = base();
    doc["entry"] = entry;
    assert!(serde_json::from_value::<ProjectManifest>(doc).is_err());
    let object_entry = base()["entry"].clone();
    assert!(
        serde_json::from_value::<ProjectManifest>(serde_json::json!([2, "*", object_entry]))
            .is_err()
    );
    // Deserialization alone preserves the old positional slots. Semantic validation
    // separately rejects this mixed-profile fixture, as it did before character support.
    let legacy: ProjectManifest = serde_json::from_value(serde_json::json!([
        2,
        "*",
        [
            "tile.json",
            "camera.json",
            "room-escape-v1",
            "room.yaml",
            "sprites.json",
            "room.models.json",
            ui
        ]
    ]))
    .unwrap();
    let entry = legacy.entry.unwrap();
    assert_eq!(entry.scene, "room.yaml");
    assert_eq!(entry.models.as_deref(), Some("room.models.json"));
    assert!(entry.lighting.is_none() && entry.character.is_none());
}
