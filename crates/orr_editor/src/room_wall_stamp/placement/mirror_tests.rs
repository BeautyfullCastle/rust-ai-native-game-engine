//! Mirror-only contracts. Keep this sibling of `placement` so the inherited
//! rotation gate's exact `placement::tests` inventory remains unchanged.
use super::{fixed, vector3, Template, ACTOR, BODY, COLLIDER, MAX_BYTES};
use crate::{Editor, HostSpec};
use orr_fp::{fp, FP};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const PIVOT: [FP; 2] = [fp!(-2), fp!(1)];
const OFFSET: [FP; 2] = [fp!(3), fp!(-2)];

fn asymmetric() -> Template {
    Template::capture(
        [
            ("west elbow", json!([-3, 0.5, 2]), json!([0.25, 1, 2])),
            ("  high wall  ", json!([2, 1.25, -4]), json!([1.5, 2, 0.5])),
            (
                "low corner",
                json!([-1, -0.25, -2]),
                json!([0.75, 1.5, 1.25]),
            ),
        ]
        .into_iter()
        .map(|(name, position, half)| {
            (
                name.into(),
                json!({
                    "RoomEscapeV1::Actor":{"kind":4,"ordinal":0},
                    "orr_physics3d::Body":{
                        "pos":position,"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
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

fn centers(template: &Template) -> Vec<[FP; 2]> {
    template
        .walls()
        .iter()
        .map(|wall| {
            let [x, _, z] = vector3(&wall.components[BODY]["pos"]).unwrap();
            [x, z]
        })
        .collect()
}

#[test]
fn all_eight_orientations_match_literal_asymmetric_nonorigin_oracle() {
    let source = asymmetric();
    // Independent literal answers for pivot (-2,1), offset (3,-2). In
    // particular reflected 90/270 distinguish mirror-before-rotation from
    // rotation-before-mirror. No production transform builds these answers.
    let expected = [
        (false, 0, [[0, 0], [5, -6], [2, -4]]),
        (false, 1, [[0, -2], [6, 3], [4, 0]]),
        (false, 2, [[2, -2], [-3, 4], [0, 2]]),
        (false, 3, [[2, 0], [-4, -5], [-2, -2]]),
        (true, 0, [[2, 0], [-3, -6], [0, -4]]),
        (true, 1, [[0, 0], [6, -5], [4, -2]]),
        (true, 2, [[0, -2], [5, 4], [2, 2]]),
        (true, 3, [[2, -2], [-4, 3], [-2, 0]]),
    ];
    let mut distinct = BTreeSet::new();
    for (mirror, turns, positions) in expected {
        let placed = source
            .placed_with_mirror(turns, mirror, PIVOT, OFFSET)
            .unwrap();
        assert_eq!(
            centers(&placed),
            positions.map(|point| point.map(FP::from_int)),
            "mirror={mirror}, turns={turns}"
        );
        distinct.insert(placed.encode().unwrap());
        for (index, wall) in placed.walls().iter().enumerate() {
            let half = vector3(&wall.components[COLLIDER]["shape"]["half_extents"]).unwrap();
            let original =
                vector3(&source.walls()[index].components[COLLIDER]["shape"]["half_extents"])
                    .unwrap();
            assert_eq!(
                half,
                if turns % 2 == 0 {
                    original
                } else {
                    [original[2], original[1], original[0]]
                }
            );
        }
    }
    assert_eq!(
        distinct.len(),
        8,
        "the asymmetric group distinguishes every planar orientation"
    );
}

#[test]
fn no_mirror_matches_legacy_four_turns_bit_exact() {
    let source = asymmetric();
    for turns in 0..4 {
        for (pivot, offset) in [(PIVOT, OFFSET), ([FP::ZERO; 2], [FP::ZERO; 2])] {
            assert_eq!(
                source
                    .placed_with_mirror(turns, false, pivot, offset)
                    .unwrap()
                    .encode()
                    .unwrap(),
                source
                    .placed(turns, pivot, offset)
                    .unwrap()
                    .encode()
                    .unwrap(),
                "legacy bytes for turns={turns}"
            );
        }
    }
}

#[test]
fn reflecting_twice_restores_geometry_and_encoded_bytes() {
    let source = asymmetric();
    // Every reflected quarter-turn is an involution around the same pivot.
    for turns in 0..4 {
        let once = source
            .placed_with_mirror(turns, true, PIVOT, [FP::ZERO; 2])
            .unwrap();
        let twice = once
            .placed_with_mirror(turns, true, PIVOT, [FP::ZERO; 2])
            .unwrap();
        assert_eq!(
            twice.encode().unwrap(),
            source.encode().unwrap(),
            "turns={turns}"
        );
    }
}

#[test]
fn raw_fixed_point_reflections_roundtrip_without_drift() {
    let mut source = asymmetric();
    source.walls[0].components[BODY]["pos"][0] =
        serde_json::from_str("-3.0000152587890625").unwrap();
    source.walls[2].components[BODY]["pos"][2] =
        serde_json::from_str("-2.0000457763671875").unwrap();
    let source_geometry = centers(&source);
    for turns in 0..4 {
        let once = source
            .placed_with_mirror(turns, true, [FP(-3), FP(5)], [FP::ZERO; 2])
            .unwrap();
        let twice = once
            .placed_with_mirror(turns, true, [FP(-3), FP(5)], [FP::ZERO; 2])
            .unwrap();
        assert_eq!(centers(&twice), source_geometry, "turns={turns}");
        for (before, after) in source.walls().iter().zip(twice.walls()) {
            assert_eq!(before.components[COLLIDER], after.components[COLLIDER]);
        }
    }
}

#[test]
fn all_orientations_leave_source_encoding_unchanged() {
    let source = asymmetric();
    let before = source.encode().unwrap();
    for mirror in [false, true] {
        for turns in 0..4 {
            let placed = source
                .placed_with_mirror(turns, mirror, PIVOT, OFFSET)
                .unwrap();
            assert_ne!(placed.encode().unwrap(), before);
            assert_eq!(source.encode().unwrap(), before);
        }
    }
}

#[test]
fn preserves_names_y_heights_identity_rotation_and_every_unowned_field() {
    let mut source = asymmetric();
    source.walls[0].components[BODY]["pos"][1] =
        serde_json::from_str("0.50000000000000001").unwrap();
    source.walls[1].components[BODY]["rot"]["w"] =
        serde_json::from_str("1.00000000000000001").unwrap();
    source.walls[2].components[BODY]["linear_damping"] =
        serde_json::from_str("0.00000000000000001").unwrap();
    source.walls[1].components[COLLIDER]["shape"]["half_extents"][1] =
        serde_json::from_str("2.00000000000000001").unwrap();
    for mirror in [false, true] {
        for turns in 0..4 {
            let placed = source
                .placed_with_mirror(turns, mirror, PIVOT, OFFSET)
                .unwrap();
            for (before, after) in source.walls().iter().zip(placed.walls()) {
                let mut expected = before.components.clone();
                expected[BODY]["pos"][0] = after.components[BODY]["pos"][0].clone();
                expected[BODY]["pos"][2] = after.components[BODY]["pos"][2].clone();
                if turns % 2 == 1 {
                    expected[COLLIDER]["shape"]["half_extents"]
                        .as_array_mut()
                        .unwrap()
                        .swap(0, 2);
                }
                assert_eq!(after.name, before.name);
                assert_eq!(
                    after.components, expected,
                    "only X/Z position and odd-turn X/Z extents may change"
                );
            }
        }
    }
}

#[test]
fn mirror_fixed_axis_retains_original_decimal_tokens() {
    let mut source = asymmetric();
    source.walls.truncate(1);
    source.walls[0].components[BODY]["pos"][0] =
        serde_json::from_str("-2.00000000000000001").unwrap();
    let bytes = source.encode().unwrap();
    let mirrored = source
        .placed_with_mirror(0, true, PIVOT, [FP::ZERO; 2])
        .unwrap();
    assert_eq!(
        mirrored.encode().unwrap(),
        bytes,
        "a wall on the mirror axis retains its numeric spelling"
    );
}

#[test]
fn wide_intermediate_differences_cancel_before_final_i64_conversion() {
    let source = asymmetric();
    // x - MIN and z - MAX can exceed i64; (MIN + MAX) + one raw
    // unit cancels exactly, giving (-z,-x), including negative source axes.
    let placed = source
        .placed_with_mirror(1, true, [FP(i64::MIN), FP(i64::MAX)], [FP(1), FP(1)])
        .unwrap();
    assert_eq!(
        centers(&placed),
        [[fp!(-2), fp!(3)], [fp!(4), fp!(-2)], [fp!(2), fp!(1)]]
    );
    let identity = source
        .placed_with_mirror(0, false, [FP(i64::MIN), FP(i64::MAX)], [FP::ZERO; 2])
        .unwrap();
    assert_eq!(identity.encode().unwrap(), source.encode().unwrap());
}

#[test]
fn wide_pivot_restoration_can_cancel_with_offset_before_bounds_check() {
    let source = asymmetric();
    // For negative centers, pivot + reflected-local-X exceeds i64::MAX,
    // but the subsequent offset brings the complete result back into Room.
    let placed = source
        .placed_with_mirror(
            0,
            true,
            [FP(i64::MAX / 2), FP::ZERO],
            [FP(-i64::MAX + 1), FP::ZERO],
        )
        .unwrap();
    assert_eq!(
        centers(&placed),
        [[fp!(3), fp!(2)], [fp!(-2), fp!(-4)], [fp!(1), fp!(-2)]]
    );
}

#[test]
fn final_i64_overflow_is_rejected_without_mutating_source() {
    let source = asymmetric();
    let bytes = source.encode().unwrap();
    for (turns, pivot, offset) in [
        (0, [FP(i64::MIN), FP::ZERO], [FP::ZERO; 2]),
        (0, [FP(i64::MAX), FP::ZERO], [FP::ZERO; 2]),
        (1, [FP::ZERO, FP(i64::MAX)], [FP::ZERO; 2]),
        (3, [FP::ZERO; 2], [FP(i64::MAX), FP(i64::MIN)]),
    ] {
        let error = source
            .placed_with_mirror(turns, true, pivot, offset)
            .unwrap_err();
        assert!(error.contains("fixed-point bounds"), "{error}");
        assert_eq!(source.encode().unwrap(), bytes);
    }
}

#[test]
fn room_boundaries_are_exact_and_one_raw_step_past_is_atomic() {
    let source = asymmetric();
    let bytes = source.encode().unwrap();
    for (offset, index, expected, outside) in [
        (
            [fp!(13), FP::ZERO],
            0,
            fp!(16),
            [FP(fp!(13).raw() + 1), FP::ZERO],
        ),
        (
            [fp!(-14), FP::ZERO],
            1,
            fp!(-16),
            [FP(fp!(-14).raw() - 1), FP::ZERO],
        ),
    ] {
        let placed = source
            .placed_with_mirror(0, true, [FP::ZERO; 2], offset)
            .unwrap();
        assert_eq!(
            fixed(&placed.walls()[index].components[BODY]["pos"][0]).unwrap(),
            expected
        );
        let error = source
            .placed_with_mirror(0, true, [FP::ZERO; 2], outside)
            .unwrap_err();
        assert!(error.contains("Room bounds"), "{error}");
        assert_eq!(source.encode().unwrap(), bytes);
    }
}

#[test]
fn invalid_turns_and_late_invalid_source_wall_leave_source_unchanged() {
    let source = asymmetric();
    let bytes = source.encode().unwrap();
    for turns in [4, 255] {
        for mirror in [false, true] {
            assert!(source
                .placed_with_mirror(turns, mirror, PIVOT, OFFSET)
                .unwrap_err()
                .contains("quarter-turns"));
            assert_eq!(source.encode().unwrap(), bytes);
        }
    }
    for (path, value) in [
        ("rot", json!({"kind":"unit","x":0,"y":0,"z":0,"w":0})),
        ("kind", json!("dynamic")),
    ] {
        let mut invalid = source.clone();
        invalid.walls[2].components[BODY][path] = value;
        let before = serde_json::to_vec(&invalid).unwrap();
        assert!(invalid.placed_with_mirror(1, true, PIVOT, OFFSET).is_err());
        assert_eq!(serde_json::to_vec(&invalid).unwrap(), before);
    }
}

#[test]
fn final_serialized_size_is_checked_after_mirroring_without_source_mutation() {
    let mut source = asymmetric();
    let mut wall = source.walls[0].clone();
    wall.name = "w".repeat(64);
    wall.components[BODY]["pos"] = json!([1, 0.5, 2]);
    source.walls = vec![wall; 64];
    let target = MAX_BYTES - 16;
    let mut size = serde_json::to_vec(&source).unwrap().len();
    'padding: for wall in &mut source.walls {
        let body = &mut wall.components[BODY];
        for field in [
            "inv_mass",
            "linear_damping",
            "angular_damping",
            "vel",
            "omega",
            "inv_inertia",
        ] {
            let values: Vec<&mut Value> = match &mut body[field] {
                Value::Array(values) => values.iter_mut().collect(),
                value => vec![value],
            };
            for value in values {
                let growth = (target - size).min(47);
                if growth < 2 {
                    break 'padding;
                }
                *value = serde_json::from_str(&format!("0.{}", "0".repeat(growth - 1))).unwrap();
                size += growth;
            }
        }
    }
    let bytes = source.encode().unwrap();
    assert_eq!(bytes.len(), size);
    assert!(
        (MAX_BYTES - 17..=MAX_BYTES - 16).contains(&size),
        "padded size={size}"
    );
    let error = source
        .placed_with_mirror(0, true, [FP::ZERO; 2], [FP::ZERO; 2])
        .unwrap_err();
    assert!(error.contains("64 KiB"), "{error}");
    assert_eq!(source.encode().unwrap(), bytes);
}

fn settle(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if !editor.sim().in_tx && !editor.history().in_tx && editor.room_camera_capture_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "settle mirror host transaction"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn host_fixture() -> (tempfile::TempDir, Editor) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("room.scene.yaml");
    let text = include_str!("../../../tests/fixtures/room_initial.scene.yaml").to_owned();
    std::fs::write(&path, &text).unwrap();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text,
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    settle(&mut editor);
    (temp, editor)
}

fn world(editor: &mut Editor) -> Value {
    editor
        .host_call(
            "world.query",
            json!({"components":[ACTOR],"values":true,"limit":69}),
        )
        .unwrap()
}

fn scene(editor: &mut Editor) -> Value {
    editor
        .host_call("scene.save", json!({"write":false}))
        .unwrap()
}

fn history(editor: &mut Editor) -> Value {
    editor.host_call("history.list", Value::Null).unwrap()
}

fn apply_mirror(
    editor: &mut Editor,
    source: &Template,
    turns: u8,
    pivot: [FP; 2],
    offset: [FP; 2],
) -> Result<Vec<String>, String> {
    source
        .placed_with_mirror(turns, true, pivot, offset)
        .and_then(|placed| editor.instantiate_room_wall_stamp(&placed, [FP::ZERO; 2]))
}

#[test]
fn host_mirrored_group_preserves_existing_entities_and_is_one_undoable_edit() {
    let (_temp, mut editor) = host_fixture();
    let source = asymmetric();
    let source_bytes = source.encode().unwrap();
    let old_world = world(&mut editor);
    let old_scene = scene(&mut editor);
    let old_history_count = editor.history().entries.len();
    let disk = std::fs::read(editor.path().unwrap()).unwrap();
    let placed = source
        .placed_with_mirror(1, true, PIVOT, [fp!(10), fp!(-9)])
        .unwrap();
    let ids = editor
        .instantiate_room_wall_stamp(&placed, [FP::ZERO; 2])
        .unwrap();
    settle(&mut editor);
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 3);
    assert_eq!(editor.history().entries.len(), old_history_count + 1);
    let new_world = world(&mut editor);
    let old_entities = old_world["entities"].as_array().unwrap();
    let new_entities = new_world["entities"].as_array().unwrap();
    assert_eq!(new_entities.len(), old_entities.len() + 3);
    for old in old_entities {
        let new = new_entities
            .iter()
            .find(|new| new["guid"] == old["guid"])
            .unwrap();
        // A scene transaction rebakes ephemeral ECS handles. Persisted GUIDs,
        // names, component inventory and all component values stay identical.
        for field in ["id", "guid", "name", "components", "values"] {
            assert_eq!(
                new[field], old[field],
                "unchanged actor {} field {field}",
                old["guid"]
            );
        }
        assert!(!ids.iter().any(|id| old["guid"].as_str() == Some(id)));
    }
    let captured = editor.capture_room_wall_stamp(&ids).unwrap();
    assert_eq!(
        captured, placed,
        "new GUIDs carry only the placed canonical wall payloads"
    );
    let new_scene = scene(&mut editor);
    assert_ne!(new_scene["checksum"], old_scene["checksum"]);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), disk);
    assert_eq!(source.encode().unwrap(), source_bytes);
    assert!(editor.undo());
    settle(&mut editor);
    let undone = scene(&mut editor);
    assert_eq!(undone["text"], old_scene["text"]);
    assert_eq!(undone["checksum"], old_scene["checksum"]);
    assert_eq!(world(&mut editor), old_world);
    assert!(editor.history().can_redo);
    assert!(editor.redo());
    settle(&mut editor);
    let redone = scene(&mut editor);
    assert_eq!(redone["text"], new_scene["text"]);
    assert_eq!(redone["checksum"], new_scene["checksum"]);
    assert_eq!(world(&mut editor), new_world);
    assert_eq!(editor.history().entries.len(), old_history_count + 1);
}

fn host_with_redo() -> (tempfile::TempDir, Editor, Value) {
    let (temp, mut editor) = host_fixture();
    editor
        .spawn_room_wall("redo sentinel", [fp!(12), fp!(12)], [fp!(0.5), fp!(0.75)])
        .unwrap();
    settle(&mut editor);
    let future = scene(&mut editor);
    assert!(editor.undo());
    settle(&mut editor);
    assert!(editor.history().can_redo);
    (temp, editor, future)
}

#[test]
fn host_invalid_mirror_preserves_scene_history_and_pending_redo() {
    let (_temp, mut editor, future) = host_with_redo();
    let source = asymmetric();
    let bytes = source.encode().unwrap();
    let before_scene = scene(&mut editor);
    let before_world = world(&mut editor);
    let before_history = history(&mut editor);
    let disk = std::fs::read(editor.path().unwrap()).unwrap();
    for (turns, pivot, offset) in [
        (4, PIVOT, OFFSET),
        (0, [FP::ZERO; 2], [fp!(-15), FP::ZERO]),
        (0, [FP(i64::MIN), FP::ZERO], [FP::ZERO; 2]),
    ] {
        assert!(apply_mirror(&mut editor, &source, turns, pivot, offset).is_err());
        settle(&mut editor);
        assert_eq!(scene(&mut editor), before_scene);
        assert_eq!(world(&mut editor), before_world);
        assert_eq!(history(&mut editor), before_history);
        assert!(editor.history().can_redo);
        assert!(!editor.history().in_tx);
        assert!(!editor.sim().in_tx);
        assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), disk);
        assert_eq!(source.encode().unwrap(), bytes);
    }
    assert!(editor.redo());
    settle(&mut editor);
    let redone = scene(&mut editor);
    assert_eq!(redone["text"], future["text"]);
    assert_eq!(redone["checksum"], future["checksum"]);
}

#[test]
fn host_invalid_final_scene_rolls_back_whole_group_and_preserves_redo() {
    let (_temp, mut editor, future) = host_with_redo();
    let mut source = asymmetric();
    source.walls[0].components[BODY]["pos"] = json!([10, 0.5, 10]);
    source.walls[1].components[BODY]["pos"] = json!([12, 1.25, 12]);
    source.walls[2].components[BODY]["pos"] = json!([0, -0.25, 0]);
    let bytes = source.encode().unwrap();
    let before_scene = scene(&mut editor);
    let before_world = world(&mut editor);
    let before_history = history(&mut editor);
    let disk = std::fs::read(editor.path().unwrap()).unwrap();
    // All three transformed walls pass Template validation. The last wall
    // covers the player, so real host admission must roll back every spawn.
    source
        .placed_with_mirror(0, true, [FP::ZERO; 2], [FP::ZERO; 2])
        .unwrap();
    let error = apply_mirror(&mut editor, &source, 0, [FP::ZERO; 2], [FP::ZERO; 2]).unwrap_err();
    assert!(
        error.contains("player overlaps or touches a wall"),
        "{error}"
    );
    settle(&mut editor);
    assert_eq!(scene(&mut editor), before_scene);
    assert_eq!(world(&mut editor), before_world);
    assert_eq!(history(&mut editor), before_history);
    assert!(editor.history().can_redo);
    assert!(!editor.history().in_tx);
    assert!(!editor.sim().in_tx);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), disk);
    assert_eq!(source.encode().unwrap(), bytes);
    assert!(editor.redo());
    settle(&mut editor);
    let redone = scene(&mut editor);
    assert_eq!(redone["text"], future["text"]);
    assert_eq!(redone["checksum"], future["checksum"]);
}
