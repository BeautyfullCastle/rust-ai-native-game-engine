use super::*;
#[cfg(feature = "mesh-mission")]
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[cfg(feature = "mesh-mission")]
use orr_remote::mesh_navigation::mesh_from_view;

const GRAPH_REVISION: [u8; 32] = [7; 32];

#[test]
fn default_hud_form_parses_and_roundtrips_as_a_bound_document() {
    let panel = Panel::default();
    let document = panel
        .parse_document(Path::new("mesh.scene.yaml"), GRAPH_REVISION)
        .unwrap();
    assert_eq!(document, Document::default());
    let encoded = document.encode("mesh.scene.yaml", GRAPH_REVISION).unwrap();
    assert_eq!(
        Document::decode(&encoded, "mesh.scene.yaml", GRAPH_REVISION).unwrap(),
        document
    );
    assert!(Document::decode(&encoded, "other.scene.yaml", GRAPH_REVISION).is_err());
    assert!(Document::decode(&encoded, "mesh.scene.yaml", [8; 32]).is_err());
}

#[test]
fn form_rejects_invalid_ranges_and_keeps_current_history_unchanged() {
    let mut panel = Panel::default();
    let initial = panel.current_document().cloned();
    panel.title.clear();
    assert!(panel
        .parse_document(Path::new("mesh.scene.yaml"), GRAPH_REVISION)
        .is_err());
    panel.title = "HUD".into();
    panel.font_size = "NaN".into();
    assert!(panel
        .parse_document(Path::new("mesh.scene.yaml"), GRAPH_REVISION)
        .is_err());
    panel.font_size = "20".into();
    panel.text_color[0] = "256".into();
    assert!(panel
        .parse_document(Path::new("mesh.scene.yaml"), GRAPH_REVISION)
        .is_err());
    assert_eq!(panel.current_document().cloned(), initial);
    assert_eq!(panel.history.len(), 1);
    assert_eq!(panel.cursor, 0);
}

#[test]
fn history_is_bounded_and_saved_identity_tracks_the_selected_entry() {
    let mut panel = Panel::default();
    panel.push_state(Some(Document::default()));
    assert_eq!(panel.history.len(), 2);
    panel.saved_id = Some(panel.history[0].id);
    assert!(panel.history_is_dirty());
    for index in 0..(MAX_HISTORY + 4) {
        let mut document = Document::default();
        document.title = format!("HUD {index}");
        panel.push_state(Some(document));
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    assert_eq!(panel.cursor, MAX_HISTORY - 1);
}

#[test]
fn invalid_color_text_is_not_accepted_as_a_byte() {
    let mut panel = Panel::default();
    for invalid in ["-1", "256", "1.5", ""] {
        panel.background_color[2] = invalid.into();
        assert!(panel
            .parse_document(Path::new("mesh.scene.yaml"), GRAPH_REVISION)
            .is_err());
    }
}

#[test]
#[cfg(feature = "mesh-mission")]
fn remove_confirmation_controls_fit_a_fixed_inspector() {
    struct LayoutState {
        editor: crate::editor::Editor,
        panel: Panel,
        checksum: u64,
        history_len: usize,
    }

    for size in [[700.0, 900.0], [1000.0, 900.0]] {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("hud-layout.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        let frame = editor.snapshot().unwrap().predicted();
        let graph_revision = mesh_from_view(frame).unwrap().graph.revision();
        crate::mesh_hud_store::save(&scene, Some(&Document::default()), None, graph_revision)
            .unwrap();
        let checksum = editor.checksum();
        let history_len = editor.history().entries.len();
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, state: &mut LayoutState| {
                egui::Panel::right("hud_layout_inspector")
                    .resizable(false)
                    .exact_size(340.0)
                    .show(ui, |panel_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("hud_layout_inspector_scroll")
                            .auto_shrink([false, false])
                            .show(panel_ui, |scroll_ui| {
                                scroll_ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                state.panel.show(scroll_ui, &state.editor);
                            });
                    });
            },
            LayoutState {
                editor,
                panel: Panel::default(),
                checksum,
                history_len,
            },
        );
        harness.run_steps(5);

        if harness.query_by_label("Remove game HUD").is_none() {
            harness.get_by_label("Mesh game HUD").click_accesskit();
            harness.run_steps(3);
        }
        assert!(harness.query_by_label("Remove game HUD").is_some());
        assert!(harness
            .get_by_label("Remove game HUD")
            .accesskit_node()
            .is_disabled());

        let panel_state = egui::containers::panel::PanelState::load(
            &harness.ctx,
            egui::Id::new("hud_layout_inspector"),
        )
        .expect("rendered HUD inspector panel state");
        let screen_rect = harness.ctx.input(|input| input.viewport_rect());
        assert!((panel_state.outer_rect.width() - 340.0).abs() < 0.5);
        assert!(panel_state.outer_rect.max.x <= screen_rect.max.x + 0.5);

        let confirmation = harness.get_by_label("I confirm removing the saved game HUD");
        confirmation.scroll_to_me();
        harness.run_steps(2);
        let confirmation_rect = harness
            .get_by_label("I confirm removing the saved game HUD")
            .rect();
        assert!(confirmation_rect.min.x >= panel_state.outer_rect.min.x - 0.5);
        assert!(confirmation_rect.max.x <= panel_state.outer_rect.max.x + 0.5);
        assert!(confirmation_rect.max.x <= screen_rect.max.x + 0.5);

        harness
            .get_by_label("I confirm removing the saved game HUD")
            .click_accesskit();
        harness.run_steps(3);
        let remove = harness.get_by_label("Remove game HUD");
        assert!(!remove.accesskit_node().is_disabled());
        let remove_rect = remove.rect();
        assert!(remove_rect.min.x >= panel_state.outer_rect.min.x - 0.5);
        assert!(remove_rect.max.x <= panel_state.outer_rect.max.x + 0.5);
        assert!(remove_rect.max.x <= screen_rect.max.x + 0.5);
        assert_eq!(harness.state().editor.checksum(), harness.state().checksum);
        assert_eq!(
            harness.state().editor.history().entries.len(),
            harness.state().history_len
        );
    }
}
