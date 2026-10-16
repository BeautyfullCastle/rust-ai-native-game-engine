#![cfg(feature = "project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use std::fs;
#[cfg(feature = "game-ui")]
use std::path::PathBuf;

use orr_bridge::PlayerSlot;
use orr_ecs::Entity;
use orr_fp::{FPVec2, FP};
use orr_reflect::Guid;
use orr_sample::{
    arena_game::{ArenaInput, Bullet, PlayerTag, Position},
    project_runtime::{build_id, AuthoredRestartSeed, PreparedRuntime, PLAYERS, SEED, TICK_RATE},
};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

const HERO: &str = "e_00000001";
const TARGET: &str = "e_00000002";

fn entity(index: &orr_reflect::SceneIndex, guid: &str) -> Entity {
    index.entity(&Guid::parse(guid).unwrap()).unwrap()
}

fn replay(seed: &AuthoredRestartSeed) -> Vec<u64> {
    let mut session = seed.session().unwrap();
    let input = ArenaInput::new(FP::ONE, FP::ZERO, false);
    let mut checksums = vec![session.frame().checksum()];
    for _ in 0..8 {
        session.set_input(PlayerSlot(0), input);
        session.set_input(PlayerSlot(1), ArenaInput::default());
        assert!(session.step_now().is_some());
        checksums.push(session.frame().checksum());
    }
    checksums
}

#[test]
fn seed_preserves_exact_admitted_identity_and_has_no_automatic_opponent() {
    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let baseline = prepared.initial_frame().checksum();
    let mut guids: Vec<_> = prepared
        .index()
        .iter()
        .map(|(guid, _)| guid.to_string())
        .collect();
    guids.sort();
    assert_eq!(guids, vec![HERO.to_owned(), TARGET.to_owned()]);

    let seed = prepared.restart_seed().clone();
    assert_eq!(seed.initial_frame().checksum(), baseline);
    assert_eq!(seed.config().game_id, "Arena");
    assert_eq!(seed.config().build_id, build_id());
    assert_eq!(seed.config().seed, SEED);
    assert_eq!(seed.config().tick_rate, TICK_RATE);
    assert_eq!(seed.config().player_count, PLAYERS);

    let mut session = seed.session().unwrap();
    assert_eq!(session.head_tick(), 0);
    assert_eq!(session.frame().checksum(), baseline);
    assert_eq!(session.player_count(), PLAYERS);
    let target = entity(prepared.index(), TARGET);
    let hero = entity(prepared.index(), HERO);
    assert_eq!(session.frame().dense::<PlayerTag>().1.len(), 2);

    for _ in 0..8 {
        session.set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, false));
        session.set_input(PlayerSlot(1), ArenaInput::default());
        assert!(session.step_now().is_some());
    }
    let frame = session.frame();
    assert_eq!(
        frame.get::<Position>(hero).unwrap().pos,
        FPVec2::new(FP::from_int(-12), FP::ZERO),
        "the authored local slot receives its explicit input"
    );
    assert_eq!(
        frame.get::<Position>(target).unwrap().pos,
        FPVec2::new(FP::from_int(60), FP::ZERO),
        "slot one remains idle instead of being driven by a bot"
    );
    assert!(frame.dense::<Bullet>().1.is_empty());
}

#[test]
fn cloned_seed_restarts_after_sources_are_removed_with_per_tick_parity() {
    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let initial_checksum = prepared.initial_frame().checksum();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    assert!(ui.is_none(), "the base fixture has no UI entry");
    assert!(presentation
        .assets()
        .values()
        .all(|asset| !asset.rgba.is_empty()));

    // Runtime state is now retained by the seed and presentation. A fresh
    // launch from disk should fail, while restart construction stays independent
    // of every source file and directory.
    fs::remove_dir_all(&fixture.root).unwrap();
    assert!(PreparedRuntime::open(&fixture.root).is_err());
    assert_eq!(seed.initial_frame().checksum(), initial_checksum);

    let original = replay(&seed);
    let restarted = replay(&seed.clone());
    assert_eq!(original[0], initial_checksum);
    assert_eq!(restarted[0], initial_checksum);
    assert_eq!(
        original, restarted,
        "each authored input tick must replay identically"
    );
}

#[cfg(feature = "game-ui")]
fn set_ui_entry(root: &std::path::Path, enabled: bool) {
    let manifest_path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    if enabled {
        manifest["entry"]["ui"] = serde_json::json!({
            "profile": "arena-korean-v1",
            "font": {
                "package": "korean-game-ui",
                "asset": "OrreryKoreanUI.otf"
            }
        });
    } else {
        manifest["entry"].as_object_mut().unwrap().remove("ui");
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "game-ui")]
#[test]
fn adding_or_removing_project_ui_metadata_does_not_change_simulation_identity() {
    let fixture = ProjectFixture::new();
    let font_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        &fixture.root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer.install(&[font_source]).unwrap();

    let scene_path = fixture.root.join("arena.scene.yaml");
    let lock_path = fixture.root.join("orr.packages.lock.json");
    let scene_bytes = fs::read(&scene_path).unwrap();
    let lock_bytes = fs::read(&lock_path).unwrap();
    let without_ui = PreparedRuntime::open(&fixture.root).unwrap();
    assert!(without_ui.project().ui().is_none());
    let original_seed = without_ui.restart_seed().clone();
    let original_checksum = original_seed.initial_frame().checksum();
    let original_build_id = original_seed.config().build_id;
    let original_ticks = replay(&original_seed);

    set_ui_entry(&fixture.root, true);
    let with_ui = PreparedRuntime::open(&fixture.root).unwrap();
    let admitted_ui = with_ui.project().ui().expect("UI font is admitted");
    assert_eq!(admitted_ui.descriptor.font.package, "korean-game-ui");
    assert!(!admitted_ui.font.is_empty());
    let ui_seed = with_ui.restart_seed().clone();
    assert_eq!(ui_seed.initial_frame().checksum(), original_checksum);
    assert_eq!(ui_seed.config().build_id, original_build_id);
    assert_eq!(replay(&ui_seed), original_ticks);
    assert_eq!(fs::read(&scene_path).unwrap(), scene_bytes);
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes);
    let legacy_error = match PreparedRuntime::open(&fixture.root).unwrap().into_parts() {
        Ok(_) => panic!("the legacy launch tuple must not discard admitted UI"),
        Err(error) => error,
    };
    assert!(legacy_error.contains("into_launch_parts"), "{legacy_error}");

    set_ui_entry(&fixture.root, false);
    let removed_ui = PreparedRuntime::open(&fixture.root).unwrap();
    assert!(removed_ui.project().ui().is_none());
    let removed_seed = removed_ui.restart_seed().clone();
    assert_eq!(removed_seed.initial_frame().checksum(), original_checksum);
    assert_eq!(removed_seed.config().build_id, original_build_id);
    assert_eq!(replay(&removed_seed), original_ticks);
    assert_eq!(fs::read(&scene_path).unwrap(), scene_bytes);
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes);
}
