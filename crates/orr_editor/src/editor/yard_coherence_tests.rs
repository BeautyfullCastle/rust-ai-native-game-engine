//! Pin the notification/row-reply ordering without relying on thread timing.

use super::*;

#[test]
fn delayed_history_notification_fences_picking_until_current_rows_reply() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml");
    let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
    editor.sync();
    let rows = editor.call("world.query", json!({"limit": ROWS_LIMIT})).unwrap();
    assert_eq!(
        rows.get("checksum").and_then(orr_remote::wire::parse_checksum),
        Some(editor.snapshot().unwrap().predicted().checksum())
    );

    // From here, drive only the production notification/answer handlers. These
    // flags model send_refreshes dispatching one rows request; no sleeps, host
    // mutations or further ERP ingestion can change the intended ordering.
    let old_generation = editor.change_gen;
    editor.dirty.rows = false;
    editor.inflight.rows = true;
    editor.on_answer(Pend::Rows(old_generation), Ok(rows.clone()));
    assert!(editor.yard_rows_coherent());
    let expected = editor.rows.iter().find(|row| row.name.as_deref() == Some("box_right"))
        .expect("fixture right box").target();
    assert!(matches!(expected, Target::Guid(_)));
    editor.camera3d = orr_render::OrbitCamera::new([2.0, 3.0, 0.0], 0.0, 0.0, 10.0);
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), Some(expected.clone()));
    let checksum = editor.checksum();
    let row_count = editor.rows.len();
    let item_count = editor.yard_frame.items.len();

    // A refresh was dispatched before the delayed notification arrived.
    editor.inflight.rows = true;
    assert!(!editor.yard_rows_coherent());
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), None);
    editor.on_notification("watch.history", &json!({
        "can_undo": false, "can_redo": false, "dirty": false, "in_tx": false
    }));
    assert!(editor.dirty.rows);
    assert_ne!(editor.change_gen, old_generation);
    assert!(!editor.yard_rows_coherent());
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), None);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.rows.len(), row_count);
    assert_eq!(editor.yard_frame.items.len(), item_count);

    // An old-generation answer with the correct checksum and entity count is
    // still stale; only the new-generation row refresh can release the fence.
    editor.on_answer(Pend::Rows(old_generation), Ok(rows.clone()));
    assert!(editor.dirty.rows);
    assert!(!editor.yard_rows_coherent());
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), None);

    editor.dirty.rows = false;
    editor.inflight.rows = true;
    assert!(!editor.yard_rows_coherent());
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), None);
    editor.on_answer(Pend::Rows(editor.change_gen), Ok(rows));
    assert!(editor.yard_rows_coherent());
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), Some(expected));
}
