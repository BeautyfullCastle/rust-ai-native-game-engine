#![allow(clippy::disallowed_types)]
use super::{edits, Spec};
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
        dimensions: [7, 7],
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel
        .document
        .apply(&[
            Edit::SetHeight {
                x: 2,
                z: 2,
                height: FP::from_raw(-589825),
            },
            Edit::SetHole {
                x: 5,
                z: 5,
                hole: true,
            },
        ])
        .unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    panel.smooth = Spec {
        min: [1, 1],
        max: [3, 3],
    };
    (temp, scene, editor, panel)
}

#[test]
fn actual_smooth_button_literal_immutable_average_single_undo_redo_save_reopen() {
    let (temp, scene, editor, panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let mut h = Harness::builder().with_size([1400., 2800.]).build_ui_state(
        |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Smooth terrain rectangle").click();
    h.run_steps(3);
    h.get_by_label("Apply one smoothing pass").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    let terrain = panel.document.terrain().unwrap();
    // Every targeted 3x3 source neighborhood contains the original negative impulse.
    // -589825 / 9 truncates toward zero to -65536; no previously edited sample is reused.
    for z in 0..7 {
        for x in 0..7 {
            let expected = if (1..=3).contains(&x) && (1..=3).contains(&z) {
                -65536
            } else {
                0
            };
            assert_eq!(terrain.heights()[(z * 7 + x) as usize].raw(), expected);
        }
    }
    assert!(terrain.holes()[35]);
    assert_eq!(terrain.holes().iter().filter(|h| **h).count(), 1);
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_ne!(changed, before);
    let model = panel.viewport_model(editor).unwrap();
    let positions: Vec<_> = model
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .map(|v| v.position[1].to_bits())
        .collect();
    assert!(positions.contains(&(-1.0_f32).to_bits()));
    assert!(positions
        .iter()
        .all(|&y| y == (-1.0_f32).to_bits() || y == 0.0_f32.to_bits()));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history(), &history);
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
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
    let model = panel.viewport_model(editor).unwrap();
    let positions: Vec<_> = model
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .map(|v| v.position[1].to_bits())
        .collect();
    assert!(positions.contains(&(-1.0_f32).to_bits()));
    assert!(positions
        .iter()
        .all(|&y| y == (-1.0_f32).to_bits() || y == 0.0_f32.to_bits()));
    let decoded = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(decoded.heights()[16].raw(), -65536);
    assert!(decoded.holes()[35]);
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}
fn terrain(width: u32, depth: u32, raw: Vec<i64>, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "smooth-test".into(),
        width,
        depth,
        [FP::ZERO; 2],
        FP::ONE,
        raw.into_iter().map(FP::from_raw).collect(),
        holes,
    )
    .unwrap()
}
fn value(t: &Terrain, x: u32, z: u32) -> i64 {
    let list = edits(t, [x, z], [x, z]).unwrap();
    assert!(list.len() <= 1);
    match list.first() {
        Some(Edit::SetHeight {
            x: ex,
            z: ez,
            height,
        }) => {
            assert_eq!((*ex, *ez), (x, z));
            height.raw()
        }
        None => t.heights()[(z * t.width() + x) as usize].raw(),
        _ => panic!("smoothing must not alter holes"),
    }
}
#[test]
fn literal_corner_edge_interior_signed_remainders_and_wide_sums() {
    let t = terrain(3, 3, vec![0, 1, 2, 3, 4, 5, 6, 7, 8], vec![false; 4]);
    assert_eq!(value(&t, 0, 0), 2);
    assert_eq!(value(&t, 1, 0), 2);
    assert_eq!(value(&t, 1, 1), 4);
    assert_eq!(value(&t, 2, 2), 6);
    for (sign, expected) in [(1, 1), (-1, -1)] {
        let t = terrain(3, 3, vec![5 * sign, 0, 0, 0, 0, 0, 0, 0, 0], vec![false; 4]);
        assert_eq!(value(&t, 0, 0), expected);
        assert_eq!(value(&t, 1, 0), 0);
        assert_eq!(value(&t, 1, 1), 0);
    }
    let mixed = terrain(3, 3, vec![-9, 2, 3, 1, 0, 0, 0, 0, 0], vec![false; 4]);
    assert_eq!(value(&mixed, 0, 0), -1);
    assert_eq!(value(&mixed, 1, 0), 0);
    assert_eq!(value(&mixed, 1, 1), 0);
    let mut raw = vec![i64::MAX; 9];
    raw[4] = i64::MIN;
    let t = terrain(3, 3, raw, vec![false; 4]);
    assert_eq!(value(&t, 1, 1), 7173733806442603405);
    let mut raw = vec![i64::MIN; 9];
    raw[4] = i64::MAX;
    let t = terrain(3, 3, raw, vec![false; 4]);
    assert_eq!(value(&t, 1, 1), -7173733806442603406);
}
#[test]
fn bounds_budget_includes_noops_and_deterministic_changed_only_order() {
    let t = terrain(65, 65, vec![17; 65 * 65], vec![false; 64 * 64]);
    let before = t.cook();
    assert!(edits(&t, [0, 0], [63, 63]).unwrap().is_empty());
    assert!(edits(&t, [0, 0], [64, 64]).is_err());
    for (min, max) in [
        ([2, 0], [1, 1]),
        ([0, 2], [1, 1]),
        ([0, 0], [65, 1]),
        ([u32::MAX, 0], [u32::MAX, 0]),
    ] {
        assert!(edits(&t, min, max).is_err());
    }
    assert_eq!(t.cook(), before);
    let mut raw = vec![0; 49];
    raw[16] = 91;
    let t = terrain(7, 7, raw, vec![false; 36]);
    let before = t.cook();
    let list = edits(&t, [1, 1], [3, 3]).unwrap();
    assert_eq!(list.len(), 9);
    for (i, e) in list.iter().enumerate() {
        assert!(
            matches!(e,Edit::SetHeight{x,z,height} if *x==1+(i as u32%3)&&*z==1+(i as u32/3)&&height.raw()==10)
        );
    }
    assert_eq!(t.cook(), before);
}
#[test]
fn any_incident_hole_in_entire_read_halo_rejects_but_distant_holes_are_preserved() {
    // target vertex [3,3] reads vertices 2..4; incident cells extend 1..4 on each axis.
    for z in 1..=4 {
        for x in 1..=4 {
            let mut holes = vec![false; 36];
            holes[z * 6 + x] = true;
            let t = terrain(7, 7, vec![0; 49], holes);
            let before = t.cook();
            assert!(
                edits(&t, [3, 3], [3, 3]).is_err(),
                "hole {x},{z} must reject even a no-op"
            );
            assert_eq!(t.cook(), before);
        }
    }
    let mut holes = vec![false; 36];
    holes[35] = true;
    let t = terrain(7, 7, vec![0; 49], holes);
    assert!(edits(&t, [3, 3], [3, 3]).unwrap().is_empty());
    assert!(t.holes()[35]);
    let t = terrain(2, 2, vec![1, 2, 3, 4], vec![true]);
    assert!(edits(&t, [0, 0], [0, 0]).is_err());
}
#[test]
fn no_op_preserves_redo_revision_model_and_dirty_state() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 6,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    panel.smooth = Spec {
        min: [0, 0],
        max: [0, 0],
    };
    let before = panel.document.bytes().unwrap().to_vec();
    let rev = panel.document.revision();
    let dirty = panel.document.dirty();
    let model = panel.viewport_model(&editor).unwrap();
    assert!(panel.document.can_redo());
    panel.apply_smooth(&editor).unwrap();
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), rev);
    assert_eq!(panel.document.dirty(), dirty);
    assert!(panel.document.can_redo());
    assert!(!panel.document.can_undo());
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &panel.viewport_model(&editor).unwrap()
    ));
}
#[test]
fn bounds_hole_and_nonempty_render_failure_preserve_all_applied_state() {
    let (temp, scene, editor, mut panel) = fixture();
    let original_file = fs::read(temp.path().join("terrain.orrt")).unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    for case in ["bounds", "hole", "renderer"] {
        panel.smooth = Spec {
            min: [1, 1],
            max: [3, 3],
        };
        match case {
            "bounds" => panel.smooth.max = [7, 3],
            "hole" => {
                panel.smooth = Spec {
                    min: [4, 4],
                    max: [4, 4],
                }
            }
            _ => {
                assert!(!edits(
                    panel.document.terrain().unwrap(),
                    panel.smooth.min,
                    panel.smooth.max
                )
                .unwrap()
                .is_empty());
                panel.document.set_render_admission_error(Some(
                    "injected nonempty smoothing render rejection".into(),
                ));
            }
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let model = panel.viewport_model(&editor).unwrap();
        assert!(panel.apply_smooth(&editor).is_err(), "{case}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert!(std::sync::Arc::ptr_eq(
            &model,
            &panel.viewport_model(&editor).unwrap()
        ));
        assert!(!panel.document.can_undo());
        assert!(!panel.document.can_redo());
        assert!(!panel.document.dirty());
    }
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original_file
    );
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}
#[test]
fn play_and_stale_scene_refuse_smooth_without_mutation() {
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
        assert!(panel.apply_smooth(&editor).is_err());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(!panel.document.can_undo());
    }
}

#[test]
fn installed_package_is_read_only_and_never_modified_by_smooth() {
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
    assert!(panel.apply_smooth(&editor).is_err());
    assert_eq!(fs::read(object).unwrap(), original);
    assert_eq!(panel.document.bytes(), Some(original.as_slice()));
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
}
