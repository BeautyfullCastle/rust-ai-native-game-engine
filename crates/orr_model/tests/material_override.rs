#![allow(clippy::float_arithmetic)]
use orr_model::{
    Dependency, IDENTITY, Image, Material, MaterialOverride, ModelSource, Primitive, StaticModel,
    Vertex, Wrap,
};

fn model() -> StaticModel {
    let primitive = Primitive {
        id: "fixture.glb#node=0/mesh=0/primitive=0".into(),
        vertices: vec![Vertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.25, 0.75],
        }],
        indices: vec![0, 0, 0],
        material: 0,
        transform: IDENTITY,
    };
    let mut second = primitive.clone();
    second.id = "fixture.glb#node=0/mesh=0/primitive=1".into();
    second.material = 1;
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "fixture.glb".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "a".repeat(64),
        }],
        primitives: vec![primitive, second],
        materials: vec![
            Material {
                base_color: [0.2, 0.4, 0.6, 0.3],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Repeat,
                wrap_t: Wrap::Clamp
            };
            3
        ],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255, 64, 128, 255],
        }],
    })
    .unwrap()
}
#[test]
fn override_replaces_only_selected_rgb_and_preserves_immutable_model_bytes() {
    let model = model();
    let bytes = model.to_bytes().unwrap();
    let a = MaterialOverride {
        material_slot: 0,
        base_color_factor: [0.8, 0.1, 0.9],
    };
    let b = MaterialOverride {
        material_slot: 0,
        base_color_factor: [0.1, 0.9, 0.2],
    };
    a.validate_for(&model).unwrap();
    b.validate_for(&model).unwrap();
    let json = serde_json::to_string(&a).unwrap();
    assert!(json.starts_with('{'));
    assert_eq!(serde_json::from_str::<MaterialOverride>(&json).unwrap(), a);
    assert_eq!(
        serde_json::from_value::<MaterialOverride>(serde_json::to_value(a).unwrap()).unwrap(),
        a
    );
    assert_eq!(
        a.effective_base_color(0, model.source().materials[0].base_color),
        [0.8, 0.1, 0.9, 0.3]
    );
    assert_eq!(
        b.effective_base_color(0, model.source().materials[0].base_color),
        [0.1, 0.9, 0.2, 0.3]
    );
    assert_eq!(
        a.effective_base_color(1, model.source().materials[1].base_color),
        model.source().materials[1].base_color
    );
    assert_eq!(model.to_bytes().unwrap(), bytes);
}
#[test]
fn invalid_values_absent_and_unused_slots_reject_without_mutation() {
    let model = model();
    let bytes = model.to_bytes().unwrap();
    for material_slot in [2, 3, 256, u32::MAX] {
        assert!(
            MaterialOverride {
                material_slot,
                base_color_factor: [1.0; 3]
            }
            .validate_for(&model)
            .is_err()
        );
    }
    for value in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -0.0001,
        1.0001,
        f32::MAX,
    ] {
        for channel in 0..3 {
            let mut factor = [0.5; 3];
            factor[channel] = value;
            assert!(
                MaterialOverride {
                    material_slot: 0,
                    base_color_factor: factor
                }
                .validate_for(&model)
                .is_err()
            );
        }
    }
    for factor in [[0.0; 3], [1.0; 3], [-0.0, 0.0, 1.0]] {
        MaterialOverride {
            material_slot: 0,
            base_color_factor: factor,
        }
        .validate_for(&model)
        .unwrap();
    }
    assert_eq!(model.to_bytes().unwrap(), bytes);
}
#[test]
fn descriptor_has_exact_shape_and_duplicate_fields_are_rejected() {
    for text in [
        "null",
        "[]",
        "[0,[1,1,1]]",
        "true",
        "0",
        "\"override\"",
        "{}",
        r#"{"material_slot":0,"base_color_factor":[1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[1,1,1,1]}"#,
        r#"{"material_slot":-1,"base_color_factor":[1,1,1]}"#,
        r#"{"material_slot":0.5,"base_color_factor":[1,1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":["1",1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[null,1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[1e999,1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[NaN,1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[1,1,1],"extra":0}"#,
        r#"{"material_slot":0,"material_slot":1,"base_color_factor":[1,1,1]}"#,
        r#"{"material_slot":0,"base_color_factor":[1,1,1],"base_color_factor":[0,0,0]}"#,
    ] {
        assert!(
            serde_json::from_str::<MaterialOverride>(text).is_err(),
            "{text}"
        );
    }
    assert!(serde_json::from_value::<MaterialOverride>(serde_json::json!([0, [1, 1, 1]])).is_err());
}
