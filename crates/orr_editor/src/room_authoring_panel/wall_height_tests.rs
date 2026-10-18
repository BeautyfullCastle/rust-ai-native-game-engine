use super::*;
use orr_fp::fp;
fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    use orr_sample::room_game::{RoomConfig, RoomEscapeV1};
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("room.scene.yaml");
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
    let text = orr_reflect::Scene::unbake(&orr_sample::room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    std::fs::write(&path, &text).unwrap();
    let mut editor = Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text,
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    settle(&mut editor);
    let panel = Panel {
        open: true,
        wall_name: "kept name".into(),
        ..Default::default()
    };
    (temporary, editor, panel)
}

fn selected() -> (tempfile::TempDir, Editor, Panel, String) {
    let (temp, mut editor, mut panel) = fixture();
    let guid = editor
        .spawn_room_wall("height subject", [fp!(5), fp!(5)], [fp!(0.5), fp!(0.75)])
        .unwrap();
    settle(&mut editor);
    panel.refresh_walls(&mut editor).unwrap();
    let wall = panel
        .walls
        .iter()
        .find(|wall| wall.guid == guid)
        .unwrap()
        .clone();
    panel.select_wall(&wall);
    (temp, editor, panel, guid)
}
fn world(editor: &mut Editor) -> serde_json::Value {
    editor.host_call("world.query",serde_json::json!({"components":[orr_sample::room_project::ACTOR],"values":true,"limit":69})).unwrap()
}
#[test]
fn height_changes_only_two_fields_in_one_undoable_edit_and_preserves_drafts() {
    let (_temp, mut editor, mut panel, guid) = selected();
    let checksum = editor.sim().checksum;
    let history = editor.history().entries.len();
    let before = world(&mut editor);
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    panel.selected_wall_name = "unapplied name".into();
    panel.selected_wall_center[0] = "9.000".into();
    panel.selected_wall_center_y = "1.5".into();
    panel.selected_wall_half_height = "2".into();
    panel.apply_wall_height(&mut editor).unwrap();
    settle(&mut editor);
    assert_eq!(panel.selected_wall_name, "unapplied name");
    assert_eq!(panel.selected_wall_center[0], "9.000");
    let mut after = world(&mut editor);
    let old = before["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["guid"].as_str() == Some(&guid))
        .unwrap();
    let changed = after["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entity| entity["guid"].as_str() == Some(&guid))
        .unwrap();
    assert_ne!(
        changed["values"]["orr_physics3d::Body"]["pos"],
        old["values"]["orr_physics3d::Body"]["pos"]
    );
    changed["values"]["orr_physics3d::Body"]["pos"] =
        old["values"]["orr_physics3d::Body"]["pos"].clone();
    changed["values"]["orr_physics3d::Collider"]["shape"]["half_extents"] =
        old["values"]["orr_physics3d::Collider"]["shape"]["half_extents"].clone();
    assert_eq!(after["entities"], before["entities"]);
    let wall = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|wall| wall.guid == guid)
        .unwrap();
    assert_eq!((wall.center_y, wall.half_height), (fp!(1.5), fp!(2)));
    assert_eq!(wall.center, [fp!(5), fp!(5)]);
    assert_eq!(wall.half_extents, [fp!(0.5), fp!(0.75)]);
    assert_eq!(editor.history().entries.len(), history + 1);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
    assert!(editor.undo());
    settle(&mut editor);
    assert_eq!(editor.sim().checksum, checksum);
    assert!(editor.redo());
    settle(&mut editor);
    assert_ne!(editor.sim().checksum, checksum);
}
#[test]
fn invalid_envelope_and_nonspanning_wall_roll_back_both_fields() {
    let (_temp, mut editor, mut panel, _guid) = selected();
    for (center, half) in [
        ("5", "2"),
        ("0.5", "9"),
        ("0.5", "0.01"),
        ("3", "0.5"),
        ("-3", "0.5"),
        ("nan", "1"),
    ] {
        panel.refresh_walls(&mut editor).unwrap();
        settle(&mut editor);
        let checksum = editor.sim().checksum;
        let history = editor
            .host_call("history.list", serde_json::Value::Null)
            .unwrap();
        let before = world(&mut editor);
        panel.selected_wall_center_y = center.into();
        panel.selected_wall_half_height = half.into();
        assert!(
            panel.apply_wall_height(&mut editor).is_err(),
            "{center}/{half}"
        );
        settle(&mut editor);
        assert_eq!(editor.sim().checksum, checksum);
        assert_eq!(world(&mut editor), before);
        assert_eq!(
            editor
                .host_call("history.list", serde_json::Value::Null)
                .unwrap(),
            history
        );
    }
}
#[test]
fn stale_unpumped_host_edit_rejected_inside_transaction() {
    let (_temp, mut editor, mut panel, guid) = selected();
    let cached = editor.sim().checksum;
    editor
        .host_call(
            "world.rename",
            serde_json::json!({"entity":guid,"name":"new authoritative name"}),
        )
        .unwrap();
    // Name-only edits may retain the Frame checksum: mutate a body as well.
    editor.host_call("world.patch",serde_json::json!({"entity":guid,"component":"orr_physics3d::Body","path":"pos.x","value":6})).unwrap();
    assert_eq!(editor.sim().checksum, cached);
    let before = world(&mut editor);
    let history = editor
        .host_call("history.list", serde_json::Value::Null)
        .unwrap();
    panel.selected_wall_center_y = "1.5".into();
    panel.selected_wall_half_height = "2".into();
    assert!(panel.apply_wall_height(&mut editor).is_err());
    settle(&mut editor);
    assert_eq!(world(&mut editor), before);
    assert_eq!(
        editor
            .host_call("history.list", serde_json::Value::Null)
            .unwrap(),
        history
    );
    assert!(!panel.walls_current);
}
#[test]
fn missing_read_and_reopened_identity_refuse_before_write() {
    let (_temp, mut editor, mut panel, _guid) = selected();
    let before = world(&mut editor);
    panel.wall_source_stamp.as_mut().unwrap().1 += 1;
    assert!(panel.apply_wall_height(&mut editor).is_err());
    panel.wall_source_stamp = None;
    assert!(panel.apply_wall_height(&mut editor).is_err());
    assert_eq!(world(&mut editor), before);
}
#[test]
fn duplicate_preserves_new_vertical_dimensions() {
    let (_temp, mut editor, mut panel, guid) = selected();
    panel.selected_wall_center_y = "1.5".into();
    panel.selected_wall_half_height = "2".into();
    panel.apply_wall_height(&mut editor).unwrap();
    settle(&mut editor);
    let duplicate = editor
        .duplicate_room_wall(&guid, "height copy", [fp!(7), fp!(5)])
        .unwrap();
    settle(&mut editor);
    let copy = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|wall| wall.guid == duplicate)
        .unwrap();
    assert_eq!((copy.center_y, copy.half_height), (fp!(1.5), fp!(2)));
    assert_eq!(copy.center, [fp!(7), fp!(5)]);
}

#[test]
fn shrinking_height_moves_center_before_reducing_extent() {
    let (_temp, mut editor, mut panel, _guid) = selected();
    panel.selected_wall_center_y = "1.5".into();
    panel.selected_wall_half_height = "2".into();
    panel.apply_wall_height(&mut editor).unwrap();
    assert!(!panel.walls_current);
    assert!(panel.apply_wall_height(&mut editor).is_err()); // Read is mandatory.
    settle(&mut editor);
    panel.selected_wall_center_y = "0.5".into();
    panel.selected_wall_half_height = "0.5".into();
    panel.refresh_walls(&mut editor).unwrap();
    panel.selected_wall_center_y = "0.5".into();
    panel.selected_wall_half_height = "0.5".into();
    panel.apply_wall_height(&mut editor).unwrap();
    settle(&mut editor);
    assert!(!panel.walls_current);
    let walls = editor.read_room_walls().unwrap();
    let wall = walls
        .iter()
        .find(|wall| Some(&wall.guid) == panel.selected_wall_guid.as_ref())
        .unwrap();
    assert_eq!((wall.center_y, wall.half_height), (fp!(0.5), fp!(0.5)));
}

fn settle(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if !editor.sim().in_tx && !editor.history().in_tx && editor.room_camera_capture_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "settle completed Room transaction before a new action"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
