use super::*;
use crate::room_wall_stamp::{vector3, ACTOR};
use orr_fp::fp;
use serde_json::json;

fn asymmetric() -> Template {
    Template::capture(
        [
            ("north", [3, 2], json!([0.25, 1, 4])),
            ("west", [-2, 1], json!([2, 2, 0.5])),
            ("corner", [1, -3], json!([0.5, 1.5, 3])),
        ]
        .into_iter()
        .map(|(name, [x, z], half)| {
            (
                name.into(),
                json!({
                    "RoomEscapeV1::Actor":{"kind":4,"ordinal":0},
                    "orr_physics3d::Body":{
                        "pos":[x,0.5,z],"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
                        "vel":[0,0,0],"omega":[0,0,0],"inv_mass":0,"inv_inertia":[0,0,0],
                        "linear_damping":0,"angular_damping":0,"kind":"static"
                    },
                    "orr_physics3d::Collider":{
                        "shape":{"kind":"box","half_extents":half},
                        "restitution":0,"friction":0.5,"layer":1,"mask":0
                    }
                }),
            )
        })
        .collect(),
    )
    .unwrap()
}

#[test]
fn all_orientations_rotate_asymmetric_group_and_swap_only_horizontal_extents() {
    let original = asymmetric();
    let bytes = original.encode().unwrap();
    let expected = [
        [[3, 2], [-2, 1], [1, -3]],
        [[-2, 3], [-1, -2], [3, 1]],
        [[-3, -2], [2, -1], [-1, 3]],
        [[2, -3], [1, 2], [-3, -1]],
    ];
    for turns in 0..4 {
        let placed = original
            .placed(turns, [FP::ZERO; 2], [FP::ZERO; 2])
            .unwrap();
        for (index, wall) in placed.walls.iter().enumerate() {
            let before = &original.walls[index];
            let [x, z] = expected[usize::from(turns)][index];
            assert_eq!(
                vector3(&wall.components[BODY]["pos"]).unwrap(),
                [FP::from_int(x), FP::HALF, FP::from_int(z)]
            );
            let mut expected_components = before.components.clone();
            expected_components[BODY]["pos"] = wall.components[BODY]["pos"].clone();
            if turns % 2 == 1 {
                expected_components[COLLIDER]["shape"]["half_extents"]
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 2);
            }
            assert_eq!(wall.components, expected_components);
            assert_eq!(wall.name, before.name);
            assert_eq!(wall.components[ACTOR], before.components[ACTOR]);
        }
    }
    assert_eq!(original.encode().unwrap(), bytes);
}

#[test]
fn nonorigin_negative_pivot_then_offset_is_exact() {
    let placed = asymmetric()
        .placed(1, [fp!(-2), fp!(1)], [fp!(3), fp!(-2)])
        .unwrap();
    let expected = [[0, 4], [1, -1], [5, 2]];
    for (wall, [x, z]) in placed.walls.iter().zip(expected) {
        assert_eq!(
            vector3(&wall.components[BODY]["pos"]).unwrap(),
            [FP::from_int(x), FP::HALF, FP::from_int(z)]
        );
    }
}

#[test]
fn four_turns_restore_original_bytes() {
    let original = asymmetric();
    let mut placed = original.clone();
    for _ in 0..4 {
        placed = placed.placed(1, [fp!(-2), fp!(1)], [FP::ZERO; 2]).unwrap();
    }
    assert_eq!(placed.encode().unwrap(), original.encode().unwrap());
}

#[test]
fn zero_turn_matches_translation_and_zero_offset_preserves_tokens() {
    let original = asymmetric();
    let placed = original
        .placed(0, [FP(i64::MIN), FP(i64::MAX)], [fp!(2), fp!(-3)])
        .unwrap();
    for (before, after) in original.walls.iter().zip(&placed.walls) {
        let center = vector3(&before.components[BODY]["pos"]).unwrap();
        assert_eq!(
            vector3(&after.components[BODY]["pos"]).unwrap(),
            [center[0] + fp!(2), center[1], center[2] - fp!(3)]
        );
    }
    let bytes = String::from_utf8(original.encode().unwrap())
        .unwrap()
        .replace("[3,0.5,2]", "[3.00000000000000001,0.5,2]");
    let original = Template::decode(bytes.as_bytes()).unwrap();
    assert_eq!(
        original
            .placed(0, [FP::ZERO; 2], [FP::ZERO; 2])
            .unwrap()
            .encode()
            .unwrap(),
        original.encode().unwrap()
    );
}

#[test]
fn raw_fixed_point_steps_roundtrip_without_drift() {
    let mut original = asymmetric();
    original.walls[0].components[BODY]["pos"][0] =
        serde_json::from_str("3.0000152587890625").unwrap();
    let mut placed = original.clone();
    for _ in 0..4 {
        placed = placed.placed(1, [FP(-3), FP(5)], [FP::ZERO; 2]).unwrap();
    }
    for (before, after) in original.walls.iter().zip(&placed.walls) {
        assert_eq!(
            vector3(&before.components[BODY]["pos"]).unwrap(),
            vector3(&after.components[BODY]["pos"]).unwrap()
        );
    }
}

#[test]
fn errors_reject_entire_placement_without_mutating_source() {
    let original = asymmetric();
    let bytes = original.encode().unwrap();
    for (turns, pivot, offset, expected) in [
        (4, [FP::ZERO; 2], [FP::ZERO; 2], "quarter-turns"),
        (255, [FP::ZERO; 2], [FP::ZERO; 2], "quarter-turns"),
        (0, [FP::ZERO; 2], [fp!(-15), FP::ZERO], "Room bounds"),
        (
            0,
            [FP::ZERO; 2],
            [FP(i64::MAX), FP::ZERO],
            "fixed-point bounds",
        ),
        (
            2,
            [FP(i64::MIN), FP(i64::MAX)],
            [FP::ZERO; 2],
            "fixed-point bounds",
        ),
    ] {
        assert!(original
            .placed(turns, pivot, offset)
            .unwrap_err()
            .contains(expected));
        assert_eq!(original.encode().unwrap(), bytes);
    }
}

#[test]
fn validates_source_before_transform_and_keeps_boundary_exact() {
    let mut invalid = asymmetric();
    invalid.walls[2].components[BODY]["rot"]["w"] = json!(0);
    let before = invalid.clone();
    assert!(invalid.placed(1, [FP::ZERO; 2], [FP::ZERO; 2]).is_err());
    assert_eq!(invalid, before);
    let original = asymmetric();
    let placed = original
        .placed(0, [FP::ZERO; 2], [fp!(13), FP::ZERO])
        .unwrap();
    assert_eq!(
        fixed(&placed.walls[0].components[BODY]["pos"][0]).unwrap(),
        fp!(16)
    );
    assert!(original
        .placed(0, [FP::ZERO; 2], [FP(fp!(13).raw() + 1), FP::ZERO])
        .is_err());
}
