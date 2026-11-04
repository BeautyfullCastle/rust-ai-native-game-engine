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
    panel.sculpt = Spec {
        center: [2, 2],
        radius: 2,
        delta: "4".into(),
    };
    (temp, scene, editor, panel)
}

#[test]
fn actual_sculpt_button_single_undo_redo_save_reopen_preserves_holes_and_scene() {
    let (temp, scene, editor, mut panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    panel.sculpt.center = [0, 0];
    panel.selected_vertex = [2, 2];
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
    h.get_by_label("Radial terrain sculpt").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    h.get_by_label("Use selected vertex as sculpt center")
        .click();
    h.run_steps(3);
    assert_eq!(h.state().0.sculpt.center, [2, 2]);
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    h.get_by_label("Apply radial sculpt").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    let terrain = panel.document.terrain().unwrap();
    for z in 0..5 {
        for x in 0..5 {
            // Independent literal footprint: center 4, axial neighbors 3,
            // diagonal neighbors 2, boundary and exterior zero.
            let bumps = [
                [0, 0, 0, 0, 0],
                [0, 2, 3, 2, 0],
                [0, 3, 4, 3, 0],
                [0, 2, 3, 2, 0],
                [0, 0, 0, 0, 0],
            ];
            let expected = FP::from_int(x - z + bumps[z as usize][x as usize]);
            assert_eq!(terrain.heights()[(z * 5 + x) as usize], expected);
        }
    }
    assert!(terrain.holes()[5]);
    assert_eq!(terrain.holes().iter().filter(|v| **v).count(), 1);
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
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
    let saved = fs::read(temp.path().join("terrain.orrt")).unwrap();
    let reopened = Terrain::load(&saved).unwrap();
    // Canonical consumer queries and render bridge use the saved bytes, not the draft.
    assert_eq!(
        reopened.sample(FP::from_int(-2), FP::from_int(-2)),
        Some(FP::from_int(4))
    );
    assert_eq!(
        reopened.sample(FP::from_int(-4), FP::from_int(-4)),
        Some(FP::ZERO)
    );
    assert_eq!(reopened.sample(FP::from_int(-3), FP::from_int(-3)), None);
    assert!(orr_terrain_view::to_static_model(&reopened, &[])
        .unwrap()
        .is_some());
    assert_eq!(
        reopened.asset_id(),
        panel.document.terrain().unwrap().asset_id()
    );
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn invalid_text_bounds_render_admission_and_noop_preserve_document_and_history() {
    let (_temp, _scene, editor, mut panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    for text in ["NaN", "inf", "bad", "1000001"] {
        panel.sculpt.delta = text.into();
        assert!(panel.apply_sculpt(&editor).is_err());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(!panel.document.can_undo());
    }
    panel.sculpt.delta = "0".into();
    panel.apply_sculpt(&editor).unwrap();
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
    panel.sculpt.center = [5, 2];
    assert!(panel.apply_sculpt(&editor).is_err());
    panel.sculpt.center = [2, 2];
    panel.sculpt.delta = "1".into();
    panel
        .document
        .set_render_admission_error(Some("test renderer capacity exhausted".into()));
    assert!(panel.apply_sculpt(&editor).is_err());
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), revision);
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
}

#[test]
fn play_and_stale_scene_refuse_sculpt_without_mutation() {
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
        assert!(panel.apply_sculpt(&editor).is_err());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(!panel.document.can_undo());
    }
}

#[test]
fn installed_package_is_read_only_and_never_modified_by_sculpt() {
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
    assert!(panel.apply_sculpt(&editor).is_err());
    assert_eq!(fs::read(object).unwrap(), original);
    assert_eq!(panel.document.bytes(), Some(original.as_slice()));
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
}

#[test]
fn noops_and_rejections_preserve_redo_and_repeated_clicks_are_explicit_sculpts() {
    let (_temp, _scene, editor, mut panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    panel.apply_sculpt(&editor).unwrap();
    let once = panel.document.bytes().unwrap().to_vec();
    panel.apply_sculpt(&editor).unwrap();
    assert_ne!(panel.document.bytes(), Some(once.as_slice()));
    panel.document.undo().unwrap();
    assert_eq!(panel.document.bytes(), Some(once.as_slice()));
    panel.document.undo().unwrap();
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(panel.document.can_redo());
    for delta in ["0", "NaN", "1000001"] {
        panel.sculpt.delta = delta.into();
        let result = panel.apply_sculpt(&editor);
        assert_eq!(result.is_ok(), delta == "0");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(panel.document.can_redo());
    }
    panel.document.redo().unwrap();
    assert_eq!(panel.document.bytes(), Some(once.as_slice()));
}
