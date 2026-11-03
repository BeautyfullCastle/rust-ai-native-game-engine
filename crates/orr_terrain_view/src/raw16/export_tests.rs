use super::*;

pub(super) fn recipe() -> Recipe {
    Recipe {
        asset_id: "export/test".into(),
        width: 2,
        depth: 2,
        endian: Endian::Little,
        height_offset: FP::from_raw(-100),
        height_scale: FP::from_raw(65535),
        origin: [FP::ZERO; 2],
        spacing: FP::ONE,
        flip_z: false,
    }
}

pub(super) fn terrain(recipe: &Recipe, raw: [i64; 4]) -> Terrain {
    Terrain::new(
        recipe.asset_id.clone(),
        recipe.width,
        recipe.depth,
        recipe.origin,
        recipe.spacing,
        raw.into_iter().map(FP::from_raw).collect(),
        vec![false],
    )
    .unwrap()
}

#[test]
fn canonical_smallest_sample_and_exact_height_roundtrip() {
    let mut r = recipe();
    r.height_scale = FP::from_raw(2);
    let t = terrain(&r, [-100, -99, -98, -100]);
    let bytes = export_raw16(&t, &r).unwrap();
    assert_eq!(bytes, [0, 0, 0, 128, 255, 255, 0, 0]);
    assert_eq!(import_raw16(&bytes, &r).unwrap().heights(), t.heights());
}

#[test]
fn gaps_bounds_and_zero_scale_are_not_quantized() {
    let mut r = recipe();
    r.height_scale = FP::from_raw(131070);
    assert!(export_raw16(&terrain(&r, [-99; 4]), &r).is_err());
    assert!(export_raw16(&terrain(&r, [-101; 4]), &r).is_err());
    assert!(export_raw16(&terrain(&r, [130971; 4]), &r).is_err());
    r.height_scale = FP::ZERO;
    assert_eq!(export_raw16(&terrain(&r, [-100; 4]), &r).unwrap(), [0; 8]);
    assert!(export_raw16(&terrain(&r, [-99; 4]), &r).is_err());
}

#[test]
fn endian_and_flip_are_inverse_of_import() {
    for endian in [Endian::Little, Endian::Big] {
        for flip_z in [false, true] {
            let mut r = recipe();
            r.endian = endian;
            r.flip_z = flip_z;
            let original = [0x01, 0x02, 0xff, 0xff, 0x00, 0x00, 0x12, 0x34];
            let t = import_raw16(&original, &r).unwrap();
            assert_eq!(export_raw16(&t, &r).unwrap(), original);
        }
    }
}

#[test]
fn all_metadata_must_match_and_recipe_must_validate() {
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    for field in 0..7 {
        let mut changed = r.clone();
        match field {
            0 => changed.asset_id = "different".into(),
            1 => changed.width = 3,
            2 => changed.depth = 3,
            3 => changed.origin[0] = FP::ONE,
            4 => changed.origin[1] = FP::ONE,
            5 => changed.spacing = FP::from_raw(131072),
            _ => changed.height_scale = FP::from_raw(-1),
        }
        assert!(export_raw16(&t, &changed).is_err(), "field {field}");
    }
}

#[test]
fn holes_are_not_encoded_and_do_not_change_height_export() {
    let r = recipe();
    let t = Terrain::new(
        r.asset_id.clone(),
        2,
        2,
        r.origin,
        r.spacing,
        vec![FP::from_raw(-100); 4],
        vec![true],
    )
    .unwrap();
    let imported = import_raw16(&export_raw16(&t, &r).unwrap(), &r).unwrap();
    assert_eq!(imported.heights(), t.heights());
    assert_ne!(imported, t);
}

#[test]
fn extreme_arithmetic_and_display_admission_fail_without_overflow() {
    let mut r = recipe();
    r.height_offset = FP::from_raw(i64::MIN);
    r.height_scale = FP::from_raw(i64::MAX);
    let t = terrain(&r, [-1; 4]);
    assert_eq!(export_raw16(&t, &r).unwrap(), [255; 8]);
    r.height_offset = FP::from_raw(i64::MAX);
    assert!(export_raw16(&t, &r).is_err());
    let r = recipe();
    assert!(export_raw16(&terrain(&r, [65_536_000_001; 4]), &r).is_err());
}
