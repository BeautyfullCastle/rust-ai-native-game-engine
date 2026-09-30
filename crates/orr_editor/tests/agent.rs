//! The Agent panel's model: proposals staged in the shared `EditorDoc` (as an
//! ERP agent does), preview, background verification, accept, reject,
//! conflicts and the script commands. No window, no GPU.

mod common;

use common::*;
use orr_edit::{EditError, Op, Origin, ProposalId, Target, VerifyInputs, VerifyOptions};
use orr_editor::agent::{bot_input, summary_line, VerifySource, DEFAULT_CHECKS, SAMPLE_EVERY};
use orr_editor::editor::{Editor, Owner, PLAYERS};
use orr_editor::{script, viewport};
use orr_reflect::Value;
use orr_sample::physics_game::{PhysGame, PhysMetrics};
use orr_sim::PlayerSlot;

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
    // Selecting works and unknown ids are refused.
    ed.select_proposal(Some(id));
    assert_eq!(ed.agent().selected(), Some(id));
    ed.select_proposal(Some(ProposalId(99)));
    assert_eq!(ed.agent().selected(), None);
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
fn verify_runs_in_the_background_and_the_default_check_passes() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    assert_eq!(ed.agent().checks, DEFAULT_CHECKS);
    ed.start_verify(id, VerifySource::Bot(120)).unwrap();
    assert_eq!(ed.agent().running(), Some(id));
    assert!(ed.start_verify(id, VerifySource::Bot(120)).is_err(), "one job at a time");
    ed.wait_verify();
    assert_eq!(ed.agent().running(), None);
    let run = ed.verify_result().expect("a result");
    assert!(run.passed(), "{run:?}");
    let report = run.report.as_ref().unwrap();
    assert_eq!((report.ticks, report.start_tick), (120, 0));
    let lost = report.metric("lost_bodies").unwrap();
    assert_eq!(lost.candidate.max, orr_edit::MetricValue::Int(0));
    let outcome = run.outcome.as_ref().unwrap();
    assert_eq!(outcome.results.len(), 1);
    assert!(outcome.results[0].passed && outcome.results[0].reason.contains("lost_bodies"));
    // The editor's own job gives exactly the report of the library call on the document.
    let inputs = VerifyInputs::<PhysGame>::scripted(120, PLAYERS, bot_input);
    let opts = VerifyOptions { sample_every: SAMPLE_EVERY, ..VerifyOptions::default() };
    let direct = ed.doc().verify_proposal::<PhysGame>(id, &inputs, &PhysMetrics, &opts).unwrap();
    assert_eq!(*report, direct);
    // The document is untouched and the proposal is still open.
    assert!(ed.doc().history().is_empty());
    assert_eq!(ed.doc().list_proposals().len(), 1);
}

#[test]
fn a_body_moved_below_the_floor_fails_the_check_with_a_reason() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 0, -10);
    ed.start_verify(id, VerifySource::Bot(60)).unwrap();
    ed.wait_verify();
    let run = ed.verify_result().unwrap();
    assert!(!run.passed());
    let r = &run.outcome.as_ref().unwrap().results[0];
    assert!(!r.passed);
    assert_eq!(r.check, "lost_bodies.max == 0");
    assert!(r.reason.contains("lost_bodies.max is 1"), "{}", r.reason);
    // Base side lost nothing: the difference is the proposal's.
    let lost = run.report.as_ref().unwrap().metric("lost_bodies").unwrap();
    assert_eq!(lost.base.max, orr_edit::MetricValue::Int(0));
    assert!(lost.delta != orr_edit::MetricValue::Int(0));
}

#[test]
fn checks_text_is_parsed_and_bad_lines_are_refused() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    ed.agent_mut().checks = "lost_bodies.max == 0\n\n# comment\ndynamic_bodies.delta == 0\n".into();
    ed.start_verify(id, VerifySource::Bot(30)).unwrap();
    ed.wait_verify();
    let run = ed.verify_result().unwrap();
    assert_eq!(run.checks.len(), 2);
    assert!(run.passed(), "{:?}", run.outcome);
    ed.agent_mut().checks = "this is nonsense".into();
    assert!(ed.start_verify(id, VerifySource::Bot(30)).is_err());
    assert!(ed.status().is_some_and(|m| m.error));
    assert_eq!(ed.agent().running(), None);
}

#[test]
fn last_play_is_a_source_once_a_play_was_stopped() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    assert!(ed.start_verify(id, VerifySource::LastPlay).is_err(), "no play recorded yet");
    ed.step(45);
    ed.stop();
    ed.start_verify(id, VerifySource::LastPlay).unwrap();
    ed.wait_verify();
    let run = ed.verify_result().unwrap();
    let report = run.report.as_ref().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.ticks, 45);
    assert!(run.passed());
    assert_eq!(run.source, VerifySource::LastPlay);
}

#[test]
fn bot_input_is_a_pure_function() {
    for t in [1, 2, 45, 46, 300, 12345] {
        for s in 0..2 {
            assert_eq!(bot_input(t, PlayerSlot(s)), bot_input(t, PlayerSlot(s)));
        }
    }
    assert_ne!(bot_input(1, PlayerSlot(0)), bot_input(50, PlayerSlot(0)));
}

#[test]
fn accept_is_one_agent_history_entry_and_undo_restores_exact_yaml() {
    let mut ed = demo_editor();
    let original = ed.doc().to_yaml();
    let checksum = ed.checksum();
    let id = rename_and_move(&mut ed, 2, 30);
    ed.select_proposal(Some(id));
    ed.set_preview(Some(id));
    let accepted = ed.accept_proposal(id).unwrap();
    assert!(accepted.history_id.is_some());
    let h = ed.doc().history();
    assert_eq!(h.len(), 1);
    assert_eq!((h[0].label.as_str(), h[0].op_count, h[0].undone), ("rename+move", 2, false));
    assert_eq!(h[0].origin, agent());
    assert_eq!(h[0].origin.to_string(), "agent:claude");
    assert!(ed.doc().list_proposals().is_empty());
    assert_eq!((ed.agent().selected(), ed.agent().preview()), (None, None), "panel state drops the accepted proposal");
    assert!(ed.is_dirty());
    assert_ne!(ed.doc().to_yaml(), original);
    assert!(ed.doc().to_yaml().contains("hero"));
    // Ctrl+Z is `undo`: one step takes it all back.
    assert!(ed.undo());
    assert_eq!(ed.doc().to_yaml(), original);
    assert_eq!(ed.checksum(), checksum);
}

#[test]
fn reject_discards_and_changes_nothing() {
    let mut ed = demo_editor();
    let original = ed.doc().to_yaml();
    let id = rename_and_move(&mut ed, 2, 30);
    ed.set_preview(Some(id));
    ed.reject_proposal(id).unwrap();
    assert!(ed.doc().list_proposals().is_empty());
    assert_eq!(ed.previewing(), None);
    assert_eq!(ed.doc().to_yaml(), original);
    assert!(ed.doc().history().is_empty());
    assert!(matches!(ed.reject_proposal(id), Err(EditError::UnknownProposal(_))));
}

#[test]
fn a_conflict_names_the_op_and_changes_nothing() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    // The person deletes the entity the proposal edits.
    ed.select_named("body_05");
    assert!(ed.delete_selected());
    let after_delete = ed.doc().to_yaml();
    let err = ed.accept_proposal(id).unwrap_err();
    assert!(matches!(err, EditError::ProposalConflict { op_index: 0, .. }), "{err}");
    let c = ed.agent().conflict().expect("the panel has a conflict to show");
    assert_eq!((c.proposal, c.op_index), (id, 0));
    assert!(c.op.starts_with("rename "), "{}", c.op);
    assert!(!c.cause.is_empty());
    assert!(ed.status().is_some_and(|m| m.error && m.text.contains("conflicts")));
    assert_eq!(ed.doc().to_yaml(), after_delete, "all or nothing");
    assert_eq!(ed.doc().history().len(), 1, "only the person's delete");
    assert_eq!(ed.doc().list_proposals().len(), 1, "the proposal stays");
    // Verify reports the conflict too, instead of running.
    assert!(ed.start_verify(id, VerifySource::Bot(10)).is_err());
    // Selecting again clears the note; rejecting the proposal removes it.
    ed.reject_proposal(id).unwrap();
    assert!(ed.agent().conflict().is_none());
}

#[test]
fn accept_is_refused_in_play_mode() {
    let mut ed = demo_editor();
    let id = rename_and_move(&mut ed, 2, 30);
    ed.step(1);
    assert!(ed.accept_proposal(id).is_err());
    assert_eq!(ed.doc().list_proposals().len(), 1);
    ed.stop();
    assert!(ed.accept_proposal(id).is_ok());
}

#[test]
fn script_commands_drive_the_whole_flow() {
    let mut ed = demo_editor();
    let original = ed.doc().to_yaml();
    let text = "
        propose --as claude wave tweak
        propose.rename last body_05 hero
        propose.set p1 body_05 orr_physics::Body pos 2 30
        checks lost_bodies.max == 0; dynamic_bodies.delta == 0
        verify last bot 60
        preview p1
        agent_tab
    ";
    script::run_script(&mut ed, text).unwrap();
    let list = ed.doc().list_proposals();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].label.as_str(), list[0].op_count), ("wave tweak", 2));
    assert_eq!(list[0].origin, agent());
    assert_eq!(ed.agent().selected(), Some(list[0].id));
    assert_eq!(ed.previewing(), Some(list[0].id));
    assert!(ed.verify_result().unwrap().passed());
    assert_eq!(ed.agent().checks.lines().count(), 2);
    assert!(ed.agent_mut().take_tab_request());

    script::run_script(&mut ed, "accept p1").unwrap();
    assert_eq!(ed.doc().history().len(), 1);
    script::run_script(&mut ed, "undo").unwrap();
    assert_eq!(ed.doc().to_yaml(), original);

    script::run_script(&mut ed, "propose second\npreview last\nreject last\n").unwrap();
    assert!(ed.doc().list_proposals().is_empty());
    assert!(script::run_script(&mut ed, "accept p9").is_err());
    assert!(script::run_script(&mut ed, "verify last").is_err(), "no proposals left");
    assert!(script::run_script(&mut ed, "propose.set p1 body_05 orr_physics::Body pos 1 2").is_err());
}
