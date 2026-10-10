#![cfg(feature = "project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use std::{fs, path::Path, sync::Arc};

use orr_bridge::{
    Bridge, BridgeStats, InProc, PlayHost, PlayMode, PlayerSlot, Snapshot, SnapshotParts, Speed,
    Timeline,
};
use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};
use orr_reflect::Guid;
use orr_sample::{
    arena_game::{ArenaInput, Bullet, PlayerTag, Position},
    arena_view::arena_bridge_config,
    project_runtime::{PreparedRuntime, PLAYERS, TICK_RATE},
};
#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::{copy_tree, ProjectFixture};

const HERO: &str = "e_00000001";
const TARGET: &str = "e_00000002";

fn entity(index: &orr_reflect::SceneIndex, guid: &str) -> Entity {
    index.entity(&Guid::parse(guid).unwrap()).unwrap()
}

fn set_position(frame: &mut Frame, entity: Entity, x: i32, y: i32) {
    frame.get_mut::<Position>(entity).unwrap().pos = FPVec2::new(FP::from_int(x), FP::from_int(y));
}

fn snapshot(frame: Frame, seq: u64, epoch: u64) -> Snapshot {
    let tick = frame.tick();
    let checksum = frame.checksum();
    Snapshot::from_parts(SnapshotParts {
        seq,
        tick,
        verified_tick: tick,
        tick_rate: TICK_RATE,
        predicted: Arc::new(frame.clone()),
        predicted_prev: None,
        verified: Some(Arc::new(frame)),
        stats: BridgeStats::default(),
        last_rollback: None,
        timeline: Some(Timeline {
            mode: PlayMode::Record,
            tick,
            verified_tick: tick,
            first_tick: 0,
            last_tick: tick,
            playing: false,
            speed: Speed::NORMAL,
            checksum,
            keyframes: Arc::from(vec![0]),
            recent_checksums: Arc::from(vec![(tick, checksum)]),
            pending_edits: 0,
            branches: 0,
            epoch,
        }),
    })
}

#[test]
fn admitted_scene_starts_two_authored_slots_and_fresh_sessions_reset() {
    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let initial_checksum = prepared.initial_frame().checksum();
    let initial_entities = prepared.initial_frame().dense::<PlayerTag>().1;
    assert_eq!(initial_entities.len(), 2);
    assert_eq!(prepared.index().len(), 2);

    let mut session = prepared.session().unwrap();
    assert_eq!(session.player_count(), PLAYERS);
    assert_eq!(session.tick_rate(), TICK_RATE);
    assert_eq!(session.head_tick(), 0);
    assert_eq!(session.frame().checksum(), initial_checksum);
    let hero = entity(prepared.index(), HERO);
    let target = entity(prepared.index(), TARGET);

    for _ in 0..2 {
        session.set_input(
            PlayerSlot(0),
            ArenaInput::new(FP::from_int(1), FP::ZERO, false),
        );
        session.set_input(
            PlayerSlot(1),
            ArenaInput::new(FP::ZERO, FP::from_int(1), false),
        );
        assert!(session.step_now().is_some());
    }
    assert_eq!(session.head_tick(), 2);
    assert_eq!(
        session.frame().get::<Position>(hero).unwrap().pos.x,
        FP::from_int(-48)
    );
    assert_eq!(
        session.frame().get::<Position>(target).unwrap().pos.y,
        FP::from_int(12)
    );
    assert_eq!(session.frame().dense::<Bullet>().1.len(), 0);

    let fresh = prepared.session().unwrap();
    assert_eq!(fresh.head_tick(), 0);
    assert_eq!(fresh.frame().checksum(), initial_checksum);
    let source_scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/saved_arena_project/arena.scene.yaml");
    assert_eq!(
        fs::read(fixture.root.join("arena.scene.yaml")).unwrap(),
        fs::read(source_scene).unwrap()
    );
}

#[test]
fn normal_bridge_maps_held_fire_once_and_has_no_automatic_opponent() {
    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let hero = entity(prepared.index(), HERO);
    let target = entity(prepared.index(), TARGET);
    let mut bridge = prepared.bridge().unwrap();
    assert_eq!(bridge.player_count(), 2);
    assert_eq!(bridge.host().session().head_tick(), 0);

    bridge
        .set_input(
            PlayerSlot(0),
            ArenaInput::new(FP::from_int(1), FP::ZERO, true),
        )
        .unwrap();
    bridge.step(1);
    let frame = bridge.host().session().frame();
    assert_eq!(
        frame.dense::<Bullet>().1.len(),
        1,
        "the bridge derives one fire command, not zero or two"
    );
    assert_eq!(
        frame.get::<Position>(hero).unwrap().pos.x,
        FP::from_int(-54)
    );
    assert_eq!(
        frame.get::<Position>(target).unwrap().pos.x,
        FP::from_int(60)
    );
    assert_eq!(frame.get::<Position>(target).unwrap().pos.y, FP::ZERO);

    bridge.step(1);
    assert_eq!(bridge.host().session().frame().dense::<Bullet>().1.len(), 2);
    assert_eq!(
        bridge
            .host()
            .session()
            .frame()
            .get::<Position>(target)
            .unwrap()
            .pos
            .y,
        FP::ZERO
    );
}

#[test]
fn presentation_uses_coherent_snapshots_and_resets_on_gap_seek_and_recycled_handles() {
    let fixture = ProjectFixture::new();
    let sidecar_path = fixture.root.join("arena.sprites.json");
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["camera_follow"] = TARGET.into();
    fs::write(&sidecar_path, serde_json::to_vec_pretty(&sidecar).unwrap()).unwrap();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let base = prepared.initial_frame().clone();
    let hero = entity(prepared.index(), HERO);
    let target = entity(prepared.index(), TARGET);
    let (session, mut presentation) = prepared.into_parts().unwrap();
    let bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    let initial = bridge.snapshot().unwrap();
    let before = bridge.host().session().frame().checksum();
    presentation.update(Some(&initial)).unwrap();
    assert_eq!(
        bridge.host().session().frame().checksum(),
        before,
        "presentation is read-only"
    );
    assert_eq!(presentation.sprites().len(), 2);
    assert_eq!(presentation.camera.center, [60.0, 0.0]);
    assert_eq!(
        presentation.shapes().shapes.len(),
        3,
        "floor and both authored actors remain as underlays"
    );
    assert_eq!(presentation.sprites()[0].instance.size, [32.0, 32.0]);

    let mut moving = base.clone();
    moving.set_tick(1);
    set_position(&mut moving, hero, -54, 0);
    let moving = snapshot(moving, initial.seq() + 1, 0);
    presentation.update(Some(&moving)).unwrap();
    let hero_state = presentation.state(HERO);
    let target_state = presentation.state(TARGET);
    assert!(hero_state.moving);
    assert_eq!(hero_state.elapsed_ms, 0);
    assert!(!target_state.moving);
    assert_eq!(target_state.elapsed_ms, 16);
    presentation.update(Some(&moving)).unwrap();
    assert_eq!(
        presentation.state(HERO),
        hero_state,
        "repeated snapshots freeze the cursor"
    );

    let mut gap = base.clone();
    gap.set_tick(12);
    set_position(&mut gap, hero, -48, 0);
    presentation
        .update(Some(&snapshot(gap, initial.seq() + 2, 0)))
        .unwrap();
    assert_eq!(
        presentation.state(HERO),
        Default::default(),
        "a >8 tick gap resets locomotion"
    );
    assert_eq!(presentation.state(TARGET), Default::default());

    let mut resumed = base.clone();
    resumed.set_tick(13);
    set_position(&mut resumed, hero, -42, 0);
    presentation
        .update(Some(&snapshot(resumed, initial.seq() + 3, 0)))
        .unwrap();
    assert!(presentation.state(HERO).moving);
    let mut seeked = base.clone();
    seeked.set_tick(2);
    set_position(&mut seeked, hero, -36, 0);
    presentation
        .update(Some(&snapshot(seeked, initial.seq() + 4, 1)))
        .unwrap();
    assert_eq!(
        presentation.state(HERO),
        Default::default(),
        "a new timeline epoch resets after seek"
    );

    let mut no_target = base.clone();
    assert!(no_target.despawn(target));
    no_target.set_tick(3);
    set_position(&mut no_target, hero, -30, 0);
    presentation
        .update(Some(&snapshot(no_target, initial.seq() + 5, 1)))
        .unwrap();
    assert_eq!(presentation.state(TARGET), Default::default());
    assert_eq!(
        presentation.camera.center,
        [60.0, 0.0],
        "a missing camera target holds its last center"
    );
    assert_eq!(
        presentation.sprites().len(),
        1,
        "missing authored targets do not leave stale sprites"
    );
    assert_eq!(
        presentation.shapes().shapes.len(),
        2,
        "missing target preserves floor and surviving actor underlays"
    );

    let mut recycled = base.clone();
    assert!(recycled.despawn(hero));
    let replacement = recycled.spawn();
    assert_eq!(replacement.index, hero.index);
    assert_ne!(replacement.version, hero.version);
    recycled.add(replacement, PlayerTag { slot: 0 });
    recycled.add(
        replacement,
        Position {
            pos: FPVec2::new(FP::from_int(-24), FP::ZERO),
        },
    );
    recycled.set_tick(4);
    presentation
        .update(Some(&snapshot(recycled, initial.seq() + 6, 1)))
        .unwrap();
    assert_eq!(
        presentation.state(HERO),
        Default::default(),
        "GUID lookup rejects the recycled index's new generation"
    );
    assert_eq!(presentation.sprites().len(), 1);
    assert_eq!(presentation.sprites()[0].instance.position, [60.0, 0.0]);

    let sim_checksum = bridge.host().session().frame().checksum();
    presentation.update(None).unwrap();
    assert_eq!(bridge.host().session().frame().checksum(), sim_checksum);
    assert!(presentation.sprites().is_empty());
}

#[test]
fn malformed_runtime_entries_fail_before_a_session_is_created() {
    let fixture = ProjectFixture::new();
    let scene_path = fixture.root.join("arena.scene.yaml");
    let original = fs::read_to_string(&scene_path).unwrap();
    fs::write(&scene_path, original.replace("slot: 0", "slot: 99")).unwrap();
    let error = PreparedRuntime::open(&fixture.root)
        .err()
        .expect("invalid Arena slot must be rejected");
    assert!(
        error.contains("Arena scene") && error.contains("99 is outside [0, 7]"),
        "{error}"
    );
}

#[test]
fn runtime_rejects_packages_outside_its_compiled_capability_inventory() {
    let fixture = ProjectFixture::new();
    let package_source = fixture.root.join("replacement-package");
    fs::create_dir(&package_source).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
    copy_tree(&source, &package_source);
    let manifest_path = package_source.join("orr.package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["capabilities"]
        .as_array_mut()
        .unwrap()
        .push("models".into());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let install = orr_package::Project::open_for_install(
        &fixture.root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    install.remove("sample-sprites").unwrap();
    install.install(&[package_source]).unwrap();

    let error = PreparedRuntime::open(&fixture.root)
        .err()
        .expect("standalone Arena does not implement the models capability");
    assert!(error.contains("compiled capability"), "{error}");

    // The shared loader accepts the same active package when the caller truthfully
    // advertises the broader editor inventory; runtime startup must pass its own.
    let mut editor_inventory = orr_package::Runtime::content_only();
    editor_inventory.capabilities.insert("sprite".into());
    editor_inventory.capabilities.insert("models".into());
    let broad =
        orr_sample::project::PreparedProject::open(&fixture.root, editor_inventory).unwrap();
    assert!(broad.sprites().is_some());
}

#[cfg(feature = "input-actions")]
#[test]
fn remapped_held_fire_and_focus_release_reach_project_bridge() {
    use orr_input::{Button, Key};
    use orr_sample::arena_input::{default_map, ArenaControls};

    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let mut bridge = prepared.bridge().unwrap();
    let mut reference = PreparedRuntime::open(&fixture.root)
        .unwrap()
        .bridge()
        .unwrap();
    let mut map = default_map();
    map.actions
        .iter_mut()
        .find(|action| action.name == "right")
        .unwrap()
        .bindings = vec![Button::Keyboard { key: Key::L }];
    let mut controls = ArenaControls::new(map).unwrap();
    controls.button(Button::Keyboard { key: Key::L }, true, false, false);
    controls.button(Button::Keyboard { key: Key::Space }, true, false, false);
    controls.publish(&mut bridge).unwrap();
    reference
        .set_input(
            PlayerSlot(0),
            ArenaInput::new(FP::from_int(1), FP::ZERO, true),
        )
        .unwrap();
    bridge.step(1);
    reference.step(1);
    let frame = bridge.host().session().frame();
    assert_eq!(
        frame
            .get::<Position>(entity(prepared.index(), HERO))
            .unwrap()
            .pos
            .x,
        FP::from_int(-54)
    );
    assert_eq!(frame.dense::<Bullet>().1.len(), 1);

    controls.button(Button::Keyboard { key: Key::L }, false, false, false);
    controls.publish(&mut bridge).unwrap();
    reference
        .set_input(PlayerSlot(0), ArenaInput::new(FP::ZERO, FP::ZERO, true))
        .unwrap();
    bridge.step(1);
    reference.step(1);
    assert_eq!(
        bridge.host().session().frame().checksum(),
        reference.host().session().frame().checksum(),
        "physical key release stops movement while held fire remains active"
    );
    assert_eq!(
        bridge
            .host()
            .session()
            .frame()
            .get::<Position>(entity(prepared.index(), HERO))
            .unwrap()
            .pos
            .x,
        FP::from_int(-54)
    );

    controls.set_focused(false);
    controls.publish(&mut bridge).unwrap();
    reference
        .set_input(PlayerSlot(0), ArenaInput::default())
        .unwrap();
    for _ in 0..8 {
        bridge.step(1);
        reference.step(1);
        assert_eq!(
            bridge.host().session().frame().checksum(),
            reference.host().session().frame().checksum(),
            "focus loss must publish a neutral input before the next tick"
        );
    }
    let frame = bridge.host().session().frame();
    assert_eq!(
        frame
            .get::<Position>(entity(prepared.index(), HERO))
            .unwrap()
            .pos
            .x,
        FP::from_int(-54),
        "focus loss releases remapped movement immediately"
    );
    assert_eq!(
        frame.checksum(),
        reference.host().session().frame().checksum(),
        "focus loss releases held fire immediately"
    );
}

#[test]
fn scene_only_project_uses_authored_shapes_without_a_sprite_sidecar() {
    let fixture = ProjectFixture::new();
    let path = fixture.root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"].as_object_mut().unwrap().remove("sprites");
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let expected = prepared.initial_frame().checksum();
    let (session, mut presentation) = prepared.into_parts().unwrap();
    let bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(bridge.host().session().frame().checksum(), expected);
    assert!(presentation.document().is_none());
    assert!(presentation.assets().is_empty());
    assert!(presentation.sprites().is_empty());
    assert_eq!(presentation.shapes().shapes.len(), 3);
}

#[test]
fn admitted_runtime_owns_initial_frame_and_atlas_before_host_creation() {
    let fixture = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let expected = prepared.initial_frame().checksum();
    fs::write(
        fixture.root.join("arena.scene.yaml"),
        b"unchecked replacement",
    )
    .unwrap();
    fs::write(
        fixture.root.join("arena.sprites.json"),
        b"unchecked replacement",
    )
    .unwrap();
    fs::remove_dir_all(fixture.root.join(".orr")).unwrap();
    let (session, mut presentation) = prepared.into_parts().unwrap();
    let bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(bridge.host().session().frame().checksum(), expected);
    assert_eq!(presentation.sprites().len(), 2);
    assert!(presentation
        .assets()
        .values()
        .all(|asset| !asset.rgba.is_empty()));
    assert!(
        PreparedRuntime::open(&fixture.root).is_err(),
        "a fresh launch must still validate every input"
    );
}
