#![allow(clippy::disallowed_types)]
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use orr_terrain::Edit;
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
        include_str!("../../../../../../scenes/yard3d_authoring.scene.yaml"),
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

use super::Snapshot;

#[test]
fn actual_export_dirty_heights_reimport_undo_redo_save_reopen() {
    let (temp, scene, editor, mut panel) = fixture();
    let backing = fs::read(temp.path().join("terrain.orrt")).unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    panel.preview_raw16(&editor).unwrap();
    panel.apply_raw16(&editor).unwrap();
    panel.preview_raw16(&editor).unwrap();
    let wanted = panel.document.terrain().unwrap().clone();
    let cook = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    let model = panel.viewport_model(&editor).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let output = temp.path().join("export.raw");
    panel.raw16.output_path = output.to_str().unwrap().into();
    let mut h = Harness::builder().with_size([1400., 3200.]).build_ui_state(
        |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Import RAW16 heights").click();
    h.run_steps(3);
    h.get_by_label("Export RAW16 heights").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert_eq!(
        fs::read(&output).unwrap(),
        SAMPLES
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<_>>()
    );
    assert_eq!(panel.document.bytes(), Some(cook.as_slice()));
    assert_eq!(panel.document.revision(), revision);
    assert!(panel.document.dirty());
    assert!(panel.document.can_undo());
    assert!(panel.raw16.has_preview());
    assert!(std::sync::Arc::ptr_eq(
        &model,
        &panel.viewport_model(editor).unwrap()
    ));
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), backing);
    // An existing terrain operation changes memory before importing the export.
    let (panel, editor) = h.state_mut();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 2,
            z: 2,
            height: FP::ONE,
        }])
        .unwrap();
    let changed = panel.document.bytes().unwrap().to_vec();
    panel.raw16.fields.path = output.to_str().unwrap().into();
    let _ = editor;
    h.get_by_label("Preview RAW16 heights").click();
    h.run_steps(3);
    h.get_by_label("Apply RAW16 heights").click();
    h.run_steps(3);
    assert_eq!(heights(&h.state().0), HEIGHTS);
    assert_eq!(
        h.state().0.document.terrain().unwrap().cook(),
        wanted.cook()
    );
    h.get_by_label("Terrain Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    h.get_by_label("Terrain Redo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(cook.as_slice()));
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), backing);
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    let (panel, editor) = h.state_mut();
    panel.open_for_editor(editor).unwrap();
    assert_eq!(heights(panel), HEIGHTS);
    assert_eq!(panel.document.terrain().unwrap().cook(), wanted.cook());
    assert!(!panel.document.dirty());
    assert!(panel.document.terrain().unwrap().holes()[0]);
    let ys: Vec<_> = panel
        .viewport_model(editor)
        .unwrap()
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .map(|v| v.position[1])
        .collect();
    assert!(ys.contains(&-1.0));
    assert!(ys
        .iter()
        .all(|y| HEIGHTS.iter().any(|raw| *y == *raw as f32 / 65536.0)));
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn export_refusals_keep_candidate_memory_history_and_files() {
    for case in [
        "unrepresentable",
        "existing",
        "relative",
        "extension",
        "play",
        "epoch",
        "symlink",
    ] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.preview_raw16(&editor).unwrap();
        let output = temp.path().join("export.raw");
        panel.raw16.output_path = output.to_str().unwrap().into();
        match case {
            "unrepresentable" => panel
                .document
                .apply(&[Edit::SetHeight {
                    x: 0,
                    z: 0,
                    height: FP::ONE,
                }])
                .unwrap(),
            "existing" => fs::write(&output, b"keep me").unwrap(),
            "relative" => panel.raw16.output_path = "export.raw".into(),
            "extension" => {
                panel.raw16.output_path = temp.path().join("export.bin").to_str().unwrap().into()
            }
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
            "symlink" => {
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(temp.path().join("heights.raw"), &output).unwrap();
                }
                #[cfg(not(unix))]
                {
                    continue;
                }
            }
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let revision = panel.document.revision();
        let flags = (
            panel.document.dirty(),
            panel.document.can_undo(),
            panel.document.can_redo(),
        );
        let history = editor.history().clone();
        let checksum = editor.checksum();
        let backing = fs::read(temp.path().join("terrain.orrt")).unwrap();
        let prior_output = fs::read(&output).ok();
        assert!(panel.export_raw16(&editor).is_err(), "{case}");
        assert!(panel.raw16.has_preview(), "{case}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert_eq!(
            (
                panel.document.dirty(),
                panel.document.can_undo(),
                panel.document.can_redo()
            ),
            flags
        );
        assert_eq!(editor.history(), &history);
        assert_eq!(editor.checksum(), checksum);
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), backing);
        assert_eq!(fs::read(output).ok(), prior_output);
    }
}

#[test]
fn snapshot_rejects_external_source_settings_memory_and_epoch_changes() {
    for case in [
        "scene",
        "terrain-file",
        "settings",
        "memory",
        "output",
        "epoch",
        "play",
    ] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.raw16.output_path = temp.path().join("export.raw").to_str().unwrap().into();
        let pinned = Snapshot::capture(&panel, &editor).unwrap();
        match case {
            "scene" => {
                let mut b = fs::read(&scene).unwrap();
                b.push(b'\n');
                fs::write(&scene, b).unwrap();
            }
            "terrain-file" => fs::write(temp.path().join("terrain.orrt"), b"changed").unwrap(),
            "settings" => panel.raw16.fields.flip_z = true,
            "memory" => panel
                .document
                .apply(&[Edit::SetHeight {
                    x: 0,
                    z: 0,
                    height: FP::ONE,
                }])
                .unwrap(),
            "output" => panel.raw16.output_path.push_str(".raw"),
            "epoch" => {
                assert!(editor.open_path(&scene));
                settle(&mut editor);
            }
            "play" => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            _ => unreachable!(),
        }
        assert!(pinned.check(&panel, &editor).is_err(), "{case}");
        assert!(!temp.path().join("export.raw").exists());
    }
}

#[test]
fn shared_endian_flip_and_zero_scale_have_literal_output() {
    for endian in [Endian::Little, Endian::Big] {
        for flip in [false, true] {
            let (temp, _scene, editor, mut panel) = fixture();
            panel.preview_raw16(&editor).unwrap();
            panel.apply_raw16(&editor).unwrap();
            panel.raw16.fields.endian = endian;
            panel.raw16.fields.flip_z = flip;
            let output = temp.path().join("export.raw");
            panel.raw16.output_path = output.to_str().unwrap().into();
            panel.export_raw16(&editor).unwrap();
            let mut samples = SAMPLES.to_vec();
            if flip {
                samples = samples.chunks(3).rev().flatten().copied().collect();
            }
            let expected: Vec<_> = samples
                .iter()
                .flat_map(|s| match endian {
                    Endian::Little => s.to_le_bytes(),
                    Endian::Big => s.to_be_bytes(),
                })
                .collect();
            assert_eq!(fs::read(output).unwrap(), expected);
        }
    }
    let (temp, _scene, editor, mut panel) = fixture();
    panel.raw16.fields.offset = "0".into();
    panel.raw16.fields.scale = "0".into();
    panel.raw16.output_path = temp.path().join("zero.raw").to_str().unwrap().into();
    panel.export_raw16(&editor).unwrap();
    assert_eq!(fs::read(temp.path().join("zero.raw")).unwrap(), [0; 18]);
}

#[test]
fn backing_reader_is_bounded_regular_and_symlink_free() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    fs::write(&path, [1; 9]).unwrap();
    assert!(super::read_bounded(&path, 8).is_err());
    assert_eq!(super::read_bounded(&path, 9).unwrap(), [1; 9]);
    assert!(super::read_bounded(temp.path(), 100).is_err());
    assert!(super::read_bounded(&temp.path().join("../source"), 100).is_err());
    #[cfg(unix)]
    {
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(super::read_bounded(&alias, 100).is_err());
    }
}
