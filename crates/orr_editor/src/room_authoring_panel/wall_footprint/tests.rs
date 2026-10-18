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
fn opposite_corners_are_order_independent_and_exact() {
    let expected = ([fp!(1), fp!(2)], [fp!(3), fp!(1)]);
    assert_eq!(rectangle([-2.0, 1.0], [4.0, 3.0]).unwrap(), expected);
    assert_eq!(rectangle([4.0, 3.0], [-2.0, 1.0]).unwrap(), expected);
    assert_eq!(rectangle([-2.0, 3.0], [4.0, 1.0]).unwrap(), expected);
}
#[test]
fn nonfinite_overflow_zero_and_subquantum_axes_are_rejected_without_clamp() {
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        -(i64::MIN as f64) / FP::SCALE as f64,
    ] {
        assert!(rectangle([bad, 1.0], [2.0, 3.0]).is_err());
    }
    for end in [[0.0, 1.0], [1.0, 0.0], [1.0 / FP::SCALE as f64, 1.0]] {
        assert!(rectangle([0.0, 0.0], end).is_err());
    }
    assert_eq!(
        checked_fixed(i64::MIN as f64 / FP::SCALE as f64).unwrap(),
        FP::MIN
    );
}
#[test]
fn perspective_and_orthographic_ground_hits_match_in_both_aspects() {
    let doc = orr_sample::room_camera::Document::readable_default();
    for size in [(800, 600), (600, 800)] {
        for cam in [camera(), doc.camera(&doc.orbit(), size).unwrap()] {
            let pixel = cam.world_to_screen([1.0, 0.0, 2.0], size).unwrap();
            let hit = ground_point(&cam, size, Some(pixel)).unwrap();
            assert!((hit[0] - 1.0).abs() < 0.0001 && (hit[1] - 2.0).abs() < 0.0001);
        }
    }
}
#[test]
fn invalid_viewport_parallel_and_behind_rays_are_rejected() {
    for pixel in [
        None,
        Some([f32::NAN, 1.0]),
        Some([-1.0, 3.0]),
        Some([800.0, 20.0]),
    ] {
        assert!(ground_point(&camera(), (800, 600), pixel).is_err());
    }
    assert!(ground_point(&camera(), (0, 600), Some([1.0, 1.0])).is_err());
    for target in [[0.0, 1.0, -1.0], [0.0, 2.0, -1.0]] {
        let cam = Camera3D::perspective([0.0, 1.0, 0.0], target, 60.0);
        assert!(ground_point(&cam, (800, 600), Some([400.0, 300.0])).is_err());
    }
}
#[test]
fn two_clicks_stage_only_create_is_one_authoritative_edit() {
    let (_temp, mut editor, mut panel) = fixture();
    let before = editor.sim().checksum;
    let history = editor.history().entries.len();
    let original = std::fs::read(editor.path().unwrap()).unwrap();
    let fields = (panel.wall_center.clone(), panel.wall_half_extents.clone());
    panel.begin_wall_footprint(&editor);
    assert!(panel.wall_footprint_busy());
    assert!(panel.wall_footprint_input(&editor, &camera(), (800, 600), click([1.0, 0.0, 1.0])));
    assert_eq!(
        (panel.wall_center.clone(), panel.wall_half_extents.clone()),
        fields
    );
    assert!(panel.wall_footprint_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 2.0])));
    assert!(!panel.wall_footprint_busy());
    assert_eq!(panel.wall_name, "kept name");
    assert_eq!(editor.sim().checksum, before);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), original);
    editor
        .spawn_room_wall(
            &panel.wall_name,
            super::super::parse_position(&panel.wall_center).unwrap(),
            super::super::parse_position(&panel.wall_half_extents).unwrap(),
        )
        .unwrap();
    editor.sync();
    assert_ne!(editor.sim().checksum, before);
    assert_eq!(editor.history().entries.len(), history + 1);
    assert!(editor.undo());
    editor.sync();
    assert_eq!(editor.sim().checksum, before);
    assert!(editor.redo());
    editor.sync();
    assert_ne!(editor.sim().checksum, before);
}
#[test]
fn invalid_second_corner_and_cancel_preserve_drafts_and_host() {
    let (_temp, editor, mut panel) = fixture();
    let before = (
        panel.wall_center.clone(),
        panel.wall_half_extents.clone(),
        editor.sim().checksum,
    );
    panel.begin_wall_footprint(&editor);
    panel.accept_corner([1.0, 2.0]).unwrap();
    assert!(panel.accept_corner([1.0, 3.0]).is_err());
    assert!(panel.wall_footprint_busy());
    assert!(panel.wall_footprint_input(
        &editor,
        &camera(),
        (800, 600),
        ViewInput {
            cancel: true,
            ..Default::default()
        }
    ));
    assert!(!panel.wall_footprint_busy());
    assert_eq!(
        (
            panel.wall_center,
            panel.wall_half_extents,
            editor.sim().checksum
        ),
        before
    );
}
#[test]
fn every_draft_identity_change_cancels_and_consumes_the_click() {
    let (_temp, editor, mut panel) = fixture();
    for which in 0..8 {
        panel.begin_wall_footprint(&editor);
        assert!(panel.wall_footprint_busy());
        match which {
            0 => panel.wall_name.push('x'),
            1 => panel.wall_center[0].push('0'),
            2 => panel.wall_half_extents[1].push('0'),
            3 => panel.footprint.context.as_mut().unwrap().scene = PathBuf::from("other"),
            4 => panel.footprint.context.as_mut().unwrap().epoch += 1,
            5 => panel.footprint.context.as_mut().unwrap().checksum ^= 1,
            6 => panel.wall_center[1].push('0'),
            _ => panel.wall_half_extents[0].push('0'),
        }
        assert!(panel.wall_footprint_input(&editor, &camera(), (800, 600), click([3.0, 0.0, 2.0])));
        assert!(!panel.wall_footprint_busy());
    }
    panel.begin_wall_footprint(&editor);
    panel.open = false;
    panel.sync_wall_footprint(&editor);
    assert!(!panel.wall_footprint_busy());
}
#[test]
fn play_transition_cancels_pending_footprint() {
    let (_temp, mut editor, mut panel) = fixture();
    panel.begin_wall_footprint(&editor);
    assert!(panel.wall_footprint_busy());
    assert!(editor.start_play());
    editor.sync();
    assert!(panel.wall_footprint_input(&editor, &camera(), (800, 600), ViewInput::default()));
    assert!(!panel.wall_footprint_busy());
}

#[test]
fn idle_cancel_preserves_create_status_and_draft() {
    let mut panel = Panel {
        status: "Created wall kept name.".into(),
        ..Default::default()
    };
    let before = (
        panel.status.clone(),
        panel.wall_name.clone(),
        panel.wall_center.clone(),
        panel.wall_half_extents.clone(),
    );
    panel.cancel_wall_footprint();
    assert_eq!(
        (
            panel.status,
            panel.wall_name,
            panel.wall_center,
            panel.wall_half_extents
        ),
        before
    );
}
