use super::*;

#[test]
fn post_process_preset_preserves_authored_and_legacy_policy() {
    let legacy = Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
    let preset = crate::mesh_lighting_preset::Preset::from_document(&legacy).unwrap();
    assert!(!preset.merge_into(&legacy).unwrap().has_post_process());
    for enabled in [false, true] {
        let target = legacy
            .clone()
            .with_post_process(orr_render::PostProcessSettings {
                enabled,
                threshold: 3.0,
                ..Default::default()
            })
            .unwrap();
        let merged = preset.merge_into(&target).unwrap();
        assert!(merged.has_post_process());
        assert_eq!(
            merged.post_process_settings(),
            target.post_process_settings()
        );
    }
}

#[cfg(feature = "mesh-mission")]
#[test]
fn post_process_apply_save_reopen_undo_preserves_simulation_and_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("effects.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let checksum = editor.checksum();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert_eq!(panel.applied_post_process(&editor), None);
    panel.apply(&editor);
    assert!(!panel.current_document().unwrap().has_post_process());
    assert_eq!(
        panel.applied_post_process(&editor),
        Some(Default::default())
    );
    panel.save();
    panel.post_process.enabled = true;
    panel.post_process.threshold = 2.0;
    panel.post_process_authored = true;
    panel.form_dirty = true;
    panel.apply(&editor);
    assert!(panel.applied_post_process(&editor).unwrap().enabled);
    assert!(panel.play_blocker(&editor).is_some());
    panel.save();
    assert!(!panel.dirty());
    let mut reopened = Panel::default();
    reopened.sync(&editor);
    assert_eq!(
        reopened.applied_post_process(&editor),
        Some(panel.post_process)
    );
    panel.undo();
    assert!(!panel.current_document().unwrap().has_post_process());
    assert!(!panel.post_process_authored);
    panel.redo();
    assert!(panel.post_process_authored);
    assert!(panel.applied_post_process(&editor).unwrap().enabled);
    panel.context_stale = true;
    assert_eq!(panel.applied_post_process(&editor), None);
    panel.context_stale = false;
    panel.sidecar_error = Some("invalid sidecar".into());
    assert_eq!(panel.applied_post_process(&editor), None);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
}

#[cfg(feature = "mesh-mission")]
#[test]
fn post_process_controls_click_apply_save_and_reload() {
    use egui_kittest::{
        kittest::Queryable,
        Harness,
    };
    struct State {
        editor: Editor,
        panel: Panel,
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("clicked.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let mut harness = Harness::builder()
        .with_size([700.0, 1600.0])
        .build_ui_state(
            |ui, state: &mut State| {
                egui::ScrollArea::vertical().show(ui, |ui| state.panel.show(ui, &state.editor));
            },
            State { editor, panel },
        );
    harness.run_steps(3);
    harness.get_by_label("Mesh game lighting").click();
    harness.run_steps(3);
    for label in [
        "Saved HDR post-processing",
        "Apply game lighting",
        "Save game lighting",
    ] {
        harness.get_by_label(label).scroll_to_me();
        harness.run_steps(2);
        harness.get_by_label(label).click_accesskit();
        harness.run_steps(3);
    }
    assert!(
        harness
            .state()
            .panel
            .applied_post_process(&harness.state().editor)
            .unwrap()
            .enabled
    );
    assert!(!harness.state().panel.dirty());
    let (document, _) =
        crate::mesh_lighting_store::load(&scene, harness.state().panel.graph_revision.unwrap())
            .unwrap()
            .unwrap();
    assert!(document.post_process_settings().enabled);
    harness.get_by_label("Reload game lighting").scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_label("Reload game lighting")
        .click_accesskit();
    harness.run_steps(3);
    assert!(harness.state().panel.post_process.enabled);
}
