use super::*;
const SCENE: &str = include_str!("../../assets/stormglass-embedded.scene.yaml");

#[test]
fn embedded_rejects_external_scene_before_access() {
    for headless in [false, true] {
        let options = Options {
            headless,
            scene: Some("missing-and-never-opened.scene.yaml".into()),
            ..Default::default()
        };
        assert!(run_embedded(options, SCENE)
            .unwrap_err()
            .contains("does not accept --scene"));
    }
}

#[test]
fn embedded_strict_parser_rejects_before_startup() {
    for bad in [
        format!("{SCENE}entities: {{}}\n"),
        format!("{SCENE}---\n{SCENE}"),
        SCENE.replace("reactor_hp: 75", "reactor_hp: 0"),
        SCENE.replace("starting_gold: 250", "starting_gold: 250, unknown: 1"),
        format!(
            "{SCENE}#{}",
            "x".repeat(orr_games::coop_defense_scene::MAX_SOURCE_BYTES)
        ),
    ] {
        for headless in [false, true] {
            assert!(run_embedded(
                Options {
                    headless,
                    ..Default::default()
                },
                &bad
            )
            .is_err());
        }
    }
}

#[test]
fn embedded_authored_scene_matches_file_session_and_changed_scene_differs() {
    let config = orr_games::coop_defense_scene::parse_config(SCENE).unwrap();
    assert_eq!(config.scenario.starting_gold, 250);
    assert_eq!(config.scenario.reactor_hp, 75);
    assert_eq!(config.campaign.last_wave, 3);
    assert_eq!(config.wave_pacing.spawn_interval_ticks, 70);
    assert_eq!(
        config.wave_counts.pair_counts,
        [2, 1, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    assert_eq!(config.enemy_health.multiplier, 3);
    assert_eq!(config.wave_composition.regular_kinds[0], [3, 2, 1, 0]);
    assert_eq!(
        config.wave_composition.opening_boss,
        [1, 0, 0, 0, 1, 0, 0, 0, 0, 1]
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.yaml");
    std::fs::write(&path, SCENE).unwrap();
    let from_file = prepare_config(&Options {
        scene: Some(path),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(from_file, config);
    let changed = orr_games::coop_defense_scene::parse_config(
        &SCENE.replace("reactor_hp: 75", "reactor_hp: 76"),
    )
    .unwrap();
    let mut embedded = session(config);
    let mut ordinary = session(from_file);
    let mut edited = session(changed);
    for _ in 0..420 {
        embedded.advance(game::DefenseInput::default(), vec![]);
        ordinary.advance(game::DefenseInput::default(), vec![]);
        edited.advance(game::DefenseInput::default(), vec![]);
        assert_eq!(
            embedded.predicted_frame().checksum(),
            ordinary.predicted_frame().checksum()
        );
        assert_ne!(
            embedded.predicted_frame().checksum(),
            edited.predicted_frame().checksum()
        );
    }
    assert_eq!(game::run(embedded.predicted_frame()).reactor_hp, 75);
    assert_eq!(game::run(edited.predicted_frame()).reactor_hp, 76);
}

#[test]
fn embedded_source_hash_binds_exact_bytes_and_strict_admission() {
    use sha2::{Digest, Sha256};
    let expected: String = Sha256::digest(SCENE.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(embedded_scene_sha256(SCENE).unwrap(), expected);
    let comment = format!("{SCENE}# same configuration, distinct source\n");
    assert_eq!(
        orr_games::coop_defense_scene::parse_config(SCENE).unwrap(),
        orr_games::coop_defense_scene::parse_config(&comment).unwrap()
    );
    assert_ne!(embedded_scene_sha256(&comment).unwrap(), expected);
    assert!(embedded_scene_sha256("invalid: source").is_err());
    assert!(embedded_scene_sha256(&"x".repeat(65537)).is_err());
}
