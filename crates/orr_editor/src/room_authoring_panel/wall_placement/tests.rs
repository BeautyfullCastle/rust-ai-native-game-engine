use super::*;

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
    editor.sync();
    let mut panel = Panel {
        open: true,
        wall_name: "kept name".into(),
        ..Default::default()
    };
    panel.refresh_walls(&mut editor).unwrap();
    panel.select_wall(&panel.walls[0].clone());
    editor.sync();
    (temporary, editor, panel)
}
fn camera() -> Camera3D {
    Camera3D::perspective([8.0, 12.0, 16.0], [0.0, 0.0, 0.0], 60.0)
}
fn click(world: [f32; 3]) -> ViewInput {
    ViewInput {
        pixel: camera().world_to_screen(world, (800, 600)),
        clicked: true,
        cancel: false,
    }
}
#[test]
fn ray_and_numeric_envelope_reject_invalid_points() {
    let top = Camera3D::perspective([0.0, 20.0, 20.0], [0.0, 0.0, 0.0], 90.0);
    for x in [-16.1, 16.1] {
        let pixel = top.world_to_screen([x, 0.0, 0.0], (800, 600)).unwrap();
        assert!(ground_position(&top, (800, 600), Some(pixel)).is_err());
    }
    assert!(ground_position(&camera(), (0, 600), Some([1.0, 1.0])).is_err());
    for pixel in [
        None,
        Some([f32::NAN, 0.0]),
        Some([-1.0, 0.0]),
        Some([800.0, 0.0]),
    ] {
        assert!(ground_position(&camera(), (800, 600), pixel).is_err());
    }
    let parallel = Camera3D::perspective([0.0, 1.0, 0.0], [0.0, 1.0, -1.0], 60.0);
    assert!(ground_position(&parallel, (800, 600), Some([400.0, 300.0])).is_err());
}

#[test]
fn current_read_required_and_click_changes_only_xz() {
    let (_temp, editor, mut panel) = fixture();
    let before = panel.wall_pin(&editor).unwrap();
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    let history = editor.history().entries.len();
    assert!(panel.begin_wall_placement(&editor));
    assert!(!panel.wall_apply_ready(&editor));
    assert!(panel.wall_placement_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 3.0])));
    assert!(!panel.wall_placement_busy());
    let after = panel.wall_pin(&editor).unwrap();
    for i in [0, 1, 2, 5, 6] {
        assert_eq!(before.fields[i], after.fields[i]);
    }
    assert_ne!(&before.fields[3..5], &after.fields[3..5]);
    assert_eq!(before.original, after.original);
    assert!(panel.wall_apply_ready(&editor));
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(editor.sim().checksum, before.stamp.2);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
    panel.walls_current = false;
    assert!(!panel.begin_wall_placement(&editor));
}

#[test]
fn cancel_consumes_click_and_preserves_all_fields() {
    let (_temp, editor, mut panel) = fixture();
    let before = panel.wall_pin(&editor).unwrap();
    assert!(panel.begin_wall_placement(&editor));
    assert!(panel.wall_placement_input(
        &editor,
        &camera(),
        (800, 600),
        ViewInput {
            cancel: true,
            ..click([3.0, 0.0, 3.0])
        }
    ));
    assert!(!panel.wall_placement_busy());
    assert_eq!(panel.wall_pin(&editor).unwrap().fields, before.fields);
}

#[test]
fn changed_draft_cancels_pending_without_applying_click() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_wall_placement(&editor));
    panel.selected_wall_half_height.push('0');
    let before = panel.wall_pin(&editor).unwrap().fields;
    assert!(panel.wall_placement_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 3.0])));
    assert!(!panel.wall_placement_busy());
    assert_eq!(panel.wall_pin(&editor).unwrap().fields, before);
}

#[test]
fn changed_staged_fields_require_read_again() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_wall_placement(&editor));
    panel.wall_placement_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 3.0]));
    panel.selected_wall_name.push('x');
    panel.sync_wall_placement(&editor);
    assert!(!panel.walls_current);
    assert!(!panel.wall_apply_ready(&editor));
}

#[test]
fn changed_scene_stamp_cancels_and_blocks_apply() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_wall_placement(&editor));
    panel.wall_placement_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 3.0]));
    panel.wall_source_stamp.as_mut().unwrap().1 += 1;
    assert!(panel.apply_selected_wall(&mut editor).is_err());
    assert!(!panel.walls_current);
}

#[test]
fn active_picker_cannot_rearm_and_closed_panel_cancels() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_wall_placement(&editor));
    assert!(!panel.begin_wall_placement(&editor));
    panel.open = false;
    panel.sync_wall_placement(&editor);
    assert!(!panel.wall_placement_busy());
}

#[test]
fn play_cancels_pending_and_blocks_apply() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_wall_placement(&editor));
    assert!(editor.start_play());
    editor.sync();
    panel.sync_wall_placement(&editor);
    assert!(!panel.wall_placement_busy());
    assert!(!panel.wall_apply_ready(&editor));
}

#[test]
fn unrelated_numeric_drafts_refuse_arming_without_erasing_them() {
    let (_temp, editor, mut panel) = fixture();
    for index in 0..4 {
        panel.select_wall(&panel.walls[0].clone());
        match index {
            0 => panel.selected_wall_name.push('x'),
            1 => panel.selected_wall_half_extents[0] = "2".into(),
            2 => panel.selected_wall_center_y = "2".into(),
            _ => panel.selected_wall_half_height = "3".into(),
        }
        let fields = panel.wall_pin(&editor).unwrap().fields;
        assert!(!panel.begin_wall_placement(&editor));
        assert_eq!(panel.wall_pin(&editor).unwrap().fields, fields);
    }
}
