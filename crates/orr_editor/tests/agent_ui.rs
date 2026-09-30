//! egui_kittest tests of the Agent tab: real widgets, real clicks, the
//! verification on its background thread. No GPU (the viewport shows its
//! notice; the panel does not need it).

mod common;

use common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_edit::{Op, Origin};
use orr_editor::agent::VerifySource;
use orr_editor::agent_ui::{LBL_ACCEPT, LBL_PREVIEW, LBL_REJECT, LBL_VERIFY, TXT_PASSED};
use orr_editor::app::BottomTab;
use orr_editor::EditorApp;

fn harness() -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None))
}

/// An agent proposal that renames body_05 and moves it to `(2, y)`.
fn stage(h: &mut Harness<'_, EditorApp>, y: i32) {
    let ed = &mut h.state_mut().editor;
    let guid = guid_named(ed, "body_05");
    let id = ed.doc_mut().propose("rename+move", Origin::Agent("claude".into())).unwrap();
    ed.doc_mut().proposal_apply(id, Op::Rename { guid: guid.clone(), name: Some("hero".into()) }).unwrap();
    ed.doc_mut().proposal_apply(id, Op::SetField { guid, component: BODY.into(), path: "pos".into(), value: vec2(2, y) }).unwrap();
}

/// True if some label contains `text` (several can: list row and detail title).
fn shows(h: &Harness<'_, EditorApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

/// Steps frames until the background verification has finished.
fn wait_for_result(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2000 {
        h.step();
        if h.state().editor.verify_result().is_some() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the verification never finished");
}

#[test]
fn tab_lists_the_proposal_with_summary_and_diff() {
    let mut h = harness();
    h.run();
    assert!(h.query_by_label("Agent").is_some());
    stage(&mut h, 30);
    h.run_steps(2);
    h.get_by_label("Agent").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.bottom_tab, BottomTab::Agent);
    assert!(shows(&h, "rename+move"));
    assert!(shows(&h, "agent:claude"));
    assert!(shows(&h, "2 ops"));
    assert!(shows(&h, "~ 1 rename, ~ 1 field"));
    assert!(shows(&h, "--- base"), "the unified diff is shown");
    assert!(shows(&h, "~ rename e_"), "the structural summary is shown");
}

#[test]
fn verify_then_accept_from_the_panel_lands_in_the_history_and_undoes() {
    let mut h = harness();
    h.run();
    let original = h.state().editor.doc().to_yaml();
    stage(&mut h, 30);
    h.get_by_label("Agent").click();
    h.run_steps(3);
    h.get_all_by_label_contains("rename+move").next().expect("the list row").click();
    h.run_steps(2);
    h.state_mut().editor.agent_mut().source = VerifySource::Bot(60);
    h.get_by_label(LBL_VERIFY).click();
    h.step();
    assert!(h.state().editor.agent().running().is_some(), "the job runs in the background");
    wait_for_result(&mut h);
    assert!(h.query_by_label(TXT_PASSED).is_some(), "the report says the checks passed");
    assert!(shows(&h, "lost_bodies"));

    h.get_by_label(LBL_ACCEPT).click();
    h.run_steps(3);
    assert!(h.state().editor.doc().list_proposals().is_empty());
    h.get_by_label("History").click();
    h.run_steps(2);
    assert!(shows(&h, "rename+move"));
    assert!(shows(&h, "[agent:claude]"));
    assert_eq!(h.state().editor.doc().history().len(), 1);

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().editor.doc().to_yaml(), original);
}

#[test]
fn a_failing_check_is_shown_and_reject_removes_the_proposal() {
    let mut h = harness();
    h.run();
    stage(&mut h, -10);
    h.get_by_label("Agent").click();
    h.run_steps(3);
    h.state_mut().editor.agent_mut().source = VerifySource::Bot(60);
    h.get_by_label(LBL_VERIFY).click();
    wait_for_result(&mut h);
    assert!(h.query_by_label(TXT_PASSED).is_none());
    assert!(shows(&h, "FAILED"));
    h.get_by_label(LBL_REJECT).click();
    h.run_steps(3);
    assert!(h.state().editor.doc().list_proposals().is_empty());
    assert!(h.state().editor.doc().history().is_empty());
}

#[test]
fn preview_button_toggles_the_viewport_preview() {
    let mut h = harness();
    h.run();
    stage(&mut h, 30);
    h.get_by_label("Agent").click();
    h.run_steps(3);
    assert!(h.state().editor.previewing().is_none());
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    assert!(h.state().editor.previewing().is_some());
    h.get_by_label(LBL_PREVIEW).click();
    h.run_steps(2);
    assert!(h.state().editor.previewing().is_none());
}

#[test]
fn a_conflict_is_shown_in_the_panel() {
    let mut h = harness();
    h.run();
    stage(&mut h, 30);
    h.state_mut().editor.select_named("body_05");
    assert!(h.state_mut().editor.delete_selected());
    h.get_by_label("Agent").click();
    h.run_steps(3);
    h.get_by_label(LBL_ACCEPT).click();
    h.run_steps(3);
    assert!(shows(&h, "no longer applies"));
    assert_eq!(h.state().editor.doc().list_proposals().len(), 1);
}
