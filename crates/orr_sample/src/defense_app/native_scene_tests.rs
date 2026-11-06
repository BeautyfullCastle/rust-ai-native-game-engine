//! Explicit delivery acceptance: require real binaries and fail if absent.
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, process::Command};

fn copy_binary(variable: &str, target: &Path) {
    let source = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is required"));
    assert!(
        Path::new(&source).is_absolute(),
        "{variable} must be absolute"
    );
    std::fs::copy(source, target).unwrap();
}
fn assert_settings(frame: &orr_ecs::Frame, config: &game::DefenseConfig) {
    assert_eq!(*frame.singleton::<game::Scenario>(), config.scenario);
    assert_eq!(*frame.singleton::<game::Campaign>(), config.campaign);
    assert_eq!(*frame.singleton::<game::WavePacing>(), config.wave_pacing);
    assert_eq!(*frame.singleton::<game::WaveCounts>(), config.wave_counts);
    assert_eq!(*frame.singleton::<game::EnemyHealth>(), config.enemy_health);
    assert_eq!(
        *frame.singleton::<game::WaveComposition>(),
        config.wave_composition
    );
}
fn observe_wave(native: &mut DefenseSession) -> Vec<(u32, u32, u32, u32)> {
    let mut seen = BTreeSet::new();
    let mut spawned = Vec::new();
    for _ in 0..420 {
        native.advance(game::DefenseInput::default(), vec![]);
        let frame = native.predicted_frame();
        let state = game::run(frame);
        for enemy in game::enemies(frame)
            .into_iter()
            .filter(|enemy| enemy.active != 0)
        {
            if seen.insert(enemy.id) {
                spawned.push((state.wave_tick, enemy.lane, enemy.kind, enemy.max_hp));
            }
        }
    }
    spawned
}

#[test]
#[ignore = "requires ORR_NATIVE_SCENE_CREATOR_BIN and ORR_NATIVE_SCENE_PLAYER_BIN from this exact source"]
fn relocated_native_creator_scene_reaches_player_session_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let creator = temp.path().join("creator");
    let player = temp.path().join("player");
    copy_binary("ORR_NATIVE_SCENE_CREATOR_BIN", &creator);
    copy_binary("ORR_NATIVE_SCENE_PLAYER_BIN", &player);
    let authored = temp.path().join("authored.yaml");
    let result = Command::new(&creator)
        .current_dir(temp.path())
        .env("PATH", "")
        .args(["create-coop-native-scene", "--output"])
        .arg(&authored)
        .args([
            "--starting-gold",
            "250",
            "--reactor-hp",
            "75",
            "--last-wave",
            "3",
            "--spawn-interval-ticks",
            "70",
            "--wave-pairs",
            "2,1,3,4,5,6,7,8,9,10",
            "--enemy-health-multiplier",
            "3",
            "--wave-kinds",
            "3,2,1,0;0,1,2,3;1,2,3,0;2,3,0,1;3,0,1,2;0,0,0,0;1,1,1,1;2,2,2,2;3,3,3,3;0,1,2,3",
            "--opening-bosses",
            "1,0,0,0,1,0,0,0,0,1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(receipt["kind"], "orr.coop-native-scene/1");
    let bytes = std::fs::read(&authored).unwrap();
    assert_eq!(
        receipt["sceneSha256"],
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(receipt["sceneBytes"], bytes.len());
    assert!(!receipt.as_object().unwrap().contains_key("wasmSha256"));
    let delivered = temp.path().join("relocated");
    std::fs::create_dir(&delivered).unwrap();
    let scene = delivered.join("scene.yaml");
    std::fs::rename(&authored, &scene).unwrap();
    assert!(!authored.exists());
    assert_eq!(std::fs::read(&scene).unwrap(), bytes);
    let text = std::str::from_utf8(&bytes).unwrap();
    for name in [
        "Scenario",
        "Campaign",
        "WavePacing",
        "WaveCounts",
        "EnemyHealth",
        "WaveComposition",
    ] {
        assert!(text.contains(&format!("  CoopDefense::{name}:\n")));
    }
    let config = prepare_config(&Options {
        scene: Some(scene.clone()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        config.scenario,
        game::Scenario {
            starting_gold: 250,
            reactor_hp: 75
        }
    );
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
    let mut native = session(config);
    assert_settings(native.predicted_frame(), &config);
    let expected = vec![
        (210, 0, game::BOSS, 1638),
        (210, 1, game::BOSS, 1638),
        (280, 0, game::ARMORED, 234),
        (280, 1, game::ARMORED, 234),
    ];
    assert_eq!(observe_wave(&mut native), expected);
    let checksum = native.predicted_frame().checksum();
    let output = Command::new(&player)
        .current_dir(&delivered)
        .env("PATH", "")
        .args(["--scene", "scene.yaml", "--headless", "--ticks", "420"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("wave=1 phase=0 reactor=75"), "{stdout}");
    assert!(
        stdout.contains(&format!("checksum=0x{checksum:016x}")),
        "{stdout}"
    );
    let delay = native.config().input_delay;
    native.advance(
        game::DefenseInput {
            action: game::RESTART,
            sequence: 1,
            ..Default::default()
        },
        vec![],
    );
    for _ in 0..delay {
        native.advance(game::DefenseInput::default(), vec![]);
    }
    assert_eq!(game::run(native.predicted_frame()).generation, 1);
    assert_eq!(game::run(native.predicted_frame()).gold, [250; 2]);
    assert_eq!(game::run(native.predicted_frame()).reactor_hp, 75);
    assert_settings(native.predicted_frame(), &config);
    assert_eq!(observe_wave(&mut native), expected);
    assert_settings(native.predicted_frame(), &config);
}
