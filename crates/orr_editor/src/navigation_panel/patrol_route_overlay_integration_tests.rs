//! Actual CPU authoring widgets and production draw adapter; no GPU claim.
#![allow(clippy::disallowed_types)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_fp::FP;
use orr_navigation::SearchBudget;
use orr_remote::navigation_yard3d::{self as remote, patrol};
use orr_render::RenderList3D;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const INSPECT: &str = "Inspect terrain patrol draft route";
const SHOW: &str = "Show inspected terrain patrol route";
fn wait(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate()
            && editor.yard_rows_coherent()
            && editor.admitted_navigation().admitted
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture() -> (
    tempfile::TempDir,
    Harness<'static, (NavigationPanel, Editor, TerrainPanel, bool)>,
) {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("terrain-patrol.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate() && editor.yard_rows_coherent() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut terrain = TerrainPanel::default();
    terrain.local_path = "route.orrt".into();
    terrain.asset_id = "navigation/patrol-fixture".into();
    terrain.dimensions = [9, 9];
    terrain.origin = ["-4".into(), "-4".into()];
    terrain.spacing = "1".into();
    terrain.initial_height = "0".into();
    terrain.create_for_editor(&editor).unwrap();
    let mut edits = Vec::new();
    for z in 0..9 {
        for x in 0..9 {
            edits.push(orr_terrain::Edit::SetHeight {
                x,
                z,
                height: FP::from_raw(i64::from(x) * 16384),
            });
        }
    }
    edits.push(orr_terrain::Edit::SetHole {
        x: 4,
        z: 6,
        hole: true,
    });
    terrain.document.apply(&edits).unwrap();
    terrain.save_for_editor(&editor).unwrap();
    let mut panel = NavigationPanel::default();
    panel.start = ["-3".into(), "0".into()];
    panel.goal = ["3".into(), "0".into()];
    panel.distance_per_tick = "1".into();
    panel.build_route(&mut editor, &terrain).unwrap();
    wait(&mut editor);
    assert!(editor.save());
    panel.sync_from_editor(&editor);
    let mut h = Harness::builder()
        .with_size([1600.0, 3000.0])
        .build_ui_state(
            |ui, state: &mut (NavigationPanel, Editor, TerrainPanel, bool)| {
                state
                    .0
                    .sync_patrol_inspection_focus(&state.1, &state.2, ui.ctx());
                if state.3 {
                    state.0.show(ui, &mut state.1, &state.2);
                }
            },
            (panel, editor, terrain, true),
        );
    h.run_steps(3);
    h.get_by_label("Point navigation").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Terrain waypoint patrol").click_accesskit();
    h.run_steps(3);
    (dir, h)
}

fn click(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(
    h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>,
    label: &str,
    value: &str,
) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .value()
            .as_deref(),
        Some(value)
    );
}
fn stage(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>) {
    click(h, "Enable terrain waypoint patrol");
    click(h, "Add terrain patrol waypoint");
    for (label, value) in [
        ("Terrain patrol waypoint 1 X", "-2"),
        ("Terrain patrol waypoint 1 Z", "0"),
        ("Terrain patrol waypoint 2 X", "2"),
        ("Terrain patrol waypoint 2 Z", "0"),
    ] {
        text(h, label, value);
    }
    click(h, "Loop terrain patrol");
}
fn paint(h: &Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>) -> RenderList3D {
    let mut list = RenderList3D::default();
    h.state()
        .0
        .paint_patrol_route(&h.state().1, &h.state().2, &mut list);
    list
}
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, p: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(p).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().into(),
                    std::fs::read(p).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}
#[test]
fn actual_terrain_patrol_inspection_matches_all_surface_paths_without_authoring_writes() {
    let (dir, mut h) = fixture();
    stage(&mut h);
    let saved = files(dir.path());
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    let selected = h.state().1.selection().cloned();
    click(&mut h, INSPECT);
    assert!(paint(&h).is_empty());
    click(&mut h, SHOW);
    let view = remote::navigation_from_view(h.state().1.snapshot().unwrap().predicted()).unwrap();
    let p = |x, z| [FP::from_int(x), FP::from_int(z)];
    let endpoints = [
        (p(-3, 0), p(-2, 0)),
        (p(-2, 0), p(2, 0)),
        (p(2, 0), p(-2, 0)),
    ];
    let paths = endpoints
        .iter()
        .map(|(a, b)| {
            view.graph
                .find_path(
                    &view.terrain,
                    *a,
                    *b,
                    SearchBudget {
                        max_expansions: patrol::MAX_ROUTE_TRIANGLES as u32,
                    },
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        paths.iter().any(|path| path.waypoints().len() > 2),
        "fixture must traverse multiple terrain portals"
    );
    let report = h
        .state()
        .0
        .inspected_patrol_route(&h.state().1, &h.state().2)
        .unwrap();
    assert_eq!(report.legs.len(), 3);
    let mut cost = FP::ZERO;
    for (leg, path) in report.legs.iter().zip(&paths) {
        assert_eq!(leg.points, path.waypoints());
        assert_eq!(leg.corridor_triangles, path.corridor().len());
        assert_eq!(leg.graph_cost, path.cost());
        cost += leg.graph_cost;
        for point in &leg.points {
            assert_eq!(point[1], view.terrain.sample(point[0], point[2]).unwrap());
        }
    }
    assert_eq!(report.total_graph_cost, cost);
    assert!(
        report
            .legs
            .iter()
            .flat_map(|leg| &leg.points)
            .any(|p| p[1] > FP::ZERO)
    );
    let list = paint(&h);
    let segments = paths
        .iter()
        .map(|path| path.waypoints().len() - 1)
        .sum::<usize>();
    assert_eq!(list.lines.len(), segments + 36);
    assert_eq!(list.instance_count(), 0);
    let mut index = 0;
    for (leg, path) in paths.iter().enumerate() {
        for pair in path.waypoints().windows(2) {
            assert_eq!(list.lines[index].a, pair[0].map(FP::to_f32));
            assert_eq!(list.lines[index].b, pair[1].map(FP::to_f32));
            assert_eq!(
                list.lines[index].color,
                if leg == 2 {
                    [0.85, 0.35, 1., 1.]
                } else {
                    [0., 0.8, 1., 1.]
                }
            );
            index += 1;
        }
    }
    assert_eq!(paint(&h).lines, list.lines);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.selection(), selected.as_ref());
    assert_eq!(files(dir.path()), saved);
    // The inspected paths are advisory. Existing explicit authoring remains authoritative.
    click(&mut h, "Apply terrain waypoint patrol");
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let expected = patrol::from_view(h.state().1.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    assert_eq!((expected.count, expected.looped), (2, 1));
    assert!(paint(&h).is_empty());
    assert!(h.state_mut().1.undo());
    h.state_mut().1.sync();
    assert!(h.state_mut().1.redo());
    h.state_mut().1.sync();
    assert!(h.state_mut().1.save());
    let doc = remote::navigation_yard3d_doc_from_path(&h.state().1.path().unwrap()).unwrap();
    assert_eq!(
        patrol::from_view(orr_bridge::FrameView::of(doc.frame()))
            .unwrap()
            .unwrap()
            .config,
        expected
    );
}
#[test]
fn late_outside_waypoint_rejects_complete_overlay_without_partial_commands() {
    let (dir, mut h) = fixture();
    stage(&mut h);
    let saved = files(dir.path());
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    text(&mut h, "Terrain patrol waypoint 2 X", "99");
    assert!(paint(&h).is_empty());
    click(&mut h, INSPECT);
    assert!(h.get_by_label(SHOW).accesskit_node().is_disabled());
    assert!(paint(&h).is_empty());
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(files(dir.path()), saved);
}
#[test]
fn draft_aba_closed_panel_source_change_and_batched_focus_loss_retire_inspection() {
    let (_dir, mut h) = fixture();
    stage(&mut h);
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    text(&mut h, "Terrain patrol waypoint 2 X", "2.5");
    assert!(paint(&h).is_empty());
    text(&mut h, "Terrain patrol waypoint 2 X", "2");
    assert!(paint(&h).is_empty());
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    h.state_mut().3 = false;
    // A real local terrain edit and Undo changes session identity even with equal restored bytes.
    h.state_mut()
        .2
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    h.step();
    h.state_mut().2.document.undo().unwrap();
    h.step();
    assert!(paint(&h).is_empty());
    h.state_mut().3 = true;
    h.run_steps(3);
    // The route remains reusable after terrain Undo, but the inspection must be explicit again.
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    h.state_mut().3 = false;
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
    ]);
    h.step();
    assert!(paint(&h).is_empty());
    h.run_steps(2);
    assert!(paint(&h).is_empty());
    h.state_mut().3 = true;
    h.run_steps(3);
    assert!(paint(&h).is_empty());
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
}
