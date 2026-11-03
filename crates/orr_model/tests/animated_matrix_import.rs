//! Matrix admission is import-only; independent source twins exercise consumers.
#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
use orr_model::{
    animation::{AnimatedModel, Trs},
    animation_import::import_with_resolver,
    Error,
};
use serde_json::{json, Value};
#[path = "common/animated_matrix_fixture.rs"]
mod fixture;
use fixture::{pack, PARENT_NODE};

fn import(bytes: &[u8]) -> Result<AnimatedModel, Error> {
    import_with_resolver("matrix.glb", bytes, |_| panic!("fixture is embedded"))
}
fn import_doc(doc: &Value, bin: &[u8]) -> Result<AnimatedModel, Error> {
    import(&pack(doc, bin))
}
fn with_matrix(matrix: [f32; 16]) -> (Value, Vec<u8>) {
    let (mut doc, bin) = fixture::matrix_parts();
    doc["nodes"][PARENT_NODE]["matrix"] = json!(matrix);
    // Numeric admission tests isolate this node from inherited transform bounds.
    doc["nodes"][PARENT_NODE]["children"] = json!([]);
    doc["scenes"][0]["nodes"] = json!([0, 3]);
    (doc, bin)
}
fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
fn near(a: f32, b: f32, tolerance: f32) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (limit {tolerance})");
}
fn same_geometry(a: &AnimatedModel, b: &AnimatedModel, tolerance: f32) {
    for clip in 0..2 {
        for time in [0.25, 0.75] {
            let pa = a.sample_clip(clip, time).unwrap();
            let pb = b.sample_clip(clip, time).unwrap();
            for (a, b) in pa
                .global()
                .iter()
                .flatten()
                .flatten()
                .zip(pb.global().iter().flatten().flatten())
            {
                near(*a, *b, tolerance);
            }
            let da = a.deform(&pa).unwrap();
            let db = b.deform(&pb).unwrap();
            assert_eq!(da.len(), db.len());
            for (a, b) in da.iter().zip(&db) {
                assert_eq!(a.indices, b.indices);
                assert_eq!(a.material, b.material);
                assert_eq!(a.vertices.len(), b.vertices.len());
                for (a, b) in a.vertices.iter().zip(&b.vertices) {
                    assert_eq!(a.uv, b.uv);
                    for (a, b) in a
                        .position
                        .into_iter()
                        .zip(b.position)
                        .chain(a.normal.into_iter().zip(b.normal))
                    {
                        near(a, b, tolerance);
                    }
                }
            }
        }
    }
}
fn reload(model: &AnimatedModel) -> AnimatedModel {
    let cooked = model.to_bytes().unwrap();
    let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
    assert_eq!(reloaded.source(), model.source());
    assert_eq!(reloaded.to_bytes().unwrap(), cooked);
    reloaded
}

#[test]
fn literal_matrix_trs_twins_match_hierarchy_two_poses_and_cooked_geometry() {
    let (matrix_bytes, trs_bytes) = fixture::twins();
    let matrix = import(&matrix_bytes).unwrap();
    let trs = import(&trs_bytes).unwrap();
    let (base_doc, bin) = fixture::parts();
    let baseline = import_doc(&base_doc, &bin).unwrap();
    assert_eq!(matrix.source().nodes, trs.source().nodes);
    assert_eq!(matrix.source().skins, trs.source().skins);
    assert_eq!(matrix.source().clips, trs.source().clips);
    assert_eq!(matrix.source().primitives, trs.source().primitives);
    assert_eq!(matrix.source().materials, trs.source().materials);
    assert_eq!(matrix.source().images, trs.source().images);
    assert_eq!(&matrix.source().nodes[..4], baseline.source().nodes);
    assert_eq!(matrix.source().nodes[PARENT_NODE].children, [0, 3]);
    assert_eq!(
        matrix.source().nodes[PARENT_NODE].rest.rotation,
        [0.0, 0.0, 1.0, 0.0]
    );
    assert_eq!(
        matrix.source().nodes[PARENT_NODE].rest.scale,
        [1.25, 0.75, 1.5]
    );
    same_geometry(&matrix, &trs, 0.0);
    same_geometry(&reload(&matrix), &reload(&trs), 0.0);
    let mut sampled = Vec::new();
    for time in [0.25, 0.75] {
        let base = baseline
            .deform(&baseline.sample_clip(0, time).unwrap())
            .unwrap();
        let actual = matrix
            .deform(&matrix.sample_clip(0, time).unwrap())
            .unwrap();
        for (a, b) in actual[0].vertices.iter().zip(&base[0].vertices) {
            // Independent literal affine action; no importer/runtime matrix builder.
            let expected = [
                0.4 - 1.25 * b.position[0],
                3.0 - 0.75 * b.position[1],
                0.2 + 1.5 * b.position[2],
            ];
            for (a, b) in a.position.into_iter().zip(expected) {
                near(a, b, 2e-6);
            }
        }
        sampled.push(actual[0].vertices.clone());
    }
    assert_ne!(sampled[0], sampled[1], "two genuine deformed poses");
}

#[test]
fn untargeted_skinned_mesh_matrix_does_not_double_apply_mesh_transform() {
    let (mut doc, bin) = fixture::parts();
    let baseline = import_doc(&doc, &bin).unwrap();
    doc["nodes"][3]
        .as_object_mut()
        .unwrap()
        .remove("translation");
    doc["nodes"][3]["matrix"] = json!(fixture::MATRIX);
    let matrix = import_doc(&doc, &bin).unwrap();
    for clip in 0..2 {
        for time in [0.25, 0.75] {
            let a = baseline
                .deform(&baseline.sample_clip(clip, time).unwrap())
                .unwrap();
            let b = matrix
                .deform(&matrix.sample_clip(clip, time).unwrap())
                .unwrap();
            assert_eq!(a, b);
        }
    }
    assert_eq!(matrix.source().nodes[3].rest.translation, [0.4, 3.0, 0.2]);
    let _ = reload(&matrix);
}

#[test]
fn untargeted_rigid_mesh_matrix_matches_literal_trs_and_vertex_oracle() {
    let (mut doc, bin) = fixture::parts();
    doc["nodes"][3].as_object_mut().unwrap().remove("skin");
    let attributes = doc["meshes"][0]["primitives"][0]["attributes"]
        .as_object_mut()
        .unwrap();
    attributes.remove("JOINTS_0");
    attributes.remove("WEIGHTS_0");
    doc["nodes"][3]
        .as_object_mut()
        .unwrap()
        .remove("translation");
    doc["nodes"][3]["matrix"] = json!(fixture::MATRIX);
    let matrix = import_doc(&doc, &bin).unwrap();
    doc["nodes"][3].as_object_mut().unwrap().remove("matrix");
    doc["nodes"][3]["translation"] = json!([0.4, 3.0, 0.2]);
    doc["nodes"][3]["rotation"] = json!([0.0, 0.0, 1.0, 0.0]);
    doc["nodes"][3]["scale"] = json!([1.25, 0.75, 1.5]);
    let trs = import_doc(&doc, &bin).unwrap();
    assert!(matrix.source().primitives[0].skin.is_none());
    same_geometry(&reload(&matrix), &reload(&trs), 0.0);
    let actual = matrix.deform(&matrix.rest_pose().unwrap()).unwrap();
    for (raw, result) in matrix.source().primitives[0]
        .vertices
        .iter()
        .zip(&actual[0].vertices)
    {
        let p = raw.vertex.position;
        assert_eq!(
            result.position,
            [0.4 - 1.25 * p[0], 3.0 - 0.75 * p[1], 0.2 + 1.5 * p[2]]
        );
    }
}

// Rodrigues' rotation formula is independent of the implementation's quaternion
// extraction and of Trs::matrix; these twins are authored separately in f64.
fn rotation_twins(axis: [f64; 3], angle: f64) -> (Value, Value, Vec<u8>) {
    let length = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
    let [x, y, z] = axis.map(|v| v / length);
    let c = angle.cos();
    let s = angle.sin();
    let d = 1.0 - c;
    let rows = [
        [c + x * x * d, x * y * d - z * s, x * z * d + y * s],
        [y * x * d + z * s, c + y * y * d, y * z * d - x * s],
        [z * x * d - y * s, z * y * d + x * s, c + z * z * d],
    ];
    let scales = [1.25, 0.75, 1.5];
    let mut matrix = identity();
    for (col, scale) in scales.iter().enumerate() {
        for (row, values) in rows.iter().enumerate() {
            matrix[4 * col + row] = (values[col] * scale) as f32;
        }
    }
    matrix[12..15].copy_from_slice(&[0.4, 3.0, 0.2]);
    let (mut matrix_doc, bin) = with_matrix(matrix);
    matrix_doc["nodes"][PARENT_NODE]["children"] = json!([0, 3]);
    matrix_doc["scenes"][0]["nodes"] = json!([PARENT_NODE]);
    let mut trs_doc = matrix_doc.clone();
    trs_doc["nodes"][PARENT_NODE]
        .as_object_mut()
        .unwrap()
        .remove("matrix");
    trs_doc["nodes"][PARENT_NODE]["translation"] = json!([0.4, 3.0, 0.2]);
    trs_doc["nodes"][PARENT_NODE]["scale"] = json!(scales);
    let half_sin = (angle / 2.0).sin();
    trs_doc["nodes"][PARENT_NODE]["rotation"] = json!([
        (x * half_sin) as f32,
        (y * half_sin) as f32,
        (z * half_sin) as f32,
        (angle / 2.0).cos() as f32
    ]);
    (matrix_doc, trs_doc, bin)
}

#[test]
fn exact_near_half_turns_and_branch_ties_match_independent_rotation_oracles() {
    for axis in [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        [1.0, -2.0, 3.0],
    ] {
        for angle in [
            0.0,
            0.63,
            std::f64::consts::TAU / 3.0,
            std::f64::consts::PI - 1e-5,
            std::f64::consts::PI,
            std::f64::consts::PI + 1e-5,
        ] {
            let (matrix_doc, trs_doc, bin) = rotation_twins(axis, angle);
            let matrix = import_doc(&matrix_doc, &bin).unwrap();
            let trs = import_doc(&trs_doc, &bin).unwrap();
            same_geometry(&matrix, &trs, 1e-5);
            same_geometry(&reload(&matrix), &trs, 1e-5);
            let q = matrix.source().nodes[PARENT_NODE].rest.rotation;
            assert!(
                [q[3], q[0], q[1], q[2]]
                    .into_iter()
                    .find(|v| *v != 0.0)
                    .unwrap()
                    > 0.0
            );
            for v in q {
                if v == 0.0 {
                    assert_eq!(v.to_bits(), 0);
                }
            }
        }
    }
}

#[test]
fn matrix_targets_reject_in_each_clip_path_and_inactive_nodes() {
    for clip in 0..2 {
        for path in ["translation", "rotation", "scale", "weights"] {
            for inactive in [false, true] {
                let (mut doc, bin) = fixture::matrix_parts();
                doc["animations"][clip]["channels"][0]["target"] =
                    json!({"node":PARENT_NODE,"path":path});
                if inactive {
                    doc["nodes"][PARENT_NODE]["children"] = json!([]);
                    doc["scenes"][0]["nodes"] = json!([0, 3]);
                }
                let error = import_doc(&doc, &bin).unwrap_err().to_string();
                assert!(
                    error.contains("animation-targeted matrix"),
                    "{clip} {path} {inactive}: {error}"
                );
            }
        }
    }
}

#[test]
fn whole_source_target_scan_precedes_matrix_conversion_and_checks_target_indices() {
    let (mut doc, bin) = fixture::matrix_parts();
    doc["nodes"][PARENT_NODE]["matrix"][4] = json!(0.2); // would fail decomposition
    doc["animations"][1]["channels"][0]["target"]["node"] = json!(9999);
    let error = import_doc(&doc, &bin).unwrap_err().to_string();
    assert!(
        error.contains("animation target node index out of range"),
        "{error}"
    );
    doc["animations"][1]["channels"][0]["target"]["node"] = json!(PARENT_NODE);
    let error = import_doc(&doc, &bin).unwrap_err().to_string();
    assert!(error.contains("animation-targeted matrix"), "{error}");
}

#[test]
fn inactive_untargeted_matrix_is_converted_but_invalid_inactive_matrix_rejects() {
    let (mut doc, bin) = fixture::matrix_parts();
    doc["nodes"][PARENT_NODE]["children"] = json!([]);
    doc["scenes"][0]["nodes"] = json!([0, 3]);
    let model = import_doc(&doc, &bin).unwrap();
    assert_eq!(
        model.source().nodes[PARENT_NODE].rest.scale,
        [1.25, 0.75, 1.5]
    );
    doc["nodes"][PARENT_NODE]["matrix"][4] = json!(0.1);
    assert!(import_doc(&doc, &bin).is_err());
}

#[test]
fn orthogonality_roundoff_boundary_is_scale_relative_and_translation_independent() {
    let inside = 8.0 * f32::EPSILON;
    let outside = f32::from_bits(inside.to_bits() + 1);
    for scale in [1.0e-6_f32, 1.0, 1.0e6] {
        for translation in [0.0, 1.0e6] {
            for (shear, accepted) in [(inside, true), (outside, false)] {
                let mut matrix = identity();
                matrix[0] = scale;
                matrix[4] = shear;
                matrix[12] = translation;
                let (doc, bin) = with_matrix(matrix);
                let result = import_doc(&doc, &bin);
                assert_eq!(
                    result.is_ok(),
                    accepted,
                    "scale={scale} translation={translation} shear={shear}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn positive_scales_keep_existing_bounds_and_inverse_transform_limits() {
    for scales in [
        [1.0e-6_f32, 1.0, 1.0],
        [1.0e6, 1.0, 1.0],
        [0.0002; 3],
        [0.01, 10.0, 3.0],
    ] {
        let mut matrix = identity();
        for (i, scale) in scales.into_iter().enumerate() {
            matrix[i * 5] = scale;
        }
        let (doc, bin) = with_matrix(matrix);
        let model = import_doc(&doc, &bin).unwrap();
        assert_eq!(model.source().nodes[PARENT_NODE].rest.scale, scales);
    }
    for scales in [
        [f32::from_bits(1.0e-6_f32.to_bits() - 1), 1.0, 1.0],
        [f32::from_bits(1.0e6_f32.to_bits() + 1), 1.0, 1.0],
        [1.0e-6; 3], // component bounds alone do not bypass determinant bound
        [0.0, 1.0, 1.0],
    ] {
        let mut matrix = identity();
        for (i, scale) in scales.into_iter().enumerate() {
            matrix[i * 5] = scale;
        }
        let (doc, bin) = with_matrix(matrix);
        assert!(import_doc(&doc, &bin).is_err(), "accepted {scales:?}");
    }
    // Every component fits the old limit, but its extracted column scale does not.
    let (doc, bin) = with_matrix([
        800000.0, 800000.0, 0.0, 0.0, -0.7, 0.7, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    assert!(import_doc(&doc, &bin)
        .unwrap_err()
        .to_string()
        .contains("scale out of range"));
}

#[test]
fn translation_bounds_are_exact_and_do_not_loosen_shear_or_affine_checks() {
    for value in [-1.0e6_f32, 1.0e6, -0.0] {
        let mut matrix = identity();
        matrix[12] = value;
        let (doc, bin) = with_matrix(matrix);
        let model = import_doc(&doc, &bin).unwrap();
        assert_eq!(
            model.source().nodes[PARENT_NODE].rest.translation[0].to_bits(),
            value.to_bits()
        );
    }
    for value in [
        -f32::from_bits(1.0e6_f32.to_bits() + 1),
        f32::from_bits(1.0e6_f32.to_bits() + 1),
    ] {
        let mut matrix = identity();
        matrix[12] = value;
        let (doc, bin) = with_matrix(matrix);
        assert!(import_doc(&doc, &bin).is_err());
    }
    for component in [3, 7, 11, 15] {
        let mut matrix = identity();
        matrix[12] = 1.0e6;
        matrix[component] = if component == 15 {
            f32::from_bits(1.0_f32.to_bits() + 1)
        } else {
            f32::from_bits(1)
        };
        let (doc, bin) = with_matrix(matrix);
        assert!(
            import_doc(&doc, &bin).is_err(),
            "accepted affine component {component}"
        );
    }
}

#[test]
fn matrix_plus_each_explicit_trs_property_rejects_even_identity_values() {
    for (property, value) in [
        ("translation", json!([0, 0, 0])),
        ("rotation", json!([0, 0, 0, 1])),
        ("scale", json!([1, 1, 1])),
    ] {
        let (mut doc, bin) = fixture::matrix_parts();
        doc["nodes"][PARENT_NODE][property] = value;
        assert!(import_doc(&doc, &bin)
            .unwrap_err()
            .to_string()
            .contains("matrix and TRS are mutually exclusive"));
    }
}

#[test]
fn reflection_and_other_invalid_matrices_reject_but_two_negative_axes_are_rotation() {
    for (label, bytes) in fixture::rejected_cases() {
        assert!(import(&bytes).is_err(), "accepted {label}");
    }
    for negative in [
        [true, false, false],
        [false, true, false],
        [false, false, true],
        [true; 3],
    ] {
        let mut matrix = identity();
        for (i, flip) in negative.into_iter().enumerate() {
            if flip {
                matrix[5 * i] = -1.0;
            }
        }
        let (doc, bin) = with_matrix(matrix);
        assert!(import_doc(&doc, &bin)
            .unwrap_err()
            .to_string()
            .contains("mirrored"));
    }
    for positive_axis in 0..3 {
        let mut matrix = identity();
        for i in 0..3 {
            if i != positive_axis {
                matrix[5 * i] = -1.0;
            }
        }
        let (doc, bin) = with_matrix(matrix);
        let model = import_doc(&doc, &bin).unwrap();
        let trs = model.source().nodes[PARENT_NODE].rest;
        assert_eq!(trs.scale, [1.0; 3]);
        let mut expected = [0.0; 4];
        expected[positive_axis] = 1.0;
        assert_eq!(trs.rotation, expected);
    }
}

#[test]
fn nonfinite_and_unrepresentable_matrix_json_never_imports() {
    let (mut doc, bin) = fixture::matrix_parts();
    for value in [
        json!(1.0e39),
        json!(-1.0e39),
        Value::Null,
        json!("NaN"),
        json!("Infinity"),
    ] {
        doc["nodes"][PARENT_NODE]["matrix"][0] = value;
        assert!(import_doc(&doc, &bin).is_err());
    }
}

#[test]
fn original_explicit_trs_source_and_cooked_geometry_preserve_authored_bits() {
    let (mut doc, bin) = fixture::parts();
    let baseline = import_doc(&doc, &bin).unwrap();
    assert_eq!(baseline.source().nodes.len(), 4);
    assert_eq!(baseline.source().nodes[0].rest.translation, [0.3, 0.2, 0.0]);
    assert_eq!(baseline.source().nodes[2].rest.translation, [0.0, 1.5, 0.0]);
    assert_eq!(baseline.source().nodes[3].rest.translation, [7.0, 0.0, 0.0]);
    let rest = baseline.deform(&baseline.rest_pose().unwrap()).unwrap();
    for (original, deformed) in baseline.source().primitives[0]
        .vertices
        .iter()
        .zip(&rest[0].vertices)
    {
        for (a, b) in original.vertex.position.into_iter().zip(deformed.position) {
            near(a, b, 1e-6);
        }
    }
    same_geometry(&baseline, &reload(&baseline), 0.0);
    doc["nodes"][3]["translation"] = json!([-0.0, 7.0, -0.0]);
    doc["nodes"][3]["rotation"] = json!([-0.0, 0.0, -0.0, -1.0]);
    doc["nodes"][3]["scale"] = json!([1.0, 1.0, 1.0]);
    let explicit = import_doc(&doc, &bin).unwrap();
    let cooked = reload(&explicit);
    let expected = Trs {
        translation: [-0.0, 7.0, -0.0],
        rotation: [-0.0, 0.0, -0.0, -1.0],
        scale: [1.0; 3],
    };
    for model in [&explicit, &cooked] {
        let actual = model.source().nodes[3].rest;
        assert_eq!(
            actual.translation.map(f32::to_bits),
            expected.translation.map(f32::to_bits)
        );
        assert_eq!(
            actual.rotation.map(f32::to_bits),
            expected.rotation.map(f32::to_bits)
        );
        assert_eq!(
            actual.scale.map(f32::to_bits),
            expected.scale.map(f32::to_bits)
        );
    }
    assert_eq!(baseline.source().primitives, explicit.source().primitives);
    for time in [0.25, 0.75] {
        assert_eq!(
            baseline
                .deform(&baseline.sample_clip(0, time).unwrap())
                .unwrap(),
            explicit
                .deform(&explicit.sample_clip(0, time).unwrap())
                .unwrap()
        );
    }
}
