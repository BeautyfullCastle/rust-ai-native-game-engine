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

#[test]
fn main_editor_play_is_separate_explicit_capability() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let features = manifest["features"].as_table().unwrap();
    assert_eq!(
        features["room-input-map-play"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["room-project", "room-input-map-editor"]
    );
    let lib = include_str!("../src/lib.rs");
    assert!(lib
        .contains("#[cfg(feature = \"room-input-map-play\")]\npub mod room_input_map_authoring;"));
}

#[test]
fn project_linking_is_separate_from_main_editor_play() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let features = manifest["features"].as_table().unwrap();
    assert_eq!(
        features["room-input-map-project"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["room-input-map-editor", "orr_sample/room-controls-attach"]
    );
    let panel = include_str!("../src/room_input_map_authoring.rs");
    assert!(!panel.contains("ProjectLink"));
    assert!(!panel.contains("room_controls_attach"));
}
