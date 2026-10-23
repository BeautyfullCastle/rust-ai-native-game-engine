use super::*;
fn atlas() -> orr_sprite::SpriteDocument {
    orr_sprite::SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1}],"clips":[]}"#).unwrap()
}
fn document() -> ValidatedDocument {
    orr_tilemap::Document {
        version: 1,
        sprite: orr_tilemap::SpriteRef {
            package: "fixture".into(),
            document: "sprites.json".into(),
        },
        width: 3,
        height: 2,
        tile_size: [1, 1],
        origin: [0., 2.],
        units_per_pixel: 1.,
        cells: vec![None; 6],
    }
    .validate(&atlas())
    .unwrap()
}
#[test]
fn paint_erase_noop_undo_and_redo_saved_bytes() {
    let atlas = atlas();
    let mut h = History::new(document()).unwrap();
    let c = CellCoord { column: 1, row: 0 };
    assert!(!h.commit(h.current.with_cell(c, None, &atlas).unwrap()));
    assert!(h.undo.is_empty());
    assert!(h.commit(h.current.with_cell(c, Some(0), &atlas).unwrap()));
    assert!(h.dirty());
    assert!(!h.commit(h.current.with_cell(c, Some(0), &atlas).unwrap()));
    assert_eq!(h.undo.len(), 1);
    let before = h.current.clone();
    assert!(h.current.with_cell(c, Some(99), &atlas).is_err());
    assert_eq!(h.current, before);
    h.undo();
    assert!(!h.dirty());
    // No-op after undo preserves redo.
    assert!(!h.commit(h.current.with_cell(c, None, &atlas).unwrap()));
    assert_eq!(h.redo.len(), 1);
    h.redo();
    assert_eq!(h.current, before);
    h.commit(h.current.with_cell(c, None, &atlas).unwrap());
    assert!(!h.dirty());
}
#[test]
fn history_is_bounded_and_budget_is_atomic() {
    let atlas = atlas();
    let mut h = History::new(document()).unwrap();
    let c = CellCoord { column: 0, row: 0 };
    for n in 0..260 {
        h.commit(h.current.with_cell(c, Some(n % 2), &atlas).unwrap());
    }
    assert_eq!(h.undo.len(), 128);
    assert!(TilemapPanel::budget(&h.current, 4095).is_ok());
    assert!(TilemapPanel::budget(&h.current, 4096).is_err());
    assert!(TilemapPanel::budget(&h.current, usize::MAX).is_err());
    for _ in 0..128 {
        h.undo();
    }
    assert_eq!(h.redo.len(), 128);
    for _ in 0..128 {
        h.redo();
    }
    assert_eq!(h.undo.len(), 128);
}

#[cfg(all(feature = "project-create", target_os = "linux"))]
mod installed {
    use super::*;
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};
    fn open(root: &Path) -> (TilemapPanel, SpritePanel) {
        let parts = PreparedProject::open_with_capabilities(
            root,
            crate::sprite_bindings::compiled_runtime(),
            UiSupport::Unsupported,
            TilemapSupport::Supported,
        )
        .unwrap()
        .into_presentation_parts();
        let panel =
            TilemapPanel::prepare(&parts.root, parts.scene.path(), parts.tilemap.unwrap()).unwrap();
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
        (panel, sprites)
    }
    fn starter() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("tile-project");
        orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
            output: root.clone(),
            template: "arena-tilemap-2d-v1".into(),
            seed: "tile-panel".into(),
        })
        .unwrap();
        (temp, root)
    }
    #[test]
    fn real_template_shared_cache_paint_save_reopen_and_stale_save() {
        let (_temp, root) = starter();
        let (mut panel, sprites) = open(&root);
        panel.install_renderer(None, &sprites).unwrap();
        let cell = CellCoord { column: 0, row: 0 };
        assert!(panel.change(cell, Some(10), &sprites).unwrap());
        assert!(!panel.change(cell, Some(10), &sprites).unwrap());
        assert!(panel.change(cell, None, &sprites).unwrap());
        panel.history.undo();
        panel.save_document().unwrap();
        assert!(!panel.dirty());
        let (reopened, _) = open(&root);
        assert_eq!(reopened.document(), panel.document());
        assert_eq!(panel.cell_at([-128., 96.]), Some(cell));
        assert_eq!(panel.cell_at([-129., 96.]), None);
        assert_eq!(panel.cell_at([f32::NAN, 0.]), None);
        panel.change(cell, Some(11), &sprites).unwrap();
        let before = panel.document().clone();
        let undo = panel.history.undo.len();
        let saved = panel.history.saved.clone();
        let external = b"external sidecar edit";
        std::fs::write(&panel.path, external).unwrap();
        assert!(panel.save_document().is_err());
        assert_eq!(std::fs::read(&panel.path).unwrap(), external);
        assert_eq!(panel.document(), &before);
        assert_eq!(panel.history.undo.len(), undo);
        assert_eq!(panel.history.saved, saved);
        assert!(panel.dirty());
    }
    #[test]
    fn stale_manifest_and_lock_preserve_saved_file_and_history() {
        for file in ["orr.project.json", "orr.packages.lock.json"] {
            let (_temp, root) = starter();
            let (mut panel, sprites) = open(&root);
            panel
                .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
                .unwrap();
            let bytes = std::fs::read(&panel.path).unwrap();
            let saved = panel.history.saved.clone();
            std::fs::write(root.join(file), b"{}").unwrap();
            assert!(panel.save_document().is_err());
            assert_eq!(std::fs::read(&panel.path).unwrap(), bytes);
            assert_eq!(panel.history.saved, saved);
            assert_eq!(panel.history.undo.len(), 1);
        }
    }
    #[test]
    fn replaced_root_cannot_receive_save() {
        let (_temp, root) = starter();
        let (mut panel, sprites) = open(&root);
        panel
            .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
            .unwrap();
        let backup = root.with_file_name("original-project");
        std::fs::rename(&root, &backup).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(panel.save_document().is_err());
        assert!(panel.dirty());
        assert!(!root.join("arena.tilemap.json").exists());
        assert!(backup.join("arena.tilemap.json").is_file());
    }
    fn assert_failed_save_preserves(panel: &mut TilemapPanel) {
        let bytes = std::fs::read(&panel.path).unwrap();
        let current = panel.document().clone();
        let saved = panel.history.saved.clone();
        let undo = panel.history.undo.clone();
        let redo = panel.history.redo.clone();
        assert!(panel.save_document().is_err());
        assert_eq!(std::fs::read(&panel.path).unwrap(), bytes);
        assert_eq!(panel.document(), &current);
        assert_eq!(panel.history.saved, saved);
        assert_eq!(panel.history.undo, undo);
        assert_eq!(panel.history.redo, redo);
        assert!(panel.dirty());
    }
    #[test]
    fn installed_png_and_sprite_document_tamper_refuse_save_with_unchanged_lock() {
        for extension in [".png", "sprites.json"] {
            let (_temp, root) = starter();
            let (mut panel, sprites) = open(&root);
            panel
                .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
                .unwrap();
            let project =
                orr_package::Project::open(&root, crate::sprite_bindings::compiled_runtime())
                    .unwrap();
            let lock = project.verify().unwrap();
            let package = lock
                .packages
                .values()
                .find(|p| p.files.keys().any(|f| f.ends_with(extension)))
                .unwrap();
            let file = package
                .files
                .keys()
                .find(|f| f.ends_with(extension))
                .unwrap();
            let path = root
                .join(".orr/packages/objects")
                .join(&package.digest)
                .join(file);
            let lock_bytes = std::fs::read(root.join("orr.packages.lock.json")).unwrap();
            std::fs::write(path, b"tampered installed content").unwrap();
            assert_failed_save_preserves(&mut panel);
            assert_eq!(
                std::fs::read(root.join("orr.packages.lock.json")).unwrap(),
                lock_bytes
            );
            assert!(sprites.tilemap_asset(&panel.key()).is_some());
        }
    }
    #[test]
    fn changed_actor_sidecar_refuses_save_and_epoch_pin_is_not_implicitly_refreshed() {
        let (_temp, root) = starter();
        let (mut panel, sprites) = open(&root);
        panel
            .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
            .unwrap();
        assert!(!panel.epoch_matches(7));
        panel.editor_epoch = Some(7);
        assert!(panel.epoch_matches(7));
        assert!(!panel.epoch_matches(8));
        let (path, original) = panel.stamp.actors.as_ref().unwrap();
        let mut changed = original.clone();
        changed.push(b' ');
        std::fs::write(path, changed).unwrap();
        assert_failed_save_preserves(&mut panel);
        assert_eq!(panel.editor_epoch, Some(7));
    }
    #[test]
    fn scene_read_limit_matches_four_mebibyte_admission() {
        let (_temp, root) = starter();
        let (mut panel, sprites) = open(&root);
        panel
            .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
            .unwrap();
        let mut bytes = std::fs::read(&panel.stamp.scene).unwrap();
        bytes.extend_from_slice(b"\n# ");
        bytes.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
        bytes.push(b'\n');
        assert!(bytes.len() < orr_sample::project::MAX_SCENE_BYTES as usize);
        std::fs::write(&panel.stamp.scene, bytes).unwrap();
        panel.save_document().unwrap();
        assert!(!panel.dirty());
    }
    #[cfg(unix)]
    #[test]
    fn sidecar_symlink_save_is_refused() {
        let (_temp, root) = starter();
        let (mut panel, sprites) = open(&root);
        panel
            .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
            .unwrap();
        let target = root.join("external.json");
        std::fs::write(&target, b"keep").unwrap();
        std::fs::remove_file(&panel.path).unwrap();
        std::os::unix::fs::symlink(&target, &panel.path).unwrap();
        assert!(panel.save_document().is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"keep");
        assert!(panel.dirty());
    }
}
