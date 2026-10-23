use super::{derived_model, identity, is_identity, validate, Settings};
use orr_model::{Error, StaticModel};

const MODEL_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");

fn fixture() -> StaticModel {
    orr_model::import::import_with_resolver("fixture.glb", MODEL_GLB, |_| {
        Err(Error::message("surface fixture must be self-contained"))
    })
    .unwrap()
}

fn used_material(model: &StaticModel) -> u32 {
    model.source().primitives[0].material
}

#[test]
fn defaults_and_identity_operations_preserve_cooked_bytes_including_signed_zero() {
    let model = fixture();
    let bytes = model.to_bytes().unwrap();
    assert!(is_identity(&identity()));
    let result = derived_model(&model, identity(), crate::mesh_material::identity()).unwrap();
    assert_eq!(result.to_bytes().unwrap(), bytes);
}

#[test]
fn uv_changes_only_the_selected_material_and_uses_scale_turn_offset_order() {
    let model = fixture();
    let selected = used_material(&model);
    let other = model
        .source()
        .primitives
        .iter()
        .find(|primitive| primitive.material != selected)
        .expect("fixture includes primitives with distinct materials");
    let other_id = other.id.clone();
    let other_uv = other.vertices[0].uv;
    let selected_primitive = model
        .source()
        .primitives
        .iter()
        .find(|primitive| primitive.material == selected)
        .unwrap();
    let original_uv = selected_primitive.vertices[0].uv;
    let settings = Settings {
        material: selected,
        uv_offset: [0.25, -0.5],
        uv_scale: [2.0, 3.0],
        uv_quarter_turns: 1,
        ..identity()
    };
    let result = derived_model(&model, settings, crate::mesh_material::identity()).unwrap();
    let selected_result = result
        .source()
        .primitives
        .iter()
        .find(|primitive| primitive.material == selected)
        .unwrap();
    let scaled = [original_uv[0] * 2.0, original_uv[1] * 3.0];
    assert_eq!(
        selected_result.vertices[0].uv,
        [-scaled[1] + 0.25, scaled[0] - 0.5]
    );
    let other_result = result
        .source()
        .primitives
        .iter()
        .find(|primitive| primitive.id == other_id)
        .unwrap();
    assert_eq!(other_result.vertices[0].uv, other_uv);
    assert_eq!(model.source().primitives[0].vertices[0].uv, original_uv);
}

#[test]
fn pivot_is_world_translation_subtraction_and_preserves_linear_transform() {
    let model = fixture();
    let before = model.source().primitives[0].transform;
    let settings = Settings {
        material: used_material(&model),
        pivot: [2.0, -3.0, 1.5],
        ..identity()
    };
    let result = derived_model(&model, settings, crate::mesh_material::identity()).unwrap();
    let after = result.source().primitives[0].transform;
    for row in 0..3 {
        assert_eq!(after[row], before[row]);
    }
    assert_eq!(
        after[3],
        [
            before[3][0] - 2.0,
            before[3][1] + 3.0,
            before[3][2] - 1.5,
            before[3][3]
        ]
    );
    assert_eq!(
        result.source().primitives[0].vertices,
        model.source().primitives[0].vertices
    );
}

#[test]
fn uv_pivot_and_tint_compose_in_one_derived_model_without_mutating_source() {
    let model = fixture();
    let original = model.to_bytes().unwrap();
    let settings = Settings {
        material: used_material(&model),
        uv_offset: [0.5, 0.25],
        uv_scale: [2.0, 0.5],
        uv_quarter_turns: 3,
        pivot: [1.0, 2.0, 3.0],
    };
    let result = derived_model(&model, settings, [0.5, 1.0, 0.25, 1.0]).unwrap();
    let material = &result.source().materials[0];
    let original_material = &model.source().materials[0];
    assert_eq!(
        material.base_color,
        [
            original_material.base_color[0] * 0.5,
            original_material.base_color[1],
            original_material.base_color[2] * 0.25,
            original_material.base_color[3],
        ]
    );
    assert_eq!(model.to_bytes().unwrap(), original);
}

#[test]
fn validation_rejects_unknown_fields_bad_ranges_and_unused_materials() {
    let model = fixture();
    let defaults = identity();
    assert!(validate(
        Settings {
            uv_scale: [0.0, 1.0],
            ..defaults
        },
        &model
    )
    .is_err());
    assert!(validate(
        Settings {
            uv_quarter_turns: 4,
            ..defaults
        },
        &model
    )
    .is_err());
    assert!(validate(
        Settings {
            material: u32::MAX,
            ..defaults
        },
        &model
    )
    .is_err());
    assert!(validate(
        Settings {
            pivot: [1_000_001.0, 0.0, 0.0],
            ..defaults
        },
        &model
    )
    .is_err());
    let valid_json = serde_json::to_string(&defaults).unwrap();
    let duplicate_json = valid_json.replacen("\"material\":0", "\"material\":0,\"material\":0", 1);
    assert!(serde_json::from_str::<Settings>(&duplicate_json).is_err());
    assert!(serde_json::from_str::<Settings>(&format!("[{valid_json}]")).is_err());
    assert!(serde_json::from_str::<Settings>(
        r#"{"material":0,"uv_offset":[0,0],"uv_scale":[1,1],"uv_quarter_turns":0,"pivot":[0,0,0],"unknown":1}"#
    )
    .is_err());
    assert!(serde_json::from_str::<Settings>(
        r#"{"material":0,"uv_offset":[0,0],"uv_scale":[1,1],"uv_quarter_turns":0}"#
    )
    .is_err());

    let mut source = model.source().clone();
    let unused_material = source.materials[0].clone();
    source.materials.push(unused_material);
    let with_unused = StaticModel::new(source).unwrap();
    assert!(validate(
        Settings {
            material: (with_unused.source().materials.len() - 1) as u32,
            ..defaults
        },
        &with_unused
    )
    .is_err());
}

#[test]
fn derived_uv_and_pivot_bounds_fail_closed() {
    let model = fixture();
    let settings = Settings {
        material: used_material(&model),
        uv_offset: [65_536.0, 0.0],
        ..identity()
    };
    assert!(derived_model(&model, settings, crate::mesh_material::identity()).is_err());
}

#[test]
fn oversized_derived_work_is_rejected_before_cloning() {
    let model = fixture();
    let mut source = model.source().clone();
    source.images[0].width = 1600;
    source.images[0].height = 1600;
    source.images[0].rgba8 = vec![0; 1600 * 1600 * 4];
    let oversized = StaticModel::new(source).unwrap();
    assert!(
        derived_model(&oversized, identity(), crate::mesh_material::identity())
            .unwrap_err()
            .contains("8 MiB")
    );
}

#[test]
fn oversized_model_metadata_is_included_in_derived_work_preflight() {
    let model = fixture();
    let mut source = model.source().clone();
    let mut dependencies = vec![source
        .dependencies
        .iter()
        .find(|dependency| dependency.uri == "$source")
        .unwrap()
        .clone()];
    for index in 1..256 {
        let prefix = format!("dependency-{index:03}/");
        let uri = format!("{prefix}{}", "a".repeat(1024 - prefix.len()));
        dependencies.push(orr_model::Dependency {
            uri,
            sha256: "0".repeat(64),
        });
    }
    source.dependencies = dependencies;
    source.images[0].width = 2048;
    source.images[0].height = 1000;
    source.images[0].rgba8 = vec![0; 2048 * 1000 * 4];
    let previous_estimate = source
        .images
        .iter()
        .map(|image| image.rgba8.len())
        .sum::<usize>()
        + source
            .primitives
            .iter()
            .map(|primitive| {
                primitive.vertices.len() * std::mem::size_of::<orr_model::Vertex>()
                    + primitive.indices.len() * std::mem::size_of::<u32>()
                    + primitive.id.len()
            })
            .sum::<usize>();
    assert!(previous_estimate <= 8 * 1024 * 1024);
    let oversized = StaticModel::new(source).unwrap();
    assert!(
        crate::mesh_material::tinted_model(&oversized, crate::mesh_material::identity())
            .unwrap_err()
            .contains("8 MiB")
    );
    assert!(derived_model(
        &oversized,
        Settings {
            material: used_material(&oversized),
            ..identity()
        },
        crate::mesh_material::identity()
    )
    .unwrap_err()
    .contains("8 MiB"));
}
