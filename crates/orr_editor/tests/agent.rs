//! The Agent tab's model: proposals staged in the shared `EditorDoc` (as an
//! ERP agent does), the view-only preview, the feed filters and the script
//! commands. No window, no GPU. (The feed itself, fed by a real agent over
//! ERP, is tested in `erp.rs` and `agent_ui.rs`.)

mod common;

use common::*;
use orr_edit::{Op, Origin, ProposalId, Target};
use orr_editor::agent::{summary_line, FeedFilter};
use orr_editor::editor::{Editor, Owner};
use orr_editor::{script, viewport};
use orr_reflect::Value;

fn agent() -> Origin {
    Origin::Agent("claude".into())
}

/// An agent proposal that renames body_05 and moves it to `(x, y)`.
fn rename_and_move(ed: &mut Editor, x: i32, y: i32) -> ProposalId {
    let guid = guid_named(ed, "body_05");
    let id = ed.doc_mut().propose("rename+move", agent()).unwrap();
    ed.doc_mut().proposal_apply(id, Op::Rename { guid: guid.clone(), name: Some("hero".into()) }).unwrap();
    ed.doc_mut().proposal_apply(id, Op::SetField { guid, component: BODY.into(), path: "pos".into(), value: vec2(x, y) }).unwrap();
    id
}

fn pos_in(view: &orr_edit::View<'_>, guid: &orr_reflect::Guid) -> Value {
    view.field(&Target::Guid(guid.clone()), BODY, "pos").unwrap()
}

#[test]
fn a_proposal_made_through_the_doc_appears_in_the_panel_list() {
    let mut ed = demo_editor();
    assert!(ed.doc().list_proposals().is_empty());
    let id = rename_and_move(&mut ed, 2, 30);
    let list = ed.doc().list_proposals();
    assert_eq!(list.len(), 1);
    let info = &list[0];
    assert_eq!((info.id, info.label.as_str(), info.op_count, info.stale), (id, "rename+move", 2, false));
    assert_eq!(info.origin, agent());
    let diff = ed.doc().proposal_diff(id).unwrap();
    assert_eq!(summary_line(&diff.summary), "~ 1 rename, ~ 1 field");
    assert!(diff.text.contains("+    pos: [2, 30]") || diff.text.contains("+      pos: [2, 30]"), "{}", diff.text);
    // The person edits something else: the proposal is marked stale but stays.
    ed.select_named("body_06");
    assert!(ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(1)));
    assert!(ed.doc().list_proposals()[0].stale);
    // Unknown ids are refused by the preview.
    assert!(!ed.set_preview(Some(ProposalId(99))));
    assert_eq!(ed.previewing(), None);
    assert!(ed.status().is_some_and(|m| m.error));
    let _ = id;
}

#[test]
fn preview_switches_what_the_viewport_draws() {
    let mut ed = demo_editor();
    let guid = guid_named(&ed, "body_05");
    let id = rename_and_move(&mut ed, 2, 30);
    let before = pos_in(&ed.view(), &guid);
    assert_eq!(pos_in(&ed.viewport_view(), &guid), before, "no preview yet");

    assert!(ed.set_preview(Some(id)));
    assert_eq!(ed.previewing(), Some(id));
    assert_eq!(pos_in(&ed.viewport_view(), &guid), vec2(2, 30), "the viewport shows the staged frame");
    assert_eq!(pos_in(&ed.view(), &guid), before, "the document's own view is unchanged");
    assert_ne!(ed.viewport_view().checksum(), ed.view().checksum());

    // The render list marks the changed entity with the preview color and draws a ghost at the old place.
    let diff = ed.doc().proposal_diff(id).unwrap();
    let vp = (800, 600);
    let plain = viewport::build_list(&ed.view(), None, &ed.camera, vp);
    let marked = viewport::build_preview_list(&ed.viewport_view(), &ed.doc().view(), &diff.summary, None, &ed.camera, vp);
    let count = |l: &orr_render::RenderList, c: [f32; 4]| l.lines.iter().filter(|x| x.color == c).count();
    assert_eq!(count(&plain, viewport::PREVIEW_CHANGED), 0);
    assert!(count(&marked, viewport::PREVIEW_CHANGED) > 0, "changed entity outlined");
    assert!(count(&marked, viewport::PREVIEW_GHOST) > 0, "ghost of the old position");

    assert!(ed.set_preview(None));
    assert_eq!(pos_in(&ed.viewport_view(), &guid), before);

    // Only in edit mode: play refuses, and an active preview does not show in play.
    assert!(ed.set_preview(Some(id)));
    ed.step(1);
    assert_eq!(ed.previewing(), None);
    assert_eq!(ed.viewport_view().tick(), ed.view().tick());
    assert!(!ed.set_preview(Some(id)));
    assert!(ed.status().is_some_and(|m| m.error));
}

#[test]
fn preview_never_changes_the_document_or_the_history() {
    let mut ed = demo_editor();
    let original = ed.doc().to_yaml();
    let id = rename_and_move(&mut ed, 2, 30);
    assert!(ed.set_preview(Some(id)));
    assert_eq!(ed.doc().to_yaml(), original);
    assert!(ed.doc().history().is_empty());
    assert!(!ed.is_dirty());
    // A proposal that goes away (an agent rejects it) takes its preview with it.
    ed.doc_mut().reject(id).unwrap();
    ed.sanitize_selection();
    assert_eq!(ed.previewing(), None);
    assert_eq!(ed.agent().preview(), None);
}

#[test]
fn filters_hide_reads_by_default() {
    let f = FeedFilter::default();
    assert!(f.edits && f.proposals && f.verify && f.sim && !f.reads);
}

#[test]
fn script_commands_stage_and_preview_without_an_approval_step() {
    let mut ed = demo_editor();
    let original = ed.doc().to_yaml();
    let text = "
        propose --as claude wave tweak
        propose.rename last body_05 hero
        propose.set p1 body_05 orr_physics::Body pos 2 30
        preview p1
        agent_tab
    ";
    script::run_script(&mut ed, text).unwrap();
    let list = ed.doc().list_proposals();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].label.as_str(), list[0].op_count), ("wave tweak", 2));
    assert_eq!(list[0].origin, agent());
    assert_eq!(ed.previewing(), Some(list[0].id));
    assert!(ed.agent_mut().take_tab_request());
    assert_eq!(ed.doc().to_yaml(), original, "staging never touches the document");

    // The approval commands are gone.
    for gone in ["accept p1", "reject p1", "verify p1", "checks default"] {
        assert!(script::run_script(&mut ed, gone).is_err(), "{gone} should be an unknown command");
    }
    script::run_script(&mut ed, "preview off").unwrap();
    assert_eq!(ed.previewing(), None);
    assert!(script::run_script(&mut ed, "propose.set p9 body_05 orr_physics::Body pos 1 2").is_err());
}
