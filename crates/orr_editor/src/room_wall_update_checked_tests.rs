use super::*;
use orr_fp::fp;

fn settle(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if !editor.sim().in_tx && !editor.history().in_tx && editor.room_camera_capture_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "settle Room transaction"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn fixture() -> (tempfile::TempDir, Editor, String) {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("room.scene.yaml");
    let text = include_str!("../tests/fixtures/room_initial.scene.yaml").to_owned();
    std::fs::write(&path, &text).unwrap();
    let mut editor = Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text,
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    settle(&mut editor);
    let guid = editor
        .spawn_room_wall("move subject", [fp!(5), fp!(5)], [fp!(0.5), fp!(0.75)])
        .unwrap();
    settle(&mut editor);
    (temporary, editor, guid)
}
fn world(editor: &mut Editor) -> J {
    editor
        .host_call(
            "world.query",
            json!({"components":[ACTOR],"values":true,"limit":69}),
        )
        .unwrap()
}
#[test]
fn checked_move_changes_only_selected_xz_and_one_history_entry() {
    let (_temp, mut editor, guid) = fixture();
    let wall = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == guid)
        .unwrap();
    let before = world(&mut editor);
    let history = editor.history().entries.len();
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    let expected = editor.sim().checksum;
    editor
        .update_room_wall_checked(
            &guid,
            &wall.name,
            [fp!(6), fp!(4)],
            wall.half_extents,
            expected,
            &wall,
        )
        .unwrap();
    settle(&mut editor);
    let mut after = world(&mut editor);
    let old = before["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["guid"].as_str() == Some(&guid))
        .unwrap();
    let changed = after["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["guid"].as_str() == Some(&guid))
        .unwrap();
    assert_eq!(
        changed["values"][BODY]["pos"]["y"],
        old["values"][BODY]["pos"]["y"]
    );
    assert_ne!(changed["values"][BODY]["pos"], old["values"][BODY]["pos"]);
    changed["values"][BODY]["pos"] = old["values"][BODY]["pos"].clone();
    assert_eq!(after["entities"], before["entities"]);
    assert_eq!(editor.history().entries.len(), history + 1);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
    let moved = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == guid)
        .unwrap();
    assert_eq!(moved.center, [fp!(6), fp!(4)]);
    assert_eq!(
        (
            moved.guid,
            moved.name,
            moved.half_extents,
            moved.center_y,
            moved.half_height
        ),
        (
            wall.guid,
            wall.name,
            wall.half_extents,
            wall.center_y,
            wall.half_height
        )
    );
}
#[test]
fn unpumped_host_change_is_refused_without_overwrite_or_history() {
    let (_temp, mut editor, guid) = fixture();
    let wall = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == guid)
        .unwrap();
    let expected = editor.sim().checksum;
    editor
        .host_call(
            "world.rename",
            json!({"entity":guid,"name":"new host name"}),
        )
        .unwrap();
    editor
        .host_call(
            "world.patch",
            json!({"entity":guid,"component":BODY,"path":"pos.x","value":6}),
        )
        .unwrap();
    assert_eq!(editor.sim().checksum, expected);
    let before = world(&mut editor);
    let history = editor.host_call("history.list", J::Null).unwrap();
    let error = editor
        .update_room_wall_checked(
            &guid,
            &wall.name,
            [fp!(7), fp!(4)],
            wall.half_extents,
            expected,
            &wall,
        )
        .unwrap_err();
    assert!(error.contains("source changed"), "{error}");
    settle(&mut editor);
    assert_eq!(world(&mut editor), before);
    assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
}
#[test]
fn invalid_checked_move_rolls_back_name_and_all_geometry() {
    let (_temp, mut editor, guid) = fixture();
    let wall = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == guid)
        .unwrap();
    let before = world(&mut editor);
    let history = editor.host_call("history.list", J::Null).unwrap();
    let expected = editor.sim().checksum;
    assert!(editor
        .update_room_wall_checked(
            &guid,
            "must roll back",
            [fp!(100), fp!(5)],
            [fp!(0.5), fp!(0.75)],
            expected,
            &wall
        )
        .is_err());
    settle(&mut editor);
    assert_eq!(world(&mut editor), before);
    assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
}

#[test]
fn unpumped_name_only_change_is_refused_by_original_row_guard() {
    let (_temp, mut editor, guid) = fixture();
    let wall = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == guid)
        .unwrap();
    let expected = editor.sim().checksum;
    editor
        .host_call(
            "world.rename",
            json!({"entity":guid,"name":"new host name"}),
        )
        .unwrap();
    let fresh = editor.host_call("sim.state", J::Null).unwrap();
    assert_eq!(
        fresh
            .get("checksum")
            .and_then(orr_remote::wire::parse_checksum),
        Some(expected)
    );
    let before = world(&mut editor);
    let history = editor.host_call("history.list", J::Null).unwrap();
    let error = editor
        .update_room_wall_checked(
            &guid,
            &wall.name,
            [fp!(6), fp!(4)],
            wall.half_extents,
            expected,
            &wall,
        )
        .unwrap_err();
    assert!(error.contains("source changed"), "{error}");
    settle(&mut editor);
    assert_eq!(world(&mut editor), before);
    assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
}
