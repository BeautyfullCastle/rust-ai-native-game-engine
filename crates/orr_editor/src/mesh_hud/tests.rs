use super::{Document, MAX_BYTES};
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationStatus};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, mesh_from_view, AgentSpec};

fn graph_revision() -> [u8; 32] {
    [0x45; 32]
}

fn mesh_view() -> orr_remote::mesh_navigation::MeshView {
    let one = FP::ONE;
    let vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, one],
        [one, FP::ZERO, FP::ZERO],
        [one, FP::ZERO, one],
    ];
    let graph =
        StaticMeshGraph::bake(&vertices, &[[0, 1, 3], [0, 3, 2]], AgentProfile::default()).unwrap();
    let document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [FP::ZERO, FP::ZERO, FP::ZERO],
            goal: [one, FP::ZERO, one],
            max_vertical: FP::ZERO,
            distance_per_tick: FP::from_raw(FP::SCALE / 4),
        },
    )
    .unwrap();
    mesh_from_view(orr_bridge::FrameView::of(document.frame())).unwrap()
}

#[test]
fn default_document_is_scene_and_graph_bound_and_roundtrips() {
    let document = Document::default();
    let bytes = document
        .encode("level.scene.yaml", graph_revision())
        .unwrap();
    assert_eq!(
        Document::decode(&bytes, "level.scene.yaml", graph_revision()).unwrap(),
        document
    );
    assert!(Document::decode(&bytes, "other.scene.yaml", graph_revision()).is_err());
    assert!(Document::decode(&bytes, "level.scene.yaml", [0x46; 32]).is_err());
    assert!(Document::decode(&bytes, "../level.scene.yaml", graph_revision()).is_err());
}

#[test]
fn strict_json_rejects_nonobjects_unknown_duplicate_trailing_and_oversized_data() {
    let bytes = Document::default()
        .encode("level.scene.yaml", graph_revision())
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for invalid in [
        format!("[{text}]"),
        text.replace("\"schema\": 1", "\"schema\": 1, \"extra\": 0"),
        text.replace("\"schema\": 1", "\"schema\": 1, \"schema\": 1"),
        text.replace("\"enabled\": true", "\"enabled\": true, \"enabled\": false"),
        format!("{text}{{}}"),
    ] {
        assert!(
            Document::decode(invalid.as_bytes(), "level.scene.yaml", graph_revision()).is_err()
        );
    }
    assert!(Document::decode(
        &vec![b' '; MAX_BYTES + 1],
        "level.scene.yaml",
        graph_revision()
    )
    .is_err());
}

#[test]
fn text_and_font_constraints_are_enforced() {
    let mut invalid = Document::default();
    invalid.title = "\n".into();
    assert!(invalid.validate().is_err());
    invalid.title = "x".repeat(81);
    assert!(invalid.validate().is_err());
    invalid.title = "ok".into();
    invalid.moving_text = "x".repeat(161);
    assert!(invalid.validate().is_err());
    invalid.moving_text = "moving\u{7f}".into();
    assert!(invalid.validate().is_err());
    invalid.moving_text = "Moving".into();
    invalid.font_size = f32::NAN;
    assert!(invalid.validate().is_err());
    invalid.font_size = 48.1;
    assert!(invalid.validate().is_err());
}

#[test]
fn state_text_distinguishes_idle_moving_and_arrived() {
    let document = Document::default();
    let mut view = mesh_view();
    assert_eq!(document.status_text(&view), document.moving_text);
    view.agent.status = NavigationStatus::Stopped;
    assert_eq!(document.status_text(&view), document.idle_text);
    view.agent.status = NavigationStatus::Moving;
    view.agent.spec.distance_per_tick = FP::ZERO;
    assert_eq!(document.status_text(&view), document.idle_text);
    view.agent.status = NavigationStatus::Arrived;
    assert_eq!(document.status_text(&view), document.arrived_text);
}

#[test]
fn hud_rectangle_is_width_bounded_and_clipped_to_viewport() {
    let viewport = egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(900.0, 500.0));
    let rectangle = super::hud_rect(viewport, egui::vec2(2_000.0, 2_000.0)).unwrap();
    assert!(rectangle.width() <= 420.0);
    assert!(rectangle.left() >= viewport.left());
    assert!(rectangle.top() >= viewport.top());
    assert!(rectangle.right() <= viewport.right());
    assert!(rectangle.bottom() <= viewport.bottom());

    let narrow_viewport = egui::Rect::from_min_size(egui::pos2(5.0, 7.0), egui::vec2(80.0, 60.0));
    let narrow_rectangle = super::hud_rect(narrow_viewport, egui::vec2(500.0, 500.0)).unwrap();
    assert!(narrow_rectangle.right() <= narrow_viewport.right());
    assert!(narrow_rectangle.bottom() <= narrow_viewport.bottom());
}
