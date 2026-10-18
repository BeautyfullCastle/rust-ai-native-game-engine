use super::{stage_position, Target};
use crate::mesh_mission_panel::Panel;
use orr_fp::FP;

#[test]
fn collectible_pick_changes_only_selected_xyz_and_preserves_radius() {
    let mut panel = Panel::default();
    panel.count = "2".into();
    panel.points[0] = ["1".into(), "2".into(), "3".into(), "0.75".into()];
    panel.points[1] = ["4".into(), "5".into(), "6".into(), "0.5".into()];
    let other = panel.points[1].clone();

    let changed = stage_position(
        &mut panel,
        Target::Collectible(0),
        [FP::from_raw(FP::SCALE / 2), -FP::ONE, FP::from_int(2)],
    )
    .unwrap();

    assert!(changed);
    assert_eq!(panel.points[0], ["0.5", "-1", "2", "0.75"]);
    assert_eq!(panel.points[1], other);
    assert!(!panel.form_dirty);
}

#[test]
fn exit_pick_preserves_radius_and_rejects_out_of_range_without_mutation() {
    let mut panel = Panel::default();
    panel.exit = ["1".into(), "2".into(), "3".into(), "0.625".into()];

    let changed = stage_position(
        &mut panel,
        Target::Exit,
        [
            FP::from_raw(FP::SCALE / 4),
            FP::from_raw(FP::SCALE / 2),
            FP::from_raw(FP::SCALE * 3 / 4),
        ],
    )
    .unwrap();
    assert!(changed);
    assert_eq!(panel.exit, ["0.25", "0.5", "0.75", "0.625"]);

    let previous = panel.exit.clone();
    let error = stage_position(
        &mut panel,
        Target::Exit,
        [FP::from_raw(257 * FP::SCALE), FP::ZERO, FP::ZERO],
    )
    .unwrap_err();
    assert!(error.contains("+/-256"));
    assert_eq!(panel.exit, previous);
}

#[test]
fn stale_collectible_selection_does_not_change_form() {
    let mut panel = Panel::default();
    panel.count = "1".into();
    let previous = panel.points.clone();

    let error = stage_position(&mut panel, Target::Collectible(1), [FP::ZERO; 3]).unwrap_err();

    assert!(error.contains("no longer exists"));
    assert_eq!(panel.points, previous);
}
