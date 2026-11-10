use super::*;
use orr_games::coop_defense_game::{
    DefenseConfig, DefenseEnemy, DefenseGame, DefenseInput, DefenseRun, SCENARIO_BUILD_ID,
    TICK_RATE,
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};

fn simulation(config: DefenseConfig) -> Simulation<DefenseGame> {
    Simulation::with_build_id(config, TICK_RATE, config.seed.into(), SCENARIO_BUILD_ID)
}

#[test]
fn authored_wave_preview_limits_and_literal_boss_ordinal() {
    let config = orr_games::coop_defense_scene::parse_config(include_str!(
        "../../../scenes/stormglass-composition.scene.yaml"
    ))
    .unwrap();
    let sim = simulation(config);
    let preview = waves(FrameView::of(sim.frame()));
    assert_eq!(preview.len(), 3);
    assert_eq!(preview[0].kinds, [BOSS, NORMAL, ARMORED, FAST, HORDE]);
    assert_eq!(
        preview[0].label(),
        "Wave 1: 5 per lane (10 total) | boss, normal, armored, fast, horde"
    );
    let default = simulation(DefenseConfig::default());
    let rows = waves(FrameView::of(default.frame()));
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].kinds[..4], [FAST, ARMORED, HORDE, NORMAL]);
    assert_eq!(rows[4].kinds[0], BOSS);
    assert_eq!(rows[9].kinds.len(), 28);
    assert_eq!(rows[9].kinds[0], BOSS);
    for interval in [0, 34, 351, u32::MAX] {
        assert_eq!(first_spawn_tick(interval), None);
    }
}

#[test]
fn authored_wave_preview_matches_real_spawn_ticks_and_kinds() {
    // Independent oracle: step the real game and inspect new enemies. No test
    // hook sets runtime wave state, and preview formulas are not the oracle.
    for (interval, count, boss, literal_first) in [
        (35, 28, 0, 210),
        (70, 5, 1, 210),
        (180, 4, 0, 180),
        (350, 1, 1, 350),
    ] {
        let mut config = DefenseConfig::default();
        config.campaign.last_wave = 1;
        config.scenario.reactor_hp = 1000;
        config.wave_pacing.spawn_interval_ticks = interval;
        config.wave_counts.pair_counts[0] = count;
        config.wave_composition.regular_kinds[0] = [HORDE, NORMAL, ARMORED, FAST];
        config.wave_composition.opening_boss[0] = boss;
        let mut sim = simulation(config);
        let before = sim.frame().to_bytes();
        let rows = waves(FrameView::of(sim.frame()));
        assert_eq!(sim.frame().to_bytes(), before);
        assert_eq!(first_spawn_tick(interval), Some(literal_first));
        let mut next_id = 1;
        let mut actual = Vec::new();
        for _ in 0..literal_first + count * interval {
            let mut inputs = TickInputs::new(sim.tick(), 1);
            inputs.set_input(PlayerSlot(0), DefenseInput::default());
            sim.step(&inputs);
            let run = sim.frame().singleton::<DefenseRun>();
            let mut spawned: Vec<_> = sim
                .frame()
                .dense::<DefenseEnemy>()
                .1
                .iter()
                .filter(|enemy| enemy.active != 0 && enemy.id >= next_id)
                .collect();
            spawned.sort_by_key(|enemy| enemy.id);
            for enemy in spawned {
                actual.push((run.wave_tick, enemy.lane, enemy.kind));
                next_id = enemy.id + 1;
            }
        }
        let expected: Vec<_> = rows[0]
            .kinds
            .iter()
            .enumerate()
            .flat_map(|(ordinal, kind)| {
                [0, 1].map(|lane| (literal_first + ordinal as u32 * interval, lane, *kind))
            })
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn authored_wave_preview_tracks_unsaved_undo_redo_save_and_reopen() {
    use orr_editor::{registered_project::PreparedRegisteredProject, Editor};
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("stormglass.scene.yaml");
    let manifest = directory.path().join("project.json");
    std::fs::write(&scene, orr_remote::coop_defense::INITIAL_SCENE).unwrap();
    std::fs::write(
        &manifest,
        include_bytes!("../../../scenes/stormglass.project.json"),
    )
    .unwrap();
    let mut editor = Editor::open_registered_project(
        PreparedRegisteredProject::open(&manifest, crate::stormglass::registry()).unwrap(),
    )
    .unwrap();
    editor.sync();
    assert_eq!(waves(editor.snapshot().unwrap().predicted()).len(), 10);
    editor
        .host_call(
            "world.singleton.patch",
            json!({"name":"CoopDefense::Campaign","path":"last_wave","value":1}),
        )
        .unwrap();
    editor.sync();
    assert_eq!(waves(editor.snapshot().unwrap().predicted()).len(), 1);
    for (name, path, value) in [
        ("CoopDefense::WaveCounts", "pair_counts[0]", 3),
        ("CoopDefense::WavePacing", "spawn_interval_ticks", 350),
    ] {
        editor
            .host_call(
                "world.singleton.patch",
                json!({"name":name,"path":path,"value":value}),
            )
            .unwrap();
    }
    editor.sync();
    let original = waves(editor.snapshot().unwrap().predicted());
    assert_eq!(original[0].kinds.len(), 3);
    assert_eq!(
        first_spawn_tick(
            editor
                .snapshot()
                .unwrap()
                .predicted()
                .singleton::<WavePacing>()
                .spawn_interval_ticks
        ),
        Some(350)
    );
    editor
        .host_call(
            "world.singleton.patch",
            json!({"name":"CoopDefense::WaveComposition","path":"opening_boss[0]","value":1}),
        )
        .unwrap();
    editor.sync();
    let authored = waves(editor.snapshot().unwrap().predicted());
    assert_eq!(authored[0].kinds[0], BOSS);
    assert_eq!(
        std::fs::read_to_string(&scene).unwrap(),
        orr_remote::coop_defense::INITIAL_SCENE
    );
    assert!(editor.undo());
    editor.sync();
    assert_eq!(
        waves(editor.snapshot().unwrap().predicted())[0],
        original[0]
    );
    assert!(editor.redo());
    editor.sync();
    assert_eq!(waves(editor.snapshot().unwrap().predicted()), authored);
    assert!(editor.save());
    let saved = std::fs::read_to_string(&scene).unwrap();
    drop(editor);
    let mut reopened = Editor::open_registered_project(
        PreparedRegisteredProject::open(&manifest, crate::stormglass::registry()).unwrap(),
    )
    .unwrap();
    reopened.sync();
    assert_eq!(waves(reopened.snapshot().unwrap().predicted()), authored);
    assert_eq!(
        first_spawn_tick(
            reopened
                .snapshot()
                .unwrap()
                .predicted()
                .singleton::<WavePacing>()
                .spawn_interval_ticks
        ),
        Some(350)
    );
    assert_eq!(
        waves(FrameView::of(
            simulation(orr_games::coop_defense_scene::parse_config(&saved).unwrap()).frame()
        )),
        authored
    );
    assert_eq!(std::fs::read_to_string(&scene).unwrap(), saved);
}

#[cfg(feature = "api-acceptance")]
#[test]
fn authored_wave_preview_details_widget_is_readonly_in_edit_and_live() {
    use crate::stormglass::StormglassView;
    use egui_kittest::{kittest::Queryable, Harness};
    use orr_editor::static_adapter::{EmbeddedGameplayView, GameplayViewContext, IntentRequest};
    let sim = simulation(
        orr_games::coop_defense_scene::parse_config(include_str!(
            "../../../scenes/stormglass-composition.scene.yaml"
        ))
        .unwrap(),
    );
    let before = sim.frame().to_bytes();
    for phase in [GameplayViewPhase::Edit, GameplayViewPhase::Live] {
        let mut harness = Harness::builder()
            .with_size([1100.0, 850.0])
            .build_ui_state(
                |ui, state: &mut (StormglassView, Vec<IntentRequest>)| {
                    if let Some(intent) = state
                        .0
                        .show(
                            ui,
                            FrameView::of(sim.frame()),
                            GameplayViewContext {
                                phase,
                                input_ready: phase == GameplayViewPhase::Live,
                            },
                        )
                        .unwrap()
                    {
                        state.1.push(intent);
                    }
                },
                (StormglassView::default(), Vec::new()),
            );
        harness.run_steps(3);
        harness.get_by_label("Details").click();
        harness.run_steps(3);
        harness.get_by_label("Authored waves").click();
        harness.run_steps(3);
        assert!(harness
            .query_by_label("Wave 1: 5 per lane (10 total) | boss, normal, armored, fast, horde")
            .is_some());
        assert!(harness
            .query_by_label("First pair at wave tick 210; then every 70 ticks (60 ticks/second).")
            .is_some());
        assert!(harness
            .query_by_label(
                "Starting gold per board: 1000 | Reactor HP: 1000 | Enemy HP multiplier: 2"
            )
            .is_some());
        let expected = if phase == GameplayViewPhase::Edit {
            "Read-only current edit snapshot, including unsaved changes. Save to keep these settings."
        } else {
            "Read-only displayed session settings, not remaining enemies or a victory prediction."
        };
        assert!(harness.query_by_label(expected).is_some());
        for _ in 0..2 {
            harness.get_by_label("Authored waves").click();
            harness.run_steps(3);
        }
        assert!(harness.state().1.is_empty());
        assert_eq!(sim.frame().to_bytes(), before);
    }
}
