//! A delayed history push must not retire a draft after its authoritative
//! history reply has already been consumed. These tests order delivery exactly
//! instead of relying on the host's 25 ms notification timer.

use super::*;

fn history(undone: &[bool], dirty: bool, in_tx: bool) -> J {
    let entries: Vec<_> = undone
        .iter()
        .enumerate()
        .map(|(index, undone)| {
            json!({"id": index + 1, "label": "edit", "origin": "user", "op_count": 1, "undone": undone})
        })
        .collect();
    json!({
        "entries": entries,
        "can_undo": undone.iter().any(|undone| !undone),
        "can_redo": undone.iter().any(|undone| *undone),
        "dirty": dirty,
        "in_tx": in_tx,
    })
}

fn notification(reply: &J) -> J {
    json!({
        "len": reply["entries"].as_array().unwrap().len(),
        "last": reply["entries"].as_array().unwrap().last(),
        "can_undo": reply["can_undo"],
        "can_redo": reply["can_redo"],
        "dirty": reply["dirty"],
        "in_tx": reply["in_tx"],
    })
}

#[test]
fn delayed_duplicate_history_and_save_notifications_preserve_material_generation() {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    let edited = history(&[false], true, false);
    editor.on_answer(Pend::History, Ok(edited.clone()));
    let current = editor.material_authoring_generation();
    editor.on_notification("watch.history", &notification(&edited));
    editor.on_answer(Pend::History, Ok(edited.clone()));
    assert!(
        Arc::ptr_eq(&current, &editor.material_authoring_generation()),
        "a delayed duplicate must not stale a draft captured after the full reply"
    );

    let saved = history(&[false], false, false);
    editor.on_answer(Pend::History, Ok(saved.clone()));
    editor.on_notification("watch.history", &notification(&saved));
    editor.on_answer(Pend::History, Ok(saved));
    assert!(
        Arc::ptr_eq(&current, &editor.material_authoring_generation()),
        "saving the same scene only changes dirty; it is not a new scene edit"
    );
}

#[test]
fn full_history_transitions_retire_material_generation_even_with_equal_note_summary() {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    let before = history(&[false, false, true, true], true, false);
    let undone = history(&[false, true, true, true], true, false);
    assert_eq!(notification(&before), notification(&undone));
    editor.on_answer(Pend::History, Ok(before.clone()));
    let initial = editor.material_authoring_generation();
    editor.on_answer(Pend::History, Ok(undone));
    let after_undo = editor.material_authoring_generation();
    assert!(
        !Arc::ptr_eq(&initial, &after_undo),
        "full history entry revisions must fence middle-entry undo"
    );
    editor.on_answer(Pend::History, Ok(before));
    assert!(
        !Arc::ptr_eq(&initial, &editor.material_authoring_generation())
            && !Arc::ptr_eq(&after_undo, &editor.material_authoring_generation()),
        "redo to the old visible history must not revive an older draft"
    );
}

#[test]
fn transaction_history_reply_retires_material_generation_once() {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    editor.on_answer(Pend::History, Ok(history(&[false], true, false)));
    let before = editor.material_authoring_generation();
    let transaction = history(&[false], true, true);
    editor.on_answer(Pend::History, Ok(transaction.clone()));
    let active = editor.material_authoring_generation();
    assert!(!Arc::ptr_eq(&before, &active));
    editor.on_notification("watch.history", &notification(&transaction));
    editor.on_answer(Pend::History, Ok(transaction));
    assert!(Arc::ptr_eq(&active, &editor.material_authoring_generation()));
}

#[test]
fn history_notification_during_a_read_requires_a_fresh_reply_before_material_editing() {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    let before = history(&[false, false, true, true], true, false);
    let changed = history(&[false, true, true, true], true, false);
    editor.on_answer(Pend::History, Ok(before.clone()));
    editor.dirty.history = false;
    editor.inflight.history = true;
    assert!(!editor.material_history_ready());

    // The push arrives while the first read is in flight. Consuming that old
    // reply must leave another refresh due, even when its summary is equal.
    editor.on_notification("watch.history", &notification(&changed));
    editor.on_answer(Pend::History, Ok(before));
    assert!(!editor.material_history_ready());
    assert!(editor.dirty.history);
    let previous = editor.material_authoring_generation();

    // Model the next refresh being dispatched, then deliver its full result.
    editor.dirty.history = false;
    editor.inflight.history = true;
    assert!(!editor.material_history_ready());
    editor.on_answer(Pend::History, Ok(changed));
    assert!(editor.material_history_ready());
    assert!(!Arc::ptr_eq(&previous, &editor.material_authoring_generation()));
}

#[test]
fn failed_history_read_blocks_material_editing_and_cannot_revive_the_old_draft() {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    let before = history(&[false], true, false);
    editor.on_answer(Pend::History, Ok(before.clone()));
    editor.dirty.history = false;
    let old_draft = editor.material_authoring_generation();
    editor.inflight.history = true;
    editor.on_answer(
        Pend::History,
        Err(orr_remote::RpcError::state("history_unavailable", "injected read failure")),
    );
    assert!(!editor.material_history_ready());
    assert!(!Arc::ptr_eq(&old_draft, &editor.material_authoring_generation()));
    assert!(editor.status().is_some_and(|message| message.error));

    // A later successful refresh restores admission, but uncertainty must not
    // revive a draft from before the failed read even when history is equal.
    editor.inflight.history = true;
    editor.on_answer(Pend::History, Ok(before));
    assert!(editor.material_history_ready());
    assert!(!Arc::ptr_eq(&old_draft, &editor.material_authoring_generation()));
}
