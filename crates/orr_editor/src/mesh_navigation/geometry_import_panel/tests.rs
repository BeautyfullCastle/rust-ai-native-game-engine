use super::Panel;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_navigation::mesh::StaticMeshGraph;

struct FormState {
    panel: Panel,
    candidate: Option<StaticMeshGraph>,
}

fn harness() -> Harness<'static, FormState> {
    Harness::builder()
        .with_size(egui::vec2(720.0, 760.0))
        .build_ui_state(
            |ui, state: &mut FormState| {
                if let Some(candidate) = state.panel.show(ui, [7; 32], Default::default()) {
                    state.candidate = Some(candidate);
                }
            },
            FormState {
                panel: Panel::default(),
                candidate: None,
            },
        )
}

#[test]
fn import_requires_both_explicit_acknowledgements() {
    let mut harness = harness();
    harness.run_steps(3);
    harness
        .get_by_label("Import walkable model")
        .click_accesskit();
    harness.run_steps(2);
    harness.state_mut().panel.package_root = "C:/verified-model-package".into();
    harness.run_steps(2);

    assert!(harness
        .get_by_label("Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
    assert!(!harness.state().panel.dirty());
    assert!(harness.state().candidate.is_none());

    harness
        .get_by_label("I have reviewed the rights to use this model license")
        .click_accesskit();
    harness.run_steps(2);
    assert!(harness
        .get_by_label("Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
    assert!(harness.state().panel.dirty());

    harness
        .get_by_label("I confirm replacing staged geometry with this imported walkable surface")
        .click_accesskit();
    harness.run_steps(2);
    assert!(!harness
        .get_by_label("Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn failed_import_preserves_the_form_and_returns_no_candidate() {
    let mut harness = harness();
    harness.run_steps(3);
    harness
        .get_by_label("Import walkable model")
        .click_accesskit();
    harness.run_steps(2);
    {
        let panel = &mut harness.state_mut().panel;
        panel.package_root = "C:/missing-walkable-model-package".into();
        panel.model_asset = "mesh.glb".into();
        panel.license_asset = "RIGHTS.txt".into();
        panel.license_acknowledged = true;
        panel.replace_confirmed = true;
        panel.dirty = true;
    }
    harness.run_steps(2);
    harness
        .get_by_label("Stage imported walkable surface")
        .click_accesskit();
    harness.run_steps(2);

    let state = harness.state();
    assert!(state.candidate.is_none());
    assert!(state.panel.dirty());
    assert_eq!(
        state.panel.package_root,
        "C:/missing-walkable-model-package"
    );
    assert_eq!(state.panel.model_asset, "mesh.glb");
    assert_eq!(state.panel.license_asset, "RIGHTS.txt");
    assert!(state.panel.error.is_some());
}

#[test]
fn reset_clears_pending_fields_and_acknowledgements() {
    let mut panel = Panel::default();
    panel.package_root = "some-package".into();
    panel.license_acknowledged = true;
    panel.replace_confirmed = true;
    panel.dirty = true;

    panel.reset();

    assert!(!panel.dirty());
    assert!(panel.package_root.is_empty());
    assert!(!panel.license_acknowledged);
    assert!(!panel.replace_confirmed);
}

#[test]
fn acknowledge_is_the_only_operation_that_consumes_confirmations() {
    let mut panel = Panel::default();
    panel.license_acknowledged = true;
    panel.replace_confirmed = true;
    panel.dirty = true;

    panel.acknowledge();

    assert!(!panel.dirty());
    assert!(!panel.license_acknowledged);
    assert!(!panel.replace_confirmed);
}
