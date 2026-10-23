use super::{parse_axis, parse_excluded_cells, parse_grid, toggle_excluded_cell, Panel};
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
fn palette_toggle_canonicalizes_sorted_indices_and_round_trips() {
    let toggled = toggle_excluded_cell("4, 0", 3, 3, 2).unwrap();
    assert_eq!(toggled, "0,2,4");
    let toggled = toggle_excluded_cell(&toggled, 3, 3, 2).unwrap();
    assert_eq!(toggled, "0,4");
    assert_eq!(toggle_excluded_cell(&toggled, 3, 3, 0).unwrap(), "4");
    assert_eq!(toggle_excluded_cell("", 7, 7, 48).unwrap(), "48");
}

#[test]
fn palette_toggle_refuses_invalid_draft_text_and_bounds_without_repair() {
    for value in ["0,", "0,,1", "0,0", "9", "-1", "١"] {
        assert!(toggle_excluded_cell(value, 3, 3, 2).is_err(), "{value:?}");
    }
    assert!(toggle_excluded_cell("", 0, 3, 0).is_err());
    assert!(toggle_excluded_cell("", 3, 8, 0).is_err());
    assert!(toggle_excluded_cell("", 3, 3, 9).is_err());
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
    let mut panel = Panel {
        width: "16".into(),
        dirty: true,
        replace_confirmed: true,
        error: Some("previous grid error".into()),
        ..Default::default()
    };
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

#[test]
fn palette_widgets_toggle_the_manual_draft_and_keep_confirmation() {
    use egui_kittest::{kittest::Queryable, Harness};

    struct FormState {
        panel: Panel,
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 800.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                state.panel.show(ui, None, profile(), false);
            },
            FormState {
                panel: Panel::default(),
            },
        );
    harness.run_steps(3);
    harness.get_by_label("Walkable grid").click_accesskit();
    harness.run_steps(3);
    harness
        .get_by_label("I confirm replacing staged geometry with this grid")
        .click_accesskit();
    harness.run_steps(2);
    harness.get_by_label("+0").click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().panel.excluded_cells, "0");
    assert!(harness.state().panel.dirty());
    assert!(harness.state().panel.replace_confirmed);
    harness.get_by_label("-0").click_accesskit();
    harness.run_steps(2);
    assert!(harness.state().panel.excluded_cells.is_empty());
    assert!(harness.state().panel.replace_confirmed);
}

#[test]
fn stale_mesh_disables_palette_without_changing_manual_draft() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    struct FormState {
        panel: Panel,
    }

    let panel = Panel {
        excluded_cells: "2,0".into(),
        ..Default::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 800.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                state.panel.show(ui, None, profile(), true);
            },
            FormState { panel },
        );
    harness.run_steps(3);
    harness.get_by_label("Walkable grid").click_accesskit();
    harness.run_steps(3);
    assert!(harness.get_by_label("-0").accesskit_node().is_disabled());
    assert_eq!(harness.state().panel.excluded_cells, "2,0");
    assert!(!harness.state().panel.dirty());
}

#[test]
fn invalid_manual_exclusions_hide_palette_without_repairing_the_text() {
    use egui_kittest::{kittest::Queryable, Harness};

    struct FormState {
        panel: Panel,
    }

    let panel = Panel {
        excluded_cells: "0,,1".into(),
        ..Default::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 800.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                state.panel.show(ui, None, profile(), false);
            },
            FormState { panel },
        );
    harness.run_steps(3);
    harness.get_by_label("Walkable grid").click_accesskit();
    harness.run_steps(3);
    assert!(harness
        .query_by_label("Grid palette unavailable: excluded grid cells must be comma-separated unsigned indices")
        .is_some());
    assert!(harness.query_by_label("+0").is_none());
    assert_eq!(harness.state().panel.excluded_cells, "0,,1");
    assert!(!harness.state().panel.dirty());
}

#[test]
fn maximum_grid_palette_fits_the_fixed_inspector_and_toggles_cell_48() {
    use egui_kittest::{kittest::Queryable, Harness};

    struct FormState {
        panel: Panel,
    }

    let panel = Panel {
        columns: "7".into(),
        rows: "7".into(),
        ..Default::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 900.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                egui::Panel::right("inspector")
                    .resizable(false)
                    .exact_size(340.0)
                    .show(ui, |panel_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("inspector_scroll")
                            .auto_shrink([false, false])
                            .show(panel_ui, |scroll_ui| {
                                state.panel.show(scroll_ui, None, profile(), false);
                            });
                    });
            },
            FormState { panel },
        );
    harness.run_steps(4);
    harness.get_by_label("Walkable grid").click_accesskit();
    harness.run_steps(4);

    let inspector =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
            .expect("fixed inspector panel state");
    let screen = harness.ctx.input(|input| input.viewport_rect());
    assert!((inspector.outer_rect.width() - 340.0).abs() < 0.5);
    assert!(inspector.outer_rect.max.x <= screen.max.x + 0.5);

    for index in 0..49 {
        let label = format!("+{index}");
        harness.get_by_label(&label).scroll_to_me();
        harness.run_steps(2);
        let cell = harness.get_by_label(&label).rect();
        assert!(
            cell.min.x >= inspector.outer_rect.min.x - 0.5,
            "{label}: {cell:?}"
        );
        assert!(
            cell.max.x <= inspector.outer_rect.max.x + 0.5,
            "{label}: {cell:?}"
        );
        assert!(cell.min.y >= screen.min.y - 0.5, "{label}: {cell:?}");
        assert!(cell.max.y <= screen.max.y + 0.5, "{label}: {cell:?}");
    }

    harness.get_by_label("+48").click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().panel.excluded_cells, "48");
    assert_eq!(
        parse_grid(&harness.state().panel).unwrap().excluded_cells,
        1_u64 << 48
    );
    assert!(harness.state().panel.dirty());
}
