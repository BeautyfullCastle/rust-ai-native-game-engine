use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn terrain() -> Terrain {
    Terrain::new(
        "terrain/expand-test".into(),
        3,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::from_raw(7); 6],
        vec![true, false],
    )
    .unwrap()
}

#[test]
fn grid_expand_request_uses_exact_decimal_and_rejects_shrink_invalid_sides_and_fill() {
    let terrain = terrain();
    let before = terrain.cook();
    let mut spec = Spec {
        dimensions: [5, 4],
        fill: "-0.0000152587890625".into(),
    };
    assert_eq!(
        spec.request(&terrain).unwrap(),
        Request {
            width: 5,
            depth: 4,
            fill: FP::from_raw(-1)
        }
    );
    for sides in [
        [1, 2],
        [3, 1],
        [2, 3],
        [4, 1],
        [130, 2],
        [3, 130],
        [u32::MAX, u32::MAX],
    ] {
        spec.dimensions = sides;
        assert!(spec.request(&terrain).is_err(), "{sides:?}");
    }
    spec.dimensions = [MAX_SIDE, MAX_SIDE];
    assert!(spec.request(&terrain).is_ok());
    for raw in ["", "NaN", "inf", "1e1000", "99999999999999999999999999999"] {
        spec.fill = raw.into();
        assert!(spec.request(&terrain).is_err(), "{raw}");
    }
    assert_eq!(terrain.cook(), before);
}

#[test]
fn grid_expand_request_rejects_positive_edge_coordinate_overflow() {
    let origin = FP::from_raw(i64::MAX - FP::ONE.raw());
    let terrain = Terrain::new(
        "edge".into(),
        2,
        2,
        [origin; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let before = terrain.cook();
    let mut spec = Spec::for_dimensions([2, 2]);
    assert!(spec.request(&terrain).is_ok());
    for dimensions in [[3, 2], [2, 3], [3, 3]] {
        spec.dimensions = dimensions;
        assert!(spec
            .request(&terrain)
            .unwrap_err()
            .contains("coordinate overflow"));
    }
    assert_eq!(terrain.cook(), before);
}

struct WidgetState {
    spec: Spec,
    terrain: Terrain,
    writable: bool,
    submitted: Vec<Request>,
}
fn harness() -> Harness<'static, WidgetState> {
    let terrain = terrain();
    let spec = Spec::for_dimensions([terrain.width(), terrain.depth()]);
    let mut h = Harness::builder()
        .with_size([1100.0, 700.0])
        .build_ui_state(
            |ui, state: &mut WidgetState| {
                if state.spec.show_controls(ui, &state.terrain, state.writable) {
                    state
                        .submitted
                        .push(state.spec.request(&state.terrain).unwrap());
                }
            },
            WidgetState {
                spec,
                terrain,
                writable: true,
                submitted: Vec::new(),
            },
        );
    h.run_steps(2);
    h
}
fn replace_fill(h: &mut Harness<'_, WidgetState>, value: &str) {
    h.get_by_label("Grid expansion fill height").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_label("Grid expansion fill height")
        .type_text(value);
    h.run_steps(2);
}

#[test]
fn grid_expand_actual_widgets_preview_then_submit_explicit_exact_fill_request() {
    let mut h = harness();
    let before = h.state().terrain.cook();
    assert_eq!(h.state().spec.dimensions, [3, 2]);
    assert!(h.query_by_label("Grid expansion target width").is_some());
    assert!(h.query_by_label("Grid expansion target depth").is_some());
    h.state_mut().spec.dimensions = [MAX_SIDE, MAX_SIDE];
    h.run_steps(2);
    assert_eq!(
        h.state().spec.dimensions,
        [129, 129],
        "dimension widgets must admit the final vertex row and column"
    );
    assert!(!h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    h.state_mut().spec.dimensions = [5, 4];
    h.run_steps(2);
    replace_fill(&mut h, "-1.25");
    assert!(h
        .query_by_label("14 new vertices; one terrain Undo step. Save is separate.")
        .is_some());
    assert_eq!(h.state().terrain.cook(), before);
    assert!(h.state().submitted.is_empty());
    assert!(!h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Apply grid expansion").click_accesskit();
    h.run_steps(2);
    assert_eq!(
        h.state().submitted,
        [Request {
            width: 5,
            depth: 4,
            fill: FP::from_raw(-81920)
        }]
    );
    assert_eq!(
        h.state().terrain.cook(),
        before,
        "controls emit intent; document admission performs the edit"
    );
}

#[test]
fn grid_expand_actual_widgets_fail_closed_for_bad_fill_shrink_and_read_only() {
    let mut h = harness();
    assert!(h
        .query_by_label("Dimensions unchanged; no Undo step.")
        .is_some());
    replace_fill(&mut h, "NaN");
    assert!(h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    replace_fill(&mut h, "0");
    h.state_mut().spec.dimensions = [2, 3];
    h.run_steps(2);
    assert!(h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    h.state_mut().spec.dimensions = [5, 4];
    h.state_mut().writable = false;
    h.run_steps(2);
    assert!(h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    h.state_mut().writable = true;
    h.run_steps(2);
    assert!(!h
        .get_by_label("Apply grid expansion")
        .accesskit_node()
        .is_disabled());
    assert!(h.state().submitted.is_empty());
}
