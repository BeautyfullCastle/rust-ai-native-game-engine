mod common;
use common::*;
use orr_fp::FP;
use orr_terrain::Terrain;
use orr_terrain_import::{import_raw16, Endian, MAX_RAW_BYTES};
#[test]
fn asymmetric_little_big_endian_floor_and_endpoints_match_integer_oracle() {
    for endian in [Endian::Little, Endian::Big] {
        let mut config = recipe();
        config.endian = endian;
        let t = import_raw16(&raw(&SAMPLES, endian), &config).unwrap();
        let expected = [-65536, -65535, -32769, -32768, -2, 0];
        assert_eq!(
            t.heights().iter().map(|h| h.raw()).collect::<Vec<_>>(),
            expected
        );
        for (i, h) in expected.into_iter().enumerate() {
            assert_eq!(
                t.vertex_position(i as u32).unwrap(),
                [
                    FP::from_int(-1 + (i % 3) as i32),
                    FP::from_raw(h),
                    FP::from_int(-1 + (i / 3) as i32)
                ]
            );
        }
        assert_eq!(t.triangles(), [[0, 3, 4], [0, 4, 1], [1, 4, 5], [1, 5, 2]]);
        assert_eq!(t.holes(), [false, false]);
    }
}
#[test]
fn rectangular_two_by_three_and_flip_z_do_not_transpose_x() {
    let mut config = recipe();
    config.width = 2;
    config.depth = 3;
    let plain = import_raw16(&raw(&SAMPLES, Endian::Little), &config).unwrap();
    config.flip_z = true;
    let flipped = import_raw16(&raw(&SAMPLES, Endian::Little), &config).unwrap();
    assert_eq!(
        flipped.heights(),
        [
            plain.heights()[4],
            plain.heights()[5],
            plain.heights()[2],
            plain.heights()[3],
            plain.heights()[0],
            plain.heights()[1]
        ]
    );
    assert_eq!(flipped.vertex_position(5).unwrap()[2], FP::ONE);
    assert_eq!(
        flipped.triangles(),
        [[0, 2, 3], [0, 3, 1], [2, 4, 5], [2, 5, 3]]
    );
}
#[test]
fn zero_scale_and_negative_offset_are_exact() {
    let mut config = recipe();
    config.height_scale = FP::ZERO;
    config.height_offset = FP::from_int(-7);
    let t = import_raw16(&raw(&SAMPLES, Endian::Little), &config).unwrap();
    assert_eq!(t.heights(), [FP::from_int(-7); 6]);
}
#[test]
fn conversion_floors_fractional_raw_product_after_decimal_rounding_once() {
    let mut config = recipe();
    config.height_scale = FP::parse("0.0000457763671875").unwrap();
    config.height_offset = FP::parse("-0.00000762939453125").unwrap();
    assert_eq!(config.height_scale.raw(), 3);
    assert_eq!(config.height_offset.raw(), -1);
    assert_eq!(
        import_raw16(&raw(&SAMPLES, Endian::Little), &config)
            .unwrap()
            .heights()
            .iter()
            .map(|h| h.raw())
            .collect::<Vec<_>>(),
        [-1, -1, 0, 0, 1, 2]
    );
}
#[test]
fn canonical_repeated_imports_and_identity_revision_are_deterministic() {
    let first = terrain();
    let second = terrain();
    let bytes = first.cook();
    assert_eq!(bytes, second.cook());
    assert_eq!(first.revision(), second.revision());
    assert_eq!(Terrain::load(&bytes).unwrap(), first);
    let mut config = recipe();
    config.asset_id = "different".into();
    let other = import_raw16(&raw(&SAMPLES, Endian::Little), &config).unwrap();
    assert_ne!(first.revision(), other.revision());
    assert_eq!(first.heights(), other.heights());
}
#[test]
fn maximum_dimensions_and_exact_byte_budget_are_admitted() {
    let mut config = recipe();
    config.width = 129;
    config.depth = 129;
    let t = import_raw16(&vec![0; MAX_RAW_BYTES], &config).unwrap();
    assert_eq!(t.heights().len(), 16641);
    assert_eq!(t.triangles().len(), 32768);
    assert_eq!(config.validate().unwrap(), MAX_RAW_BYTES);
}
#[test]
fn dimension_identity_and_scale_bounds_fail_before_allocation() {
    for side in [0, 1, 130, u32::MAX] {
        for axis in [0, 1] {
            let mut r = recipe();
            if axis == 0 {
                r.width = side;
            } else {
                r.depth = side;
            }
            assert!(r.validate().is_err());
        }
    }
    for id in [
        "",
        "/absolute",
        "a/../b",
        "a//b",
        "snow☃",
        "a\\b",
        ".",
        "..",
    ] {
        let mut r = recipe();
        r.asset_id = id.into();
        assert!(r.validate().is_err(), "{id}");
    }
    let mut r = recipe();
    r.asset_id = "a".repeat(257);
    assert!(r.validate().is_err());
    let mut r = recipe();
    r.height_scale = FP::from_raw(-1);
    assert!(r.validate().is_err());
    r.height_scale = FP::from_raw(i64::MAX);
    r.height_offset = FP::ONE;
    assert!(r.validate().is_err());
}
#[test]
fn spacing_coordinate_overflow_and_display_precision_are_rejected() {
    for spacing in [FP::ZERO, FP::from_raw(-1), FP::from_raw(i64::MAX)] {
        let mut r = recipe();
        r.spacing = spacing;
        assert!(r.validate().is_err());
    }
    for origin in [
        FP::from_int(1_000_001),
        FP::from_raw(i64::MIN),
        FP::from_raw(i64::MAX),
        FP::from_raw(16_777_217),
    ] {
        let mut r = recipe();
        r.origin[0] = origin;
        assert!(r.validate().is_err());
    }
    let mut r = recipe();
    r.height_offset = FP::from_raw(16_777_217);
    r.height_scale = FP::ZERO;
    assert!(import_raw16(&raw(&SAMPLES, Endian::Little), &r).is_err());
}
#[test]
fn nonexact_spacing_is_rejected_even_with_exact_displayable_vertices() {
    let mut r = recipe();
    r.width = 2;
    r.depth = 2;
    r.spacing = FP::from_raw(16_777_217);
    r.origin = [FP::from_raw(-1); 2];
    let t = Terrain::new(
        "witness".into(),
        2,
        2,
        r.origin,
        r.spacing,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    assert!(orr_terrain_view::to_static_model(&t, &[]).is_ok());
    assert!(r.validate().unwrap_err().0.contains("spacing"));
}
#[test]
fn truncated_odd_padded_and_oversized_input_lengths_fail() {
    for len in [0, 1, 10, 11, 13, 14, MAX_RAW_BYTES + 1] {
        assert!(import_raw16(&vec![0; len], &recipe()).is_err(), "{len}");
    }
}
#[test]
fn canonical_surface_uses_triangle_plane_not_bilinear_interpolation() {
    let mut r = recipe();
    r.width = 2;
    r.depth = 2;
    r.origin = [FP::ZERO; 2];
    r.height_offset = FP::ZERO;
    let t = import_raw16(&raw(&[0, 0, 0, 65535], Endian::Little), &r).unwrap();
    let half = FP::from_raw(32768);
    let quarter = FP::from_raw(16384);
    assert_eq!(t.sample(half, half), Some(half)); // bilinear would be 1/4
    assert_eq!(t.sample(half, quarter), Some(quarter));
    assert_eq!(t.surface(half, quarter).unwrap().height, quarter);
    assert_eq!(t.sample(FP::from_raw(-1), FP::ZERO), None);
    let model = orr_terrain_view::to_static_model(&t, &[]).unwrap().unwrap();
    assert_eq!(model.source().primitives[0].indices, [0, 1, 2, 3, 4, 5]);
    let positions: Vec<_> = model.source().primitives[0]
        .vertices
        .iter()
        .map(|v| v.position)
        .collect();
    assert_eq!(
        positions,
        vec![
            [0., 0., 0.],
            [0., 0., 1.],
            [1., 1., 1.],
            [0., 0., 0.],
            [1., 1., 1.],
            [1., 0., 0.]
        ]
    );
}
