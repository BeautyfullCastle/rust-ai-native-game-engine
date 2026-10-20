use super::*;
use crate::game::EditorGame;
use orr_fp::fp;

fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("hazards.scene.yaml");
    let mut editor =
        crate::mesh_hazards_creator::prepare(&path, &Default::default(), true).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    (temp, editor, panel)
}
fn camera() -> Camera3D {
    Camera3D::perspective([4.0, 8.0, 10.0], [4.0, 0.0, 3.0], 60.0)
}
fn click(position: [f32; 3]) -> ViewInput {
    ViewInput {
        pixel: camera().world_to_screen(position, (800, 600)),
        clicked: true,
        cancel: false,
    }
}
#[test]
fn selected_center_only_and_exact_decimal_preserve_radius_and_other_spheres() {
    let mut panel = Panel {
        enabled: true,
        count: "2".into(),
        ..Default::default()
    };
    panel.spheres[1] = ["1", "2", "3", "0.750000"].map(str::to_owned);
    let before = panel.spheres.clone();
    assert!(stage_position(&mut panel, 1, [fp!(0.5), fp!(-1), fp!(2.25)]).unwrap());
    assert_eq!(panel.spheres[1], ["0.5", "-1", "2.25", "0.750000"]);
    assert_eq!(panel.spheres[0], before[0]);
    assert_eq!(panel.spheres[2..], before[2..]);
    assert!(!panel.form_dirty);
    assert!(!stage_position(&mut panel, 1, [fp!(0.5), fp!(-1), fp!(2.25)]).unwrap());
}
#[test]
fn invalid_index_and_bounds_preserve_every_draft_field() {
    let mut panel = Panel {
        enabled: true,
        ..Default::default()
    };
    let before = panel.spheres.clone();
    assert!(stage_position(&mut panel, 1, [FP::ZERO; 3]).is_err());
    assert!(stage_position(&mut panel, 0, [fp!(257), FP::ZERO, FP::ZERO]).is_err());
    panel.enabled = false;
    assert!(stage_position(&mut panel, 0, [FP::ZERO; 3]).is_err());
    assert_eq!(panel.spheres, before);
}
#[test]
fn admitted_viewport_hit_stages_only_then_apply_undo_redo() {
    let (_temp, mut editor, mut panel) = fixture();
    let before = editor.checksum();
    let history = editor.history().entries.len();
    let scene = std::fs::read(editor.path().unwrap()).unwrap();
    let radius = panel.spheres[0][3].clone();
    assert!(panel.begin_hazard_pick(&editor, 0));
    assert!(panel.dirty());
    assert!(panel.play_blocker(&editor).is_some());
    // (4, 0, 3) is exactly the shared diagonal of this 8x6 two-triangle
    // floor. The existing admitted picker intentionally rejects a floating-ray
    // hit whose triangle differs after fixed-point projection. Use a strict
    // interior point for this positive test, without weakening that contract.
    let input = click([3.5, 0.0, 3.0]);
    let (context, _) = current(&editor).unwrap();
    let (origin, direction) = camera().screen_ray(input.pixel.unwrap(), (800, 600));
    let hit = crate::mesh_agent_view::pick(
        &context.mesh.graph,
        origin,
        direction,
        context.mesh.pin.max_vertical,
    )
    .unwrap()
    .expect("positive fixture must hit an admitted triangle interior");
    assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), input));
    assert!(
        !panel.hazard_pick_busy(),
        "{}",
        panel.viewport_picking.status
    );
    assert_eq!(
        super::super::parse_sphere(&panel.spheres[0])
            .unwrap()
            .center,
        hit.position
    );
    assert!(panel.form_dirty);
    assert_eq!(panel.spheres[0][3], radius);
    assert_eq!(editor.checksum(), before);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), scene);
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    let after = editor.checksum();
    assert_ne!(after, before);
    assert_eq!(editor.history().entries.len(), history + 1);
    assert!(editor.undo());
    editor.sync();
    assert_eq!(editor.checksum(), before);
    assert!(editor.redo());
    editor.sync();
    assert_eq!(editor.checksum(), after);
}
#[test]
fn misses_and_invalid_pixels_remain_armed_without_edits() {
    let (_temp, editor, mut panel) = fixture();
    let before = panel.spheres.clone();
    assert!(panel.begin_hazard_pick(&editor, 0));
    for input in [
        ViewInput {
            pixel: None,
            clicked: true,
            cancel: false,
        },
        ViewInput {
            pixel: Some([f32::NAN, 0.0]),
            clicked: true,
            cancel: false,
        },
        ViewInput {
            pixel: Some([-1.0, 2.0]),
            clicked: true,
            cancel: false,
        },
        ViewInput {
            pixel: Some([800.0, 2.0]),
            clicked: true,
            cancel: false,
        },
    ] {
        assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), input));
        assert!(panel.hazard_pick_busy());
        assert_eq!(panel.spheres, before);
    }
    let away = Camera3D::perspective([4.0, 8.0, 10.0], [4.0, 9.0, 10.0], 60.0);
    assert!(panel.hazard_viewport_input(
        &editor,
        &away,
        (800, 600),
        ViewInput {
            pixel: Some([400.0, 300.0]),
            clicked: true,
            cancel: false
        }
    ));
    assert!(panel.hazard_pick_busy());
    assert_eq!(panel.spheres, before);
}
#[test]
fn cancel_consumes_click_preserves_draft_and_does_not_write() {
    let (_temp, editor, mut panel) = fixture();
    panel.spheres[0][3] = "0.75".into();
    panel.form_dirty = true;
    let before = panel.spheres.clone();
    let checksum = editor.checksum();
    assert!(panel.begin_hazard_pick(&editor, 0));
    assert!(panel.hazard_viewport_input(
        &editor,
        &camera(),
        (800, 600),
        ViewInput {
            cancel: true,
            ..click([4.0, 0.0, 3.0])
        }
    ));
    assert!(!panel.hazard_pick_busy());
    assert!(panel.form_dirty);
    assert_eq!(panel.spheres, before);
    assert_eq!(editor.checksum(), checksum);
    assert!(!panel.hazard_viewport_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 3.0])));
}
#[test]
fn draft_count_or_radius_changes_cancel_before_click_and_preserve_changes() {
    for change in 0..3 {
        let (_temp, editor, mut panel) = fixture();
        assert!(panel.begin_hazard_pick(&editor, 0));
        match change {
            0 => panel.count = "2".into(),
            1 => panel.spheres[0][3] = "0.75".into(),
            _ => panel.enabled = false,
        }
        let before = panel.spheres.clone();
        assert!(panel.hazard_viewport_input(
            &editor,
            &camera(),
            (800, 600),
            click([4.0, 0.0, 3.0])
        ));
        assert!(!panel.hazard_pick_busy());
        assert_eq!(panel.spheres, before);
    }
}
#[test]
fn checksum_change_cancels_stale_pick_without_staging() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_hazard_pick(&editor, 0));
    let before = panel.spheres.clone();
    let (context, view) = current(&editor).unwrap();
    let mut config = view.unwrap().config;
    config.spheres[0].radius = fp!(0.75);
    editor
        .host_call(
            "world.singleton.patch",
            orr_remote::mesh_navigation::hazards::patch_request(&config, &context.mesh).unwrap(),
        )
        .unwrap();
    editor.sync();
    assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 3.0])));
    assert!(!panel.hazard_pick_busy());
    assert_eq!(panel.spheres, before);
}
#[test]
fn same_path_reopen_rejects_old_context_even_with_same_checksum() {
    let (_temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_hazard_pick(&editor, 0));
    let checksum = editor.checksum();
    let before = panel.spheres.clone();
    let path = editor.path().unwrap();
    assert!(editor.open_path(&path));
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 3.0])));
    assert!(!panel.hazard_pick_busy());
    assert_eq!(panel.spheres, before);
}
#[test]
fn play_and_legacy_profiles_refuse_picking_and_apply_while_armed_refuses() {
    let (_temp, mut editor, mut panel) = fixture();
    let before = editor.checksum();
    assert!(panel.begin_hazard_pick(&editor, 0));
    panel.apply(&mut editor);
    assert_eq!(editor.checksum(), before);
    assert!(panel.hazard_pick_busy());
    panel.cancel_hazard_pick();
    assert!(editor.start_play());
    editor.sync();
    assert!(!panel.begin_hazard_pick(&editor, 0));
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("legacy.scene.yaml");
    let legacy = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    assert_eq!(legacy.game(), EditorGame::MeshNavigation);
    assert!(!panel.begin_hazard_pick(&legacy, 0));
}

#[test]
fn mismatched_recorded_graph_identity_consumes_click_without_staging() {
    let (_temp, editor, mut panel) = fixture();
    assert!(panel.begin_hazard_pick(&editor, 0));
    panel
        .viewport_picking
        .context
        .as_mut()
        .unwrap()
        .identity
        .revision[0] ^= 1;
    let before = panel.spheres.clone();
    let checksum = editor.checksum();
    assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 3.0])));
    assert!(!panel.hazard_pick_busy());
    assert_eq!(panel.spheres, before);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn different_scene_path_invalidates_pick_before_input() {
    let (temp, mut editor, mut panel) = fixture();
    assert!(panel.begin_hazard_pick(&editor, 0));
    let before = panel.spheres.clone();
    let next = temp.path().join("other.scene.yaml");
    std::fs::copy(editor.path().unwrap(), &next).unwrap();
    assert!(editor.open_path(&next));
    editor.sync();
    assert!(panel.hazard_viewport_input(&editor, &camera(), (800, 600), click([4.0, 0.0, 3.0])));
    assert!(!panel.hazard_pick_busy());
    assert_eq!(panel.spheres, before);
}
