//! Child of the existing panel tests: exercises real draft widgets and existing
//! document operations; full EditorApp Play/stale gating remains a separate test.
use super::*;
use crate::mesh_prop_shading::{Settings as Shading, Slot};
use egui_kittest::{kittest::Queryable, Harness};
fn settings(art: &Snapshot) -> Shading {
    let model = art.source_model_arc();
    Shading::new(
        vec![Slot {
            material: model.source().primitives[0].material,
            metallic: 0.7,
            roughness: 0.2,
        }],
        model,
    )
    .unwrap()
}
fn panel(shaded: bool) -> Panel {
    let mut doc = document(1);
    if shaded {
        doc.props[0].shading = Some(settings(&doc.props[0].art));
    }
    let mut panel = Panel {
        graph_revision: Some(GRAPH_REVISION),
        ..Default::default()
    };
    panel.reset_history(doc, Some(0));
    panel.selected = Some(1);
    panel.copy_selection_fields();
    panel
}
#[test]
fn invalid_scalar_widget_draft_cannot_change_applied_document_history_or_files() {
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"scene sentinel").unwrap();
    let sidecar = root.path().join("mesh.scene.yaml.mesh-props.json");
    std::fs::write(&sidecar, b"sidecar sentinel").unwrap();
    let mut panel = panel(true);
    panel.scene = Some(scene.clone());
    let before = panel.document.props[0].shading.clone();
    let art = panel.document.props[0].art.render_identity();
    let mut h = Harness::builder().with_size([700.0, 500.0]).build_ui_state(
        |ui, panel: &mut Panel| {
            let model = panel
                .selected_prop()
                .unwrap()
                .art
                .source_model_arc()
                .clone();
            panel.shading.show(ui, &model);
        },
        panel,
    );
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Prop slot metallic")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Prop slot metallic")
        .type_text("NaN");
    h.run_steps(2);
    h.state_mut().update_selected();
    assert!(h
        .state()
        .error
        .as_ref()
        .is_some_and(|e| e.contains("finite")));
    assert_eq!(h.state().document.props[0].shading, before);
    assert_eq!(h.state().document.props[0].art.render_identity(), art);
    assert_eq!(h.state().history.len(), 1);
    assert_eq!(h.state().cursor, 0);
    assert_eq!(std::fs::read(scene).unwrap(), b"scene sentinel");
    assert_eq!(std::fs::read(sidecar).unwrap(), b"sidecar sentinel");
}
#[test]
fn explicit_update_undo_redo_and_duplicate_grid_preserve_scalar_settings() {
    let mut panel = panel(false);
    let art = panel.document.props[0].art.clone();
    let wanted = settings(&art);
    panel.shading.load(art.source_model_arc(), Some(&wanted));
    assert!(panel.document.props[0].shading.is_none());
    assert_eq!(panel.history.len(), 1);
    panel.update_selected();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props[0].shading.as_ref(), Some(&wanted));
    assert_eq!(panel.history.len(), 2);
    panel.undo();
    assert!(panel.document.props[0].shading.is_none());
    panel.redo();
    assert_eq!(panel.document.props[0].shading.as_ref(), Some(&wanted));
    let duplicate = super::super::duplicate_prop_document(&panel.document, 1, 2).unwrap();
    assert_eq!(duplicate.props[1].shading.as_ref(), Some(&wanted));
    let grid = super::super::stamp_prop_grid(&panel.document, 1, 2, 2, 2, [1.5; 2], GRAPH_REVISION)
        .unwrap();
    for prop in &grid.props {
        assert_eq!(prop.shading.as_ref(), Some(&wanted));
    }
    assert_eq!(panel.document.props.len(), 1);
}
#[test]
fn unshaded_prefab_candidate_only_clears_shading_after_explicit_update() {
    let mut panel = panel(true);
    let original = panel.document.props[0].shading.clone();
    let template = crate::mesh_prop_prefab::Template::capture("Unshaded", &snapshot()).unwrap();
    panel.stage_prefab(template.clone()).unwrap();
    assert_eq!(panel.document.props[0].shading, original);
    assert_eq!(panel.history.len(), 1);
    assert!(panel.pending_prefab());
    panel.copy_selection_fields();
    assert!(!panel.pending_prefab());
    let model = panel.selected_prop().unwrap().art.source_model_arc();
    assert_eq!(panel.shading.settings(model).unwrap(), original);
    panel.stage_prefab(template).unwrap();
    panel.update_selected();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(panel.document.props[0].shading.is_none());
    assert_eq!(panel.history.len(), 2);
    panel.undo();
    assert_eq!(panel.document.props[0].shading, original);
    panel.redo();
    assert!(panel.document.props[0].shading.is_none());
}
