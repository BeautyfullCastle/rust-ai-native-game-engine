use super::{parse_settings, Panel};

#[test]
fn parses_settings_without_rewriting_the_fixed_point_speed() {
    let settings = parse_settings("12", "7", "3", "0.125").expect("valid settings");

    assert_eq!(settings.width, 12);
    assert_eq!(settings.depth, 7);
    assert_eq!(settings.collectible_count, 3);
    assert_eq!(settings.distance_per_tick, "0.125");
}

#[test]
fn rejects_non_integer_dimensions_and_collectible_counts() {
    assert!(parse_settings("8.5", "6", "2", "0.125").is_err());
    assert!(parse_settings("8", "-6", "2", "0.125").is_err());
    assert!(parse_settings("8", "6", "2.5", "0.125").is_err());
}

#[test]
fn rejects_missing_distance_per_tick() {
    assert!(parse_settings("8", "6", "2", " ").is_err());
}

#[test]
fn wizard_defaults_match_workspace_settings() {
    let panel = Panel::default();

    assert_eq!(panel.width, "8");
    assert_eq!(panel.depth, "6");
    assert_eq!(panel.collectible_count, "2");
    assert_eq!(panel.distance_per_tick, "0.125");
}

#[test]
fn failure_keeps_the_wizard_open_and_preserves_entered_values() {
    let mut panel = Panel {
        open: true,
        #[cfg(feature = "mesh-hazards")]
        hazards: false,
        #[cfg(feature = "mesh-hazards")]
        include_hazard: false,
        scene_path: "C:\\games\\starter.scene.yaml".into(),
        width: "11".into(),
        depth: "9".into(),
        collectible_count: "4".into(),
        distance_per_tick: "0.25".into(),
        error: None,
    };

    panel.failed("destination already exists".into());

    assert!(panel.open);
    assert_eq!(panel.scene_path, "C:\\games\\starter.scene.yaml");
    assert_eq!(panel.width, "11");
    assert_eq!(panel.depth, "9");
    assert_eq!(panel.collectible_count, "4");
    assert_eq!(panel.distance_per_tick, "0.25");
    assert_eq!(panel.error.as_deref(), Some("destination already exists"));
}

#[cfg(feature = "mesh-hazards")]
#[test]
fn hazard_mode_is_explicit_and_repeated_open_does_not_switch_it() {
    let mut panel = Panel::default();
    panel.open_hazards();
    assert_eq!(panel.hazard_request(), Some(false));
    panel.open();
    assert_eq!(panel.hazard_request(), Some(false));
    panel.succeeded();
    panel.open();
    assert_eq!(panel.hazard_request(), None);
    panel.open_hazards();
    assert_eq!(panel.hazard_request(), None);
}

#[cfg(feature = "mesh-hazards")]
#[test]
fn escape_cancels_hazard_creation_without_emitting_request() {
    let mut panel = Panel::default();
    panel.open_hazards();
    panel.scene_path = "/tmp/never-created.scene.yaml".into();
    let context = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = context.run_ui(input, |ui| assert!(panel.show(ui.ctx(), None).is_none()));
    output.textures_delta.clear();
    assert!(!panel.busy());
    panel.open();
    assert_eq!(panel.hazard_request(), None);
}
