//! Actual CPU widgets plus production viewport adapter; synthetic camera input,
//! not a native window/GPU/OS-input playtest.
use super::patrol_tests::{camera, click, fixture};
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_remote::mesh_navigation::{mesh_from_view, patrol};
use std::fs;
fn row(h: &Harness<'_, (Panel, Editor)>, index: usize) -> [FP; 3] {
    ["X", "Y", "Z"].map(|axis| {
        let value = h
            .get_by_role_and_label(Role::TextInput, &format!("Patrol waypoint {index} {axis}"))
            .accesskit_node()
            .value()
            .unwrap();
        super::super::parse_fixed(&value).unwrap()
    })
}
fn hit(h: &mut Harness<'_, (Panel, Editor)>) -> [FP; 3] {
    let camera = camera();
    let size = (1280, 720);
    let pixel = camera.world_to_screen([2.0, 1.0, 1.0], size).unwrap();
    let (origin, direction) = camera.screen_ray(pixel, size);
    let view = mesh_from_view(h.state().1.snapshot().unwrap().predicted()).unwrap();
    let expected =
        crate::mesh_agent_view::pick(&view.graph, origin, direction, view.pin.max_vertical)
            .unwrap()
            .unwrap()
            .position;
    let state = h.state_mut();
    assert!(state.0.patrol_viewport_input(
        &state.1,
        &camera,
        size,
        ViewInput {
            pixel: Some(pixel),
            clicked: true,
            cancel: false
        },
        true
    ));
    expected
}
#[test]
fn actual_row_pick_survives_panel_redraw_and_applies_undo_redo_save_reopen_atomically() {
    let (_dir, mut h) = fixture(EditorGame::MeshNavigation);
    let scene = h.state().1.path().unwrap();
    let saved = fs::read(&scene).unwrap();
    let before = h.state().1.checksum();
    let history = h.state().1.history().entries.len();
    let start = h.state().0.start.clone();
    let goal = h.state().0.goal.clone();
    let draft_dirty = h.state().0.draft_dirty;
    let first = row(&h, 1);
    let second = row(&h, 2);
    click(&mut h, "Pick patrol waypoint 2");
    assert!(h.state().0.patrol_pick_busy());
    // Every frame renders the same panel while its own ticket owns input.
    h.run_steps(3);
    assert!(h.state().0.patrol_pick_busy());
    let expected = hit(&mut h);
    h.run_steps(2);
    assert!(!h.state().0.patrol_pick_busy());
    assert_eq!(row(&h, 1), first);
    assert_eq!(row(&h, 2), expected);
    assert_ne!(expected, second);
    assert_eq!(h.state().0.start, start);
    assert_eq!(h.state().0.goal, goal);
    assert_eq!(h.state().0.draft_dirty, draft_dirty);
    assert_eq!(h.state().1.checksum(), before);
    assert_eq!(h.state().1.history().entries.len(), history);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    click(&mut h, "Apply waypoint patrol");
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let config = patrol::from_view(h.state().1.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    assert_eq!(config.points[0], first);
    assert_eq!(config.points[1], expected);
    assert_eq!(config.count, 2);
    assert_eq!(h.state().1.history().entries.len(), history + 1);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    assert!(h.state_mut().1.undo());
    h.state_mut().1.sync();
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert!(h.state_mut().1.redo());
    h.state_mut().1.sync();
    assert_eq!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        config
    );
    assert!(h.state_mut().1.save());
    let reopened = orr_remote::mesh_navigation::mesh_doc_from_path(&scene).unwrap();
    assert_eq!(
        patrol::from_view(orr_bridge::FrameView::of(reopened.frame()))
            .unwrap()
            .unwrap()
            .config,
        config
    );
}
#[test]
fn actual_patrol_pick_miss_cancel_and_focus_loss_preserve_draft_frame_history_and_files() {
    #[cfg(feature = "mesh-hazards")]
    let game = EditorGame::MeshHazardsV1;
    #[cfg(not(feature = "mesh-hazards"))]
    let game = EditorGame::MeshNavigation;
    let (_dir, mut h) = fixture(game);
    let scene = h.state().1.path().unwrap();
    let saved = fs::read(&scene).unwrap();
    let before = h.state().1.checksum();
    let history = h.state().1.history().clone();
    let first = row(&h, 1);
    let second = row(&h, 2);
    click(&mut h, "Inspect patrol draft route");
    click(&mut h, "Show inspected patrol route");
    assert!(h.state().0.inspected_patrol_route(&h.state().1).is_some());
    click(&mut h, "Pick patrol waypoint 1");
    assert!(h.state().0.patrol_pick_busy());
    assert!(h.state().0.inspected_patrol_route(&h.state().1).is_none());
    let state = h.state_mut();
    assert!(state.0.patrol_viewport_input(
        &state.1,
        &camera(),
        (1280, 720),
        ViewInput {
            pixel: Some([0.0, 0.0]),
            clicked: true,
            cancel: false
        },
        true
    ));
    h.run_steps(2);
    assert!(h.state().0.patrol_pick_busy());
    assert_eq!(row(&h, 1), first);
    assert_eq!(row(&h, 2), second);
    click(&mut h, "Cancel patrol waypoint pick");
    assert!(!h.state().0.patrol_pick_busy());
    h.run_steps(2);
    click(&mut h, "Pick patrol waypoint 2");
    assert!(h.state().0.patrol_pick_busy());
    h.input_mut().focused = false;
    h.step();
    assert!(!h.state().0.patrol_pick_busy());
    assert_eq!(row(&h, 1), first);
    assert_eq!(row(&h, 2), second);
    assert_eq!(h.state().1.checksum(), before);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(fs::read(scene).unwrap(), saved);
}
