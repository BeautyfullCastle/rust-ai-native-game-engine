use super::Panel;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_navigation::mesh::StaticMeshGraph;
use std::path::Path;

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

fn cooked_report(destination: &Path) -> crate::mesh_art_import::Report {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("walkable.glb");
    let license = temporary.path().join("license.txt");
    std::fs::write(
        &source,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../orr_model/tests/fixtures/model.glb"
        )),
    )
    .unwrap();
    std::fs::write(&license, "Example license\n").unwrap();
    crate::mesh_art_import::import_package(crate::mesh_art_import::Request {
        source,
        license,
        destination: destination.to_path_buf(),
        name: "walkable-surface".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        graph_revision: [7; 32],
    })
    .unwrap()
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

#[test]
fn selecting_cooked_package_marks_form_dirty_without_acknowledging_or_staging() {
    let temporary = tempfile::tempdir().unwrap();
    let report = cooked_report(&temporary.path().join("created-walkable-package"));
    let mut harness = harness();
    harness.run_steps(3);
    harness
        .get_by_label("Import walkable model")
        .click_accesskit();
    harness.run_steps(2);
    {
        let panel = &mut harness.state_mut().panel;
        panel.license_acknowledged = true;
        panel.replace_confirmed = true;
        panel.error = Some("old error".into());
        panel.select_package(&report);
    }
    harness.run_steps(2);

    let panel = &harness.state().panel;
    assert_eq!(panel.package_root, report.root.display().to_string());
    assert_eq!(panel.model_asset, "model.glb");
    assert_eq!(panel.license_asset, "LICENSE.txt");
    assert!(panel.dirty());
    assert!(!panel.license_acknowledged);
    assert!(!panel.replace_confirmed);
    assert!(panel.error.is_none());
    assert!(harness.state().candidate.is_none());
    assert!(harness
        .get_by_label("Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
}
