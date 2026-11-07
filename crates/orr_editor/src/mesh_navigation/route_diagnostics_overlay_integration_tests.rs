//! Actual CPU Panel input -> production draw-command adapter. No renderer/GPU claim.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_render::RenderList3D;
use std::path::Path;
const SHOW: &str = "Show inspected draft route";
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.push((p.strip_prefix(root).unwrap().into(), fs::read(p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("overlay.scene.yaml");
    fs::write(&path, "schema: orr.scene/1\nsingletons: {}\nentities: {}\n").unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.create_ramp_and_bridge().unwrap();
    let mut h = Harness::builder()
        .with_size([1600.0, 3800.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
    h.run_steps(3);
    h.get_by_label("Static-mesh navigation").click_accesskit();
    h.run_steps(3);
    (temp, h)
}
fn inspect(h: &mut Harness<'_, (Panel, Editor)>) {
    h.get_by_label("Inspect draft route").click_accesskit();
    h.run_steps(3);
}
fn paint(h: &Harness<'_, (Panel, Editor)>) -> RenderList3D {
    let mut list = RenderList3D::default();
    h.state().0.paint_draft_route(&h.state().1, &mut list);
    list
}
#[test]
fn actual_inspection_toggle_draws_independent_bent_elevated_path_without_host_or_file_edits() {
    let (temp, mut h) = fixture();
    let initial_files = files(temp.path());
    let graph = h.state().0.graph.as_ref().unwrap().clone();
    let graph_bytes = graph.to_bytes();
    let editor = &h.state().1;
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let selection = editor.selection().cloned();
    let tick = editor.snapshot().unwrap().predicted().tick();
    assert!(
        editor
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<MeshScenePin>()
            .is_empty(),
        "draft inspection must not require placing an agent"
    );
    let expected = graph
        .find_path(
            [fp!(1), fp!(0), fp!(1)],
            [fp!(3), fp!(2), fp!(1)],
            fp!(2),
            SearchBudget {
                max_expansions: graph.triangles().len() as u32,
            },
        )
        .unwrap();
    assert!(
        expected.points.len() > 2,
        "fixture must contain a bent portal route"
    );
    assert!(expected.points.iter().any(|p| p[1] > FP::ZERO));
    assert!(paint(&h).is_empty());
    inspect(&mut h);
    assert!(
        paint(&h).is_empty(),
        "inspection alone never enables overlay"
    );
    h.get_by_label(SHOW).click_accesskit();
    h.run_steps(3);
    // Call the same adapter without rendering the navigation panel again.
    let list = paint(&h);
    let segments = expected.points.len() - 1;
    assert_eq!(list.lines.len(), segments + 24);
    assert_eq!(list.instance_count(), 0);
    for (line, pair) in list.lines[..segments]
        .iter()
        .zip(expected.points.windows(2))
    {
        assert_eq!(line.a, pair[0].map(FP::to_f32));
        assert_eq!(line.b, pair[1].map(FP::to_f32));
        assert_eq!(line.color, [0.0, 0.9, 1.0, 1.0]);
        assert_eq!(line.width, 3.0);
    }
    for (endpoint, lines, color) in [
        (
            expected.points[0],
            &list.lines[segments..segments + 12],
            [0.1, 1.0, 0.25, 1.0],
        ),
        (
            *expected.points.last().unwrap(),
            &list.lines[segments + 12..],
            [1.0, 0.15, 0.1, 1.0],
        ),
    ] {
        let center = endpoint.map(FP::to_f32);
        for axis in 0..3 {
            let min = lines
                .iter()
                .flat_map(|l| [l.a[axis], l.b[axis]])
                .fold(f32::INFINITY, f32::min);
            let max = lines
                .iter()
                .flat_map(|l| [l.a[axis], l.b[axis]])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(((min + max) * 0.5 - center[axis]).abs() < 0.00001);
        }
        assert!(lines.iter().all(|line| line.color == color));
    }
    let repeat = paint(&h);
    assert_eq!(repeat.lines, list.lines);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.selection(), selection.as_ref());
    assert_eq!(h.state().1.snapshot().unwrap().predicted().tick(), tick);
    assert!(h
        .state()
        .1
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshScenePin>()
        .is_empty());
    assert_eq!(h.state().0.graph.as_ref().unwrap().to_bytes(), graph_bytes);
    assert_eq!(files(temp.path()), initial_files);
    // No UI sync: changed draft must be rejected by the production draw getter immediately.
    h.state_mut().0.goal[0] = "250".into();
    let mut retained = list.clone();
    h.state().0.paint_draft_route(&h.state().1, &mut retained);
    assert_eq!(retained.lines, list.lines);
    assert_eq!(retained.lighting, list.lighting);
    assert_eq!(retained.instance_count(), list.instance_count());
}
#[test]
fn outside_and_disconnected_inspections_disable_overlay_and_append_no_commands() {
    for disconnected in [false, true] {
        let (temp, mut h) = fixture();
        if disconnected {
            let p = |x, y, z| [FP::from_int(x), FP::from_int(y), FP::from_int(z)];
            h.state_mut().0.graph = Some(
                StaticMeshGraph::bake(
                    &[
                        p(0, 2, 0),
                        p(0, 2, 4),
                        p(4, 2, 0),
                        p(8, 2, 0),
                        p(8, 2, 4),
                        p(12, 2, 0),
                    ],
                    &[[0, 1, 2], [3, 4, 5]],
                    AgentProfile {
                        max_slope: fp!(4),
                        ..AgentProfile::default()
                    },
                )
                .unwrap(),
            );
            h.state_mut().0.start = ["1", "2", "1"].map(str::to_owned);
            h.state_mut().0.goal = ["9", "2", "1"].map(str::to_owned);
        } else {
            h.state_mut().0.goal[0] = "250".into();
        }
        let before = files(temp.path());
        let checksum = h.state().1.checksum();
        let history = h.state().1.history().clone();
        inspect(&mut h);
        if disconnected {
            assert!(h.query_by_label("Route search failed: endpoints project, but no connected triangle route exists").is_some());
        } else {
            assert!(h
                .query_by_label("Route not searched: fix endpoint projection first.")
                .is_some());
        }
        assert!(h.get_by_label(SHOW).accesskit_node().is_disabled());
        h.get_by_label(SHOW).click_accesskit();
        h.run_steps(3);
        assert!(paint(&h).is_empty());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(files(temp.path()), before);
    }
}
