use super::*;
use serde_json::json;
fn model() -> StaticModel {
    orr_model::import::import_with_resolver(
        "model.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap()
}
fn slot(material: u32) -> Slot {
    Slot {
        material,
        metallic: 0.5,
        roughness: 0.25,
    }
}
#[test]
fn canonical_settings_validate_used_slots_and_build_full_legacy_default_table() {
    let model = model();
    let used = model.source().primitives[0].material;
    let settings = Settings::new(vec![slot(used)], &model).unwrap();
    assert_eq!(settings.slots(), &[slot(used)]);
    let table = settings.materials(&model).unwrap();
    assert_eq!(table.len(), model.source().materials.len());
    for (i, value) in table.iter().enumerate() {
        assert_eq!(
            *value,
            if i == used as usize {
                orr_render::MaterialShading::MetallicRoughness(
                    orr_render::ScalarMaterial::new(0.5, 0.25).unwrap(),
                )
            } else {
                orr_render::MaterialShading::LegacyDiffuse
            }
        );
    }
    assert!(Settings::new(vec![slot(u32::MAX)], &model).is_err());
    let mut source = model.source().clone();
    source.materials.push(source.materials[0].clone());
    let unused = source.materials.len() as u32 - 1;
    let model = StaticModel::new(source).unwrap();
    assert!(Settings::new(vec![slot(unused)], &model).is_err());
}
#[test]
fn ordering_and_signed_zero_have_one_identity_but_scalar_changes_do_not() {
    let a = Settings::canonical(vec![
        slot(2),
        Slot {
            material: 1,
            metallic: -0.0,
            roughness: -0.0,
        },
    ])
    .unwrap();
    let b = Settings::canonical(vec![
        Slot {
            material: 1,
            metallic: 0.0,
            roughness: 0.0,
        },
        slot(2),
    ])
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.identity(), b.identity());
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert_eq!(a.slots()[0].metallic.to_bits(), 0);
    let mut slots = b.slots().to_vec();
    slots[0].roughness = 0.1;
    assert_ne!(a.identity(), Settings::canonical(slots).unwrap().identity());
}
#[test]
fn strict_version_shapes_duplicates_and_budget_are_rejected() {
    for value in [
        json!(null),
        json!([]),
        json!({"version":1,"slots":[]}),
        json!({"version":2,"slots":[{"material":0,"metallic":0,"roughness":1}]}),
        json!({"version":1,"slots":[[0,0,1]]}),
        json!({"version":1,"slots":[{"material":0,"metallic":0,"roughness":1,"extra":0}]}),
        json!({"version":1,"slots":[{"material":0,"metallic":0,"roughness":1}],"extra":1}),
    ] {
        assert!(serde_json::from_value::<Settings>(value).is_err());
    }
    for raw in [
        r#"{"version":1,"version":1,"slots":[]}"#,
        r#"{"version":1,"slots":[{"material":0,"material":1,"metallic":0,"roughness":1}]}"#,
    ] {
        assert!(serde_json::from_str::<Settings>(raw).is_err());
    }
    assert!(Settings::canonical(vec![slot(0), slot(0)]).is_err());
    let slots: Vec<_> = (0..257).map(slot).collect();
    assert!(serde_json::from_value::<Settings>(json!({"version":1,"slots":slots})).is_err());
    assert!(Settings::canonical((0..257).map(slot).collect()).is_err());
}
#[test]
fn invalid_scalar_values_fail_without_clamping() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1] {
        for metallic in [true, false] {
            let mut s = slot(0);
            if metallic {
                s.metallic = value
            } else {
                s.roughness = value
            };
            assert!(Settings::canonical(vec![s]).is_err());
        }
    }
    assert!(Settings::canonical(vec![Slot {
        material: 0,
        metallic: 1.0,
        roughness: 0.0
    }])
    .is_ok());
}
