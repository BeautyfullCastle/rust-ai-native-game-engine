use super::*;

#[cfg(all(feature = "project-create", target_os = "linux"))]
#[test]
fn installed_replace_dispatch_is_atomic_for_budget_error_and_noop() {
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("replace-panel");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "replace-panel".into(),
    })
    .unwrap();
    let parts = PreparedProject::open_with_capabilities(
        &root,
        crate::sprite_bindings::compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
    .unwrap()
    .into_presentation_parts();
    let mut panel =
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
    panel.mode = ToolMode::Replace;
    let seed = CellCoord { column: 0, row: 0 };
    assert!(panel.change(seed, Some(10), &sprites).unwrap());
    assert_eq!(panel.history.undo.len(), 1);
    panel.history.undo();
    let snapshot = |panel: &TilemapPanel| {
        (
            panel.document().clone(),
            panel.history.saved.clone(),
            panel.history.undo.clone(),
            panel.history.redo.clone(),
            panel.dirty(),
        )
    };
    let before = snapshot(&panel);
    assert!(!panel.change(seed, None, &sprites).unwrap());
    assert_eq!(snapshot(&panel), before);
    assert!(panel.change(seed, Some(u32::MAX), &sprites).is_err());
    assert_eq!(snapshot(&panel), before);
    panel.history.redo();
    assert_eq!(panel.document().occupied_count(), 192);

    // A valid maximum-grid candidate would exceed the combined budget
    // because the real starter has actor bindings. It must never commit.
    assert!(TilemapPanel::actor_count(&sprites) > 0);
    let mut large = panel.document().document().clone();
    large.width = 64;
    large.height = 64;
    large.cells = vec![None; 4096];
    let (asset, _) = sprites.tilemap_asset(&panel.key()).unwrap();
    panel.history = History::new(large.validate(&asset.document).unwrap()).unwrap();
    let single = panel
        .history
        .current
        .with_cell(seed, Some(10), &asset.document)
        .unwrap();
    assert!(panel.history.commit(single));
    panel.history.undo();
    assert_eq!(panel.history.redo.len(), 1);
    let before = snapshot(&panel);
    assert!(
        panel
            .change(seed, Some(10), &sprites)
            .unwrap_err()
            .contains("4096")
    );
    assert_eq!(snapshot(&panel), before);
}

#[test]
fn grouped_replace_undo_redo_noop_and_budget_failure_preserve_history() {
    let atlas = orr_sprite::SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
        )
        .unwrap();
    let document = orr_tilemap::Document {
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
    .validate(&atlas)
    .unwrap();
    let mut history = History::new(document.clone()).unwrap();
    let seed = CellCoord { column: 0, row: 0 };
    let (replaced, changed) = replace::candidate(&history.current, seed, Some(0), &atlas).unwrap();
    assert_eq!(changed, 6);
    TilemapPanel::budget(&replaced, 4090).unwrap();
    assert!(history.commit(replaced.clone()));
    assert_eq!(history.undo.len(), 1);
    assert!(history.dirty());
    history.undo();
    assert_eq!(history.current, document);
    assert!(!history.dirty());
    let saved = history.saved.clone();
    let (same, changed) = replace::candidate(&history.current, seed, None, &atlas).unwrap();
    assert_eq!(changed, 0);
    assert!(!history.commit(same));
    assert_eq!(history.redo, vec![replaced.clone()]);
    assert_eq!(history.saved, saved);

    // Candidate and budget failures precede commit, preserving every marker.
    let undo = history.undo.clone();
    let redo = history.redo.clone();
    assert!(replace::candidate(&history.current, seed, Some(99), &atlas).is_err());
    let (over_budget, _) = replace::candidate(&history.current, seed, Some(0), &atlas).unwrap();
    assert!(TilemapPanel::budget(&over_budget, 4091).is_err());
    assert!(TilemapPanel::budget(&over_budget, usize::MAX).is_err());
    assert_eq!(history.current, document);
    assert_eq!(history.undo, undo);
    assert_eq!(history.redo, redo);
    assert_eq!(history.saved, saved);
    assert!(!history.dirty());
    history.redo();
    assert_eq!(history.current, replaced);
    assert_eq!(history.undo.len(), 1);
    let (erased, changed) = replace::candidate(&history.current, seed, None, &atlas).unwrap();
    assert_eq!(changed, 6);
    assert!(history.commit(erased));
    assert!(!history.dirty());
}
