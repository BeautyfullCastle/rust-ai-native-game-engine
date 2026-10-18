use super::*;

#[cfg(feature = "mesh-mission")]
#[test]
fn workspace_save_is_explicit_and_preserves_external_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("lighting-workspace.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();

    assert!(!panel.save_workspace_changes(&editor).unwrap());
    assert!(!sidecar.exists());

    panel.form_dirty = true;
    assert!(panel.workspace_save_blocker(&editor).is_some());
    assert!(panel.save_workspace_changes(&editor).is_err());
    assert!(!sidecar.exists());
    panel.form_dirty = false;

    let initial = Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
    panel.push_initial(Some(initial.clone()));
    let mut changed_lighting = initial.lighting().unwrap();
    changed_lighting.intensity += 1.0;
    panel.push_state(Some(Document::from_lighting(changed_lighting).unwrap()));
    assert!(panel.save_workspace_changes(&editor).unwrap());
    assert!(!panel.dirty());
    let saved_bytes = std::fs::read(&sidecar).unwrap();
    assert!(!saved_bytes.is_empty());

    changed_lighting = panel.current_document().unwrap().lighting().unwrap();
    changed_lighting.intensity += 1.0;
    panel.push_state(Some(Document::from_lighting(changed_lighting).unwrap()));
    let external_bytes = b"external lighting sidecar update";
    std::fs::write(&sidecar, external_bytes).unwrap();
    assert!(panel.save_workspace_changes(&editor).is_err());
    assert!(panel.dirty());
    assert_eq!(std::fs::read(&sidecar).unwrap(), external_bytes);
}

fn panel_with_scene(scene: &Path) -> Panel {
    let mut panel = Panel::default();
    panel.scene = Some(scene.to_path_buf());
    panel.scene_epoch = Some(1);
    panel.graph_revision = Some([7; 32]);
    panel.document_pending = false;
    panel
}

#[test]
fn default_lighting_roundtrips_through_strict_document() {
    let panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let document = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    assert_eq!(
        document.lighting().unwrap(),
        orr_sample::yard3d_view::yard_lighting()
    );
    let encoded = document.encode("lighting.scene.yaml", [7; 32]).unwrap();
    assert!(Document::decode(&encoded, "lighting.scene.yaml", [7; 32]).is_ok());
}

#[test]
fn invalid_field_values_do_not_create_history_entries() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let initial_history = panel.history.len();
    panel.direction = ["0".into(), "0".into(), "0".into()];
    assert!(panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .is_err());
    assert_eq!(panel.history.len(), initial_history);
    panel.direction = ["-0.45".into(), "-0.8".into(), "-0.35".into()];
    panel.exposure = "0".into();
    assert!(panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .is_err());
    assert_eq!(panel.history.len(), initial_history);
}

#[test]
fn local_history_changes_dirty_state_and_undo_redo_preserve_values() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let initial = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_initial(Some(initial.clone()));
    panel.saved_id = Some(panel.history[0].id);
    panel.intensity = "3.5".into();
    let changed = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_state(Some(changed));
    assert!(panel.history_is_dirty());
    panel.undo();
    assert_eq!(panel.current_document(), Some(&initial));
    assert!(!panel.history_is_dirty());
    panel.redo();
    assert_eq!(
        panel
            .current_document()
            .unwrap()
            .lighting()
            .unwrap()
            .intensity,
        3.5
    );
    assert!(panel.history_is_dirty());
}

#[test]
fn history_is_bounded_and_truncates_redo_after_new_apply() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    for index in 0..MAX_HISTORY + 3 {
        panel.intensity = (index as f32 + 1.0).to_string();
        let document = panel
            .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
            .unwrap();
        panel.push_state(Some(document));
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    panel.undo();
    panel.intensity = "7".into();
    let document = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_state(Some(document));
    assert_eq!(panel.cursor + 1, panel.history.len());
}

#[test]
#[cfg(feature = "mesh-mission")]
fn confirmation_and_remove_controls_fit_a_fixed_inspector() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    struct UiState {
        _directory: tempfile::TempDir,
        editor: crate::editor::Editor,
        panel: Panel,
    }

    for size in [[700.0, 900.0], [1000.0, 900.0]] {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("lighting.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        let mut panel = Panel::default();
        panel.sync(&editor);
        panel.push_initial(Some(
            Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap(),
        ));

        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, state: &mut UiState| {
                egui::Panel::right("inspector")
                    .resizable(false)
                    .exact_size(340.0)
                    .show(ui, |panel_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("inspector_scroll")
                            .auto_shrink([false, false])
                            .show(panel_ui, |scroll_ui| {
                                scroll_ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                state.panel.show(scroll_ui, &state.editor);
                            });
                    });
            },
            UiState {
                _directory: directory,
                editor,
                panel,
            },
        );
        harness.run_steps(5);
        harness.get_by_label("Mesh game lighting").click();
        harness.run_steps(4);
        harness
            .get_by_label("I confirm removing the saved game lighting")
            .scroll_to_me();
        harness.run_steps(3);
        assert!(harness
            .get_by_label("Remove game lighting")
            .accesskit_node()
            .is_disabled());
        harness
            .get_by_label("I confirm removing the saved game lighting")
            .click_accesskit();
        harness.run_steps(3);
        assert!(!harness
            .get_by_label("Remove game lighting")
            .accesskit_node()
            .is_disabled());

        for label in [
            "Save game lighting",
            "Reload game lighting",
            "I confirm removing the saved game lighting",
            "Remove game lighting",
        ] {
            harness.get_by_label(label).scroll_to_me();
            harness.run_steps(3);
            let panel_state =
                egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
                    .expect("fixed inspector panel state");
            let screen_rect = harness.ctx.input(|input| input.viewport_rect());
            assert!((panel_state.outer_rect.width() - 340.0).abs() < 0.5);
            assert!(panel_state.outer_rect.min.x >= screen_rect.min.x - 0.5);
            assert!(panel_state.outer_rect.max.x <= screen_rect.max.x + 0.5);
            let control_rect = harness.get_by_label(label).rect();
            assert!(control_rect.min.x >= panel_state.outer_rect.min.x - 0.5);
            assert!(control_rect.max.x <= panel_state.outer_rect.max.x + 0.5);
        }
    }
}
