use super::{apply, validate, Settings, MAX_SETTINGS};
use orr_model::{StaticModel, Wrap};

fn model() -> StaticModel {
    orr_model::import::import_with_resolver(
        "fixture.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| {
            Err(orr_model::Error::message(
                "material-slot fixture must be self-contained",
            ))
        },
    )
    .unwrap()
}

fn used_material(model: &StaticModel) -> u32 {
    model.source().primitives[0].material
}

fn settings_for(material: u32, model: &StaticModel) -> Settings {
    Settings::identity(material, &model.source().materials[material as usize])
}

#[test]
fn empty_and_identity_overrides_preserve_cooked_bytes() {
    let source = model();
    let original_bytes = source.to_bytes().unwrap();
    let identity = settings_for(used_material(&source), &source);

    assert_eq!(
        apply(&source, &[]).unwrap().to_bytes().unwrap(),
        original_bytes
    );
    assert_eq!(
        apply(&source, &[identity]).unwrap().to_bytes().unwrap(),
        original_bytes
    );
}

#[test]
fn apply_changes_only_the_selected_used_material_slot() {
    let source = model();
    let original_source = source.source().clone();
    let material_index = used_material(&source);
    let mut selected = settings_for(material_index, &source);
    selected.tint = [0.5, 0.25, 1.0, 0.5];
    selected.linear_filter = !selected.linear_filter;
    selected.wrap_s = Wrap::Mirror;
    selected.wrap_t = Wrap::Repeat;

    let changed = apply(&source, &[selected]).unwrap();
    let mut expected = original_source.clone();
    let selected_material = &mut expected.materials[material_index as usize];
    for (component, multiplier) in selected_material.base_color.iter_mut().zip(selected.tint) {
        *component *= multiplier;
    }
    selected_material.linear_filter = selected.linear_filter;
    selected_material.wrap_s = selected.wrap_s;
    selected_material.wrap_t = selected.wrap_t;
    assert_eq!(changed.source(), &expected);
    assert_eq!(source.source(), &original_source);
}

#[test]
fn validate_rejects_duplicates_unused_indices_and_invalid_tints() {
    let source = model();
    let material_index = used_material(&source);
    let defaults = settings_for(material_index, &source);

    assert!(validate(&[defaults, defaults], &source).is_err());
    let mut unused_source = source.source().clone();
    let unused_material = unused_source.materials[0].clone();
    unused_source.materials.push(unused_material);
    let with_unused = StaticModel::new(unused_source).unwrap();
    let unused_index = (with_unused.source().materials.len() - 1) as u32;
    assert!(validate(&[settings_for(unused_index, &with_unused)], &with_unused).is_err());
    let mut outside_model = defaults;
    outside_model.material = u32::MAX;
    assert!(validate(&[outside_model], &source).is_err());
    assert!(apply(&source, &[outside_model]).is_err());
    for tint in [
        [f32::NAN, 1.0, 1.0, 1.0],
        [f32::INFINITY, 1.0, 1.0, 1.0],
        [-0.01, 1.0, 1.0, 1.0],
        [1.0, 1.0, 1.01, 1.0],
    ] {
        let mut invalid = defaults;
        invalid.tint = tint;
        assert!(validate(&[invalid], &source).is_err());
        assert!(apply(&source, &[invalid]).is_err());
    }
}

#[test]
fn settings_are_strict_objects_and_vector_limit_is_bounded() {
    let source = model();
    let valid = settings_for(used_material(&source), &source);
    let serialized = serde_json::to_string(&valid).unwrap();
    let material_field = format!("\"material\":{}", valid.material);
    let duplicate = serialized.replacen(
        &material_field,
        &format!("{material_field},{material_field}"),
        1,
    );
    let unknown = format!("{},\"extra\":1}}", serialized.trim_end_matches('}'));

    assert!(serde_json::from_str::<Settings>(&duplicate).is_err());
    assert!(serde_json::from_str::<Settings>("null").is_err());
    assert!(serde_json::from_str::<Settings>(&unknown).is_err());

    let too_many = vec![valid; MAX_SETTINGS + 1];
    assert!(validate(&too_many, &source).is_err());
    assert!(validate(&[], &source).is_ok());
}

#[test]
fn derived_work_budget_is_checked_before_cloning_material_slots() {
    let mut source = model().source().clone();
    source.images[0].width = 1536;
    source.images[0].height = 1536;
    source.images[0].rgba8 = vec![0; 1536 * 1536 * 4];
    let oversized = StaticModel::new(source).unwrap();

    assert!(apply(&oversized, &[]).unwrap_err().contains("8 MiB"));
}
