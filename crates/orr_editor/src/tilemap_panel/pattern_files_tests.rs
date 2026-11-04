#![cfg(target_os = "linux")]
use super::*;
use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};

fn cell(column: u32, row: u32) -> CellCoord {
    CellCoord { column, row }
}

fn fixture() -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
    fixture_with_geometry(false)
}

fn fixture_with_geometry(
    changed_geometry: bool,
) -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "pattern-files".into(),
    })
    .unwrap();
    if changed_geometry {
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("orr.project.json")).unwrap()).unwrap();
        let path = root.join(manifest["entry"]["tilemap"].as_str().unwrap());
        let mut map = orr_tilemap::Document::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        map.width = 8;
        map.height = 6;
        map.origin = [-32.0, 48.0];
        map.units_per_pixel = 0.5;
        map.cells = vec![None; 48];
        std::fs::write(&path, serde_json::to_vec_pretty(&map).unwrap()).unwrap();
    }
    let admitted = crate::project::PreparedProject::open(&root).unwrap();
    let mut editor = Editor::start(&admitted.host_spec()).unwrap();
    editor.sync();
    let parts = PreparedProject::open_with_capabilities(
        &root,
        crate::sprite_bindings::compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
    .unwrap()
    .into_presentation_parts();
    let mut panel =
        TilemapPanel::prepare(&root, parts.scene.path(), parts.tilemap.unwrap()).unwrap();
    let mut sprites = SpritePanel::default();
    let ctx = egui::Context::default();
    if let Some(prepared) = parts.sprites {
        sprites.install_prepared(
            &ctx,
            crate::sprite_bindings::Bindings::from_document(prepared.path, prepared.document),
            prepared.assets,
        );
    }
    sprites.install_tilemap_asset(&ctx, parts.tilemap_asset);
    panel.install_editor_identity(&editor).unwrap();
    assert!(panel.change(cell(0, 0), Some(10), &sprites).unwrap());
    assert!(panel.change(cell(1, 0), Some(11), &sprites).unwrap());
    let (asset, texture) = sprites.tilemap_asset(&panel.key()).unwrap();
    panel.pattern = Some(
        copy::Pattern::capture(
            &panel.history.current,
            cell(0, 0),
            cell(1, 0),
            editor.scene_load_epoch(),
            &asset.document,
        )
        .unwrap(),
    );
    panel.pattern_texture = Some(texture);
    panel.pattern_file = temp.path().join("pattern.json").to_str().unwrap().into();
    (temp, panel, sprites, editor)
}

fn disk_snapshot(panel: &TilemapPanel) -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths = vec![
        panel.path.clone(),
        panel.stamp.scene.clone(),
        panel.stamp.root.join("orr.project.json"),
        panel.stamp.root.join("orr.packages.lock.json"),
    ];
    if let Some((path, _)) = &panel.stamp.actors {
        paths.push(path.clone());
    }
    paths
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn save_and_load_preserve_dirty_map_all_history_and_project_files() {
    let (_temp, mut panel, sprites, editor) = fixture();
    assert!(panel.dirty());
    let before = panel.history_snapshot();
    let files = disk_snapshot(&panel);
    let checksum = editor.checksum();
    let scene_history = editor.history().entries.len();
    let copied = format!("{:?}", panel.pattern);
    let texture = panel.pattern_texture;
    let mode = panel.mode;
    let report = panel.save_pattern(&editor, &sprites).unwrap();
    assert!(report.warning.is_none());
    assert_eq!(
        report.bytes,
        std::fs::read(&panel.pattern_file).unwrap().len()
    );
    panel.pattern = None;
    panel.pattern_texture = None;
    panel.load_pattern(&editor, &sprites).unwrap();
    assert_eq!(format!("{:?}", panel.pattern), copied);
    assert_eq!(panel.pattern_texture, texture);
    assert!(panel.mode == mode);
    assert_eq!(panel.history_snapshot(), before);
    assert_eq!(disk_snapshot(&panel), files);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), scene_history);
}

#[test]
fn existing_save_and_invalid_load_keep_bytes_clipboard_texture_and_history() {
    let (_temp, mut panel, sprites, editor) = fixture();
    panel.save_pattern(&editor, &sprites).unwrap();
    let bytes = std::fs::read(&panel.pattern_file).unwrap();
    let before = panel.history_snapshot();
    let copied = format!("{:?}", panel.pattern);
    let texture = panel.pattern_texture;
    assert!(
        panel
            .save_pattern(&editor, &sprites)
            .unwrap_err()
            .contains("already exists")
    );
    assert_eq!(std::fs::read(&panel.pattern_file).unwrap(), bytes);
    std::fs::write(&panel.pattern_file, b"{invalid JSON").unwrap();
    assert!(panel.load_pattern(&editor, &sprites).is_err());
    assert_eq!(format!("{:?}", panel.pattern), copied);
    assert_eq!(panel.pattern_texture, texture);
    assert_eq!(panel.history_snapshot(), before);
    assert_eq!(
        std::fs::read(&panel.pattern_file).unwrap(),
        b"{invalid JSON"
    );
}

#[test]
fn play_epoch_and_active_gestures_refuse_save_and_load_without_changes() {
    let (_temp, mut panel, sprites, mut editor) = fixture();
    panel.save_pattern(&editor, &sprites).unwrap();
    let input = panel.pattern_file.clone();
    let before = panel.history_snapshot();
    let copied = format!("{:?}", panel.pattern);
    let texture = panel.pattern_texture;
    for guard in 0..4 {
        match guard {
            0 => panel.painting = true,
            1 => {
                panel.rectangle = Some(RectangleGesture {
                    purpose: RectanglePurpose::Copy,
                    copy_atlas: None,
                    start: cell(0, 0),
                    end: cell(1, 0),
                    region: None,
                    source: panel.history.current.clone(),
                    scene_bytes: Vec::new(),
                    frame: 0,
                })
            }
            2 => panel.editor_epoch = Some(editor.scene_load_epoch().wrapping_add(1)),
            _ => {
                assert!(editor.start_play());
                editor.sync();
            }
        }
        panel.pattern_file = input.clone();
        assert!(panel.load_pattern(&editor, &sprites).is_err());
        panel.pattern_file = format!("{input}.{guard}.json");
        assert!(panel.save_pattern(&editor, &sprites).is_err());
        assert!(!Path::new(&panel.pattern_file).exists());
        assert_eq!(format!("{:?}", panel.pattern), copied);
        assert_eq!(panel.pattern_texture, texture);
        assert_eq!(panel.history_snapshot(), before);
        panel.painting = false;
        panel.rectangle = None;
        panel.editor_epoch = Some(editor.scene_load_epoch());
    }
}

#[test]
fn changed_installed_package_refuses_both_operations_and_preserves_clipboard() {
    let (_temp, mut panel, sprites, editor) = fixture();
    panel.save_pattern(&editor, &sprites).unwrap();
    let key = panel.key();
    let project = orr_package::Project::open(
        &panel.stamp.root,
        crate::sprite_bindings::compiled_runtime(),
    )
    .unwrap();
    let lock = project.verify().unwrap();
    let image = &sprites
        .tilemap_asset(&key)
        .unwrap()
        .0
        .document
        .atlas()
        .image;
    let path = panel
        .stamp
        .root
        .join(".orr/packages/objects")
        .join(&lock.packages[&key.0].digest)
        .join(image);
    let before = panel.history_snapshot();
    let copied = format!("{:?}", panel.pattern);
    let texture = panel.pattern_texture;
    std::fs::write(&path, b"changed installed artwork").unwrap();
    assert!(panel.load_pattern(&editor, &sprites).is_err());
    panel.pattern_file.push_str(".new.json");
    assert!(panel.save_pattern(&editor, &sprites).is_err());
    assert!(!Path::new(&panel.pattern_file).exists());
    assert_eq!(format!("{:?}", panel.pattern), copied);
    assert_eq!(panel.pattern_texture, texture);
    assert_eq!(panel.history_snapshot(), before);
    assert_eq!(std::fs::read(path).unwrap(), b"changed installed artwork");
}

#[test]
fn loaded_pattern_stamp_is_one_history_step_noop_preserves_redo_and_budget_refuses() {
    let (_temp, mut panel, sprites, editor) = fixture();
    panel.save_pattern(&editor, &sprites).unwrap();
    panel.pattern = None;
    panel.load_pattern(&editor, &sprites).unwrap();
    let before = panel.history_snapshot();
    assert!(panel.stamp_pattern(cell(3, 2), &editor, &sprites).unwrap());
    assert_eq!(panel.history.current.cell(cell(3, 2)).unwrap(), Some(10));
    assert_eq!(panel.history.current.cell(cell(4, 2)).unwrap(), Some(11));
    assert_eq!(panel.history.undo.len(), before.2.len() + 1);
    let stamped = panel.history.current.clone();
    panel.history.undo();
    assert_eq!(panel.history.current, before.0);
    let undone = panel.history_snapshot();
    assert!(!panel.stamp_pattern(cell(0, 0), &editor, &sprites).unwrap());
    assert_eq!(panel.history_snapshot(), undone);
    panel.history.redo();
    assert_eq!(panel.history.current, stamped);

    // Loading rebinds the portable pattern to the larger map, but must not
    // circumvent the ordinary Stamp combined tile-and-actor limit.
    let (asset, _) = sprites.tilemap_asset(&panel.key()).unwrap();
    let mut large = panel.history.current.document().clone();
    large.width = 64;
    large.height = 64;
    large.cells = vec![Some(10); 4096];
    large.cells[0] = None;
    large.cells[1] = None;
    panel.history = History::new(large.validate(&asset.document).unwrap()).unwrap();
    assert!(TilemapPanel::actor_count(&sprites) > 0);
    panel.load_pattern(&editor, &sprites).unwrap();
    let full = panel.history_snapshot();
    assert!(
        panel
            .stamp_pattern(cell(0, 0), &editor, &sprites)
            .unwrap_err()
            .contains("4096")
    );
    assert_eq!(panel.history_snapshot(), full);
}

#[test]
fn installed_second_map_load_rebinds_pattern_and_stamp_preserves_target_geometry() {
    let (_source_temp, source, source_sprites, source_editor) = fixture();
    source
        .save_pattern(&source_editor, &source_sprites)
        .unwrap();
    let (_target_temp, mut target, target_sprites, target_editor) = fixture_with_geometry(true);
    let original = target.history.current.document().clone();
    assert_ne!(original.width, source.history.current.document().width);
    assert_ne!(original.origin, source.history.current.document().origin);
    assert_ne!(
        original.units_per_pixel,
        source.history.current.document().units_per_pixel
    );
    target.pattern = None;
    target.pattern_texture = None;
    target.pattern_file = source.pattern_file.clone();
    let history = target.history_snapshot();
    let disk = disk_snapshot(&target);
    target
        .load_pattern(&target_editor, &target_sprites)
        .unwrap();
    assert_eq!(target.history_snapshot(), history);
    assert!(
        target
            .stamp_pattern(cell(3, 2), &target_editor, &target_sprites)
            .unwrap()
    );
    let actual = target.history.current.document();
    let mut expected = original;
    expected.cells[(2 * expected.width + 3) as usize] = Some(10);
    expected.cells[(2 * expected.width + 4) as usize] = Some(11);
    assert_eq!(actual, &expected);
    assert_eq!(target.history.undo.len(), history.2.len() + 1);
    assert_eq!(disk_snapshot(&target), disk);
}
