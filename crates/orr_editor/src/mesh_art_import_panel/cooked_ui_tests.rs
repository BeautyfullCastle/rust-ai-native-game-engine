//! CPU form/worker-context checks; no source files, package publication or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(panel: Panel, editable: bool) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1400.0, 1700.0])
        .build_ui_state(
            move |ui, p: &mut Panel| {
                p.show(
                    ui,
                    Some(Path::new("/scene.yaml")),
                    4,
                    Some([7; 32]),
                    editable,
                );
            },
            panel,
        );
    h.run_steps(2);
    h.get_by_label("Import agent art source").click_accesskit();
    h.run_steps(2);
    h
}
fn filled() -> Panel {
    Panel {
        source: "hero.model.json".into(),
        license: "license.txt".into(),
        destination: "new-package".into(),
        name: "hero".into(),
        version: "1.0.0".into(),
        ..Default::default()
    }
}
#[test]
fn unified_static_source_labels_and_suffixes_cover_native_variants_without_json_fallback() {
    let h = harness(Panel::default(), true);
    assert!(h
        .query_by_label("Agent art source (.glb or cooked model)")
        .is_some());
    assert!(h.query_by_label("Agent art source license").is_some());
    for path in ["a.glb", "a.MODEL.JSON", "a.orrmodel.json", " a.ORRMODEL "] {
        assert!(supported_source(path), "{path}");
    }
    for path in [
        "a.json",
        "a.animated.json",
        "a.model.json.bak",
        "a.glb.tmp",
        "",
    ] {
        assert!(!supported_source(path), "{path}");
    }
}
#[test]
fn rights_all_fields_and_supported_source_gate_the_import_button() {
    let mut h = harness(filled(), true);
    assert!(h
        .get_by_label("Import agent art package")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("I have reviewed rights to import this agent art")
        .click_accesskit();
    h.run_steps(2);
    assert!(!h
        .get_by_label("Import agent art package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().source = "unknown.json".into();
    h.run_steps(2);
    assert!(h
        .get_by_label("Import agent art package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().source = "hero.orrmodel".into();
    h.state_mut().license.clear();
    h.run_steps(2);
    assert!(h
        .get_by_label("Import agent art package")
        .accesskit_node()
        .is_disabled());
    assert!(!h.state().busy());
}
#[test]
fn readonly_and_pending_workers_disable_fields_and_inputs_revoke_rights() {
    let h = harness(filled(), false);
    for label in [
        "Agent art source (.glb or cooked model)",
        "Agent art source license",
        "I have reviewed rights to import this agent art",
        "Import agent art package",
    ] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    let (sender, receiver) = mpsc::channel();
    let mut p = filled();
    p.pending = Some(receiver);
    let mut h = harness(p, true);
    assert!(h.state().busy());
    assert!(h
        .get_by_label("Import agent art package")
        .accesskit_node()
        .is_disabled());
    drop(sender);
    h.run_steps(2);
    assert!(!h.state().busy());
    assert!(h.state().status.contains("disconnected"));
    let mut h = harness(filled(), true);
    h.get_by_label("I have reviewed rights to import this agent art")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_role_and_label(
        egui::accesskit::Role::TextInput,
        "Agent art source (.glb or cooked model)",
    )
    .focus();
    h.run_steps(2);
    h.get_by_role_and_label(
        egui::accesskit::Role::TextInput,
        "Agent art source (.glb or cooked model)",
    )
    .type_text("changed");
    h.run_steps(2);
    assert!(!h.state().license_acknowledged);
    assert!(h.state().input_generation > 1);
}
#[test]
fn static_completion_context_and_invalid_suffix_refusal_remain_atomic_without_io() {
    let stamp = ContextStamp {
        scene: Some("/scene.yaml".into()),
        scene_epoch: 4,
        graph_revision: Some([7; 32]),
        input_generation: 2,
    };
    for change in 0..4 {
        assert!(!context_matches(
            &stamp,
            Some(Path::new(if change == 0 {
                "/other.yaml"
            } else {
                "/scene.yaml"
            })),
            if change == 1 { 5 } else { 4 },
            Some(if change == 2 { [8; 32] } else { [7; 32] }),
            if change == 3 { 3 } else { 2 }
        ));
    }
    let mut p = filled();
    p.license_acknowledged = true;
    p.source = "unknown.json".into();
    p.start_import("/scene.yaml".into(), 4, [7; 32]);
    assert!(!p.busy());
    assert!(p.status.contains("no worker was started"));
}
