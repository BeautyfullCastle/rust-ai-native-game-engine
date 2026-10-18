use super::{Settings, Snapshot};
use std::path::Path;

const GRAPH_REVISION: [u8; 32] = [0x5a; 32];

fn fixture() -> Snapshot {
    fixture_for_revision(GRAPH_REVISION)
}

fn fixture_for_revision(graph_revision: [u8; 32]) -> Snapshot {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", graph_revision).unwrap()
}

#[test]
fn animated_package_is_source_independent_and_scene_bound() {
    let snapshot = fixture();
    assert!(!snapshot.clips().is_empty());
    let bytes = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let reopened = Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", GRAPH_REVISION).unwrap();
    assert_eq!(snapshot.clips(), reopened.clips());
    assert_eq!(snapshot.settings(), Settings::default());
    let default_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(default_json["settings"].get("facing").is_none());
    assert!(Snapshot::decode_embedded(&bytes, "other.scene.yaml", GRAPH_REVISION).is_err());
    assert!(Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", [0x13; 32]).is_err());
}

#[test]
fn route_facing_roundtrips_and_old_sidecars_default_to_fixed() {
    let snapshot = fixture();
    let legacy_bytes = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let legacy =
        Snapshot::decode_embedded(&legacy_bytes, "mesh.scene.yaml", GRAPH_REVISION).unwrap();
    assert_eq!(legacy.settings().facing, crate::mesh_facing::Facing::Fixed);

    let mut settings = snapshot.settings();
    settings.facing = crate::mesh_facing::Facing::Route;
    let route_facing = snapshot.clone().with_settings(settings).unwrap();
    let bytes = route_facing.encode_embedded("mesh.scene.yaml").unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["settings"]["facing"], "route");
    let reopened = Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", GRAPH_REVISION).unwrap();
    assert_eq!(
        reopened.settings().facing,
        crate::mesh_facing::Facing::Route
    );

    let mut unknown_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown_value["settings"]["facing"] = serde_json::json!("camera");
    let unknown = serde_json::to_vec(&unknown_value).unwrap();
    assert!(Snapshot::decode_embedded(&unknown, "mesh.scene.yaml", GRAPH_REVISION).is_err());
}

#[test]
fn settings_reject_invalid_transform_rate_and_clip_without_mutation() {
    let snapshot = fixture();
    let original = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let invalid = [
        Settings {
            rate: 0.0,
            ..snapshot.settings()
        },
        Settings {
            scale: 0.0,
            ..snapshot.settings()
        },
        Settings {
            offset: [f32::NAN, 0.0, 0.0],
            ..snapshot.settings()
        },
        Settings {
            rotation: [0.0; 4],
            ..snapshot.settings()
        },
        Settings {
            clip: u32::MAX,
            ..snapshot.settings()
        },
    ];
    for settings in invalid {
        assert!(snapshot.clone().with_settings(settings).is_err());
    }
    assert_eq!(
        snapshot.encode_embedded("mesh.scene.yaml").unwrap(),
        original
    );
}

#[test]
fn animated_placement_is_deterministic_and_validates_pose() {
    let snapshot = fixture();
    let entity = orr_ecs::Entity {
        index: 1,
        version: 1,
    };
    let position = [orr_fp::FP::ZERO; 3];
    let first = snapshot.placement(entity, position, 30).unwrap();
    let second = snapshot.placement(entity, position, 30).unwrap();
    assert_eq!(first.pose, second.pose);
    assert_eq!(first.instance, second.instance);
    assert_eq!(first.entity, entity);
}

#[test]
fn route_facing_uses_current_validated_path_and_authored_rotation() {
    use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
    use orr_remote::mesh_navigation::{authoring::create_mesh_scene, mesh_from_view, AgentSpec};

    let vertices = [
        [orr_fp::FP::ZERO, orr_fp::FP::ZERO, orr_fp::FP::ZERO],
        [orr_fp::FP::ZERO, orr_fp::FP::ZERO, orr_fp::FP::from_int(4)],
        [orr_fp::FP::from_int(4), orr_fp::FP::ZERO, orr_fp::FP::ZERO],
        [
            orr_fp::FP::from_int(4),
            orr_fp::FP::ZERO,
            orr_fp::FP::from_int(4),
        ],
    ];
    let graph =
        StaticMeshGraph::bake(&vertices, &[[0, 1, 3], [0, 3, 2]], AgentProfile::default()).unwrap();
    let spec = AgentSpec {
        start: [orr_fp::FP::ONE, orr_fp::FP::ZERO, orr_fp::FP::ONE],
        goal: [orr_fp::FP::from_int(3), orr_fp::FP::ZERO, orr_fp::FP::ONE],
        max_vertical: orr_fp::FP::ZERO,
        distance_per_tick: orr_fp::FP::from_ratio(1, 4),
    };
    let document = create_mesh_scene(&graph, spec).unwrap();
    let view = mesh_from_view(orr_bridge::FrameView::of(document.frame())).unwrap();
    let snapshot = fixture_for_revision(graph.revision());
    let fixed = snapshot.placement_for_view(&view, 12).unwrap();
    let mut settings = snapshot.settings();
    settings.facing = crate::mesh_facing::Facing::Route;
    settings.rotation = crate::mesh_rotation::from_degrees([0.0, 25.0, 0.0]).unwrap();
    let route_snapshot = snapshot.clone().with_settings(settings).unwrap();
    let route = route_snapshot.placement_for_view(&view, 12).unwrap();
    let repeated = route_snapshot.placement_for_view(&view, 12).unwrap();
    assert_ne!(route.instance, fixed.instance);
    assert_eq!(route.instance, repeated.instance);
    assert_eq!(route.pose, fixed.pose);
    assert_eq!(route.entity, view.agent.entity);

    let mismatched_snapshot = fixture_for_revision([0x23; 32]);
    let mut mismatched_settings = mismatched_snapshot.settings();
    mismatched_settings.facing = crate::mesh_facing::Facing::Route;
    let mismatched = mismatched_snapshot
        .with_settings(mismatched_settings)
        .unwrap();
    assert!(mismatched.placement_for_view(&view, 12).is_err());
}

#[test]
fn placement_uses_absolute_ticks_rate_and_loop_policy() {
    let snapshot = fixture();
    let entity = orr_ecs::Entity {
        index: 2,
        version: 1,
    };
    let position = [orr_fp::FP::ZERO; 3];
    let duration = snapshot.clips()[0].2;
    let once = snapshot
        .clone()
        .with_settings(Settings {
            looped: false,
            ..snapshot.settings()
        })
        .unwrap();
    let completed = once.placement(entity, position, u64::MAX).unwrap();
    let endpoint = snapshot.model.sample_clip(0, duration).unwrap();
    assert_eq!(completed.pose, endpoint);

    let looping = snapshot
        .clone()
        .with_settings(Settings {
            looped: true,
            ..snapshot.settings()
        })
        .unwrap();
    let first = looping.placement(entity, position, u64::MAX).unwrap();
    let repeated = looping.placement(entity, position, u64::MAX).unwrap();
    assert_eq!(first.pose, repeated.pose);

    let accelerated = snapshot
        .clone()
        .with_settings(Settings {
            rate: 2.0,
            looped: false,
            ..snapshot.settings()
        })
        .unwrap();
    assert_eq!(
        accelerated.placement(entity, position, 30).unwrap().pose,
        once.placement(entity, position, 60).unwrap().pose
    );
}

#[test]
fn decode_rejects_unknown_fields_and_cooked_digest_tampering() {
    let snapshot = fixture();
    let bytes = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["unexpected"] = serde_json::json!(true);
    let unknown = serde_json::to_vec(&value).unwrap();
    assert!(Snapshot::decode_embedded(&unknown, "mesh.scene.yaml", GRAPH_REVISION).is_err());

    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["cooked_sha256"] = serde_json::json!("0".repeat(64));
    let tampered = serde_json::to_vec(&value).unwrap();
    assert!(Snapshot::decode_embedded(&tampered, "mesh.scene.yaml", GRAPH_REVISION).is_err());
}
