#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/saved_project.rs"]
#[allow(unused_imports)]
mod fixture;

use fixture::*;
use orr_bridge::{Bridge, PlaySession, PlayerSlot};
use orr_edit::EditorDoc;
use orr_fp::FP;
use orr_reflect::Guid;
use orr_sample::{
    arena_game::{Arena, ArenaConfig, ArenaInput},
    arena_view::arena_fire_commands,
    project_runtime::{build_id, PreparedRuntime, PLAYERS, TICK_RATE},
};

const HERO: &str = "e_00000001";
const TARGET: &str = "e_00000002";
const TICKS: u64 = 180;

#[test]
fn saved_editor_scene_replays_through_authored_runtime_without_simulation_drift() {
    let fixture = SavedFixture::new();
    let mut app = fixture.headless();
    settle(&mut app);
    assert_loaded(&app, HERO);
    let original_sidecar = std::fs::read(fixture.sidecar()).unwrap();

    // Exercise the actual EditorApp edit path, then save its authored scene.
    let original_checksum = app.state().editor.checksum();
    edit_hero_x(&mut app);
    assert_ne!(app.state().editor.checksum(), original_checksum);
    let authored_checksum = app.state().editor.checksum();
    assert!(app.state_mut().editor.save());
    settle(&mut app);
    assert!(!app.state().editor.is_dirty());
    assert_loaded(&app, HERO);
    assert_eq!(std::fs::read(fixture.sidecar()).unwrap(), original_sidecar);
    let saved_scene = std::fs::read(fixture.scene()).unwrap();
    drop(app);

    let scene_text = std::str::from_utf8(&saved_scene).unwrap();
    let editor_doc: EditorDoc = orr_remote::sample::arena_doc(scene_text).unwrap();
    assert_eq!(editor_doc.checksum(), authored_checksum);
    let runtime = PreparedRuntime::open(fixture.root.path()).unwrap();
    assert_eq!(runtime.initial_frame().checksum(), authored_checksum);
    assert_eq!(build_id(), orr_remote::default_build_id("Arena"));
    let hero_guid = Guid::parse(HERO).unwrap();
    let target_guid = Guid::parse(TARGET).unwrap();
    assert_eq!(
        runtime.index().entity(&hero_guid),
        editor_doc.index().entity(&hero_guid)
    );
    assert_eq!(
        runtime.index().entity(&target_guid),
        editor_doc.index().entity(&target_guid)
    );

    let mut cfg = editor_doc.play_config(PLAYERS, TICK_RATE);
    cfg.game_id = "Arena".into();
    cfg.build_id = build_id();
    cfg.start_paused = true;
    let mut editor_play = PlaySession::<Arena>::from_frame(cfg, editor_doc.frame()).unwrap();
    // This is the editor host's normal-input command derivation. Runtime bridge()
    // instead uses arena_bridge_config, so this adapter is installed only here.
    editor_play
        .set_commands_from_input(|slot, input| arena_fire_commands(u32::from(slot.0), input));

    let runtime_session = runtime.session().unwrap();
    assert_eq!(runtime_session.frame().checksum(), authored_checksum);
    assert_eq!(runtime_session.head_tick(), 0);
    let mut runtime_bridge = runtime.bridge().unwrap();
    assert_eq!(runtime_bridge.host().session().player_count(), 2);
    assert_eq!(runtime_bridge.host().session().head_tick(), 0);
    assert_eq!(
        runtime_bridge.host().session().simulation().build_hash(),
        editor_play.simulation().build_hash(),
        "runtime replay identity matches the local Arena editor host"
    );
    let mut presentation = runtime.into_parts().unwrap().1;
    let initial = runtime_bridge.snapshot().unwrap();
    let initial_checksum = runtime_bridge.host().session().frame().checksum();
    presentation.update(Some(&initial)).unwrap();
    assert_eq!(
        runtime_bridge.host().session().frame().checksum(),
        initial_checksum
    );
    assert_eq!(presentation.sprites().len(), 2);
    assert_eq!(
        presentation.shapes().shapes.len(),
        3,
        "project sprites retain Arena shape underlays"
    );
    assert_eq!(presentation.sprites()[0].instance.size, [32.0, 32.0]);

    let mut checksums = Vec::with_capacity(TICKS as usize + 1);
    checksums.push(authored_checksum);
    for tick in 1..=TICKS {
        let phase = (tick / 30) % 4;
        let (axis_x, axis_y) = match phase {
            0 => (1, 0),
            1 => (0, 1),
            2 => (-1, 0),
            _ => (0, -1),
        };
        let input = ArenaInput::new(FP::from_int(axis_x), FP::from_int(axis_y), tick % 13 < 3);
        runtime_bridge.set_input(PlayerSlot(0), input).unwrap();
        runtime_bridge.step(1);
        editor_play.set_input(PlayerSlot(0), input);
        assert!(editor_play.step_now().is_some());
        let runtime_checksum = runtime_bridge.host().session().frame().checksum();
        assert_eq!(
            runtime_checksum,
            editor_play.frame().checksum(),
            "tick {tick}"
        );
        assert_eq!(
            runtime_bridge.host().session().checksum_at(tick),
            Some(runtime_checksum)
        );
        assert_eq!(editor_play.checksum_at(tick), Some(runtime_checksum));
        checksums.push(runtime_checksum);

        let snapshot = runtime_bridge.snapshot().unwrap();
        let before_presentation = runtime_bridge.host().session().frame().checksum();
        presentation.update(Some(&snapshot)).unwrap();
        assert_eq!(
            runtime_bridge.host().session().frame().checksum(),
            before_presentation,
            "presentation update at tick {tick} must not mutate simulation state"
        );
        if tick == 1 {
            let state = presentation.state(HERO);
            presentation.update(Some(&snapshot)).unwrap();
            presentation.update(Some(&snapshot)).unwrap();
            assert_eq!(
                presentation.state(HERO),
                state,
                "duplicate/repeated snapshots do not advance presentation time"
            );
        }
    }

    let replay_bytes = runtime_bridge.host().session().save_replay();
    let mut viewer = PlaySession::<Arena>::open_replay(
        &replay_bytes,
        ArenaConfig {
            player_count: PLAYERS,
        },
        build_id(),
    )
    .unwrap();
    for (tick, expected) in checksums.iter().copied().enumerate() {
        viewer.seek(tick as u64).unwrap();
        assert_eq!(viewer.frame().checksum(), expected, "replayed tick {tick}");
    }

    // A second session and a fresh launch both return to the exact saved scene.
    assert_eq!(runtime_session.head_tick(), 0);
    assert_eq!(runtime_session.frame().checksum(), authored_checksum);
    let reopened = PreparedRuntime::open(fixture.root.path()).unwrap();
    let relaunched = reopened.session().unwrap();
    assert_eq!(relaunched.head_tick(), 0);
    assert_eq!(relaunched.frame().checksum(), authored_checksum);
    assert_eq!(std::fs::read(fixture.scene()).unwrap(), saved_scene);
    assert_eq!(std::fs::read(fixture.sidecar()).unwrap(), original_sidecar);
}
