//! The standalone Room tool stays out of the default editor dependency/UI path.
#[test]
fn room_editor_is_explicit_and_default_off() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let features = manifest["features"].as_table().unwrap();
    assert!(features["default"].as_array().unwrap().is_empty());
    let room = features["room-input-map-editor"].as_array().unwrap();
    assert_eq!(
        room.iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["dep:orr_input", "orr_sample/room-input-actions"]
    );
    let binary = manifest["bin"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bin| bin["name"].as_str() == Some("orr_room_input_map"))
        .unwrap();
    assert_eq!(
        binary["required-features"].as_array().unwrap()[0].as_str(),
        Some("room-input-map-editor")
    );
    assert!(!include_str!("../src/app.rs").contains("room_input_map_authoring"));
}
