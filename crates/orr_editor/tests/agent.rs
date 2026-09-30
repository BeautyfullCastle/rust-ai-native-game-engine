//! The Agent tab's model: proposals staged on the host (as an ERP agent
//! does), the view-only preview (the staged frame comes from the host as a
//! second frame stream), the feed filters and the script commands. No
//! window, no GPU. (The feed itself, fed by a real agent over a socket, is
//! tested in `erp.rs` and `agent_ui.rs`.)
#![allow(clippy::disallowed_types)]

mod common;

use common::*;
use orr_editor::agent::{summary_line, FeedFilter};
use orr_editor::editor::{Editor, Owner};
use orr_editor::viewport;
use orr_editor::{script, Target};
use orr_reflect::Value;
use orr_remote::json::value_to_json;
use serde_json::json;

/// An agent proposal that renames body_05 and moves it to `(x, y)`, staged by a client named `claude`.
fn rename_and_move(ed: &mut Editor, x: i32, y: i32) -> String {
    let guid = guid_named(ed, "body_05").to_string();
    let c = ed.agent_client("claude").unwrap();
    let id = c.call("proposal.begin", json!({"label": "rename+move"})).unwrap()["id"].as_str().unwrap().to_string();
    c.call(
        "proposal.apply",
        json!({"id": id, "ops": [
            {"op": "rename", "entity": guid, "name": "hero"},
            {"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": value_to_json(&vec2(x, y))},
        ]}),
    )
    .unwrap();
    ed.sync();
    id
}

fn count(l: &orr_render::RenderList, c: [f32; 4]) -> usize {
    l.lines.iter().filter(|x| x.color == c).count()
}

#[test]
fn a_proposal_made_on_the_host_appears_in_the_panel_list() {
    let mut ed = demo_editor();
    assert!(ed.proposals().is_empty());
    let id = rename_and_move(&mut ed, 2, 30);
    let list = ed.proposals();
    assert_eq!(list.len(), 1);
    let info = &list[0];
    assert_eq!((info.id.as_str(), info.label.as_str(), info.op_count, info.stale), (id.as_str(), "rename+move", 2, false));
    assert_eq!(info.origin, "agent:claude");
    let detail = ed.proposal_detail(&id).expect("the diff reached the editor");
    assert_eq!(summary_line(&detail.summary), "~ 1 rename, ~ 1 field");
    assert!(detail.diff.contains("+    pos: [2, 30]") || detail.diff.contains("+      pos: [2, 30]"), "{}", detail.diff);
    // The person edits something else: the proposal is marked stale but stays.
    ed.select_named("body_06");
    assert!(ed.set_field(&Owner::Component(BODY.into()), "pos.x", fixed(1)));
    ed.sync();
    assert!(ed.proposals()[0].stale);
    // Unknown ids are refused by the preview.
    assert!(!ed.set_preview(Some("p99".into())));
    assert_eq!(ed.previewing(), None);
    assert!(ed.status().is_some_and(|m| m.error));
}

#[test]
fn preview_switches_what_the_viewport_draws() {
    let mut ed = demo_editor();
    let guid = guid_named(&ed, "body_05");
    let t = Target::Guid(guid.clone());
    let id = rename_and_move(&mut ed, 2, 30);
    let before = xy(&field(&mut ed, &t, BODY, "pos"));
    let vp = (800, 600);
    let plain = ed.viewport_list(vp, &[]);
    assert_eq!(count(&plain, viewport::PREVIEW_CHANGED), 0);

    assert!(ed.set_preview(Some(id.clone())));
    ed.sync();
    assert_eq!(ed.previewing(), Some(id.as_str()));
    // The render list marks the changed entity with the preview color and draws a ghost at the old place.
    let marked = ed.viewport_list(vp, &[]);
    assert!(count(&marked, viewport::PREVIEW_CHANGED) > 0, "changed entity outlined");
    assert!(count(&marked, viewport::PREVIEW_GHOST) > 0, "ghost of the old position");
    // The document's own view is unchanged.
    assert_eq!(xy(&field(&mut ed, &t, BODY, "pos")), before);
    assert_eq!(ed.checksum(), doc_checksum(&mut ed));
    // The body is drawn at the staged place: a pick there finds it in the staged frame? (picking is on the document's frame).
    assert!(ed.pick(before).is_some());

    assert!(ed.set_preview(None));
    assert_eq!(count(&ed.viewport_list(vp, &[]), viewport::PREVIEW_CHANGED), 0);

    // Only in edit mode: play refuses, and an active preview does not show in play.
    assert!(ed.set_preview(Some(id.clone())));
    ed.step(1);
    assert_eq!(ed.previewing(), None);
    assert!(!ed.set_preview(Some(id)));
    assert!(ed.status().is_some_and(|m| m.error));
}

#[test]
fn the_staged_frame_really_comes_from_the_host() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    assert!(ed.preview_bodies().is_none());
    assert!(ed.set_preview(Some(id)));
    ed.sync();
    // The staged frame has a body at (2, 30); the document's own frame has none there.
    let at = |bodies: &[orr_sample::physics_view::BodyView]| bodies.iter().any(|b| (b.pos[0] - 2.0).abs() < 1e-3 && (b.pos[1] - 30.0).abs() < 1e-3);
    assert!(at(ed.preview_bodies().expect("the staged frame arrived")), "staged frame");
    assert!(!at(ed.bodies()), "the document's frame is unchanged");
}

#[test]
fn preview_never_changes_the_document_or_the_history() {
    let mut ed = demo_editor();
    let original = scene_text(&mut ed);
    let id = rename_and_move(&mut ed, 2, 30);
    assert!(ed.set_preview(Some(id.clone())));
    ed.sync();
    assert_eq!(scene_text(&mut ed), original);
    assert!(ed.history().entries.is_empty());
    assert!(!ed.is_dirty());
    // A proposal that goes away (an agent rejects it) takes its preview with it.
    ed.agent_client("claude").unwrap().call("proposal.reject", json!({"id": id})).unwrap();
    ed.sync();
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
    let original = scene_text(&mut ed);
    let text = "
        propose --as claude wave tweak
        propose.rename last body_05 hero
        propose.set p1 body_05 orr_physics::Body pos 2 30
        preview p1
        agent_tab
    ";
    script::run_script(&mut ed, text).unwrap();
    let list = ed.proposals();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].label.as_str(), list[0].op_count), ("wave tweak", 2));
    assert_eq!(list[0].origin, "agent:claude");
    assert_eq!(ed.previewing(), Some(list[0].id.as_str()));
    assert!(ed.agent_mut().take_tab_request());
    assert_eq!(scene_text(&mut ed), original, "staging never touches the document");

    // The approval commands are gone.
    for gone in ["accept p1", "reject p1", "verify p1", "checks default"] {
        assert!(script::run_script(&mut ed, gone).is_err(), "{gone} should be an unknown command");
    }
    script::run_script(&mut ed, "preview off").unwrap();
    assert_eq!(ed.previewing(), None);
    assert!(script::run_script(&mut ed, "propose.set p9 body_05 orr_physics::Body pos 1 2").is_err());
    let _ = Value::Bool(true);
}
