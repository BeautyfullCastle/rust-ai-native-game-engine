use super::{parse_axis, parse_excluded_cells, parse_grid, Panel};
use orr_fp::{fp, FP};
use orr_navigation::AgentProfile;

fn profile() -> AgentProfile {
    AgentProfile {
        max_slope: fp!(4),
        ..AgentProfile::default()
    }
}

#[test]
fn default_grid_fields_parse_to_the_documented_dimensions() {
    let grid = parse_grid(&Panel::default()).unwrap();
    assert_eq!(grid.origin, [FP::ZERO; 3]);
    assert_eq!(grid.width, fp!(12));
    assert_eq!(grid.depth, fp!(8));
    assert_eq!((grid.columns, grid.rows), (3, 3));
    assert_eq!(grid.excluded_cells, 0);
}

#[test]
fn excluded_cells_use_row_major_bits_and_allow_the_maximum_index() {
    assert_eq!(
        parse_excluded_cells("0, 4, 8", 3, 3).unwrap(),
        0b1_0001_0001
    );
    assert_eq!(parse_excluded_cells("48", 7, 7).unwrap(), 1_u64 << 48);
    assert_eq!(parse_excluded_cells("", 7, 7).unwrap(), 0);
}

#[test]
fn excluded_cell_parser_rejects_malformed_duplicate_and_out_of_range_values() {
    for value in [",0", "0,", "0,,1", "0,0", "9", "-1", "١"] {
        assert!(parse_excluded_cells(value, 3, 3).is_err(), "{value:?}");
    }
    assert!(parse_excluded_cells(&"0".repeat(197), 3, 3).is_err());
}

#[test]
fn axis_parser_is_bounded_and_requires_unsigned_decimal_values() {
    for value in ["", "0", "8", "-1", "+2", " 2", "2.0", "4294967296"] {
        assert!(parse_axis(value, "columns").is_err(), "{value:?}");
    }
    assert_eq!(parse_axis("1", "rows").unwrap(), 1);
    assert_eq!(parse_axis("7", "columns").unwrap(), 7);
}

#[test]
fn confirmation_and_current_mesh_are_required_before_staging() {
    let mut panel = Panel::default();
    assert!(panel.stage_candidate(profile(), false).is_err());
    panel.replace_confirmed = true;
    assert!(panel.stage_candidate(profile(), true).is_err());
    assert!(panel.stage_candidate(profile(), false).is_ok());
}

#[test]
fn acknowledge_clears_form_dirty_state_and_replacement_confirmation() {
    let mut panel = Panel::default();
    panel.width = "16".into();
    panel.dirty = true;
    panel.replace_confirmed = true;
    panel.error = Some("previous grid error".into());
    panel.acknowledge();
    assert!(!panel.dirty());
    assert!(!panel.replace_confirmed);
    assert!(panel.error.is_none());
    assert_eq!(panel.width, "16");
}

#[test]
fn walkable_grid_form_requires_confirmation_before_returning_a_candidate() {
    use egui_kittest::{kittest::Queryable, Harness};

    struct FormState {
        panel: Panel,
        staged: Option<super::Grid>,
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 800.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                let candidate = state.panel.show(ui, None, profile(), false);
                if let Some(grid) = candidate {
                    state.staged = Some(grid);
                    state.panel.acknowledge();
                }
            },
            FormState {
                panel: Panel::default(),
                staged: None,
            },
        );
    harness.run_steps(3);
    harness.get_by_label("Walkable grid").click();
    harness.run_steps(3);
    harness.get_by_label("Stage walkable grid").click();
    harness.run_steps(3);
    assert!(harness.state().staged.is_none());
    assert!(harness.state().panel.error.is_some());

    harness
        .get_by_label("I confirm replacing staged geometry with this grid")
        .click();
    harness.run_steps(3);
    assert!(harness.state().panel.dirty());
    harness.get_by_label("Stage walkable grid").click();
    harness.run_steps(3);
    let staged = harness.state().staged.expect("confirmed grid should stage");
    assert_eq!((staged.columns, staged.rows), (3, 3));
    assert_eq!(staged.excluded_cells, 0);
    assert!(!harness.state().panel.dirty());
    assert!(!harness.state().panel.replace_confirmed);
    assert!(harness.state().panel.error.is_none());
}
