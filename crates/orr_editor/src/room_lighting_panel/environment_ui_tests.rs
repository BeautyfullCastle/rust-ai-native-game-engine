//! In-memory Panel tests exercise real widgets and existing history without
//! directory handles, file publication, an OS window or a GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn panel() -> Panel {
    let document = Document::enabled_default();
    Panel {
        path: PathBuf::from("/not-accessed/room.lighting.json"),
        scene_path: PathBuf::from("/not-accessed/room.scene.yaml"),
        manifest_path: PathBuf::from("/not-accessed/orr.project.json"),
        root: Err("in-memory fixture".into()),
        candidate: document.clone(),
        saved: document.clone(),
        saved_bytes: document.to_bytes().unwrap(),
        document,
        manifest_bytes: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        source: SourcePin::Unbound,
        error: None,
        viewport_pick: Default::default(),
    }
}
fn harness(p: Panel, editable: bool) -> Harness<'static, (Panel, usize)> {
    let mut h = Harness::builder()
        .with_size([1300.0, 1800.0])
        .build_ui_state(
            move |ui, state: &mut (Panel, usize)| {
                state.1 += usize::from(state.0.show_impl(ui, editable, None))
            },
            (p, 0usize),
        );
    h.run_steps(3);
    h.get_by_label("Room lighting").click_accesskit();
    h.run_steps(3);
    h
}
fn click(h: &mut Harness<'_, (Panel, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
#[test]
fn environment_enable_stages_only_and_explicit_apply_undo_redo_preserve_point_and_saved_bytes() {
    let p = panel();
    let old = p.document.clone();
    let bytes = p.saved_bytes.clone();
    let mut h = harness(p, true);
    click(&mut h, "Environment lighting enabled");
    assert_eq!(
        h.state().0.candidate.environment,
        Some(Environment::default())
    );
    assert_eq!(h.state().0.document, old);
    assert!(h.state().0.undo.is_empty());
    assert_eq!(h.state().1, 0);
    click(&mut h, "Apply lighting");
    let applied = h.state().0.document.clone();
    assert_eq!(applied.environment, Some(Environment::default()));
    assert_eq!(applied.point_light, old.point_light);
    assert_eq!(h.state().1, 1);
    click(&mut h, "Undo lighting edit");
    assert_eq!(h.state().0.document, old);
    click(&mut h, "Redo lighting edit");
    assert_eq!(h.state().0.document, applied);
    assert_eq!(h.state().0.saved_bytes, bytes);
}
#[test]
fn point_enable_disable_keeps_environment_and_environment_disable_keeps_point_policy() {
    let mut p = panel();
    let mut environment = Environment::default();
    environment.intensity = 7.0;
    p.candidate.environment = Some(environment.clone());
    let mut h = harness(p, true);
    click(&mut h, "Point light enabled");
    assert!(h.state().0.candidate.point_light.is_none());
    assert_eq!(h.state().0.candidate.environment, Some(environment.clone()));
    click(&mut h, "Point light enabled");
    assert_eq!(h.state().0.candidate.environment, Some(environment));
    let point = h.state().0.candidate.point_light;
    click(&mut h, "Environment lighting enabled");
    assert!(h.state().0.candidate.environment.is_none());
    assert_eq!(h.state().0.candidate.point_light, point);
    assert!(h.state().0.undo.is_empty());
}
#[test]
fn environment_reset_is_draft_only_and_invalid_apply_is_atomic() {
    let mut p = panel();
    let old = p.document.clone();
    let mut environment = Environment::default();
    environment.intensity = 7.0;
    p.candidate.environment = Some(environment);
    let mut h = harness(p, true);
    click(&mut h, "Environment lighting controls");
    click(&mut h, "Reset environment defaults");
    assert_eq!(
        h.state().0.candidate.environment,
        Some(Environment::default())
    );
    assert_eq!(h.state().0.document, old);
    assert!(h.state().0.undo.is_empty());
    h.state_mut()
        .0
        .candidate
        .environment
        .as_mut()
        .unwrap()
        .direction = [0.0; 3];
    click(&mut h, "Apply lighting");
    assert!(h.state().0.error.is_some());
    assert_eq!(h.state().0.document, old);
    assert!(h.state().0.undo.is_empty());
    assert_eq!(h.state().1, 0);
}
#[test]
fn environment_checkbox_is_disabled_in_readonly_mode() {
    let p = panel();
    let old = p.candidate.clone();
    let mut h = harness(p, false);
    assert!(h
        .get_by_label("Environment lighting enabled")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Environment lighting enabled");
    assert_eq!(h.state().0.candidate, old);
    assert_eq!(h.state().1, 0);
}
