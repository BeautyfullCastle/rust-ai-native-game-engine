//! CPU button dispatch with synthetic eligibility/active flags. Backend tests
//! and Linux lifecycle acceptance separately verify real admitted placements.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[test]
fn explicit_preview_button_is_disabled_without_matching_selected_editable_candidate() {
    let mut h = Harness::builder().build_ui_state(
        |ui, requests: &mut usize| {
            *requests += usize::from(draft_preview_start_button(ui, false));
        },
        0usize,
    );
    h.run_steps(2);
    assert!(h
        .get_by_label("Preview model draft on selected object")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Preview model draft on selected object")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(*h.state(), 0);
}
#[test]
fn actual_start_and_cancel_buttons_emit_only_explicit_requests_without_touching_form_or_bindings() {
    let panel = ModelPanel::default();
    let transform = panel.transform;
    let mut h = Harness::builder().with_size([900.0, 500.0]).build_ui_state(
        |ui, state: &mut (ModelPanel, usize, usize)| {
            state.1 += usize::from(draft_preview_start_button(ui, true));
            if draft_preview_cancel_button(ui, true) {
                state.0.cancel_draft_preview();
                state.2 += 1;
            }
        },
        (panel, 0usize, 0usize),
    );
    h.run_steps(2);
    assert_eq!((h.state().1, h.state().2), (0, 0));
    h.get_by_label("Preview model draft on selected object")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_label("Cancel model draft preview")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!((h.state().1, h.state().2), (1, 1));
    assert_eq!(h.state().0.transform, transform);
    assert!(h.state().0.candidate.is_none());
    assert!(h.state().0.loaded.is_empty());
    assert!(h.state().0.bindings.is_none());
    assert!(!h.state().0.draft_preview_active());
}
#[test]
fn cancel_is_available_when_start_is_disabled_and_absent_when_inactive() {
    let mut h = Harness::builder().build_ui_state(
        |ui, state: &mut (bool, usize)| {
            draft_preview_start_button(ui, false);
            if draft_preview_cancel_button(ui, state.0) {
                state.0 = false;
                state.1 += 1;
            }
        },
        (true, 0usize),
    );
    h.run_steps(2);
    assert!(!h
        .get_by_label("Cancel model draft preview")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Cancel model draft preview")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().1, 1);
    assert!(h.query_by_label("Cancel model draft preview").is_none());
}
