//! Read-only proof of the currently saved editor tilemap, never an implicit Save.
use super::*;

impl TilemapPanel {
    pub(crate) fn arena_export_ready(&self, editor: &Editor, root: &Path) -> Result<(), String> {
        if !self.editable(editor) || self.dirty() || self.painting || self.rectangle.is_some() {
            return Err("Tilemap export requires the matching idle editable project and explicitly saved tiles".into());
        }
        if crate::sprite_bindings::resolve_project(root, ".")? != self.stamp.root {
            return Err("Tilemap export project differs from the loaded project".into());
        }
        self.check_stamp()?;
        if orr_tilemap::Document::from_bytes(&self.stamp.sidecar)?
            != *self.history.current.document()
        {
            return Err("Saved tilemap differs from the loaded accepted document".into());
        }
        Ok(())
    }

    pub(crate) fn arena_export_matches(
        &self,
        editor: &Editor,
        root: &Path,
        admitted: &PreparedTilemap,
    ) -> Result<(), String> {
        self.arena_export_ready(editor, root)?;
        if admitted.path != self.path
            || admitted.original_bytes() != self.stamp.sidecar
            || admitted.document != self.history.current
        {
            return Err(
                "Saved admitted tilemap differs from the loaded tilemap; reopen explicitly".into(),
            );
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "project-create"))]
mod tests {
    use super::*;
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};
    fn fresh(root: &Path) -> orr_sample::project::PreparedProjectParts {
        PreparedProject::open_with_capabilities(
            root,
            crate::sprite_bindings::compiled_runtime(),
            UiSupport::Unsupported,
            TilemapSupport::Supported,
        )
        .unwrap()
        .into_presentation_parts()
    }
    fn fixture(tile_only: bool) -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
            output: root.clone(),
            template: "arena-tilemap-2d-v1".into(),
            seed: "editor-export".into(),
        })
        .unwrap();
        if tile_only {
            let p = root.join("orr.project.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            v["entry"].as_object_mut().unwrap().remove("sprites");
            std::fs::write(p, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let admitted = crate::project::PreparedProject::open(&root).unwrap();
        let mut editor = Editor::start(&admitted.host_spec()).unwrap();
        editor.sync();
        let parts = fresh(&root);
        let mut panel =
            TilemapPanel::prepare(&root, parts.scene.path(), parts.tilemap.unwrap()).unwrap();
        let mut sprites = SpritePanel::default();
        let ctx = egui::Context::default();
        if let Some(p) = parts.sprites {
            sprites.install_prepared(
                &ctx,
                crate::sprite_bindings::Bindings::from_document(p.path, p.document),
                p.assets,
            );
        }
        sprites.install_tilemap_asset(&ctx, parts.tilemap_asset);
        panel.install_editor_identity(&editor).unwrap();
        (temp, panel, sprites, editor)
    }
    #[test]
    fn export_uses_current_saved_tiles_not_initial_prepared_bytes() {
        let (_temp, mut panel, sprites, mut editor) = fixture(false);
        let root = panel.stamp.root.clone();
        let original = panel.prepared.original_bytes().to_vec();
        assert!(panel
            .change(CellCoord { column: 0, row: 0 }, Some(10), &sprites)
            .unwrap());
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        panel.save(&editor, &sprites).unwrap();
        assert_ne!(panel.stamp.sidecar, original);
        assert_eq!(panel.prepared.original_bytes(), original);
        let before = panel.history_snapshot();
        let source = editor
            .arena_export_source_with_tilemap(&root, &sprites, None, Some(&panel))
            .unwrap();
        assert_eq!(source.inventory().len(), 5);
        assert_eq!(panel.history_snapshot(), before);
        panel.history.undo();
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        panel.history.redo();
        assert!(panel.arena_export_ready(&editor, &root).is_ok());
    }
    #[test]
    fn gesture_epoch_and_external_stamp_refuse_without_mutation() {
        let (_temp, mut panel, _sprites, editor) = fixture(false);
        let root = panel.stamp.root.clone();
        let before = panel.history_snapshot();
        panel.painting = true;
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        panel.painting = false;
        panel.rectangle = Some(RectangleGesture {
            purpose: RectanglePurpose::PaintRectangle,
            copy_atlas: None,
            repeat_pattern: None,
            start: CellCoord { column: 0, row: 0 },
            end: CellCoord { column: 1, row: 1 },
            region: Some(10),
            source: panel.history.current.clone(),
            scene_bytes: Vec::new(),
            frame: 0,
        });
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        panel.rectangle = None;
        panel.editor_epoch = None;
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        panel.editor_epoch = Some(editor.scene_load_epoch());
        let bytes = std::fs::read(&panel.path).unwrap();
        let mut changed = bytes.clone();
        changed.push(b' ');
        std::fs::write(&panel.path, &changed).unwrap();
        assert!(panel.arena_export_ready(&editor, &root).is_err());
        assert_eq!(std::fs::read(&panel.path).unwrap(), changed);
        assert_eq!(panel.history_snapshot(), before);
    }
    #[test]
    fn tile_only_cached_atlas_and_loaded_presence_are_required() {
        let (_temp, panel, sprites, mut editor) = fixture(true);
        let root = panel.stamp.root.clone();
        assert!(sprites.bindings.is_none());
        assert!(editor
            .arena_export_source_with_tilemap(&root, &sprites, None, Some(&panel))
            .is_ok());
        assert!(editor.arena_export_source(&root, &sprites, None).is_err());
        let empty = SpritePanel::default();
        assert!(editor
            .arena_export_source_with_tilemap(&root, &empty, None, Some(&panel))
            .unwrap_err()
            .contains("cache"));
    }
}
