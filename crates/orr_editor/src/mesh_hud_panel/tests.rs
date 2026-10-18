use super::*;

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
