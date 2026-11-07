//! Real Content packages permission/busy gate; no worker or filesystem action.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[test]
fn parent_busy_gate_keeps_preview_cancel_enabled_and_start_disabled() {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let mut h = Harness::builder()
        .with_size([1600.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, PathBuf)| state.0.show(ui.ctx(), Some(&state.1), None),
            (
                Panel {
                    open: true,
                    ..Default::default()
                },
                f.root.clone(),
            ),
        );
    h.run_steps(3);
    let release = h.state_mut().0.model_import.hold_variant_preview_for_test();
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.busy());
    assert!(!h
        .get_by_label("Cancel variant preview")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Preview animated variant")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Cancel variant preview").click_accesskit();
    h.run_steps(3);
    assert!(
        h.state().0.busy(),
        "held worker still fences a second request"
    );
    assert!(h.state_mut().0.variant_preview_frame().is_none());
    release();
    h.run_steps(3);
    assert!(!h.state().0.busy());
    assert!(h.state_mut().0.variant_preview_frame().is_none());
}
