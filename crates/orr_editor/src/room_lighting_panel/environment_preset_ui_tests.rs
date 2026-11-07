//! Real CPU widgets/file persistence; no native window, renderer or OS input.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_sample::room_lighting::{Activation, Environment};
fn fixture() -> (tempfile::TempDir, Panel) {
    let dir = tempfile::tempdir().unwrap();
    let mut document = Document::enabled_default();
    document.environment = Some(Environment::default());
    document.follow_player = Some([1.0, 2.0, 3.0]);
    document.activation = Some(Activation::KeyCollected);
    let bytes = document.to_bytes().unwrap();
    let path = dir.path().join("room.lighting.json");
    let scene_path = dir.path().join("room.scene.yaml");
    let manifest_path = dir.path().join("orr.project.json");
    let manifest_bytes =
        b"{\"schema\":2,\"entry\":{\"lighting\":\"room.lighting.json\"}}\n".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(&manifest_path, &manifest_bytes).unwrap();
    (
        dir,
        Panel::new(PreparedRoomLighting {
            path,
            scene_path,
            manifest_path,
            document,
            bytes,
            manifest_bytes,
        }),
    )
}
fn harness(panel: Panel, editable: bool) -> Harness<'static, (Panel, usize)> {
    let mut h = Harness::builder()
        .with_size([1500.0, 2400.0])
        .build_ui_state(
            move |ui, state: &mut (Panel, usize)| {
                state.1 += usize::from(state.0.show_impl(ui, editable, None));
            },
            (panel, 0),
        );
    h.run_steps(2);
    h.get_by_label("Room lighting").click_accesskit();
    h.run_steps(2);
    h.get_by_label("Room environment presets").click_accesskit();
    h.run_steps(2);
    h
}
fn path(h: &mut Harness<'_, (Panel, usize)>, value: &Path) {
    let value = value.to_str().unwrap();
    h.get_by_role_and_label(Role::TextInput, "Room environment preset file")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Room environment preset file")
        .type_text(value);
    h.run_steps(2);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Room environment preset file")
            .accesskit_node()
            .value()
            .as_deref(),
        Some(value)
    );
}
fn click(h: &mut Harness<'_, (Panel, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn actual_preset_save_load_stages_environment_then_existing_apply_history_save_reopen() {
    let (dir, panel) = fixture();
    let before = panel.document.clone();
    let saved = panel.saved_bytes.clone();
    let source = dir.path().join("new.room-environment-preset.json");
    let mut h = harness(panel, true);
    path(&mut h, &source);
    click(&mut h, "Save applied environment preset");
    assert!(h.state().0.error.is_none());
    let first = std::fs::read(&source).unwrap();
    click(&mut h, "Save applied environment preset");
    assert!(h.state().0.error.is_some());
    assert_eq!(std::fs::read(&source).unwrap(), first);
    assert_eq!(h.state().0.document, before);
    assert_eq!(h.state().1, 0);
    let mut different = before.clone();
    different.environment.as_mut().unwrap().intensity = 0.25;
    let variant = dir.path().join("other.room-environment-preset.json");
    environment_preset::save_new(
        &variant,
        &Preset::from_document(&different).unwrap().encode().unwrap(),
    )
    .unwrap();
    path(&mut h, &variant);
    click(&mut h, "Load environment preset");
    assert!(h.state().0.error.is_none());
    assert_eq!(h.state().0.candidate, different);
    assert_eq!(h.state().0.candidate.point_light, before.point_light);
    assert_eq!(h.state().0.candidate.follow_player, before.follow_player);
    assert_eq!(h.state().0.candidate.activation, before.activation);
    assert_eq!(h.state().0.document, before);
    assert_eq!(h.state().0.saved_bytes, saved);
    assert!(h.state().0.undo.is_empty());
    assert_eq!(h.state().1, 0);
    for (label, expected, count) in [
        ("Apply lighting", different.clone(), 1),
        ("Undo lighting edit", before.clone(), 2),
        ("Redo lighting edit", different.clone(), 3),
    ] {
        click(&mut h, label);
        assert_eq!(h.state().0.document, expected);
        assert_eq!(h.state().1, count);
    }
    click(&mut h, "Save lighting");
    assert!(h.state().0.error.is_none());
    assert_eq!(
        Document::parse(&std::fs::read(&h.state().0.path).unwrap()).unwrap(),
        different
    );
}
#[test]
fn malformed_preset_or_changed_source_never_changes_draft_history_or_saved_bytes() {
    for changed_source in [false, true] {
        let (dir, panel) = fixture();
        let before = panel.candidate.clone();
        let preset = dir.path().join("bad.room-environment-preset.json");
        std::fs::write(&preset, b"invalid").unwrap();
        let mut h = harness(panel, true);
        path(&mut h, &preset);
        if changed_source {
            std::fs::write(&h.state().0.path, b"changed source").unwrap();
        }
        let saved = std::fs::read(&h.state().0.path).unwrap();
        click(&mut h, "Load environment preset");
        assert!(h.state().0.error.is_some());
        assert_eq!(h.state().0.candidate, before);
        assert_eq!(h.state().0.document, before);
        assert!(h.state().0.undo.is_empty() && h.state().0.redo.is_empty());
        assert_eq!(h.state().1, 0);
        assert_eq!(std::fs::read(&h.state().0.path).unwrap(), saved);
    }
}
#[test]
fn readonly_and_no_applied_environment_refuse_preset_export() {
    let (_dir, mut panel) = fixture();
    panel.document.environment = None;
    panel.candidate = panel.document.clone();
    let h = harness(panel, true);
    assert!(
        h.get_by_label("Save applied environment preset")
            .accesskit_node()
            .is_disabled()
    );
    let (_dir, panel) = fixture();
    let h = harness(panel, false);
    // The parent disables this collapsed group as well as its actions.
    assert!(
        h.get_by_label("Room environment presets")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(h.state().1, 0);
    assert!(h.state().0.undo.is_empty());
}
