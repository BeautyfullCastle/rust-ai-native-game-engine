use super::*;
fn approved() -> Panel {
    Panel {
        runtime: "approved.exe".into(),
        runtime_sha256: "a".repeat(64),
        trusted_runtime: true,
        ..Default::default()
    }
}
#[test]
fn path_change_and_revert_revoke_trust() {
    let mut panel = approved();
    panel.set_runtime_identity("replacement.exe".into(), panel.runtime_sha256.clone());
    assert!(!panel.trusted_runtime);
    panel.set_runtime_identity("approved.exe".into(), panel.runtime_sha256.clone());
    assert!(!panel.trusted_runtime);
}
#[test]
fn digest_change_and_revert_revoke_trust() {
    let mut panel = approved();
    panel.set_runtime_identity(panel.runtime.clone(), "b".repeat(64));
    assert!(!panel.trusted_runtime);
    panel.set_runtime_identity(panel.runtime.clone(), "a".repeat(64));
    assert!(!panel.trusted_runtime);
}
#[test]
fn unchanged_identity_preserves_explicit_consent_only() {
    let mut panel = approved();
    panel.set_runtime_identity(panel.runtime.clone(), panel.runtime_sha256.clone());
    assert!(panel.trusted_runtime);
    panel.trusted_runtime = false;
    panel.set_runtime_identity(panel.runtime.clone(), panel.runtime_sha256.clone());
    assert!(!panel.trusted_runtime);
}
fn complete_measurement(panel: &mut Panel, path: String, sha256: String) {
    let (sender, receiver) = mpsc::channel();
    panel.pending = Some(receiver);
    sender
        .send(Ok(Completed::Measured { path, sha256 }))
        .unwrap();
    panel.poll();
    assert!(!panel.busy());
}
#[test]
fn newly_measured_bytes_revoke_prior_consent() {
    let mut panel = approved();
    let path = panel.runtime.clone();
    complete_measurement(&mut panel, path, "b".repeat(64));
    assert_eq!(panel.runtime_sha256, "b".repeat(64));
    assert!(!panel.trusted_runtime);
}
#[test]
fn measuring_identical_bytes_preserves_existing_consent() {
    let mut panel = approved();
    let path = panel.runtime.clone();
    let digest = panel.runtime_sha256.clone();
    complete_measurement(&mut panel, path, digest);
    assert!(panel.trusted_runtime);
}
#[test]
fn measurement_never_grants_consent() {
    let mut panel = approved();
    panel.trusted_runtime = false;
    let path = panel.runtime.clone();
    complete_measurement(&mut panel, path, "b".repeat(64));
    assert!(!panel.trusted_runtime);
}
#[test]
fn stale_measurement_cannot_restore_consent_or_overwrite_identity() {
    let mut panel = approved();
    panel.set_runtime_identity("replacement.exe".into(), "b".repeat(64));
    complete_measurement(&mut panel, "approved.exe".into(), "a".repeat(64));
    assert_eq!(panel.runtime, "replacement.exe");
    assert_eq!(panel.runtime_sha256, "b".repeat(64));
    assert!(!panel.trusted_runtime);
    assert!(panel.status.contains("path changed"));
}

#[test]
fn actual_runtime_text_edits_revoke_consent_without_dispatch() {
    use egui_kittest::kittest::Queryable;
    let editor = Editor::open(&crate::editor::default_scene_path()).unwrap();
    let mut panel = approved();
    panel.open = true;
    let mut harness = egui_kittest::Harness::builder()
        .with_size([900.0, 700.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui.ctx(), &mut state.1, false),
            (panel, editor),
        );
    harness.run_steps(2);
    for label in ["Runtime executable", "Runtime SHA-256"] {
        harness.state_mut().0.trusted_runtime = true;
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .focus();
        harness.run_steps(2);
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .type_text("x");
        harness.run_steps(2);
        assert!(!harness.state().0.trusted_runtime);
        assert!(!harness.state().0.busy());
    }
}
