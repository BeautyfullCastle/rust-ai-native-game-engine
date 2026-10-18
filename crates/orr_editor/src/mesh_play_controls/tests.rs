use super::*;

#[test]
fn input_origin_preserves_exact_fixed_point_coordinates_at_bounds() {
    assert_eq!(
        input_origin([
            FP::from_raw(-(256 * FP::SCALE)),
            FP::from_raw(256 * FP::SCALE),
            FP::from_raw(-1),
        ])
        .unwrap(),
        [-(256 * FP::SCALE) as i32, (256 * FP::SCALE) as i32, -1]
    );
}

#[test]
fn input_origin_rejects_out_of_range_without_saturating() {
    for position in [
        [FP::from_raw(256 * FP::SCALE + 1), FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::from_raw(-256 * FP::SCALE - 1), FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_raw(i64::MAX)],
    ] {
        assert!(input_origin(position).is_err());
    }
}

#[test]
fn scene_reload_and_graph_revision_changes_are_distinct_contexts() {
    let original = Context {
        scene: PathBuf::from("C:/mesh/arena.scene.yaml"),
        epoch: 4,
        graph_revision: [1; 32],
    };
    let reloaded = Context {
        epoch: 5,
        ..original.clone()
    };
    let edited_graph = Context {
        graph_revision: [2; 32],
        ..original.clone()
    };
    assert_ne!(original, reloaded);
    assert_ne!(original, edited_graph);
}
