use super::super::tests::{fixture, settle};
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::fs;

#[test]
fn widgets_save_then_explicit_load_on_new_terrain_paste_save_and_consume() {
    let (temp, scene, editor, mut panel) = fixture();
    let original = panel.document.bytes().unwrap().to_vec();
    let asset = temp.path().join("hill.json");
    panel.patch.asset_path = asset.to_string_lossy().into_owned();
    let mut h = Harness::builder().with_size([1400., 2800.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
        (panel, editor),
    );
    h.run_steps(2);
    for label in [
        "Terrain authoring",
        "Terrain patch clipboard",
        "Copy terrain patch",
        "Save terrain patch asset",
    ] {
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    let encoded = fs::read(&asset).unwrap();
    assert_eq!(h.state().0.document.bytes(), Some(original.as_slice()));
    assert!(!h.state().0.document.can_undo());
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original
    );

    // An entirely new editor/panel and terrain identity, dimensions and origin.
    let target_scene = temp.path().join("target.yaml");
    fs::copy(&scene, &target_scene).unwrap();
    let mut editor = Editor::open_game(&target_scene, crate::game::EditorGame::Yard3D).unwrap();
    settle(&mut editor);
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let mut panel = TerrainPanel {
        dimensions: [8, 6],
        origin: ["10".into(), "-4".into()],
        asset_id: "terrain/different.orrt".into(),
        local_path: "target.orrt".into(),
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.patch.asset_path = asset.to_string_lossy().into_owned();
    panel.patch.destination = [3, 2];
    panel.patch.height_offset = "0.5".into();
    let before = panel.document.bytes().unwrap().to_vec();
    let mut h = Harness::builder().with_size([1400., 2800.]).build_ui_state(
        |ui, s: &mut (TerrainPanel, Editor)| s.0.show_in_parent_scroll(ui, &s.1),
        (panel, editor),
    );
    h.run_steps(2);
    for label in [
        "Terrain authoring",
        "Terrain patch clipboard",
        "Load terrain patch asset",
    ] {
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Paste terrain patch").click();
    h.run_steps(3);
    assert!(h.state().0.error().is_none(), "{:?}", h.state().0.error());
    let after = h.state().0.document.bytes().unwrap().to_vec();
    assert_ne!(after, before);
    for label in ["Terrain Undo", "Terrain Redo", "Save terrain"] {
        h.get_by_label(label).click();
        h.run_steps(3);
        assert!(h.state().0.error().is_none());
        if label == "Terrain Undo" {
            assert_eq!(h.state().0.document.bytes(), Some(before.as_slice()));
            assert!(!h.state().0.document.can_undo());
        }
    }
    let mut fresh = TerrainPanel {
        local_path: "target.orrt".into(),
        ..TerrainPanel::default()
    };
    fresh.open_for_editor(&h.state().1).unwrap();
    assert_eq!(fresh.document.bytes(), Some(after.as_slice()));
    let saved = Terrain::load(&fs::read(temp.path().join("target.orrt")).unwrap()).unwrap();
    assert_eq!(
        saved.sample(FP::from_raw(884736), FP::from_raw(-98304)),
        None
    ); //13.5,-1.5 hole
    assert_eq!(
        saved.sample(FP::from_raw(950272), FP::from_raw(-98304)),
        Some(FP::from_int(8))
    );
    assert!(
        orr_terrain_view::to_static_model(&saved, &[])
            .unwrap()
            .is_some()
    );
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(fs::read(&asset).unwrap(), encoded);
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original
    );
}

#[test]
fn failed_asset_actions_preserve_clipboard_document_redo_and_files() {
    let (temp, _, mut editor, mut panel) = fixture();
    panel.copy_terrain_patch(&editor).unwrap();
    let original = panel
        .patch
        .clipboard
        .as_ref()
        .unwrap()
        .patch
        .encode_asset()
        .unwrap();
    let asset = temp.path().join("patch.json");
    panel.patch.asset_path = asset.to_string_lossy().into_owned();
    // Saving ignores pending paste coordinates/offset, and later source edits.
    panel.patch.destination = [u32::MAX; 2];
    panel.patch.height_offset = "invalid".into();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_int(99),
        }])
        .unwrap();
    panel.undo(&editor).unwrap();
    let before = panel.document.bytes().unwrap().to_vec();
    panel.save_patch_asset(&editor).unwrap();
    assert_eq!(fs::read(&asset).unwrap(), original);
    assert!(panel.save_patch_asset(&editor).is_err());
    assert_eq!(fs::read(&asset).unwrap(), original);
    for bad in [
        b"broken".to_vec(),
        br#"{"format":"orr_terrain_patch","version":2}"#.to_vec(),
    ] {
        fs::write(&asset, &bad).unwrap();
        assert!(panel.load_patch_asset(&editor).is_err());
        assert_eq!(
            panel
                .patch
                .clipboard
                .as_ref()
                .unwrap()
                .patch
                .encode_asset()
                .unwrap(),
            original
        );
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert!(panel.document.can_redo());
        assert_eq!(fs::read(&asset).unwrap(), bad);
    }
    fs::write(&asset, &original).unwrap();
    editor.play();
    settle(&mut editor);
    assert!(panel.load_patch_asset(&editor).is_err());
    assert!(panel.save_patch_asset(&editor).is_err());
    editor.stop();
    settle(&mut editor);
    panel.load_patch_asset(&editor).unwrap();
    panel.patch.clipboard.as_mut().unwrap().epoch = u64::MAX;
    assert!(panel.save_patch_asset(&editor).is_err());
    assert!(panel.patch_edits(&editor).is_err());
    assert_eq!(fs::read(&asset).unwrap(), original);
    assert_eq!(panel.document.bytes(), Some(before.as_slice()));
    assert!(panel.document.can_redo());
}
