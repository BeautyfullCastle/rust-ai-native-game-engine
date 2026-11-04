#![allow(clippy::disallowed_types)]
use super::*;
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
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
        dimensions: [5, 5],
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel
        .document
        .apply(&[Edit::SetHole {
            x: 1,
            z: 1,
            hole: true,
        }])
        .unwrap();
    let heights: Vec<_> = (0..5)
        .flat_map(|z| {
            (0..5).map(move |x| Edit::SetHeight {
                x,
                z,
                height: FP::from_int(x as i32 - z as i32),
            })
        })
        .collect();
    panel.document.apply(&heights).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    panel.holes = Spec {
        min: [1, 1],
        max: [3, 2],
        hole: true,
    };
    (temp, scene, editor, panel)
}

#[test]
fn widget_carve_restore_undo_save_reopen_and_saved_consumer() {
    let (temp, scene, editor, panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    let heights = panel.document.terrain().unwrap().heights().to_vec();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let scene_bytes = fs::read(&scene).unwrap();
    let mut h = Harness::builder()
        .with_size([1400.0, 2400.0])
        .build_ui_state(
            |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
            (panel, editor),
        );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Terrain hole rectangle").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    h.get_by_label("Apply hole rectangle").click();
    h.run_steps(3);
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    let changed = h.state().0.document.bytes().unwrap().to_vec();
    assert_eq!(
        h.state().0.document.terrain().unwrap().holes(),
        &[
            false, false, false, false, false, true, true, true, false, true, true, true, false,
            false, false, false
        ]
    );
    assert_eq!(h.state().0.document.terrain().unwrap().heights(), heights);
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
    h.get_by_label("Apply hole rectangle").click();
    h.run_steps(3); // idempotent
    h.get_by_label("Terrain Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Terrain Redo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    {
        let (panel, editor) = h.state_mut();
        panel.open_for_editor(editor).unwrap();
        assert_eq!(panel.document.bytes(), Some(changed.as_slice()));
        assert!(!panel.document.dirty());
    }
    let reopened = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(
        reopened.sample(FP::from_raw(-163840), FP::from_raw(-163840)),
        None
    ); // cell 1,1
    assert_eq!(
        reopened.sample(FP::from_raw(-229376), FP::from_raw(-229376)),
        Some(FP::ZERO)
    ); // cell 0,0
    let model = orr_terrain_view::to_static_model(&reopened, &[])
        .unwrap()
        .unwrap();
    assert_eq!(reopened.triangles().len(), 20);
    assert_eq!(
        model
            .source()
            .primitives
            .iter()
            .map(|p| p.indices.len())
            .sum::<usize>(),
        60
    );
    h.get_by_label("Carve holes (unchecked restores surface)")
        .click();
    h.run_steps(3);
    h.get_by_label("Apply hole rectangle").click();
    h.run_steps(3);
    assert!(h
        .state()
        .0
        .document
        .terrain()
        .unwrap()
        .holes()
        .iter()
        .all(|v| !v));
    assert_eq!(h.state().0.document.terrain().unwrap().heights(), heights);
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    let restored = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(
        restored.sample(FP::from_raw(-163840), FP::from_raw(-163840)),
        Some(FP::ZERO)
    );
    assert_eq!(restored.triangles().len(), 32);
    assert_eq!(restored.asset_id(), reopened.asset_id());
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.checksum(), checksum);
}

#[test]
fn all_holes_has_no_fake_mesh_and_restore_retains_heights() {
    let (_temp, _scene, editor, mut panel) = fixture();
    let heights = panel.document.terrain().unwrap().heights().to_vec();
    panel.holes.min = [0, 0];
    panel.holes.max = [3, 3];
    panel.apply_hole_rectangle(&editor).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    assert!(panel.document.model().is_none());
    assert!(panel.attached_for_editor(&editor));
    assert!(panel.document.terrain().unwrap().triangles().is_empty());
    assert_eq!(
        panel
            .document
            .terrain()
            .unwrap()
            .sample(FP::from_int(-4), FP::from_int(-4)),
        None
    );
    panel.holes.hole = false;
    panel.apply_hole_rectangle(&editor).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    assert_eq!(panel.document.terrain().unwrap().heights(), heights);
    assert_eq!(panel.document.terrain().unwrap().triangles().len(), 32);
    assert!(panel.document.model().is_some());
}

#[test]
fn bounds_order_changed_only_and_full_footprint_budget() {
    let terrain = Terrain::new(
        "terrain/test".into(),
        129,
        129,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 129 * 129],
        vec![false; 128 * 128],
    )
    .unwrap();
    assert_eq!(edits(&terrain, [0, 0], [63, 63], true).unwrap().len(), 4096);
    assert!(edits(&terrain, [0, 0], [63, 64], false).is_err()); // noops still bounded
    for (min, max) in [
        ([2, 0], [1, 0]),
        ([0, 2], [0, 1]),
        ([0, 0], [128, 0]),
        ([0, 0], [0, 128]),
        ([u32::MAX, 0], [u32::MAX, 0]),
    ] {
        assert!(edits(&terrain, min, max, true).is_err());
    }
    assert!(edits(&terrain, [127, 127], [127, 127], false)
        .unwrap()
        .is_empty());
    let got = edits(&terrain, [126, 126], [127, 127], true).unwrap();
    for (e, [x, z]) in got
        .iter()
        .zip([[126, 126], [127, 126], [126, 127], [127, 127]])
    {
        assert!(matches!(e, Edit::SetHole{x:a,z:b,hole:true} if *a==x && *b==z));
    }
}

#[test]
fn noops_invalid_bounds_and_admission_failure_preserve_redo() {
    let (_temp, _scene, editor, mut panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    panel.apply_hole_rectangle(&editor).unwrap();
    let changed = panel.document.bytes().unwrap().to_vec();
    panel.document.undo().unwrap();
    let revision = panel.document.revision();
    for (min, max, hole, ok) in [
        ([0, 0], [0, 0], false, true),
        ([3, 0], [2, 0], true, false),
        ([0, 0], [4, 3], true, false),
    ] {
        panel.holes = Spec { min, max, hole };
        assert_eq!(panel.apply_hole_rectangle(&editor).is_ok(), ok);
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(!panel.document.dirty());
        assert!(panel.document.can_redo());
        assert!(!panel.document.can_undo());
    }
    panel.holes = Spec {
        min: [1, 1],
        max: [3, 2],
        hole: true,
    };
    panel
        .document
        .set_render_admission_error(Some("test capacity exhausted".into()));
    assert!(panel.apply_hole_rectangle(&editor).is_err());
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(panel.document.can_redo());
    assert!(!panel.document.dirty());
    panel.document.set_render_admission_error(None);
    panel.document.redo().unwrap();
    assert_eq!(panel.document.bytes(), Some(changed.as_slice()));
}
#[test]
fn play_and_stale_scene_refuse_hole_rectangle_without_mutation() {
    for stale in [false, true] {
        let (temp, scene, mut editor, mut panel) = fixture();
        let before = panel.document.bytes().unwrap().to_vec();
        if stale {
            let next = temp.path().join("next.yaml");
            fs::copy(scene, &next).unwrap();
            assert!(editor.open_path(&next));
        } else {
            assert!(editor.start_play());
        }
        settle(&mut editor);
        assert!(panel.apply_hole_rectangle(&editor).is_err());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(!panel.document.can_undo());
    }
}

#[test]
fn installed_package_is_read_only_and_never_modified_by_hole_rectangle() {
    use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};
    let (temp, _scene, editor, mut panel) = fixture();
    let project = temp.path().join("project");
    let installed = install_and_reload(
        panel.document.terrain().unwrap(),
        &project,
        &temp.path().join("package-source"),
        "1.0.0",
    )
    .unwrap();
    let object = project
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join(ASSET_PATH);
    let original = fs::read(&object).unwrap();
    panel
        .document
        .open_package(&project, PACKAGE_NAME, ASSET_PATH)
        .unwrap();
    assert!(panel.document.read_only());
    assert!(panel.apply_hole_rectangle(&editor).is_err());
    assert_eq!(fs::read(object).unwrap(), original);
    assert_eq!(panel.document.bytes(), Some(original.as_slice()));
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
}

#[test]
fn minimal_and_nonsquare_last_cells_use_cell_not_vertex_bounds() {
    for (width, depth) in [(2, 2), (3, 6), (6, 3)] {
        let terrain = Terrain::new(
            "terrain/edge".into(),
            width,
            depth,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO; (width * depth) as usize],
            vec![false; ((width - 1) * (depth - 1)) as usize],
        )
        .unwrap();
        let last = [width - 2, depth - 2];
        let edits = edits(&terrain, last, last, true).unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(edits[0],Edit::SetHole{x,z,hole:true} if x==last[0] && z==last[1]));
        assert!(super::edits(&terrain, last, [width - 1, depth - 2], true).is_err());
        assert!(super::edits(&terrain, last, [width - 2, depth - 1], true).is_err());
    }
}
