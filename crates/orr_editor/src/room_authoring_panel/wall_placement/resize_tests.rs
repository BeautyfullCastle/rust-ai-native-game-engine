use super::*;
use orr_fp::fp;

fn settle(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if !editor.sim().in_tx && !editor.history().in_tx && editor.room_camera_capture_coherent() {
            return;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
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
    let guid = editor
        .spawn_room_wall("resize subject", [fp!(5), fp!(5)], [fp!(0.5), fp!(0.75)])
        .unwrap();
    settle(&mut editor);
    let mut panel = Panel {
        open: true,
        ..Default::default()
    };
    panel.refresh_walls(&mut editor).unwrap();
    panel.select_wall(
        &panel
            .walls
            .iter()
            .find(|wall| wall.guid == guid)
            .unwrap()
            .clone(),
    );
    settle(&mut editor);
    (temporary, editor, panel)
}
fn corner(panel: &mut Panel, editor: &Editor, point: [f64; 2]) -> Result<(), String> {
    let Gesture::Resize { first } = panel.wall_placement.gesture else {
        panic!("resize armed")
    };
    panel.accept_resize_corner(editor, first, point)
}
fn camera() -> Camera3D {
    Camera3D::perspective([8.0, 12.0, 16.0], [0.0, 0.0, 0.0], 60.0)
}
fn click(point: [f32; 3]) -> ViewInput {
    ViewInput {
        pixel: camera().world_to_screen(point, (800, 600)),
        clicked: true,
        cancel: false,
    }
}
#[test]
fn two_corners_stage_both_arrays_without_host_or_vertical_changes() {
    let (_temp, editor, mut panel) = fixture();
    let before = panel.wall_pin(&editor).unwrap();
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    let history = editor.history().clone();
    assert!(panel.begin_wall_resize(&editor));
    assert!(panel.wall_placement_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 4.0])));
    assert_eq!(panel.wall_pin(&editor).unwrap().fields, before.fields);
    assert!(panel.wall_placement_busy());
    assert!(!panel.wall_apply_ready(&editor));
    assert!(panel.wall_placement_input(&editor, &camera(), (800, 600), click([7.0, 0.0, 6.0])));
    let after = panel.wall_pin(&editor).unwrap();
    assert_eq!(&after.fields[..3], &before.fields[..3]);
    assert_eq!(after.original, before.original);
    assert_eq!(after.guid, before.guid);
    assert_ne!(&after.fields[3..5], &before.fields[3..5]);
    assert_ne!(&after.fields[5..], &before.fields[5..]);
    assert!(panel.wall_apply_ready(&editor));
    assert_eq!(editor.sim().checksum, before.stamp.2);
    assert_eq!(editor.history(), &history);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
}
#[test]
fn reversed_corners_match_and_new_wall_grid_does_not_snap_resize() {
    let (_temp, editor, mut panel) = fixture();
    let original = panel
        .walls
        .iter()
        .find(|w| Some(&w.guid) == panel.selected_wall_guid.as_ref())
        .unwrap()
        .clone();
    let mut expected = None;
    for (a, b) in [
        ([4.125, 4.375], [6.625, 5.875]),
        ([6.625, 5.875], [4.125, 4.375]),
    ] {
        panel.select_wall(&original);
        panel.footprint_grid = super::super::wall_footprint::GridStep::Two;
        assert!(panel.begin_wall_resize(&editor));
        corner(&mut panel, &editor, a).unwrap();
        corner(&mut panel, &editor, b).unwrap();
        let values = (
            parse_position(&panel.selected_wall_center).unwrap(),
            parse_position(&panel.selected_wall_half_extents).unwrap(),
        );
        assert_eq!(values, ([fp!(5.375), fp!(5.125)], [fp!(1.25), fp!(0.75)]));
        if let Some(expected) = expected {
            assert_eq!(values, expected);
        }
        expected = Some(values);
    }
}
#[test]
fn invalid_second_corner_retains_first_and_fields_then_can_retry() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    let fields = panel.wall_pin(&editor).unwrap().fields;
    for point in [
        [4.0, 6.0],
        [4.000001, 6.0],
        [4.1, 6.0],
        [30.0, 6.0],
        [f64::NAN, 6.0],
        [f64::MAX, 6.0],
    ] {
        assert!(corner(&mut panel, &editor, point).is_err());
        assert_eq!(panel.wall_pin(&editor).unwrap().fields, fields);
        assert!(matches!(
            panel.wall_placement.gesture,
            Gesture::Resize {
                first: Some([4.0, 4.0])
            }
        ));
        assert!(panel.wall_placement_busy());
    }
    corner(&mut panel, &editor, [6.0, 6.0]).unwrap();
    assert!(panel.wall_apply_ready(&editor));
}
#[test]
fn cancel_and_draft_change_consume_click_and_preserve_fields() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    let fields = panel.wall_pin(&editor).unwrap().fields;
    assert!(panel.wall_placement_input(
        &editor,
        &camera(),
        (800, 600),
        ViewInput {
            cancel: true,
            ..click([6.0, 0.0, 6.0])
        }
    ));
    assert!(!panel.wall_placement_busy());
    assert_eq!(panel.wall_pin(&editor).unwrap().fields, fields);
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    panel.selected_wall_half_height.push('0');
    let changed = panel.wall_pin(&editor).unwrap().fields;
    assert!(panel.wall_placement_input(&editor, &camera(), (800, 600), click([6.0, 0.0, 6.0])));
    assert!(!panel.wall_placement_busy());
    assert_eq!(panel.wall_pin(&editor).unwrap().fields, changed);
}
#[test]
fn resize_modes_exclude_rearming_and_play_cancels() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_wall_resize(&editor));
    assert!(!panel.begin_wall_resize(&editor));
    assert!(!panel.begin_wall_placement(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    assert!(editor.start_play());
    editor.sync();
    panel.sync_wall_placement(&editor);
    assert!(!panel.wall_placement_busy());
    assert!(!panel.wall_apply_ready(&editor));
}
#[test]
fn apply_resize_preserves_identity_vertical_and_other_walls_as_one_edit() {
    let (_temp, mut editor, mut panel) = fixture();
    let before = panel.wall_pin(&editor).unwrap();
    let walls = panel.walls.clone();
    let history = editor.history().entries.len();
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    corner(&mut panel, &editor, [7.0, 6.0]).unwrap();
    panel.apply_selected_wall(&mut editor).unwrap();
    settle(&mut editor);
    let after = editor.read_room_walls().unwrap();
    assert_eq!(editor.history().entries.len(), history + 1);
    for wall in &after {
        let original = walls.iter().find(|w| w.guid == wall.guid).unwrap();
        if wall.guid == before.guid {
            assert_eq!(wall.name, original.name);
            assert_eq!(wall.center_y, original.center_y);
            assert_eq!(wall.half_height, original.half_height);
            assert_eq!(wall.center, [fp!(5.5), fp!(5)]);
            assert_eq!(wall.half_extents, [fp!(1.5), fp!(1)]);
        } else {
            assert_eq!(wall, original);
        }
    }
    assert!(!panel.wall_apply_ready(&editor));
}
#[test]
fn resize_apply_rejects_unpumped_name_change_even_with_same_checksum() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [4.0, 4.0]).unwrap();
    corner(&mut panel, &editor, [7.0, 6.0]).unwrap();
    let pin = panel.wall_pin(&editor).unwrap();
    editor
        .host_call(
            "world.rename",
            serde_json::json!({"entity":pin.guid,"name":"fresh host name"}),
        )
        .unwrap();
    let history = editor
        .host_call("history.list", serde_json::Value::Null)
        .unwrap();
    assert_eq!(editor.sim().checksum, pin.stamp.2);
    assert!(
        panel
            .apply_selected_wall(&mut editor)
            .unwrap_err()
            .contains("source changed")
    );
    settle(&mut editor);
    assert_eq!(
        editor
            .host_call("history.list", serde_json::Value::Null)
            .unwrap(),
        history
    );
    let after = editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|w| w.guid == pin.guid)
        .unwrap();
    assert_eq!(after.name, "fresh host name");
    assert_eq!(after.center, pin.original.center);
    assert_eq!(after.half_extents, pin.original.half_extents);
}
#[test]
fn overlapping_resize_rolls_back_geometry_and_history() {
    let (_temp, mut editor, mut panel) = fixture();
    let before = panel.walls.clone();
    let history = editor
        .host_call("history.list", serde_json::Value::Null)
        .unwrap();
    assert!(panel.begin_wall_resize(&editor));
    corner(&mut panel, &editor, [-1.0, -1.0]).unwrap();
    corner(&mut panel, &editor, [1.0, 1.0]).unwrap();
    assert!(panel.apply_selected_wall(&mut editor).is_err());
    settle(&mut editor);
    assert_eq!(editor.read_room_walls().unwrap(), before);
    assert_eq!(
        editor
            .host_call("history.list", serde_json::Value::Null)
            .unwrap(),
        history
    );
}
