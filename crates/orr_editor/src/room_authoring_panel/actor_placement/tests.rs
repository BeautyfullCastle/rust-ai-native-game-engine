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
    editor.sync();
    let panel = Panel {
        open: true,
        wall_name: "kept name".into(),
        ..Default::default()
    };
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
fn successful_current_read_required_and_each_role_changes_only_two_strings() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(!panel.begin_actor_placement(&editor, 0));
    assert!(!panel.layout_apply_ready(&editor));
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    let checksum = editor.sim().checksum;
    let history = editor.history().entries.len();
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    for role in 0..3 {
        let before = panel.layout_fields();
        assert!(panel.begin_actor_placement(&editor, role));
        assert!(!panel.layout_apply_ready(&editor));
        assert!(panel.actor_placement_input(
            &editor,
            &camera(),
            (800, 600),
            click([1.0 + role as f32, 0.0, 1.0])
        ));
        assert!(!panel.actor_placement_busy());
        for (index, fields) in panel.layout_fields().iter().enumerate() {
            if index != role {
                assert_eq!(fields, &before[index]);
            }
        }
        assert!(panel.layout_apply_ready(&editor));
    }
    assert_eq!(editor.sim().checksum, checksum);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
}
#[test]
fn stale_source_after_staging_blocks_apply_until_read() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    assert!(panel.begin_actor_placement(&editor, 1));
    panel.actor_placement_input(&editor, &camera(), (800, 600), click([1.0, 0.0, 1.0]));
    editor
        .spawn_room_wall("elsewhere", [fp!(10), fp!(10)], [fp!(0.5), fp!(0.5)])
        .unwrap();
    editor.sync();
    let checksum = editor.sim().checksum;
    assert!(
        panel
            .apply_actor_layout(&mut editor, [[FP::ZERO; 2]; 3])
            .is_err()
    );
    assert_eq!(editor.sim().checksum, checksum);
    assert!(!panel.begin_actor_placement(&editor, 0));
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    assert!(panel.begin_actor_placement(&editor, 0));
}
#[test]
fn draft_scene_epoch_and_path_changes_cancel_and_consume() {
    let (_temp, mut editor, mut panel) = fixture();
    for kind in 0..4 {
        panel.read_actor_layout(&mut editor).unwrap();
        editor.sync();
        assert!(panel.begin_actor_placement(&editor, 1));
        match kind {
            0 => panel.player[0].push('0'),
            1 => panel.actor_placement.source.as_mut().unwrap().epoch += 1,
            2 => {
                panel.actor_placement.source.as_mut().unwrap().scene =
                    PathBuf::from("other.scene.yaml")
            }
            _ => panel.actor_placement.source.as_mut().unwrap().checksum ^= 1,
        }
        let before = panel.layout_fields();
        assert!(panel.actor_placement_input(
            &editor,
            &camera(),
            (800, 600),
            click([2.0, 0.0, 1.0])
        ));
        assert!(!panel.actor_placement_busy());
        assert_eq!(panel.layout_fields(), before);
        if kind != 0 {
            assert!(!panel.layout_apply_ready(&editor));
        }
    }
}
#[test]
fn cancel_and_play_preserve_fields_and_require_new_read_after_play() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    let before = panel.layout_fields();
    assert!(panel.begin_actor_placement(&editor, 0));
    assert!(panel.actor_placement_input(
        &editor,
        &camera(),
        (800, 600),
        ViewInput {
            cancel: true,
            ..Default::default()
        }
    ));
    assert_eq!(panel.layout_fields(), before);
    assert!(panel.begin_actor_placement(&editor, 2));
    assert!(editor.start_play());
    editor.sync();
    panel.sync_actor_placement(&editor);
    assert!(!panel.actor_placement_busy());
    assert!(!panel.layout_apply_ready(&editor));
    assert_eq!(panel.layout_fields(), before);
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
fn apply_is_one_atomic_edit_and_overlap_rolls_back_every_role() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    let before = editor.sim().checksum;
    let history = editor.history().entries.len();
    let original = editor.read_room_layout().unwrap();
    let positions = [[fp!(0), fp!(0)], [fp!(1), fp!(0)], [fp!(2), fp!(0)]];
    panel.apply_actor_layout(&mut editor, positions).unwrap();
    editor.sync();
    assert_eq!(editor.history().entries.len(), history + 1);
    assert_eq!(editor.read_room_layout().unwrap(), positions);
    assert!(editor.undo());
    editor.sync();
    assert_eq!(editor.sim().checksum, before);
    assert_eq!(editor.read_room_layout().unwrap(), original);
    assert!(editor.redo());
    editor.sync();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    let before = editor.sim().checksum;
    let history = editor.history().entries.len();
    let wall = editor.read_room_walls().unwrap()[0].clone();
    editor.sync();
    let bad = [wall.center, [fp!(4), fp!(0)], [fp!(5), fp!(0)]];
    assert!(panel.apply_actor_layout(&mut editor, bad).is_err());
    editor.sync();
    assert_eq!(editor.sim().checksum, before);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(editor.read_room_layout().unwrap(), positions);
}
#[test]
fn active_actor_picker_cannot_rearm_or_apply() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    assert!(panel.begin_actor_placement(&editor, 0));
    assert!(!panel.begin_actor_placement(&editor, 1));
    assert!(!panel.layout_apply_ready(&editor));
}

#[test]
fn unpumped_host_change_is_rejected_inside_transaction_without_overwriting_roles() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.read_actor_layout(&mut editor).unwrap();
    editor.sync();
    let cached = editor.sim().checksum;
    let query=editor.host_call("world.query",serde_json::json!({"components":[orr_sample::room_project::ACTOR],"values":true,"limit":69})).unwrap();
    let key = query["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| {
            entity["values"][orr_sample::room_project::ACTOR]["kind"].as_u64() == Some(2)
        })
        .unwrap();
    let guid = key["guid"].as_str().unwrap();
    editor.host_call("world.patch",serde_json::json!({"entity":guid,"component":"orr_physics3d::Body","path":"pos.x","value":3})).unwrap();
    let after = editor
        .host_call("sim.state", serde_json::Value::Null)
        .unwrap();
    let changed = orr_remote::wire::parse_checksum(&after["checksum"]).unwrap();
    assert_ne!(changed, cached);
    assert_eq!(editor.sim().checksum, cached); // Deliberately do not pump the editor.
    let history = editor
        .host_call("history.list", serde_json::Value::Null)
        .unwrap();
    assert!(
        panel
            .apply_actor_layout(&mut editor, [[FP::ZERO; 2]; 3])
            .is_err()
    );
    let after_rejection = editor
        .host_call("sim.state", serde_json::Value::Null)
        .unwrap();
    assert_eq!(
        orr_remote::wire::parse_checksum(&after_rejection["checksum"]),
        Some(changed)
    );
    assert_eq!(
        editor
            .host_call("history.list", serde_json::Value::Null)
            .unwrap(),
        history
    );
    assert!(!panel.layout_apply_ready(&editor));
}
