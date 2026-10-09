use orr_model_bindings::model_bindings::{
    AnimationDescriptor, Binding, Document, LocalTransform, MaterialOverride, ModelKind,
    PlaybackMode,
};

fn document() -> Document {
    Document {
        version: 3,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings: [(
            "e_00000001".into(),
            Binding {
                kind: ModelKind::Static,
                package: "fixture".into(),
                asset: "model.glb".into(),
                package_digest: "a".repeat(64),
                source_hash: "b".repeat(64),
                transform: LocalTransform::default(),
                animation: None,
                material_override: Some(MaterialOverride {
                    material_slot: 0,
                    base_color_factor: [0.2, 0.7, 0.8],
                }),
            },
        )]
        .into_iter()
        .collect(),
    }
}

#[test]
fn sidecar_override_requires_object_shape_and_preserves_valid_roundtrip() {
    let original = document();
    original.validate().unwrap();
    let bytes = serde_json::to_vec(&original).unwrap();
    assert_eq!(
        serde_json::from_slice::<Document>(&bytes).unwrap(),
        original
    );
    for invalid in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!([0, [0.2, 0.7, 0.8]]),
        serde_json::json!({}),
        serde_json::json!({"material_slot": 0}),
        serde_json::json!({"base_color_factor": [0.2, 0.7, 0.8]}),
        serde_json::json!(false),
        serde_json::json!(0),
        serde_json::json!("override"),
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        value["bindings"]["e_00000001"]["material_override"] = invalid;
        let encoded = serde_json::to_vec(&value).unwrap();
        assert!(serde_json::from_slice::<Document>(&encoded).is_err());
        assert!(serde_json::from_value::<Document>(value).is_err());
        assert_eq!(serde_json::to_vec(&original).unwrap(), bytes);
    }
}

#[test]
fn legacy_versions_still_roundtrip_without_override_and_reject_override_objects() {
    for version in [1, 2] {
        let mut original = document();
        original.version = version;
        original
            .bindings
            .get_mut("e_00000001")
            .unwrap()
            .material_override = None;
        let value = serde_json::to_value(&original).unwrap();
        assert!(
            value["bindings"]["e_00000001"]
                .get("material_override")
                .is_none()
        );
        let decoded: Document = serde_json::from_value(value).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, original);
        original
            .bindings
            .get_mut("e_00000001")
            .unwrap()
            .material_override = document().bindings["e_00000001"].material_override;
        let decoded: Document =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(decoded.validate().unwrap_err().contains("version-3"));
    }
}

#[test]
fn animated_override_is_rejected_regardless_of_animation_feature_unification() {
    let mut original = document();
    let binding = original.bindings.get_mut("e_00000001").unwrap();
    binding.kind = ModelKind::Animated;
    binding.animation = Some(AnimationDescriptor {
        clip_index: 0,
        playback: PlaybackMode::Once,
    });
    let decoded: Document =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert!(
        decoded
            .validate()
            .unwrap_err()
            .contains("animated model binding cannot declare")
    );
}
