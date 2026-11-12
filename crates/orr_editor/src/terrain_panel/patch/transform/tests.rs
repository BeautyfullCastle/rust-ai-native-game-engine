use super::super::tests::{fixture, settle, terrain};
use super::*;
#[cfg(unix)]
use crate::{editor::Editor, terrain_panel::TerrainPanel};
#[cfg(unix)]
use egui_kittest::{Harness, kittest::Queryable};
use orr_fp::FP;
use orr_terrain::Terrain;
use std::fs;

fn assert_patch(actual: &Clipboard, expected: &Clipboard) {
    assert_eq!(actual.geometry, expected.geometry);
    assert_eq!(actual.cells, expected.cells);
    assert_eq!(actual.heights, expected.heights);
    assert_eq!(actual.holes, expected.holes);
}

#[test]
fn asymmetric_vertex_and_cell_arrays_have_literal_transform_mappings() {
    let source = Terrain::new(
        "terrain/patch".into(),
        4,
        3,
        [FP::from_int(-2), FP::from_int(7)],
        FP::from_int(2),
        [1, 2, 3, 4, 11, 12, 13, 14, 21, 22, 23, 24]
            .into_iter()
            .map(FP::from_raw)
            .collect(),
        vec![true, false, false, false, true, false],
    )
    .unwrap();
    let original = Clipboard::capture(&source, [0, 0], [2, 1]).unwrap();
    for (transform, cells, heights, holes) in [
        (
            Transform::RotateClockwise,
            [2, 3],
            vec![21, 11, 1, 22, 12, 2, 23, 13, 3, 24, 14, 4],
            vec![false, true, true, false, false, false],
        ),
        (
            Transform::MirrorX,
            [3, 2],
            vec![4, 3, 2, 1, 14, 13, 12, 11, 24, 23, 22, 21],
            vec![false, false, true, false, true, false],
        ),
        (
            Transform::MirrorZ,
            [3, 2],
            vec![21, 22, 23, 24, 11, 12, 13, 14, 1, 2, 3, 4],
            vec![false, true, false, true, false, false],
        ),
    ] {
        let changed = original.transformed(transform);
        assert_eq!(changed.cells, cells);
        assert_eq!(
            changed.heights,
            heights.into_iter().map(FP::from_raw).collect::<Vec<_>>()
        );
        assert_eq!(changed.holes, holes);
        assert_eq!(changed.geometry, original.geometry);
        assert_eq!(changed.footprint(), original.footprint());
    }
    assert_eq!(original.heights, source.heights());
    assert_eq!(original.holes, source.holes());
    let mut rotated = original.clone();
    for _ in 0..4 {
        rotated = rotated.transformed(Transform::RotateClockwise);
    }
    assert_patch(&rotated, &original);
    for transform in [Transform::MirrorX, Transform::MirrorZ] {
        assert_patch(
            &original.transformed(transform).transformed(transform),
            &original,
        );
    }
}

#[test]
fn singleton_and_maximum_admitted_rectangles_preserve_exact_raw_values() {
    for (width, depth) in [(2, 2), (45, 46), (46, 45)] {
        let source = terrain(width, depth);
        let original = Clipboard::capture(&source, [0, 0], [width - 2, depth - 2]).unwrap();
        for transform in [
            Transform::RotateClockwise,
            Transform::MirrorX,
            Transform::MirrorZ,
        ] {
            let changed = original.transformed(transform);
            assert_eq!(changed.footprint(), original.footprint());
            assert_eq!(changed.heights, original.heights);
            assert_eq!(changed.holes, original.holes);
        }
    }
    let source = Terrain::new(
        "terrain/raw".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        [i64::MIN, -1, 1, i64::MAX]
            .into_iter()
            .map(FP::from_raw)
            .collect(),
        vec![true],
    )
    .unwrap();
    let original = Clipboard::capture(&source, [0, 0], [0, 0]).unwrap();
    assert_eq!(
        original.transformed(Transform::RotateClockwise).heights,
        [1, i64::MIN, i64::MAX, -1].map(FP::from_raw)
    );
}

#[test]
fn transforms_preserve_document_redo_files_and_ignore_pending_paste_fields() {
    let (temp, scene, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    panel.paste_terrain_patch(&editor).unwrap();
    panel.document.undo().unwrap();
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let original = panel.patch.clipboard.as_ref().unwrap().patch.clone();
    panel.patch.destination = [u32::MAX; 2];
    panel.patch.height_offset = "not a height".into();
    for transform in [
        Transform::MirrorX,
        Transform::MirrorX,
        Transform::MirrorZ,
        Transform::MirrorZ,
    ] {
        panel.transform_terrain_patch(&editor, transform).unwrap();
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(panel.document.can_redo());
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
    }
    assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, &original);
    // A geometrically unchanged paste must not consume the existing redo entry.
    panel.patch.destination = [0, 0];
    panel.patch.height_offset = "0".into();
    panel.paste_terrain_patch(&editor).unwrap();
    assert!(panel.document.can_redo());
    assert_eq!(panel.document.revision(), revision);
    panel.patch.destination = [u32::MAX; 2];
    assert!(panel.paste_terrain_patch(&editor).is_err());
    assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, &original);
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(panel.document.can_redo());
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn missing_stale_play_and_changed_geometry_refuse_transforms_without_mutation() {
    for case in ["missing", "scene", "epoch", "play", "source", "geometry"] {
        let (temp, scene, mut editor, mut panel) = fixture();
        if case != "missing" {
            panel.copy_terrain_patch(&editor).unwrap();
        }
        match case {
            "scene" => {
                let next = temp.path().join("next.yaml");
                fs::copy(&scene, &next).unwrap();
                assert!(editor.open_path(&next));
                settle(&mut editor);
            }
            "epoch" => {
                assert!(editor.open_path(&scene));
                settle(&mut editor);
            }
            "play" => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            "source" => {
                fs::copy(
                    temp.path().join("terrain.orrt"),
                    temp.path().join("other.orrt"),
                )
                .unwrap();
                panel.document.open_local("other.orrt").unwrap();
            }
            "geometry" => {
                panel
                    .patch
                    .clipboard
                    .as_mut()
                    .unwrap()
                    .patch
                    .geometry
                    .spacing = FP::from_int(2)
            }
            _ => {}
        }
        let original = panel
            .patch
            .clipboard
            .as_ref()
            .map(|bound| bound.patch.clone());
        let before = panel.document.bytes().unwrap().to_vec();
        let revision = panel.document.revision();
        for transform in [
            Transform::RotateClockwise,
            Transform::MirrorX,
            Transform::MirrorZ,
        ] {
            assert!(
                panel.transform_terrain_patch(&editor, transform).is_err(),
                "{case}"
            );
            assert_eq!(panel.document.bytes(), Some(before.as_slice()), "{case}");
            assert_eq!(panel.document.revision(), revision);
            assert!(!panel.document.can_undo());
            match &original {
                Some(original) => {
                    assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, original)
                }
                None => assert!(!panel.patch.has_clipboard()),
            }
        }
    }
}

// Terrain patch asset reads are implemented for Unix only.
#[cfg(unix)]
#[test]
fn widgets_transform_asset_roundtrip_paste_undo_redo_save_reopen_and_consume() {
    for (label, cells, heights, holes, surface, hole, triangle_count) in [
        (
            "Rotate terrain patch clockwise",
            [1, 2],
            vec![11, 1, 12, 2, 13, 3],
            vec![true, false],
            [3.5, 3.5],
            [3.5, 2.5],
            34,
        ),
        (
            "Mirror terrain patch X",
            [2, 1],
            vec![3, 2, 1, 13, 12, 11],
            vec![false, true],
            [3.5, 2.5],
            [4.5, 2.5],
            36,
        ),
        (
            "Mirror terrain patch Z",
            [2, 1],
            vec![11, 12, 13, 1, 2, 3],
            vec![true, false],
            [4.5, 2.5],
            [3.5, 2.5],
            36,
        ),
    ] {
        let (temp, scene, editor, mut panel) = fixture();
        let before = panel.document.bytes().unwrap().to_vec();
        let scene_bytes = fs::read(&scene).unwrap();
        let history = editor.history().clone();
        let checksum = editor.checksum();
        let asset = temp.path().join("transformed.json");
        panel.patch.asset_path = asset.to_string_lossy().into_owned();
        let mut h = Harness::builder().with_size([1400., 3200.]).build_ui_state(
            |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
            (panel, editor),
        );
        h.run_steps(2);
        for action in [
            "Terrain authoring",
            "Terrain patch clipboard",
            "Copy terrain patch",
            label,
            "Save terrain patch asset",
        ] {
            h.get_by_label(action).click();
            h.run_steps(3);
            assert!(
                h.state().0.error().is_none(),
                "{action}: {:?}",
                h.state().0.error()
            );
        }
        let encoded = fs::read(&asset).unwrap();
        let expected = h.state().0.patch.clipboard.as_ref().unwrap().patch.clone();
        assert_eq!(expected.cells, cells);
        assert_eq!(
            expected.heights,
            heights
                .iter()
                .copied()
                .map(FP::from_int)
                .collect::<Vec<_>>()
        );
        assert_eq!(expected.holes, holes);
        // Deliberately discard transformed contents to prove explicit load restores them.
        for action in ["Copy terrain patch", "Load terrain patch asset"] {
            h.get_by_label(action).click();
            h.run_steps(3);
        }
        assert_patch(
            &h.state().0.patch.clipboard.as_ref().unwrap().patch,
            &expected,
        );
        assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
        assert!(!h.state().0.document.can_undo());
        h.get_by_label("Paste terrain patch").click();
        h.run_steps(3);
        assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
        let changed = h.state().0.document.bytes().unwrap().to_vec();
        assert_ne!(changed, before);
        let target = h.state().0.document.terrain().unwrap();
        for z in 0..=cells[1] {
            for x in 0..=cells[0] {
                assert_eq!(
                    target.heights()[((z + 2) * 6 + x + 3) as usize],
                    FP::from_raw(
                        i64::from(heights[(z * (cells[0] + 1) + x) as usize]) * 65536 + 32768,
                    )
                );
            }
        }
        for z in 0..cells[1] {
            for x in 0..cells[0] {
                assert_eq!(
                    target.holes()[((z + 2) * 5 + x + 3) as usize],
                    holes[(z * cells[0] + x) as usize]
                );
            }
        }
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
        h.get_by_label("Paste terrain patch").click();
        h.run_steps(3);
        h.get_by_label("Terrain Undo").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
        assert!(!h.state().0.document.can_undo());
        h.get_by_label("Terrain Redo").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
        h.get_by_label("Save terrain").click();
        h.run_steps(3);
        let mut fresh = TerrainPanel::default();
        fresh.open_for_editor(&h.state().1).unwrap();
        assert_eq!(fresh.document.bytes(), Some(changed.as_slice()));
        let saved = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
        let fixed = |n: f64| FP::from_raw((n * 65536.) as i64);
        assert_eq!(saved.sample(fixed(hole[0]), fixed(hole[1])), None);
        assert_eq!(
            saved.sample(fixed(surface[0]), fixed(surface[1])),
            Some(FP::from_int(8))
        );
        assert_eq!(saved.triangles().len(), triangle_count);
        let model = orr_terrain_view::to_static_model(&saved, &[])
            .unwrap()
            .unwrap();
        assert_eq!(
            model
                .source()
                .primitives
                .iter()
                .map(|p| p.indices.len())
                .sum::<usize>(),
            triangle_count * 3
        );
        assert_eq!(fs::read(&asset).unwrap(), encoded);
        assert_eq!(fs::read(scene).unwrap(), scene_bytes);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(h.state().1.checksum(), checksum);
    }
}

#[test]
fn installed_package_refuses_transform_without_changing_clipboard_or_object() {
    use orr_terrain_view::package::{ASSET_PATH, PACKAGE_NAME, install_and_reload};
    let (temp, _, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    let original = panel.patch.clipboard.as_ref().unwrap().patch.clone();
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
    let before = fs::read(&object).unwrap();
    panel
        .document
        .open_package(&project, PACKAGE_NAME, ASSET_PATH)
        .unwrap();
    assert!(panel.document.read_only());
    for transform in [
        Transform::RotateClockwise,
        Transform::MirrorX,
        Transform::MirrorZ,
    ] {
        assert!(panel.transform_terrain_patch(&editor, transform).is_err());
        assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, &original);
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
        assert_eq!(fs::read(&object).unwrap(), before);
    }
}

#[test]
fn rotated_footprint_rejects_old_edge_destination_then_relocates_without_clipping() {
    let (temp, _, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    panel.patch.destination = [3, 3];
    assert!(panel.patch_edits(&editor).is_ok()); // Two X cells, one Z cell fit.
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    panel
        .transform_terrain_patch(&editor, Transform::RotateClockwise)
        .unwrap();
    let rotated = panel.patch.clipboard.as_ref().unwrap().patch.clone();
    assert_eq!(rotated.cells, [1, 2]);
    assert_eq!(panel.patch.destination, [3, 3]);
    assert!(panel.patch_edits(&editor).is_err());
    assert!(panel.paste_terrain_patch(&editor).is_err());
    assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, &rotated);
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert_eq!(panel.document.revision(), revision);
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);

    panel.patch.destination = [3, 2];
    panel.paste_terrain_patch(&editor).unwrap();
    let target = panel.document.terrain().unwrap();
    for (z, row) in [[11, 1], [12, 2], [13, 3]].into_iter().enumerate() {
        for (x, value) in row.into_iter().enumerate() {
            assert_eq!(
                target.heights()[(z + 2) * 6 + x + 3],
                FP::from_raw(value * 65536 + 32768)
            );
        }
    }
    assert!(target.holes()[13]);
    assert!(!target.holes()[18]);
    assert_patch(&panel.patch.clipboard.as_ref().unwrap().patch, &rotated);
    assert!(panel.document.can_undo());
    assert!(panel.document.dirty());
    panel.document.undo().unwrap();
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(!panel.document.can_undo());
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
}
