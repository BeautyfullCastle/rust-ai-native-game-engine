use super::{identity, is_identity, tinted_model, validate};
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap};

fn model() -> StaticModel {
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "material-test".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: vec![Primitive {
            id: "material-test#node=0".into(),
            vertices: vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
            material: 0,
            transform: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, 2.0, 0.0, 0.0],
                [0.0, 0.0, 2.0, 0.0],
                [3.0, 4.0, 5.0, 1.0],
            ],
        }],
        materials: vec![
            Material {
                base_color: [0.8, 0.4, 0.2, 0.6],
                image: 0,
                linear_filter: true,
                wrap_s: Wrap::Repeat,
                wrap_t: Wrap::Clamp,
            },
            Material {
                base_color: [0.2, 0.8, 0.4, 1.0],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Mirror,
                wrap_t: Wrap::Repeat,
            },
        ],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![16, 64, 128, 255],
        }],
    })
    .unwrap()
}

#[test]
fn identity_tint_preserves_cooked_bytes() {
    let source = model();
    let source_bytes = source.to_bytes().unwrap();

    assert!(is_identity(&identity()));
    assert_eq!(
        tinted_model(&source, identity())
            .unwrap()
            .to_bytes()
            .unwrap(),
        source_bytes
    );
    assert_eq!(validate(identity()).unwrap(), identity());
}

#[test]
fn tint_multiplies_every_material_without_changing_other_source_fields() {
    let source = model();
    let original = source.source().clone();
    let tint = [0.5, 0.25, 1.0, 0.5];
    let tinted = tinted_model(&source, tint).unwrap();
    let mut expected = original.clone();
    for material in &mut expected.materials {
        for (component, multiplier) in material.base_color.iter_mut().zip(tint) {
            *component *= multiplier;
        }
    }

    assert_eq!(tinted.source(), &expected);
    assert_eq!(source.source(), &original);
    assert_eq!(
        tinted.source().materials[0].base_color,
        [0.4, 0.1, 0.2, 0.3]
    );
    assert_eq!(
        tinted.source().materials[1].base_color,
        [0.1, 0.2, 0.4, 0.5]
    );
    assert_eq!(tinted.source().images, original.images);
    assert_eq!(tinted.source().primitives, original.primitives);
    assert_eq!(tinted.source().dependencies, original.dependencies);
}

#[test]
fn tint_rejects_nonfinite_and_out_of_range_components() {
    for invalid_tint in [
        [f32::NAN, 1.0, 1.0, 1.0],
        [1.0, f32::INFINITY, 1.0, 1.0],
        [-0.01, 1.0, 1.0, 1.0],
        [1.0, 1.0, 1.01, 1.0],
    ] {
        assert!(validate(invalid_tint).is_err());
        assert!(tinted_model(&model(), invalid_tint).is_err());
    }
}

#[test]
fn derived_model_budget_is_rejected_before_material_clone() {
    let mut source = model().source().clone();
    source.images[0] = Image {
        width: 1536,
        height: 1536,
        rgba8: vec![255; 1536 * 1536 * 4],
    };
    let original = StaticModel::new(source).unwrap();
    let error = tinted_model(&original, [0.5, 0.5, 0.5, 1.0]).unwrap_err();
    assert!(error.contains("8 MiB"));
    assert_eq!(
        original.source().materials[0].base_color,
        [0.8, 0.4, 0.2, 0.6]
    );
}
