//! Read-only form and immutable context tests; no host, files or GPU required.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(editable: bool) -> Harness<'static, (State, usize)> {
    Harness::builder()
        .with_size([1300.0, 1400.0])
        .build_ui_state(
            move |ui, s: &mut (State, usize)| {
                s.1 += usize::from(show_inputs(ui, &mut s.0, editable))
            },
            (State::default(), 0usize),
        )
}
#[test]
fn unified_source_form_labels_cover_glb_and_cooked_animation_without_format_fallback() {
    let mut h = harness(true);
    h.run_steps(2);
    assert!(h
        .query_by_label("Animation source (.glb or cooked JSON)")
        .is_some());
    assert!(h.query_by_label("Animation source license").is_some());
    assert!(!h.state().0.inputs.acknowledged);
    assert_eq!(h.state().1, 0);
    for path in [
        "hero.animated.json",
        " /local/HERO.ANIMATED.JSON ",
        "courier.orrmodel.json",
        "a.GLB",
    ] {
        assert!(supported_source(path), "{path}");
    }
    for path in ["", "model.json", "hero.animated.json.bak", "model.glb.tmp"] {
        assert!(!supported_source(path), "{path}");
    }
}
#[test]
fn cooked_import_requires_rights_all_inputs_and_recognized_suffix() {
    let mut h = harness(true);
    h.state_mut().0.inputs = Inputs {
        source: "hero.animated.json".into(),
        license: "license.txt".into(),
        destination: "new".into(),
        name: "animation".into(),
        version: "1.0.0".into(),
        acknowledged: false,
    };
    h.run_steps(2);
    assert!(h
        .get_by_label("Import animation package")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("I have reviewed rights to import this animation")
        .click_accesskit();
    h.run_steps(2);
    assert!(!h
        .get_by_label("Import animation package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.inputs.source = "hero.json".into();
    h.run_steps(2);
    assert!(h
        .get_by_label("Import animation package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.inputs.source = "courier.orrmodel.json".into();
    h.state_mut().0.inputs.license.clear();
    h.run_steps(2);
    assert!(h
        .get_by_label("Import animation package")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn blocked_form_disables_source_rights_and_import_without_dispatching_worker() {
    let mut blocked = harness(false);
    blocked.run_steps(2);
    for label in [
        "Animation source (.glb or cooked JSON)",
        "Animation source license",
        "I have reviewed rights to import this animation",
        "Import animation package",
    ] {
        assert!(blocked.get_by_label(label).accesskit_node().is_disabled());
    }
    assert_eq!(blocked.state().1, 0);
}
#[test]
fn scene_candidate_generation_and_cooked_source_path_remain_exact_completion_identity() {
    let inputs = Inputs {
        source: "hero.animated.json".into(),
        ..Default::default()
    };
    let original = Context {
        scene: "scene.yaml".into(),
        epoch: 1,
        revision: [2; 32],
        generation: 3,
        candidate_generation: 4,
        inputs,
    };
    for change in 0..7 {
        let mut next = original.clone();
        match change {
            0 => next.inputs.source = "hero.glb".into(),
            1 => next.scene = "other.yaml".into(),
            2 => next.epoch += 1,
            3 => next.revision[0] ^= 1,
            4 => next.generation += 1,
            5 => next.candidate_generation += 1,
            _ => next.inputs.source = "other.animated.json".into(),
        };
        assert!(original != next);
    }
    let mut state = State::default();
    state.inputs.acknowledged = true;
    state.changed(true);
    assert!(!state.inputs.acknowledged);
    assert_eq!(state.generation, 1);
}
