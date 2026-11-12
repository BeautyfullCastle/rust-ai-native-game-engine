use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_sample::room_character::ForwardAxis;
fn harness() -> Harness<'static, Fixture> {
    let mut h = Harness::builder()
        .with_size([1600.0, 2600.0])
        .build_ui_state(
            |ui, f: &mut Fixture| {
                f.panel.show(ui, &mut f.editor, &mut f.models);
            },
            Fixture::new(),
        );
    h.run_steps(2);
    h.get_by_label("Room character").click_accesskit();
    h.run_steps(2);
    h
}
fn click(h: &mut Harness<'_, Fixture>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
fn facing(doc: &Document) -> Option<ForwardAxis> {
    doc.motion.as_ref().and_then(|motion| motion.facing)
}
#[test]
fn explicit_facing_axis_stages_then_apply_undo_redo_pair_save_and_reopen() {
    let mut h = harness();
    let original = h.state().panel.document.clone();
    let files = h.state().pair_bytes();
    let checksum = h.state().editor.checksum();
    click(&mut h, "Face observed movement direction");
    assert_eq!(h.state().panel.candidate.schema, 7);
    assert_eq!(
        facing(&h.state().panel.candidate),
        Some(ForwardAxis::PositiveZ)
    );
    click(&mut h, "Authored forward direction");
    click(&mut h, "Model forward -X");
    let expected = h.state().panel.candidate.clone();
    assert_eq!(facing(&expected), Some(ForwardAxis::NegativeX));
    assert_eq!(h.state().panel.document, original);
    assert!(h.state().panel.undo.is_empty());
    assert_eq!(h.state().pair_bytes(), files);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(
        h.get_by_label("Save character and model bindings")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Apply character settings");
    assert!(h.state().panel.error.is_none());
    assert_eq!(h.state().panel.document, expected);
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().panel.document, original);
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().panel.document, expected);
    assert_eq!(h.state().pair_bytes(), files);
    assert_eq!(h.state().editor.checksum(), checksum);
    click(&mut h, "Save character and model bindings");
    assert!(h.state().panel.error.is_none());
    h.state().reopen(&expected);
}
#[test]
fn other_policy_edits_preserve_facing_and_explicit_optout_preserves_pickup() {
    let mut h = harness();
    click(&mut h, "Face observed movement direction");
    click(&mut h, "Searching speed");
    click(&mut h, "Searching speed 2x");
    click(&mut h, "Play escaped clip once");
    let (target_node, target_tree) = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Crossfade ticks")
        .accesskit_node()
        .locate();
    h.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::SetValue,
            data: Some(egui::accesskit::ActionData::NumericValue(4.0)),
        },
    ));
    h.run_steps(2);
    assert_eq!(h.state().panel.candidate.crossfade_ticks, Some(4));
    assert_eq!(h.state().panel.candidate.schema, 7);
    assert_eq!(
        facing(&h.state().panel.candidate),
        Some(ForwardAxis::PositiveZ)
    );
    assert!(
        h.get_by_label("Use idle clips")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Play key pickup clip once");
    assert_eq!(h.state().panel.candidate.schema, 7);
    click(&mut h, "Play key pickup clip once");
    assert_eq!(h.state().panel.candidate.schema, 7);
    assert_eq!(
        facing(&h.state().panel.candidate),
        Some(ForwardAxis::PositiveZ)
    );
    click(&mut h, "Play key pickup clip once");
    let pickup = h.state().panel.candidate.motion.unwrap().pickup;
    click(&mut h, "Face observed movement direction");
    assert_eq!(h.state().panel.candidate.schema, 6);
    assert_eq!(facing(&h.state().panel.candidate), None);
    assert_eq!(h.state().panel.candidate.motion.unwrap().pickup, pickup);
    click(&mut h, "Play key pickup clip once");
    assert_eq!(h.state().panel.candidate.schema, 5);
    assert!(h.state().panel.candidate.motion.is_some());
    click(&mut h, "Apply character settings");
    assert!(h.state().panel.error.is_none());
}
#[test]
fn focus_and_changed_source_disable_facing_and_reset_preserves_applied_history() {
    let mut h = harness();
    let original = h.state().panel.document.clone();
    let files = h.state().pair_bytes();
    h.input_mut().focused = false;
    h.step();
    assert!(
        h.get_by_label("Face observed movement direction")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Face observed movement direction");
    assert_eq!(h.state().panel.candidate, original);
    h.input_mut().focused = true;
    h.step();
    click(&mut h, "Face observed movement direction");
    click(&mut h, "Apply character settings");
    let applied = h.state().panel.document.clone();
    assert_eq!(applied.schema, 7);
    click(&mut h, "Face observed movement direction");
    assert!(h.state().panel.has_unapplied_changes());
    click(&mut h, "Reset character draft");
    assert_eq!(h.state().panel.candidate, applied);
    assert_eq!(h.state().panel.document, applied);
    assert_eq!(h.state().pair_bytes(), files);
    assert!(!h.state().panel.undo.is_empty());
    let mut replacement = applied.clone();
    std::mem::swap(&mut replacement.searching, &mut replacement.carrying);
    h.state_mut()
        .editor
        .install_room_character(replacement)
        .unwrap();
    h.run_steps(2);
    assert!(
        h.get_by_label("Face observed movement direction")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(h.state().panel.document, applied);
    assert_eq!(h.state().pair_bytes(), files);
}

#[test]
fn same_frame_focus_loss_refuses_facing_optin() {
    let mut h = harness();
    let original = h.state().panel.candidate.clone();
    let (target_node, target_tree) = h
        .get_by_label("Face observed movement direction")
        .accesskit_node()
        .locate();
    h.input_mut().events.extend([
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::Click,
            data: None,
        }),
        egui::Event::WindowFocused(false),
    ]);
    h.step();
    assert_eq!(h.state().panel.candidate, original);
    assert!(
        h.get_by_label("Face observed movement direction")
            .accesskit_node()
            .is_disabled()
    );
}
