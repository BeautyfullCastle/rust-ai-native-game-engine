use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_input::{Button, Key};
use std::{fs, path::PathBuf};

fn fixture() -> (tempfile::TempDir, PathBuf, ProjectLink, Panel) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("한글 Room");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: orr_sample::project_create::ROOM_TEMPLATE.into(),
        seed: "ui-link".into(),
    })
    .unwrap();
    let panel = Panel::default();
    panel.save_as(&root.join("custom.json")).unwrap();
    let link = ProjectLink {
        request: Request {
            root: root.to_str().unwrap().into(),
            controls: "custom.json".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    (temp, root, link, panel)
}

#[test]
fn attaches_reopens_and_revises_without_changing_sidecars_or_history() {
    let (_temp, root, mut link, mut panel) = fixture();
    let original = fs::read(root.join("orr.project.json")).unwrap();
    link.prepare(&panel).unwrap();
    assert_eq!(fs::read(root.join("orr.project.json")).unwrap(), original);
    assert!(!root.join("orr.project.before-controls.json").exists());
    assert!(link.commit(&panel).unwrap().contains("Reopen"));
    assert!(link.commit(&panel).is_err());
    assert_eq!(
        fs::read(root.join("orr.project.before-controls.json")).unwrap(),
        original
    );
    panel
        .replace_draft(vec![Button::Keyboard { key: Key::J }])
        .unwrap();
    panel.apply_draft().unwrap();
    panel.save_as(&root.join("next.json")).unwrap();
    link.request.controls = "next.json".into();
    link.request.revise = true;
    link.request.expected = "custom.json".into();
    link.request.backup = "before-next.json".into();
    let prior = fs::read(root.join("orr.project.json")).unwrap();
    link.prepare(&panel).unwrap();
    link.commit(&panel).unwrap();
    assert_eq!(fs::read(root.join("before-next.json")).unwrap(), prior);
    let project = orr_sample::room_project::PreparedProject::open(&root).unwrap();
    assert_eq!(
        project.controls().unwrap().bytes,
        fs::read(root.join("next.json")).unwrap()
    );
    assert_eq!(
        ActionMap::load_file(root.join("custom.json")).unwrap(),
        Panel::default().document().clone()
    );
    assert!(panel.can_undo());
}

#[test]
fn edits_drafts_and_every_request_field_invalidate_confirmation() {
    for field in 0..7 {
        let (_temp, root, mut link, mut panel) = fixture();
        link.prepare(&panel).unwrap();
        match field {
            0 => link.request.root.push('x'),
            1 => link.request.controls.push('x'),
            2 => link.request.revise = true,
            3 => link.request.expected.push('x'),
            4 => link.request.backup.push('x'),
            5 => panel
                .replace_draft(vec![Button::Keyboard { key: Key::J }])
                .unwrap(),
            _ => {
                panel
                    .replace_draft(vec![Button::Keyboard { key: Key::J }])
                    .unwrap();
                panel.apply_draft().unwrap();
            }
        }
        assert!(link.commit(&panel).is_err());
        assert!(link.preview.is_none());
        assert!(!root.join("orr.project.before-controls.json").exists());
    }
}

#[test]
fn mismatch_pending_stale_and_external_changes_fail_without_retry() {
    let (_temp, root, mut link, mut panel) = fixture();
    panel
        .replace_draft(vec![Button::Keyboard { key: Key::J }])
        .unwrap();
    assert!(link.prepare(&panel).is_err());
    panel.apply_draft().unwrap();
    assert!(link.prepare(&panel).unwrap_err().contains("differs"));
    panel.use_defaults().unwrap();
    link.prepare(&panel).unwrap();
    let manifest = root.join("orr.project.json");
    let mut changed = fs::read(&manifest).unwrap();
    changed.push(b' ');
    fs::write(&manifest, &changed).unwrap();
    assert!(link.commit(&panel).is_err());
    assert!(link.preview.is_none());
    assert_eq!(fs::read(manifest).unwrap(), changed);
    assert!(!root.join("orr.project.before-controls.json").exists());
    link.request.revise = true;
    link.request.expected = "missing.json".into();
    link.request.backup = "backup.json".into();
    assert!(link.prepare(&panel).is_err());
}

#[test]
fn applied_uncertain_outcome_is_not_reported_as_no_change() {
    let message = outcome_message(Outcome {
        backup: "backup.json".into(),
        durability_warning: Some("injected sync failure".into()),
    });
    assert!(message.contains("Project linked"));
    assert!(message.contains("Applied, but durability is uncertain"));
    assert!(message.contains("Inspect files before any retry"));
}

#[test]
fn production_widgets_cancel_repeat_edit_fail_and_commit() {
    let (_temp, root, link, panel) = fixture();
    let original = fs::read(root.join("orr.project.json")).unwrap();
    let mut h = Harness::builder()
        .with_size([900.0, 1000.0])
        .build_ui_state(|ui, (link, panel)| link.show(ui, panel), (link, panel));
    h.run();
    h.get_by_label("Preview project link").click();
    h.run();
    assert!(h.state().0.preview.is_some());
    h.get_by_label("Cancel project link").click();
    h.run();
    assert!(h.state().0.preview.is_none());
    assert_eq!(fs::read(root.join("orr.project.json")).unwrap(), original);
    for _ in 0..2 {
        h.get_by_label("Preview project link").click();
        h.run();
    }
    h.get_by_label("Saved controls filename").focus();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.get_by_label("Saved controls filename")
        .type_text("missing.json");
    h.run();
    assert!(h.query_by_label("Confirm project link").is_none());
    h.get_by_label("Preview project link").click();
    h.run();
    assert!(h.state().0.preview.is_none());
    h.state_mut().0.request.controls = "custom.json".into();
    h.run();
    h.get_by_label("Preview project link").click();
    h.run();
    h.get_by_label("Confirm project link").click();
    h.run();
    assert!(h.query_by_label("Confirm project link").is_none());
    assert!(h.state().0.message.contains("Project linked"));
    h.get_by_label("Preview project link").click();
    h.run();
    assert!(h.state().0.preview.is_none());
    assert_eq!(
        fs::read(root.join("orr.project.before-controls.json")).unwrap(),
        original
    );
}

#[test]
fn changed_sidecar_after_preview_is_rejected_and_preview_consumed() {
    let (_temp, root, mut link, panel) = fixture();
    link.prepare(&panel).unwrap();
    let path = root.join("custom.json");
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    fs::write(path, bytes).unwrap();
    assert!(link.commit(&panel).is_err());
    assert!(link.preview.is_none());
    assert!(!root.join("orr.project.before-controls.json").exists());
}

#[test]
fn production_widgets_revision_and_restore_do_not_reuse_confirmation() {
    let (_temp, root, mut link, panel) = fixture();
    link.prepare(&panel).unwrap();
    link.commit(&panel).unwrap();
    panel.save_as(&root.join("next.json")).unwrap();
    let mut h = Harness::builder()
        .with_size([900.0, 1000.0])
        .build_ui_state(|ui, (link, panel)| link.show(ui, panel), (link, panel));
    h.run();
    h.get_by_label("Revise an existing controls reference")
        .click();
    h.run();
    for (label, value) in [
        ("Saved controls filename", "next.json"),
        ("Expected current controls filename", "custom.json"),
        ("Fresh backup filename", "before-next.json"),
    ] {
        h.get_by_label(label).focus();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        h.get_by_label(label).type_text(value);
        h.run();
    }
    for change in 0..3 {
        h.get_by_label("Preview project link").click();
        h.run();
        assert!(h.state().0.preview.is_some());
        match change {
            0 => {
                h.state_mut()
                    .1
                    .replace_draft(vec![Button::Keyboard { key: Key::J }])
                    .unwrap();
                h.run();
                h.state_mut().1.discard_draft();
            }
            1 => {
                h.state_mut()
                    .1
                    .replace_draft(vec![Button::Keyboard { key: Key::J }])
                    .unwrap();
                h.state_mut().1.apply_draft().unwrap();
                h.run();
                h.state_mut().1.undo().unwrap();
            }
            _ => {
                h.state_mut().0.request.backup = "other.json".into();
                h.run();
                h.state_mut().0.request.backup = "before-next.json".into();
            }
        }
        h.run();
        assert!(h.query_by_label("Confirm project link").is_none());
    }
    h.get_by_label("Preview project link").click();
    h.run();
    h.get_by_label("Confirm project link").click();
    h.get_by_label("Confirm project link").click();
    h.run();
    assert!(h.state().0.message.contains("Project linked"));
    assert!(h.state().0.preview.is_none());
    assert!(root.join("before-next.json").exists());
    assert!(orr_sample::room_project::PreparedProject::open(&root).is_ok());
}
