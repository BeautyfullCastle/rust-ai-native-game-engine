#![allow(clippy::disallowed_types)]
use super::edits;
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    terrain_panel::{TerrainPanel, TerrainSelectionMode},
};
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
fn settle(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        editor.sync();
        if editor.yard_rows_coherent()
            && editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == (editor.mode() == Mode::Play))
        {
            return;
        }
        assert!(Instant::now() < deadline, "Yard snapshot not coherent");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, TerrainPanel) {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("yard.yaml");
    fs::write(
        &scene,
        include_str!("../../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    settle(&mut editor);
    let mut panel = TerrainPanel {
        dimensions: [9, 9],
        initial_height: "1".into(),
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel
        .document
        .apply(&[
            Edit::SetHeight {
                x: 4,
                z: 4,
                height: FP::from_raw(131072),
            },
            Edit::SetHole {
                x: 0,
                z: 0,
                hole: true,
            },
        ])
        .unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    panel.selected_vertex = [4, 4];
    panel.selection_mode = TerrainSelectionMode::Vertex;
    panel.stamp.fields.radius = 3;
    panel.stamp.fields.delta = "3".into();
    (temp, scene, editor, panel)
}
fn terrain(side: u32, raw: Vec<i64>, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "stamp-test".into(),
        side,
        side,
        [FP::ZERO; 2],
        FP::ONE,
        raw.into_iter().map(FP::from_raw).collect(),
        holes,
    )
    .unwrap()
}
fn applied_heights(t: &Terrain, changes: &[Edit]) -> Vec<i64> {
    let mut result: Vec<_> = t.heights().iter().map(|h| h.raw()).collect();
    for e in changes {
        match e {
            Edit::SetHeight { x, z, height } => result[(z * t.width() + x) as usize] = height.raw(),
            _ => panic!("stamp may only alter heights"),
        }
    }
    result
}
#[test]
fn literal_signed_radius_three_adds_to_original_nonflat_surface() {
    let raw: Vec<i64> = (0..81).map(|n| n * 7 - 200).collect();
    let t = terrain(9, raw.clone(), vec![false; 64]);
    let bytes = t.cook();
    // Literal radius-three interior weights for delta raw 5. The zero-weight
    // outer diamond is intentionally absent from the changed-only edits.
    let expected_changes = [
        (4, 2, 1),
        (3, 3, 1),
        (4, 3, 3),
        (5, 3, 1),
        (2, 4, 1),
        (3, 4, 3),
        (4, 4, 5),
        (5, 4, 3),
        (6, 4, 1),
        (3, 5, 1),
        (4, 5, 3),
        (5, 5, 1),
        (4, 6, 1),
    ];
    for sign in [-1, 1] {
        let changes = edits(&t, [4, 4], 3, FP::from_raw(5 * sign)).unwrap();
        assert_eq!(changes.len(), expected_changes.len());
        let mut expected = raw.clone();
        for (actual, (x, z, delta)) in changes.iter().zip(expected_changes) {
            let value = raw[(z * 9 + x) as usize] + delta * sign;
            assert!(
                matches!(actual,Edit::SetHeight{x:ax,z:az,height} if *ax==x&&*az==z&&height.raw()==value)
            );
            expected[(z * 9 + x) as usize] = value;
        }
        assert_eq!(applied_heights(&t, &changes), expected);
        assert_eq!(t.cook(), bytes);
    }
}
#[test]
fn complete_extent_and_footprint_budget_even_when_every_height_is_unchanged() {
    let t = terrain(129, vec![0; 129 * 129], vec![false; 128 * 128]);
    let before = t.cook();
    assert!(edits(&t, [64, 64], 44, FP::ZERO).unwrap().is_empty()); //3961
    assert!(edits(&t, [64, 64], 45, FP::ZERO).is_err()); //4141
    for (center, radius) in [
        ([64, 64], 0),
        ([64, 64], u32::MAX),
        ([0, 0], 1),
        ([128, 128], 1),
        ([u32::MAX, 64], 1),
    ] {
        assert!(edits(&t, center, radius, FP::ZERO).is_err());
    }
    assert_eq!(t.cook(), before);
}
#[test]
fn exact_incident_holes_include_zero_rim_but_not_unrelated_square_corners() {
    // Check every cell: acceptance is decided by an independently enumerated
    // literal diamond's vertex incidence, not by the implementation's bounds.
    let vertices = [
        (4, 1),
        (3, 2),
        (4, 2),
        (5, 2),
        (2, 3),
        (3, 3),
        (4, 3),
        (5, 3),
        (6, 3),
        (1, 4),
        (2, 4),
        (3, 4),
        (4, 4),
        (5, 4),
        (6, 4),
        (7, 4),
        (2, 5),
        (3, 5),
        (4, 5),
        (5, 5),
        (6, 5),
        (3, 6),
        (4, 6),
        (5, 6),
        (4, 7),
    ];
    for z in 0..8 {
        for x in 0..8 {
            let touches = vertices
                .iter()
                .any(|&(vx, vz)| (vx == x || vx == x + 1) && (vz == z || vz == z + 1));
            let mut holes = vec![false; 64];
            holes[(z * 8 + x) as usize] = true;
            let t = terrain(9, vec![0; 81], holes);
            let before = t.cook();
            let result = edits(&t, [4, 4], 3, FP::ZERO);
            assert_eq!(result.is_err(), touches, "cell {x},{z}");
            assert_eq!(t.cook(), before);
        }
    }
    // Cell(1,1) lies inside the bounding square but touches no diamond vertex.
    let mut holes = vec![false; 64];
    holes[9] = true;
    let t = terrain(9, vec![0; 81], holes);
    assert_eq!(edits(&t, [4, 4], 3, FP::ONE).unwrap().len(), 13);
}
#[test]
fn overflow_is_atomic_even_after_earlier_vertices_would_change() {
    for (old, delta) in [(i64::MAX, 1), (i64::MIN, -1)] {
        let mut raw = vec![0; 81];
        raw[40] = old;
        let t = terrain(9, raw, vec![false; 64]);
        let before = t.cook();
        assert!(edits(&t, [4, 4], 3, FP::from_raw(delta * 9)).is_err());
        assert_eq!(t.cook(), before);
    }
}
#[test]
fn actual_vertex_preview_apply_undo_redo_save_reopen() {
    let (temp, scene, editor, mut panel) = fixture();
    let original_terrain = panel.document.terrain().unwrap().clone();
    panel.selection_mode = TerrainSelectionMode::Entity;
    let before = panel.document.bytes().unwrap().to_vec();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let model = panel.viewport_model(&editor).unwrap();
    let mut h = Harness::builder().with_size([1400., 3400.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Vertex").click();
    h.run_steps(3);
    h.get_by_label("Stamp terrain relief").click();
    h.run_steps(3);
    h.get_by_label("Preview terrain stamp").click();
    h.run_steps(3);
    assert!(h.state().0.stamp.has_preview(), "{:?}", h.state().0.error());
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &h.state().0.viewport_model(&h.state().1).unwrap()
    ));
    h.get_by_label("Apply terrain stamp").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert!(!panel.stamp.has_preview());
    let t = panel.document.terrain().unwrap();
    assert_eq!(t.asset_id(), original_terrain.asset_id());
    assert_eq!(t.origin(), original_terrain.origin());
    assert_eq!(t.spacing(), original_terrain.spacing());
    assert_eq!(
        (t.width(), t.depth()),
        (original_terrain.width(), original_terrain.depth())
    );
    let expected = [
        (4, 2, 2),
        (3, 3, 2),
        (4, 3, 3),
        (5, 3, 2),
        (2, 4, 2),
        (3, 4, 3),
        (4, 4, 5),
        (5, 4, 3),
        (6, 4, 2),
        (3, 5, 2),
        (4, 5, 3),
        (5, 5, 2),
        (4, 6, 2),
    ];
    for z in 0..9 {
        for x in 0..9 {
            let height = expected
                .iter()
                .find(|&&(ex, ez, _)| ex == x && ez == z)
                .map_or(1, |e| e.2);
            assert_eq!(t.heights()[(z * 9 + x) as usize].raw(), height * 65536);
        }
    }
    assert!(t.holes()[0]);
    assert_eq!(t.holes().iter().filter(|h| **h).count(), 1);
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    h.get_by_label("Terrain Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Terrain Redo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    let (panel, editor) = h.state_mut();
    panel.open_for_editor(editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(changed.as_slice()));
    assert!(!panel.document.dirty());
    let decoded = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(decoded.heights()[40].raw(), 5 * 65536);
    assert!(decoded.holes()[0]);
    let model = panel.viewport_model(editor).unwrap();
    assert!(model
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .any(|v| v.position[1] == 5.0));
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}
#[test]
fn no_op_preserves_redo_revision_model_and_dirty_state() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 8,
            z: 8,
            height: FP::ZERO,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    assert!(panel.document.can_redo());
    panel.stamp.fields.delta = "0".into();
    let before = panel.document.bytes().unwrap().to_vec();
    let rev = panel.document.revision();
    let dirty = panel.document.dirty();
    let model = panel.viewport_model(&editor).unwrap();
    panel.preview_stamp(&editor).unwrap();
    panel.apply_stamp(&editor).unwrap();
    assert!(!panel.stamp.has_preview());
    assert!(panel.document.can_redo());
    assert!(!panel.document.can_undo());
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), rev);
    assert_eq!(panel.document.dirty(), dirty);
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &panel.viewport_model(&editor).unwrap()
    ));
}
#[test]
fn stale_fields_selection_content_context_and_render_failure_preserve_applied_state() {
    for case in [
        "fields", "vertex", "mode", "content", "epoch", "play", "readonly", "renderer",
    ] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.preview_stamp(&editor).unwrap();
        match case {
            "fields" => panel.stamp.fields.delta = "-3".into(),
            "vertex" => panel.selected_vertex = [5, 4],
            "mode" => panel.selection_mode = TerrainSelectionMode::Cell,
            "content" => panel
                .document
                .apply(&[Edit::SetHeight {
                    x: 8,
                    z: 8,
                    height: FP::ZERO,
                }])
                .unwrap(),
            "epoch" => {
                let next = temp.path().join("next.yaml");
                fs::copy(&scene, &next).unwrap();
                assert!(editor.open_path(&next));
                settle(&mut editor);
            }
            "play" => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            "readonly" => {
                use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};
                let project = temp.path().join("project");
                install_and_reload(
                    panel.document.terrain().unwrap(),
                    &project,
                    &temp.path().join("package-source"),
                    "1.0.0",
                )
                .unwrap();
                panel
                    .document
                    .open_package(&project, PACKAGE_NAME, ASSET_PATH)
                    .unwrap();
                assert!(panel.document.read_only());
            }
            "renderer" => {
                assert!(!edits(
                    panel.document.terrain().unwrap(),
                    [4, 4],
                    3,
                    FP::from_raw(3 * 65536)
                )
                .unwrap()
                .is_empty());
                panel
                    .document
                    .set_render_admission_error(Some("injected stamp render rejection".into()));
            }
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let dirty = panel.document.dirty();
        let undo = panel.document.can_undo();
        let redo = panel.document.can_redo();
        let model = panel.document.model().unwrap().clone();
        let file = fs::read(temp.path().join("terrain.orrt")).unwrap();
        let scene_bytes = fs::read(&scene).unwrap();
        let history = editor.history().clone();
        let checksum = editor.checksum();
        assert!(panel.apply_stamp(&editor).is_err(), "{case}");
        assert!(!panel.stamp.has_preview());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert_eq!(panel.document.dirty(), dirty);
        assert_eq!(panel.document.can_undo(), undo);
        assert_eq!(panel.document.can_redo(), redo);
        assert!(std::sync::Arc::ptr_eq(
            &model,
            panel.document.model().unwrap()
        ));
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), file);
        assert_eq!(fs::read(scene).unwrap(), scene_bytes);
        assert_eq!(editor.history(), &history);
        assert_eq!(editor.checksum(), checksum);
    }
}
#[test]
fn failed_repreview_revokes_candidate_and_ui_selection_revokes_while_collapsed() {
    for case in ["parse", "length", "radius", "edge", "hole"] {
        let (_temp, _scene, editor, mut panel) = fixture();
        panel.preview_stamp(&editor).unwrap();
        match case {
            "parse" => panel.stamp.fields.delta = "NaN".into(),
            "length" => panel.stamp.fields.delta = "0".repeat(129),
            "radius" => panel.stamp.fields.radius = 45,
            "edge" => panel.selected_vertex = [0, 0],
            "hole" => panel
                .document
                .apply(&[Edit::SetHole {
                    x: 4,
                    z: 0,
                    hole: true,
                }])
                .unwrap(), //incident to zero-weight top rim vertex(4,1)
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        assert!(panel.preview_stamp(&editor).is_err(), "{case}");
        assert!(!panel.stamp.has_preview());
        assert!(panel.apply_stamp(&editor).is_err());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
    }
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.preview_stamp(&editor).unwrap();
    let mut h = Harness::builder().with_size([1200., 900.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_stamp(ui, &s.1, true),
        (panel, editor),
    );
    h.run_steps(2);
    assert!(h.state().0.stamp.has_preview());
    h.state_mut().0.selected_vertex = [5, 4];
    h.run_steps(3);
    assert!(!h.state().0.stamp.has_preview());
    assert!(!h.state().0.document.can_undo());
}
