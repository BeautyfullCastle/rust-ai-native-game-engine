#![allow(clippy::disallowed_types)]
use super::*;
use crate::{editor::Editor, game::EditorGame, model::Mode, terrain_panel::TerrainPanel};
use egui_kittest::{kittest::Queryable, Harness};
use orr_terrain::TerrainDocument;
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
fn terrain(width: u32, depth: u32) -> Terrain {
    Terrain::new(
        "terrain/patch".into(),
        width,
        depth,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; (width * depth) as usize],
        vec![false; ((width - 1) * (depth - 1)) as usize],
    )
    .unwrap()
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
        dimensions: [6, 5],
        origin: ["0".into(), "0".into()],
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    let mut edits = Vec::new();
    for (z, row) in [[1, 2, 3], [11, 12, 13]].into_iter().enumerate() {
        for (x, height) in row.into_iter().enumerate() {
            edits.push(Edit::SetHeight {
                x: x as u32,
                z: z as u32,
                height: FP::from_int(height),
            });
        }
    }
    edits.push(Edit::SetHole {
        x: 0,
        z: 0,
        hole: true,
    });
    edits.push(Edit::SetHole {
        x: 4,
        z: 2,
        hole: true,
    }); // Paste must restore this surface.
    panel.document.apply(&edits).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    panel.patch.source_min = [0, 0];
    panel.patch.source_max = [1, 0];
    panel.patch.destination = [3, 2];
    panel.patch.height_offset = "0.5".into();
    (temp, scene, editor, panel)
}
#[test]
fn widget_copy_paste_undo_save_reopen_and_real_consumers() {
    let (temp, scene, editor, panel) = fixture();
    let before = panel.document.bytes().unwrap().to_vec();
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let scene_bytes = fs::read(&scene).unwrap();
    let mut h = Harness::builder().with_size([1400., 2600.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("Terrain patch clipboard").click();
    h.run_steps(3);
    h.get_by_label("Copy terrain patch").click();
    h.run_steps(3);
    assert!(h.state().0.patch.has_clipboard());
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Paste terrain patch").click();
    h.run_steps(3);
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    let changed = h.state().0.document.bytes().unwrap().to_vec();
    let t = h.state().0.document.terrain().unwrap();
    assert_eq!(
        &t.heights()[15..18],
        &[
            FP::from_raw(98304),
            FP::from_raw(163840),
            FP::from_raw(229376)
        ]
    );
    assert_eq!(
        &t.heights()[21..24],
        &[
            FP::from_raw(753664),
            FP::from_raw(819200),
            FP::from_raw(884736)
        ]
    );
    assert!(t.holes()[13]);
    assert!(!t.holes()[14]);
    assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), before);
    h.get_by_label("Paste terrain patch").click();
    h.run_steps(3); // Exact repeat is a no-op.
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
    }
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    let saved = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(
        saved.sample(FP::from_raw(229376), FP::from_raw(163840)),
        None
    ); // 3.5,2.5 copied hole
    assert_eq!(
        saved.sample(FP::from_raw(294912), FP::from_raw(163840)),
        Some(FP::from_int(8))
    ); // 4.5,2.5 restored slope
    assert_eq!(saved.triangles().len(), 36);
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
        108
    );
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.checksum(), checksum);
}
#[test]
fn overlapping_pastes_and_later_source_edits_use_original_snapshot() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.patch.height_offset = "0".into();
    panel.copy_terrain_patch(&editor).unwrap();
    panel.patch.destination = [1, 0];
    panel.paste_terrain_patch(&editor).unwrap();
    assert_eq!(
        &panel.document.terrain().unwrap().heights()[0..4],
        &[FP::ONE, FP::ONE, FP::from_int(2), FP::from_int(3)]
    );
    panel
        .document
        .apply(&[
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::from_int(99),
            },
            Edit::SetHole {
                x: 0,
                z: 0,
                hole: false,
            },
        ])
        .unwrap();
    panel.patch.destination = [3, 2];
    panel.paste_terrain_patch(&editor).unwrap();
    let t = panel.document.terrain().unwrap();
    assert_eq!(
        &t.heights()[15..18],
        &[FP::ONE, FP::from_int(2), FP::from_int(3)]
    );
    assert!(t.holes()[13]);
    assert!(!t.holes()[14]);
}
#[test]
fn exact_raw_offset_and_overflow_are_checked_before_apply() {
    for (raw, delta) in [(1, 1), (-1, -1), (65537, -2), (i64::MAX, 1), (i64::MIN, -1)] {
        let t = Terrain::new(
            "terrain/raw".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::from_raw(raw); 4],
            vec![false],
        )
        .unwrap();
        let clip = Clipboard::capture(&t, [0, 0], [0, 0]).unwrap();
        let result = clip.edits(&t, [0, 0], FP::from_raw(delta));
        if raw == i64::MAX || raw == i64::MIN {
            assert!(result.is_err());
        } else {
            let mut doc = TerrainDocument::new(t);
            doc.apply(&result.unwrap()).unwrap();
            assert_eq!(doc.terrain().heights(), &[FP::from_raw(raw + delta); 4]);
        }
    }
}
#[test]
fn full_vertex_and_cell_budget_includes_noops_and_rejects_invalid_bounds() {
    let t = terrain(129, 129);
    // 44*45 cells + 45*46 surrounding vertices = 4050 edits.
    let clip = Clipboard::capture(&t, [0, 0], [43, 44]).unwrap();
    assert_eq!(clip.footprint(), 4050);
    assert!(clip.edits(&t, [0, 0], FP::ZERO).unwrap().is_empty());
    // 45*45 + 46*46 = 4141, despite all values being unchanged.
    assert!(Clipboard::capture(&t, [0, 0], [44, 44]).is_err());
    for (min, max) in [
        ([1, 0], [0, 0]),
        ([0, 1], [0, 0]),
        ([0, 0], [128, 0]),
        ([0, 0], [0, 128]),
        ([u32::MAX, 0], [u32::MAX, 0]),
    ] {
        assert!(Clipboard::capture(&t, min, max).is_err());
    }
    for dest in [[128, 0], [0, 128], [100, 100], [u32::MAX, u32::MAX]] {
        assert!(clip.edits(&t, dest, FP::ZERO).is_err());
    }
    for (w, d) in [(2, 2), (3, 6), (6, 3)] {
        let t = terrain(w, d);
        let last = [w - 2, d - 2];
        let clip = Clipboard::capture(&t, last, last).unwrap();
        assert_eq!(clip.footprint(), 5);
        assert!(clip.edits(&t, last, FP::ZERO).unwrap().is_empty());
        assert!(clip.edits(&t, [w - 1, d - 2], FP::ZERO).is_err());
    }
}
#[test]
fn noops_bad_destinations_parse_and_render_failure_preserve_redo() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    panel.paste_terrain_patch(&editor).unwrap();
    let changed = panel.document.bytes().unwrap().to_vec();
    panel.document.undo().unwrap();
    let before = panel.document.bytes().unwrap().to_vec();
    let revision = panel.document.revision();
    for case in ["noop", "destination", "parse", "length", "render"] {
        panel.patch.destination = [3, 2];
        panel.patch.height_offset = "0.5".into();
        match case {
            "noop" => {
                panel.patch.destination = [0, 0];
                panel.patch.height_offset = "0".into();
            }
            "destination" => panel.patch.destination = [4, 3],
            "parse" => panel.patch.height_offset = "NaN".into(),
            "length" => panel.patch.height_offset = "0".repeat(129),
            "render" => panel
                .document
                .set_render_admission_error(Some("injected patch rejection".into())),
            _ => unreachable!(),
        }
        assert_eq!(
            panel.paste_terrain_patch(&editor).is_ok(),
            case == "noop",
            "{case}"
        );
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(panel.document.can_redo());
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
    }
    panel.document.set_render_admission_error(None);
    panel.document.redo().unwrap();
    assert_eq!(panel.document.bytes(), Some(changed.as_slice()));
}
#[test]
fn invalid_copy_clears_snapshot_without_mutating_document() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    let before = panel.document.bytes().unwrap().to_vec();
    panel.patch.source_max = [u32::MAX, 0];
    assert!(panel.copy_terrain_patch(&editor).is_err());
    assert!(!panel.patch.has_clipboard());
    assert!(panel.paste_terrain_patch(&editor).is_err());
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(!panel.document.can_undo());
}
#[test]
fn changed_scene_epoch_play_and_source_refuse_stale_snapshot() {
    for case in ["scene", "epoch", "play", "source"] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.copy_terrain_patch(&editor).unwrap();
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
            _ => unreachable!(),
        }
        let before = panel.document.bytes().unwrap().to_vec();
        let revision = panel.document.revision();
        assert!(panel.paste_terrain_patch(&editor).is_err(), "{case}");
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), revision);
        assert!(!panel.document.can_undo());
    }
}
#[test]
fn installed_package_is_read_only_for_patch_copy_and_paste() {
    use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};
    let (temp, _scene, editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
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
    assert!(panel.paste_terrain_patch(&editor).is_err());
    assert!(panel.copy_terrain_patch(&editor).is_err());
    assert!(!panel.patch.has_clipboard());
    assert_eq!(fs::read(object).unwrap(), before);
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(!panel.document.can_undo());
}

#[test]
fn changed_geometry_or_asset_identity_rejects_copied_patch() {
    let source = terrain(3, 4);
    let clip = Clipboard::capture(&source, [0, 0], [0, 0]).unwrap();
    for (id, width, depth, origin, spacing) in [
        ("terrain/other", 3, 4, [FP::ZERO; 2], FP::ONE),
        ("terrain/patch", 4, 4, [FP::ZERO; 2], FP::ONE),
        ("terrain/patch", 3, 5, [FP::ZERO; 2], FP::ONE),
        ("terrain/patch", 3, 4, [FP::ONE, FP::ZERO], FP::ONE),
        ("terrain/patch", 3, 4, [FP::ZERO; 2], FP::from_int(2)),
    ] {
        let other = Terrain::new(
            id.into(),
            width,
            depth,
            origin,
            spacing,
            vec![FP::ZERO; (width * depth) as usize],
            vec![false; ((width - 1) * (depth - 1)) as usize],
        )
        .unwrap();
        assert!(clip.edits(&other, [0, 0], FP::ZERO).is_err());
    }
}
