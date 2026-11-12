#![allow(clippy::disallowed_types)]
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain};
use orr_terrain_view::raw16::Endian;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const SAMPLES: [u16; 9] = [0, 65535, 32768, 16384, 49152, 1, 65534, 0, 65535];
// offset -1, scale 1: -65536 + floor(sample * 65536 / 65535).
const HEIGHTS: [i64; 9] = [-65536, 0, -32768, -49152, -16384, -65535, -2, -65536, 0];
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
        dimensions: [3, 3],
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel
        .document
        .apply(&[Edit::SetHole {
            x: 0,
            z: 0,
            hole: true,
        }])
        .unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    let source = temp.path().join("heights.raw");
    fs::write(
        &source,
        SAMPLES
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    panel.raw16.fields.path = source.to_str().unwrap().into();
    panel.raw16.fields.offset = "-1".into();
    panel.raw16.fields.scale = "1".into();
    panel.raw16.fields.endian = Endian::Little;
    panel.raw16.fields.flip_z = false;
    (temp, scene, editor, panel)
}
fn heights(panel: &TerrainPanel) -> Vec<i64> {
    panel
        .document
        .terrain()
        .unwrap()
        .heights()
        .iter()
        .map(|h| h.raw())
        .collect()
}
#[test]
fn actual_preview_apply_single_undo_redo_save_reopen_preserves_holes_and_source() {
    let (temp, scene, editor, panel) = fixture();
    let original = panel.document.bytes().unwrap().to_vec();
    let original_terrain = panel.document.terrain().unwrap().clone();
    let model = panel.viewport_model(&editor).unwrap();
    let raw = fs::read(temp.path().join("heights.raw")).unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let mut h = Harness::builder().with_size([1400., 3200.]).build_ui_state(
        |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Import RAW16 heights").click();
    h.run_steps(3);
    h.get_by_label("Preview RAW16 heights").click();
    h.run_steps(3);
    assert!(h.state().0.raw16.has_preview(), "{:?}", h.state().0.error());
    assert_eq!(h.state().0.document.bytes(), Some(original.as_slice()));
    assert!(!h.state().0.document.can_undo());
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &h.state().0.viewport_model(&h.state().1).unwrap()
    ));
    h.get_by_label("Apply RAW16 heights").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert!(!panel.raw16.has_preview());
    assert_eq!(heights(panel), HEIGHTS);
    let t = panel.document.terrain().unwrap();
    assert_eq!(t.holes(), original_terrain.holes());
    // Compare every non-height byte through a terrain reconstructed from the old
    // geometry; no import-generated identity/origin/spacing may replace it.
    let expected = Terrain::new(
        original_terrain.asset_id().to_owned(),
        original_terrain.width(),
        original_terrain.depth(),
        original_terrain.origin(),
        original_terrain.spacing(),
        HEIGHTS.into_iter().map(FP::from_raw).collect(),
        original_terrain.holes().to_vec(),
    )
    .unwrap();
    assert_eq!(t.cook(), expected.cook());
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original
    );
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    h.get_by_label("Terrain Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(original.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Terrain Redo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    let (panel, editor) = h.state_mut();
    panel.open_for_editor(editor).unwrap();
    assert_eq!(heights(panel), HEIGHTS);
    assert!(!panel.document.dirty());
    let decoded = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(decoded.cook(), expected.cook());
    let model = panel.viewport_model(editor).unwrap();
    let ys: Vec<_> = model
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .map(|v| v.position[1])
        .collect();
    assert!(!ys.is_empty());
    assert!(ys.contains(&-1.0));
    assert!(ys
        .iter()
        .all(|y| HEIGHTS.iter().any(|raw| *y == *raw as f32 / 65536.0)));
    assert_eq!(fs::read(temp.path().join("heights.raw")).unwrap(), raw);
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}
#[test]
fn endian_and_flip_use_literal_heights_without_preview_mutation() {
    for endian in [Endian::Little, Endian::Big] {
        for flip in [false, true] {
            let (temp, _scene, editor, mut panel) = fixture();
            let bytes: Vec<_> = SAMPLES
                .iter()
                .flat_map(|n| match endian {
                    Endian::Little => n.to_le_bytes(),
                    Endian::Big => n.to_be_bytes(),
                })
                .collect();
            fs::write(temp.path().join("heights.raw"), &bytes).unwrap();
            panel.raw16.fields.endian = endian;
            panel.raw16.fields.flip_z = flip;
            let before = panel.document.bytes().unwrap().to_vec();
            panel.preview_raw16(&editor).unwrap();
            assert_eq!(panel.document.bytes(), Some(before.as_slice()));
            panel.apply_raw16(&editor).unwrap();
            let expected = if flip {
                vec![
                    HEIGHTS[6], HEIGHTS[7], HEIGHTS[8], HEIGHTS[3], HEIGHTS[4], HEIGHTS[5],
                    HEIGHTS[0], HEIGHTS[1], HEIGHTS[2],
                ]
            } else {
                HEIGHTS.to_vec()
            };
            assert_eq!(heights(&panel), expected);
            assert!(panel.document.terrain().unwrap().holes()[0]);
            assert_eq!(fs::read(temp.path().join("heights.raw")).unwrap(), bytes);
        }
    }
}
#[test]
fn failed_preview_revokes_prior_candidate_and_preserves_every_applied_state() {
    for case in [
        "negative-scale",
        "invalid-offset",
        "overflow",
        "relative",
        "traversal",
        "alias",
        "missing",
        "directory",
        "short",
        "long",
        "symlink",
    ] {
        let (temp, scene, editor, mut panel) = fixture();
        panel.preview_raw16(&editor).unwrap();
        assert!(panel.raw16.has_preview());
        match case {
            "negative-scale" => panel.raw16.fields.scale = "-1".into(),
            "invalid-offset" => panel.raw16.fields.offset = "NaN".into(),
            "overflow" => panel.raw16.fields.offset = "9999999999999999999999999999".into(),
            "relative" => panel.raw16.fields.path = "heights.raw".into(),
            "traversal" => {
                panel.raw16.fields.path =
                    temp.path().join("../heights.raw").to_str().unwrap().into()
            }
            "alias" => {
                panel.raw16.fields.path = temp.path().join("terrain.orrt").to_str().unwrap().into()
            }
            "missing" => {
                panel.raw16.fields.path = temp.path().join("missing.raw").to_str().unwrap().into()
            }
            "directory" => panel.raw16.fields.path = temp.path().to_str().unwrap().into(),
            "short" => fs::write(temp.path().join("heights.raw"), [0; 17]).unwrap(),
            "long" => fs::write(temp.path().join("heights.raw"), [0; 19]).unwrap(),
            "symlink" => {
                #[cfg(unix)]
                {
                    let link = temp.path().join("link.raw");
                    std::os::unix::fs::symlink(temp.path().join("heights.raw"), &link).unwrap();
                    panel.raw16.fields.path = link.to_str().unwrap().into();
                }
                #[cfg(not(unix))]
                {
                    continue;
                }
            }
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let model = panel.viewport_model(&editor).unwrap();
        let history = editor.history().clone();
        let checksum = editor.checksum();
        let scene_bytes = fs::read(&scene).unwrap();
        let source = fs::read(temp.path().join("heights.raw")).unwrap();
        assert!(panel.preview_raw16(&editor).is_err(), "{case}");
        assert!(!panel.raw16.has_preview(), "{case}");
        assert!(panel.apply_raw16(&editor).is_err(), "{case}");
        assert!(!panel.raw16.has_preview());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
        assert!(std::sync::Arc::ptr_eq(
            &model,
            &panel.viewport_model(&editor).unwrap()
        ));
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
        assert_eq!(fs::read(temp.path().join("heights.raw")).unwrap(), source);
        assert_eq!(fs::read(scene).unwrap(), scene_bytes);
        assert_eq!(editor.history(), &history);
        assert_eq!(editor.checksum(), checksum);
    }
}
#[test]
fn stale_source_parameters_terrain_and_render_failure_cannot_apply() {
    for case in ["source", "parameters", "terrain", "renderer"] {
        let (temp, scene, editor, mut panel) = fixture();
        panel.preview_raw16(&editor).unwrap();
        match case {
            "source" => fs::write(temp.path().join("heights.raw"), [0; 18]).unwrap(),
            "parameters" => panel.raw16.fields.flip_z = true,
            "terrain" => panel
                .document
                .apply(&[Edit::SetHeight {
                    x: 2,
                    z: 2,
                    height: FP::ONE,
                }])
                .unwrap(),
            "renderer" => panel
                .document
                .set_render_admission_error(Some("injected RAW16 render failure".into())),
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let dirty = panel.document.dirty();
        let undo = panel.document.can_undo();
        let redo = panel.document.can_redo();
        let model = panel.viewport_model(&editor).unwrap();
        let file = fs::read(temp.path().join("terrain.orrt")).unwrap();
        let raw = fs::read(temp.path().join("heights.raw")).unwrap();
        let scene_bytes = fs::read(&scene).unwrap();
        let history = editor.history().clone();
        let checksum = editor.checksum();
        assert!(panel.apply_raw16(&editor).is_err(), "{case}");
        assert!(!panel.raw16.has_preview());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert_eq!(panel.document.dirty(), dirty);
        assert_eq!(panel.document.can_undo(), undo);
        assert_eq!(panel.document.can_redo(), redo);
        assert!(std::sync::Arc::ptr_eq(
            &model,
            &panel.viewport_model(&editor).unwrap()
        ));
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), file);
        assert_eq!(fs::read(temp.path().join("heights.raw")).unwrap(), raw);
        assert_eq!(fs::read(scene).unwrap(), scene_bytes);
        assert_eq!(editor.history(), &history);
        assert_eq!(editor.checksum(), checksum);
    }
}
#[test]
fn undo_to_identical_source_allows_preview_reuse_and_noop_keeps_redo() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.preview_raw16(&editor).unwrap();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 2,
            z: 2,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    panel.apply_raw16(&editor).unwrap();
    assert_eq!(heights(&panel), HEIGHTS);
    // A second import is a no-op. A different unapplied redo branch must survive.
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 2,
            z: 2,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    assert!(panel.document.can_redo());
    let bytes = panel.document.bytes().unwrap().to_vec();
    let rev = panel.document.revision();
    let dirty = panel.document.dirty();
    let model = panel.viewport_model(&editor).unwrap();
    panel.preview_raw16(&editor).unwrap();
    panel.apply_raw16(&editor).unwrap();
    assert!(!panel.raw16.has_preview());
    assert!(panel.document.can_redo());
    assert_eq!(panel.document.bytes(), Some(bytes.as_slice()));
    assert_eq!(panel.document.revision(), rev);
    assert_eq!(panel.document.dirty(), dirty);
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &panel.viewport_model(&editor).unwrap()
    ));
}
#[test]
fn play_epoch_and_readonly_reject_saved_preview_without_document_changes() {
    for case in ["play", "epoch", "readonly"] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.preview_raw16(&editor).unwrap();
        match case {
            "play" => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            "epoch" => {
                let next = temp.path().join("next.yaml");
                fs::copy(&scene, &next).unwrap();
                assert!(editor.open_path(&next));
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
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let raw = fs::read(temp.path().join("heights.raw")).unwrap();
        assert!(panel.apply_raw16(&editor).is_err(), "{case}");
        assert!(panel.preview_raw16(&editor).is_err(), "{case}");
        assert!(!panel.raw16.has_preview());
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
        assert_eq!(fs::read(temp.path().join("heights.raw")).unwrap(), raw);
    }
}
#[test]
fn full_footprint_budget_counts_unchanged_vertices() {
    let (temp, _scene, editor, mut panel) = fixture();
    for side in [64, 65] {
        panel.dimensions = [side, side];
        panel.local_path = format!("terrain-{side}.orrt");
        panel.create_for_editor(&editor).unwrap();
        panel.save_for_editor(&editor).unwrap();
        panel.open_for_editor(&editor).unwrap();
        fs::write(
            temp.path().join("heights.raw"),
            vec![0; 2 * (side * side) as usize],
        )
        .unwrap();
        panel.raw16.fields.offset = "0".into();
        panel.raw16.fields.scale = "0".into();
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let result = panel.preview_raw16(&editor);
        if side == 64 {
            result.unwrap();
            panel.apply_raw16(&editor).unwrap();
        } else {
            assert!(result.is_err());
            assert!(!panel.raw16.has_preview());
        }
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
    }
}
