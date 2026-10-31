//! Real editor widgets and host; no renderer is initialized by these layout tests.
#![allow(clippy::float_arithmetic)] // Screen geometry only.
mod common;

use common::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::editor::Mode;
use orr_editor::{
    app::{BottomTab, LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    EditorApp,
};

fn harness() -> Harness<'static, EditorApp> {
    let mut h = Harness::builder()
        .with_size([960.0, 720.0])
        .build_eframe(|_| EditorApp::new(demo_editor(), None));
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn toggle(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    settle(h);
}

fn controls(h: &Harness<'_, EditorApp>) {
    for label in [LBL_PLAY, LBL_PAUSE, LBL_STEP, LBL_STOP] {
        assert!(
            h.query_by_label(label).is_some(),
            "missing lifecycle control {label}"
        );
    }
}

#[test]
fn viewport_focus_restores_panel_state_without_document_changes() {
    let mut h = harness();
    assert!(!h.state().ui.viewport_focus);
    h.state_mut().ui.filter = "body_05".into();
    h.state_mut().ui.bottom_tab = BottomTab::History;
    h.state_mut().editor.select_named("body_05");
    settle(&mut h);
    let selection = h.state().editor.selection().cloned();
    let original = h.state().ui.viewport_rect.unwrap();
    let document = scene_text(&mut h.state_mut().editor);
    let checksum = doc_checksum(&mut h.state_mut().editor);
    let history = h.state().editor.history().entries.len();
    for _ in 0..3 {
        toggle(&mut h, "Focus viewport");
        assert!(h.state().ui.viewport_focus);
        let focused = h.state().ui.viewport_rect.unwrap();
        assert!(focused.width() > original.width() + 400.0);
        assert!(
            focused.height() > original.height() + 1.0,
            "hidden bottom panel must return space: {original:?} -> {focused:?}"
        );
        assert!(h
            .query_by_role_and_label(egui::accesskit::Role::Button, "body_05")
            .is_none());
        controls(&h);
        toggle(&mut h, "Restore panels");
        assert!(!h.state().ui.viewport_focus);
        assert_eq!(h.state().ui.filter, "body_05");
        assert_eq!(h.state().ui.bottom_tab, BottomTab::History);
        assert_eq!(h.state().editor.selection(), selection.as_ref());
        assert!(h
            .query_by_role_and_label(egui::accesskit::Role::Button, "body_05")
            .is_some());
        let restored = h.state().ui.viewport_rect.unwrap();
        assert!((restored.width() - original.width()).abs() < 1.0);
        assert!((restored.height() - original.height()).abs() < 1.0);
        assert_eq!(h.state().editor.mode(), Mode::Edit);
        assert_eq!(h.state().editor.history().entries.len(), history);
        assert_eq!(doc_checksum(&mut h.state_mut().editor), checksum);
        assert_eq!(scene_text(&mut h.state_mut().editor), document);
    }
}

#[test]
fn viewport_focus_keeps_playback_controls_and_paused_timeline() {
    let mut h = harness();
    toggle(&mut h, "Focus viewport");
    h.get_by_label(LBL_STEP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    let before = h.state().editor.timeline().unwrap();
    assert!(!before.playing);
    let checksum = checksums(&mut h.state_mut().editor, before.tick, before.tick);
    toggle(&mut h, "Restore panels");
    toggle(&mut h, "Focus viewport");
    assert_eq!(h.state().editor.timeline().unwrap().tick, before.tick);
    assert!(!h.state().editor.timeline().unwrap().playing);
    assert_eq!(
        checksums(&mut h.state_mut().editor, before.tick, before.tick),
        checksum
    );
    controls(&h);
    h.get_by_label(LBL_PLAY).click();
    settle(&mut h);
    assert!(h.state().editor.timeline().unwrap().playing);
    toggle(&mut h, "Restore panels");
    assert!(h.state().editor.timeline().unwrap().playing);
    toggle(&mut h, "Focus viewport");
    h.get_by_label(LBL_PAUSE).click();
    settle(&mut h);
    assert!(!h.state().editor.timeline().unwrap().playing);
    h.get_by_label(LBL_STOP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert!(h.state().ui.viewport_focus, "layout choice survives Stop");
    toggle(&mut h, "Restore panels");
}

#[test]
fn viewport_focus_reveals_explicit_navigation_but_not_passive_close_review() {
    let mut h = harness();
    toggle(&mut h, "Focus viewport");
    toggle(&mut h, "Unsaved work");
    assert!(!h.state().ui.viewport_focus);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::UnsavedWork);
    toggle(&mut h, "Focus viewport");
    h.state_mut().editor.agent_mut().request_tab();
    settle(&mut h);
    assert!(!h.state().ui.viewport_focus);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Agent);
    h.state_mut().editor.select_named("body_05");
    assert!(h.state_mut().editor.set_field(
        &orr_editor::editor::Owner::Component(BODY.into()),
        "pos.x",
        fixed(2)
    ));
    settle(&mut h);
    assert!(h.state().editor.is_dirty());
    toggle(&mut h, "Focus viewport");
    let rect = h.state().ui.viewport_rect;
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(1);
    assert!(h
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .unwrap()
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    assert!(
        h.state().ui.viewport_focus,
        "passive close review cannot change layout"
    );
    assert_eq!(h.state().ui.viewport_rect, rect);
    h.run_steps(2);
    toggle(&mut h, "Close blocked");
    assert!(!h.state().ui.viewport_focus);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::UnsavedWork);
}
