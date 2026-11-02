use super::*;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationStatus};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, mesh_from_view, AgentSpec};

fn fixture() -> (Snapshot, MeshView) {
    use orr_fp::FP;
    let vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_int(4)],
        [FP::from_int(4), FP::ZERO, FP::ZERO],
        [FP::from_int(4), FP::ZERO, FP::from_int(4)],
    ];
    let graph =
        StaticMeshGraph::bake(&vertices, &[[0, 1, 3], [0, 3, 2]], AgentProfile::default()).unwrap();
    let spec = AgentSpec {
        start: [FP::ONE, FP::ZERO, FP::ONE],
        goal: [FP::from_int(3), FP::ZERO, FP::ONE],
        max_vertical: FP::ZERO,
        distance_per_tick: FP::from_ratio(1, 4),
    };
    let document = create_mesh_scene(&graph, spec).unwrap();
    let view = mesh_from_view(orr_bridge::FrameView::of(document.frame())).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let snapshot =
        Snapshot::from_package(&path, "animated.glb", "LICENSE.txt", graph.revision()).unwrap();
    assert!(snapshot.clips().len() >= 2);
    let settings = Settings {
        clip: 1,
        idle_clip: Some(0),
        crossfade_ticks: Some(4),
        ..snapshot.settings()
    };
    (snapshot.with_settings(settings).unwrap(), view)
}
fn context() -> PlaybackContext {
    PlaybackContext {
        scene_epoch: 1,
        sim_epoch: 2,
        discontinuity: 3,
        rest: false,
    }
}
fn moving(view: &mut MeshView, yes: bool) {
    // Moving with zero speed is the existing idle selector; no guessed status variant.
    view.agent.status = NavigationStatus::Moving;
    view.agent.spec.distance_per_tick = if yes {
        orr_fp::FP::from_ratio(1, 4)
    } else {
        orr_fp::FP::ZERO
    };
}

#[test]
fn crossfade_endpoints_midpoint_interruption_and_repeated_draw() {
    let (snapshot, mut view) = fixture();
    let mut cursor = PlaybackCursor::default();
    moving(&mut view, false);
    let outgoing = cursor
        .observe(&snapshot, &view, 10, context())
        .unwrap()
        .pose;
    moving(&mut view, true);
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 11, context())
            .unwrap()
            .pose,
        outgoing
    );
    let target = snapshot.placement_for_view(&view, 12).unwrap().pose;
    let quarter = snapshot
        .model
        .blend_poses(&outgoing, &target, 0.25)
        .unwrap();
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 12, context())
            .unwrap()
            .pose,
        quarter
    );
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 12, context())
            .unwrap()
            .pose,
        quarter
    );
    let target = snapshot.placement_for_view(&view, 13).unwrap().pose;
    let midpoint = snapshot.model.blend_poses(&outgoing, &target, 0.5).unwrap();
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 13, context())
            .unwrap()
            .pose,
        midpoint
    );
    moving(&mut view, false);
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 14, context())
            .unwrap()
            .pose,
        midpoint
    );
    for tick in 15..=18 {
        let target = snapshot.placement_for_view(&view, tick).unwrap().pose;
        let expected = snapshot
            .model
            .blend_poses(&midpoint, &target, (tick - 14) as f32 / 4.0)
            .unwrap();
        assert_eq!(
            cursor
                .observe(&snapshot, &view, tick, context())
                .unwrap()
                .pose,
            expected
        );
    }
    assert!(cursor.observed.as_ref().unwrap().outgoing.is_none());
}

#[test]
fn crossfade_discontinuities_errors_and_configuration_changes_snap() {
    let (snapshot, mut view) = fixture();
    let mut cursor = PlaybackCursor::default();
    for changed in [
        PlaybackContext {
            scene_epoch: 9,
            ..context()
        },
        PlaybackContext {
            sim_epoch: 9,
            ..context()
        },
        PlaybackContext {
            discontinuity: 9,
            ..context()
        },
        PlaybackContext {
            rest: true,
            ..context()
        },
    ] {
        moving(&mut view, false);
        cursor.observe(&snapshot, &view, 10, context()).unwrap();
        moving(&mut view, true);
        let actual = cursor.observe(&snapshot, &view, 11, changed).unwrap();
        assert_eq!(
            actual.pose,
            snapshot.placement_for_view(&view, 11).unwrap().pose
        );
    }
    for tick in [10, 9, 12, u64::MAX] {
        cursor.reset();
        moving(&mut view, false);
        cursor.observe(&snapshot, &view, 10, context()).unwrap();
        moving(&mut view, true);
        assert_eq!(
            cursor
                .observe(&snapshot, &view, tick, context())
                .unwrap()
                .pose,
            snapshot.placement_for_view(&view, tick).unwrap().pose
        );
    }
    moving(&mut view, false);
    cursor.observe(&snapshot, &view, 10, context()).unwrap();
    moving(&mut view, true);
    let changed = snapshot
        .clone()
        .with_settings(Settings {
            rate: 2.0,
            ..snapshot.settings()
        })
        .unwrap();
    assert_eq!(
        cursor.observe(&changed, &view, 11, context()).unwrap().pose,
        changed.placement_for_view(&view, 11).unwrap().pose
    );
    let mut invalid = snapshot.clone();
    invalid.graph_revision = [0xff; 32];
    assert!(cursor.observe(&invalid, &view, 12, context()).is_err());
    assert!(cursor.observed.is_none());
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 13, context())
            .unwrap()
            .pose,
        snapshot.placement_for_view(&view, 13).unwrap().pose
    );
}

#[test]
fn crossfade_disabled_and_same_clip_preserve_stateless_sampling() {
    let (snapshot, mut view) = fixture();
    for ticks in [None, Some(0), Some(4)] {
        let settings = Settings {
            idle_clip: Some(snapshot.settings().clip),
            crossfade_ticks: ticks,
            ..snapshot.settings()
        };
        let configured = snapshot.clone().with_settings(settings).unwrap();
        let mut cursor = PlaybackCursor::default();
        for tick in 1..=8 {
            moving(&mut view, tick % 2 == 0);
            assert_eq!(
                cursor
                    .observe(&configured, &view, tick, context())
                    .unwrap()
                    .pose,
                configured.placement_for_view(&view, tick).unwrap().pose
            );
        }
    }
}

#[test]
fn crossfade_asset_entity_instance_changes_and_explicit_reset_do_not_reuse_history() {
    let (snapshot, mut view) = fixture();
    let mut cursor = PlaybackCursor::default();
    moving(&mut view, false);
    cursor.observe(&snapshot, &view, 10, context()).unwrap();
    let bytes = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let reopened =
        Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", snapshot.graph_revision).unwrap();
    moving(&mut view, true);
    let actual = cursor.observe(&reopened, &view, 11, context()).unwrap();
    assert_eq!(
        actual.pose,
        reopened.placement_for_view(&view, 11).unwrap().pose
    );
    moving(&mut view, false);
    cursor.reset();
    cursor.observe(&snapshot, &view, 10, context()).unwrap();
    moving(&mut view, true);
    view.agent.entity.version += 1;
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 11, context())
            .unwrap()
            .pose,
        snapshot.placement_for_view(&view, 11).unwrap().pose
    );
    moving(&mut view, false);
    cursor.reset();
    cursor.observe(&snapshot, &view, 10, context()).unwrap();
    moving(&mut view, true);
    cursor.observe(&snapshot, &view, 11, context()).unwrap();
    // Same tick with a corrected placement invalidates the cached visible pose.
    view.agent.position[0] += orr_fp::FP::ONE;
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 11, context())
            .unwrap()
            .pose,
        snapshot.placement_for_view(&view, 11).unwrap().pose
    );
    cursor.reset();
    assert_eq!(
        cursor
            .observe(&snapshot, &view, 12, context())
            .unwrap()
            .pose,
        snapshot.placement_for_view(&view, 12).unwrap().pose
    );
}
