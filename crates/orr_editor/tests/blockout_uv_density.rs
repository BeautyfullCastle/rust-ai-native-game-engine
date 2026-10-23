//! Recipe-space density is measured against independently baked physical edges.
#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic)]

use orr_editor::blockout::{Blockout, Face, FaceUv, Recipe};
use orr_editor::blockout_uv_density::face_scale;

fn asymmetric_recipe() -> Recipe {
    let mut recipe = Recipe::default();
    recipe.bounds.min = [-1.0, -2.0, -3.0];
    recipe.bounds.max = [1.0, 1.0, 2.0];
    for (index, settings) in recipe.faces.iter_mut().enumerate() {
        settings.material = (index % 2) as u8;
        settings.uv = FaceUv {
            offset: [0.125, -0.25],
            scale: [0.5, 1.5],
            quarter_turn: (index % 4) as u8,
        };
    }
    recipe
}

fn length<const N: usize>(a: [f32; N], b: [f32; N]) -> f32 {
    a.into_iter()
        .zip(b)
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}

#[test]
fn all_six_faces_and_four_turns_have_requested_baked_edge_density() {
    // X/Y/Z extents differ, so swapping U/V for 90/270 degrees fails this oracle.
    let expected_scales = [
        [1.5, 2.5],
        [1.5, 2.5],
        [1.0, 2.5],
        [1.0, 2.5],
        [1.0, 1.5],
        [1.0, 1.5],
    ];
    let units_per_repeat = 2.0;
    for face in Face::ALL {
        for turn in 0..4 {
            let mut source = asymmetric_recipe();
            source.faces[face.index()].uv.quarter_turn = turn;
            let before = source.clone();
            let source_bytes = source.to_bytes().unwrap();
            let scale = face_scale(&source, face, units_per_repeat).unwrap();
            assert_eq!(scale, expected_scales[face.index()]);
            assert_eq!(source, before);
            assert_eq!(source.to_bytes().unwrap(), source_bytes);

            let mut candidate = source.clone();
            candidate.faces[face.index()].uv.scale = scale;
            let model = candidate
                .bake_model(&candidate.to_bytes().unwrap())
                .unwrap();
            let primitive = &model.source().primitives[face.index()];
            for [a, b] in [[0, 1], [1, 2], [2, 3], [3, 0]] {
                let [a, b] = [&primitive.vertices[a], &primitive.vertices[b]];
                let density = length(a.uv, b.uv) / length(a.position, b.position);
                assert!(
                    (density - 1.0 / units_per_repeat).abs() < 1.0e-6,
                    "{} turn {turn}: got {density}",
                    face.name(),
                );
            }
            for other in Face::ALL {
                if other != face {
                    assert_eq!(candidate.faces[other.index()], source.faces[other.index()]);
                }
            }
            assert_eq!(candidate.bounds, source.bounds);
            assert_eq!(candidate.format, source.format);
            assert_eq!(candidate.version, source.version);
            assert_eq!(
                candidate.faces[face.index()].material,
                source.faces[face.index()].material
            );
            assert_eq!(
                candidate.faces[face.index()].uv.offset,
                source.faces[face.index()].uv.offset
            );
            assert_eq!(candidate.faces[face.index()].uv.quarter_turn, turn);
        }
    }
}

#[test]
fn resizing_recalculates_scales_from_current_recipe_dimensions() {
    let mut recipe = asymmetric_recipe();
    let original = face_scale(&recipe, Face::PosX, 2.0).unwrap();
    assert_eq!(original, [1.5, 2.5]);
    recipe.bounds.max[1] += 1.0;
    recipe.bounds.min[2] -= 3.0;
    let before = recipe.clone();
    assert_eq!(face_scale(&recipe, Face::PosX, 2.0).unwrap(), [2.0, 4.0]);
    assert_eq!(face_scale(&recipe, Face::NegX, 2.0).unwrap(), [2.0, 4.0]);
    assert_eq!(face_scale(&recipe, Face::PosY, 2.0).unwrap(), [1.0, 4.0]);
    assert_eq!(face_scale(&recipe, Face::NegZ, 2.0).unwrap(), [1.0, 2.0]);
    assert_eq!(recipe, before);
}

#[test]
fn nonpositive_nonfinite_overflow_and_out_of_range_density_are_rejected_without_clamping() {
    let recipe = Recipe::default();
    let before = recipe.to_bytes().unwrap();
    for units in [
        0.0,
        -0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(1), // Positive input whose division overflows.
        f32::MIN_POSITIVE,
        f32::MAX,
        0.01,
        10_000.0,
    ] {
        for face in Face::ALL {
            assert!(
                face_scale(&recipe, face, units).is_err(),
                "accepted {units}"
            );
            assert_eq!(recipe.to_bytes().unwrap(), before);
        }
    }
    assert_eq!(
        face_scale(&recipe, Face::PosX, 1_000.0).unwrap(),
        [0.001; 2]
    );
    assert_eq!(face_scale(&recipe, Face::PosX, 0.0625).unwrap(), [16.0; 2]);
    assert!(face_scale(&recipe, Face::PosX, 1_001.0).is_err());
    assert!(face_scale(&recipe, Face::PosX, 0.0624).is_err());

    // One legal axis must not hide a rejected scale on the other axis.
    let recipe = asymmetric_recipe();
    assert!(face_scale(&recipe, Face::PosX, 0.2).is_err());
    assert!(face_scale(&recipe, Face::PosY, 3_000.0).is_err());
}

#[test]
fn invalid_source_recipes_are_rejected_before_proposing_scales() {
    let mut invalid = Vec::new();
    let mut recipe = asymmetric_recipe();
    recipe.version = 2;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.format = "another-format".into();
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.bounds.max[0] = f32::INFINITY;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.bounds.max[0] = recipe.bounds.min[0];
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.bounds.min[1] = -101.0;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.faces[Face::NegZ.index()].material = 2;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.faces[Face::NegZ.index()].uv.scale[0] = 0.0;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.faces[Face::NegZ.index()].uv.offset[1] = 17.0;
    invalid.push(recipe);
    let mut recipe = asymmetric_recipe();
    recipe.faces[Face::NegZ.index()].uv.quarter_turn = 4;
    invalid.push(recipe);
    for recipe in invalid {
        let before = recipe.clone();
        assert!(face_scale(&recipe, Face::PosX, 2.0).is_err());
        assert_eq!(recipe, before);
    }
}

#[test]
fn computing_or_rejecting_density_preserves_source_files_history_and_redo() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = Blockout::create(directory.path(), "box.blockout.json").unwrap();
    document.save().unwrap();
    document.move_face(Face::PosY, 1.0).unwrap();
    document.move_face(Face::PosZ, 2.0).unwrap();
    document.undo();
    let before = document.recipe().clone();
    let history = document.history_len();
    let dirty = document.dirty();
    let bytes = std::fs::read(document.path()).unwrap();
    assert_eq!(
        face_scale(document.recipe(), Face::PosX, 1.0).unwrap(),
        [2.0, 1.0]
    );
    assert!(face_scale(document.recipe(), Face::PosX, f32::from_bits(1)).is_err());
    assert_eq!(document.recipe(), &before);
    assert_eq!(document.history_len(), history);
    assert_eq!(document.dirty(), dirty);
    assert_eq!(std::fs::read(document.path()).unwrap(), bytes);
    document.redo();
    assert_eq!(
        face_scale(document.recipe(), Face::PosX, 1.0).unwrap(),
        [2.0, 3.0]
    );
}
