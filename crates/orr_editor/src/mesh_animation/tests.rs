use super::{Settings, Snapshot};
use std::path::Path;

const GRAPH_REVISION: [u8; 32] = [0x5a; 32];

fn fixture() -> Snapshot {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", GRAPH_REVISION).unwrap()
}

#[test]
fn animated_package_is_source_independent_and_scene_bound() {
    let snapshot = fixture();
    assert!(!snapshot.clips().is_empty());
    let bytes = snapshot.encode_embedded("mesh.scene.yaml").unwrap();
    let reopened = Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", GRAPH_REVISION).unwrap();
    assert_eq!(snapshot.clips(), reopened.clips());
    assert_eq!(snapshot.settings(), Settings::default());
    assert!(Snapshot::decode_embedded(&bytes, "other.scene.yaml", GRAPH_REVISION).is_err());
    assert!(Snapshot::decode_embedded(&bytes, "mesh.scene.yaml", [0x13; 32]).is_err());
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
