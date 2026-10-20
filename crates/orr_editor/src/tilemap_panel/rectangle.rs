//! Pure, bounded inclusive rectangles. The panel owns budgets and history.
use orr_sprite::SpriteDocument;
use orr_tilemap::{CellCoord, ValidatedDocument};

/// Build one candidate without modifying the source, including on errors/no-ops.
pub(super) fn candidate(
    current: &ValidatedDocument,
    start: CellCoord,
    end: CellCoord,
    region: Option<u32>,
    atlas: &SpriteDocument,
) -> Result<ValidatedDocument, String> {
    // Check both original endpoints before normalization or considering a no-op.
    current.cell(start)?;
    current.cell(end)?;
    let source = current.document();
    let mut next = source.clone();
    // ValidatedDocument bounds both axes to 64 and the grid to 4096 cells.
    // Inclusive ranges are nonempty, so even an unchanged replacement region
    // will be checked against the supplied atlas by the final validation pass.
    for row in start.row.min(end.row)..=start.row.max(end.row) {
        for column in start.column.min(end.column)..=start.column.max(end.column) {
            next.cells[(row * source.width + column) as usize] = region;
        }
    }
    // Preserve all metadata and validate the complete candidate once, not once
    // per cell. validate owns its resulting clone, as for paint/fill/resize.
    next.validate(atlas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tilemap_panel::{History, TilemapPanel};
    use orr_tilemap::{Document, SpriteRef, MAX_CELLS};

    fn atlas() -> SpriteDocument {
        SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":4,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1},{"id":2,"x":2,"y":0,"width":2,"height":1}],"clips":[]}"#,
        )
        .unwrap()
    }

    fn document(width: u32, height: u32, cells: Vec<Option<u32>>) -> ValidatedDocument {
        Document {
            version: 1,
            sprite: SpriteRef {
                package: "fixture".into(),
                document: "art/sprites.json".into(),
            },
            width,
            height,
            tile_size: [1, 1],
            origin: [-0.0, 7.25],
            units_per_pixel: 0.125,
            cells,
        }
        .validate(&atlas())
        .unwrap()
    }

    fn cell(column: u32, row: u32) -> CellCoord {
        CellCoord { column, row }
    }

    #[test]
    fn asymmetric_literal_oracle_normalizes_all_corner_orders_and_preserves_metadata() {
        let atlas = atlas();
        let source = document(
            5,
            3,
            vec![
                Some(0),
                None,
                Some(1),
                None,
                Some(0),
                Some(1),
                None,
                Some(0),
                Some(1),
                None,
                None,
                Some(0),
                None,
                Some(1),
                Some(0),
            ],
        );
        let before = source.clone();
        // Independent 5x3 oracle catches a wrong row stride, flat interval,
        // off-by-one endpoints, and accidental edits outside the rectangle.
        let expected = vec![
            Some(0),
            None,
            Some(1),
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            None,
            None,
            Some(1),
            Some(1),
            Some(1),
            Some(0),
        ];
        for (start, end) in [
            (cell(1, 1), cell(3, 2)),
            (cell(3, 2), cell(1, 1)),
            (cell(3, 1), cell(1, 2)),
            (cell(1, 2), cell(3, 1)),
        ] {
            let painted = candidate(&source, start, end, Some(1), &atlas).unwrap();
            assert_eq!(painted.document().cells, expected);
            let old = source.document();
            let new = painted.document();
            assert_eq!(new.version, old.version);
            assert_eq!(new.sprite, old.sprite);
            assert_eq!((new.width, new.height), (old.width, old.height));
            assert_eq!(new.tile_size, old.tile_size);
            assert_eq!(new.origin.map(f32::to_bits), old.origin.map(f32::to_bits));
            assert_eq!(new.units_per_pixel.to_bits(), old.units_per_pixel.to_bits());
            assert_eq!(source, before);
        }
        let erased = candidate(&source, cell(3, 2), cell(1, 1), None, &atlas).unwrap();
        assert_eq!(
            erased.document().cells,
            vec![
                Some(0),
                None,
                Some(1),
                None,
                Some(0),
                Some(1),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            ]
        );
        assert_eq!(source, before);
    }

    #[test]
    fn single_cells_rows_and_columns_preserve_empty_versus_region_zero() {
        let atlas = atlas();
        let source = document(3, 5, vec![None; 15]);
        let before = source.clone();
        let single = candidate(&source, cell(2, 4), cell(2, 4), Some(0), &atlas).unwrap();
        assert_eq!(
            single.document().cells,
            vec![
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0)
            ]
        );
        assert_eq!(single.occupied_count(), 1);
        assert_eq!(
            candidate(&single, cell(2, 4), cell(2, 4), None, &atlas).unwrap(),
            source
        );
        let row = candidate(&source, cell(2, 2), cell(0, 2), Some(0), &atlas).unwrap();
        assert_eq!(
            row.document().cells,
            vec![
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
                Some(0),
                Some(0),
                None,
                None,
                None,
                None,
                None,
                None
            ]
        );
        let column = candidate(&source, cell(1, 4), cell(1, 1), Some(0), &atlas).unwrap();
        assert_eq!(
            column.document().cells,
            vec![
                None,
                None,
                None,
                None,
                Some(0),
                None,
                None,
                Some(0),
                None,
                None,
                Some(0),
                None,
                None,
                Some(0),
                None
            ]
        );
        assert_eq!(source, before);
    }

    #[test]
    fn whole_maximum_grid_paint_and_erase_are_bounded_and_immutable() {
        let atlas = atlas();
        let source = document(64, 64, vec![None; MAX_CELLS]);
        let before = source.clone();
        let painted = candidate(&source, cell(63, 63), cell(0, 0), Some(0), &atlas).unwrap();
        assert_eq!(painted.document().cells, vec![Some(0); 4096]);
        assert_eq!(painted.occupied_count(), 4096);
        assert_eq!(source, before);
        let erased = candidate(&painted, cell(0, 63), cell(63, 0), None, &atlas).unwrap();
        assert_eq!(erased, source);
        assert_eq!(painted.document().cells, vec![Some(0); 4096]);
    }

    #[test]
    fn invalid_original_endpoints_and_regions_reject_without_mutation() {
        let atlas = atlas();
        let source = document(3, 2, vec![None, Some(0), Some(1), None, None, Some(0)]);
        let before = source.clone();
        for invalid in [
            cell(3, 0),
            cell(0, 2),
            cell(3, 2),
            cell(u32::MAX, 0),
            cell(0, u32::MAX),
            cell(u32::MAX, u32::MAX),
        ] {
            // Erasing the valid endpoint is already a no-op, but must not hide
            // either invalid endpoint, wrap arithmetic, or clip the rectangle.
            for (start, end) in [(invalid, cell(0, 0)), (cell(0, 0), invalid)] {
                assert!(candidate(&source, start, end, None, &atlas).is_err());
                assert_eq!(source, before);
            }
        }
        for region in [2, 99, u32::MAX] {
            assert!(candidate(&source, cell(0, 0), cell(2, 1), Some(region), &atlas).is_err());
            assert_eq!(source, before);
        }
    }

    #[test]
    fn noops_still_validate_supplied_atlas_and_untouched_cells() {
        let atlas = atlas();
        let source = document(2, 1, vec![Some(0), None]);
        let before = source.clone();
        assert_eq!(
            candidate(&source, cell(0, 0), cell(0, 0), Some(0), &atlas).unwrap(),
            source
        );
        assert_eq!(
            candidate(&source, cell(1, 0), cell(1, 0), None, &atlas).unwrap(),
            source
        );
        for json in [
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"other.png","width":1,"height":1},"regions":[{"id":9,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"other.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":2,"height":1}],"clips":[]}"#,
        ] {
            let other = SpriteDocument::from_json(json).unwrap();
            assert!(candidate(&source, cell(0, 0), cell(0, 0), Some(0), &other).is_err());
            assert!(candidate(&source, cell(1, 0), cell(1, 0), None, &other).is_err());
            assert_eq!(source, before);
        }
    }

    #[test]
    fn grouped_history_noop_saved_redo_and_budget_failures_are_atomic() {
        let atlas = atlas();
        let source = document(5, 3, vec![None; 15]);
        let mut history = History::new(source.clone()).unwrap();
        let start = cell(1, 1);
        let end = cell(3, 2);
        let saved = history.saved.clone();
        let painted = candidate(&history.current, start, end, Some(0), &atlas).unwrap();
        assert_eq!(painted.occupied_count(), 6);
        TilemapPanel::budget(&painted, 4090).unwrap();
        assert!(history.commit(painted.clone()));
        assert_eq!(history.undo, vec![source.clone()]);
        assert!(history.redo.is_empty());
        assert!(history.dirty());
        assert_eq!(history.saved, saved);

        history.undo();
        assert_eq!(history.current, source);
        assert!(!history.dirty());
        let same = candidate(&history.current, end, start, None, &atlas).unwrap();
        assert!(!history.commit(same));
        assert!(history.undo.is_empty());
        assert_eq!(history.redo, vec![painted.clone()]);
        assert_eq!(history.saved, saved);

        let snapshot = (
            history.current.clone(),
            history.undo.clone(),
            history.redo.clone(),
            history.saved.clone(),
        );
        // Exercise the transaction order: candidate and combined-budget admission
        // both precede the only commit, so neither failure consumes the redo.
        for (end, region, actors) in [
            (cell(5, 2), None, 0),
            (end, Some(99), 0),
            (end, Some(2), 0),
            (end, Some(0), 4091),
            (end, Some(0), usize::MAX),
        ] {
            let result = candidate(&history.current, start, end, region, &atlas).and_then(|next| {
                TilemapPanel::budget(&next, actors)?;
                Ok(history.commit(next))
            });
            assert!(result.is_err());
            assert_eq!(
                (
                    history.current.clone(),
                    history.undo.clone(),
                    history.redo.clone(),
                    history.saved.clone()
                ),
                snapshot
            );
            assert!(!history.dirty());
        }
        history.redo();
        assert_eq!(history.current, painted);
        assert_eq!(history.undo, vec![source.clone()]);
        assert!(history.redo.is_empty());
        let same = candidate(&history.current, start, end, Some(0), &atlas).unwrap();
        assert!(!history.commit(same));
        assert_eq!(history.undo.len(), 1);
        let erased = candidate(&history.current, end, start, None, &atlas).unwrap();
        assert!(history.commit(erased));
        assert_eq!(history.current, source);
        assert_eq!(history.undo.len(), 2);
        assert_eq!(history.saved, saved);
        assert!(!history.dirty());
        history.undo();
        assert_eq!(history.current, painted);
        history.redo();
        assert_eq!(history.current, source);
    }
}

#[cfg(all(test, feature = "project-create", target_os = "linux"))]
mod installed_gestures {
    use crate::{
        editor::Editor,
        sprite_panel::SpritePanel,
        tilemap_panel::{TilemapPanel, ToolMode},
    };
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};
    use orr_tilemap::CellCoord;

    const PIXELS: (u32, u32) = (1280, 960);

    fn viewport() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(80.0, 60.0), egui::vec2(640.0, 480.0))
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    struct Harness {
        _temp: tempfile::TempDir,
        panel: TilemapPanel,
        sprites: SpritePanel,
        editor: Editor,
        ctx: egui::Context,
    }

    impl Harness {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("rectangle-gestures");
            orr_sample::project_create::create_tilemap(
                &orr_sample::project_create::CreateOptions {
                    output: root.clone(),
                    template: "arena-tilemap-2d-v1".into(),
                    seed: "rectangle-gestures".into(),
                },
            )
            .unwrap();
            let admitted = crate::project::PreparedProject::open(&root).unwrap();
            let mut editor = Editor::start(&admitted.host_spec()).unwrap();
            editor.sync();
            // Show the complete 256x192 grid with a margin, independently of the
            // starter's normal camera. Logical pixels deliberately differ from
            // framebuffer pixels to exercise the real viewport conversion.
            editor.camera = orr_render::Camera::new([0.0, 0.0], 160.0);
            let parts = PreparedProject::open_with_capabilities(
                &root,
                crate::sprite_bindings::compiled_runtime(),
                UiSupport::Unsupported,
                TilemapSupport::Supported,
            )
            .unwrap()
            .into_presentation_parts();
            let mut panel =
                TilemapPanel::prepare(&parts.root, parts.scene.path(), parts.tilemap.unwrap())
                    .unwrap();
            let mut sprites = SpritePanel::default();
            let ctx = egui::Context::default();
            if let Some(prepared) = parts.sprites {
                sprites.install_prepared(
                    &ctx,
                    crate::sprite_bindings::Bindings::from_document(
                        prepared.path,
                        prepared.document,
                    ),
                    prepared.assets,
                );
            }
            sprites.install_tilemap_asset(&ctx, parts.tilemap_asset);
            panel.install_editor_identity(&editor).unwrap();
            assert!(panel.editable(&editor));
            panel.tool = true;
            panel.mode = ToolMode::Rectangle;
            panel.selected = Some(10);
            let mut harness = Self {
                _temp: temp,
                panel,
                sprites,
                editor,
                ctx,
            };
            // Register the root layer before injecting pointer input.
            harness.frame(Vec::new(), true);
            harness
        }

        fn point(&self, column: u32, row: u32) -> egui::Pos2 {
            let doc = self.panel.document().document();
            let world = [
                doc.origin[0]
                    + (column as f32 + 0.5) * doc.tile_size[0] as f32 * doc.units_per_pixel,
                doc.origin[1] - (row as f32 + 0.5) * doc.tile_size[1] as f32 * doc.units_per_pixel,
            ];
            assert_eq!(self.panel.cell_at(world), Some(CellCoord { column, row }));
            let screen = self.editor.camera.world_to_screen(world, PIXELS);
            let rect = viewport();
            rect.min
                + egui::vec2(
                    screen[0] * rect.width() / PIXELS.0 as f32,
                    screen[1] * rect.height() / PIXELS.1 as f32,
                )
        }

        fn frame(&mut self, events: Vec<egui::Event>, focused: bool) {
            let Self {
                panel,
                sprites,
                editor,
                ctx,
                ..
            } = self;
            // egui 0.36 exposes the frame runner as run_ui. No GPU, eframe
            // window or renderer is involved in this actual panel input path.
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    focused,
                    ..Default::default()
                },
                |ui| {
                    assert!(panel.paint_viewport(ui, editor, sprites, viewport(), PIXELS));
                    panel.paint_grid(ui, editor, viewport(), PIXELS);
                },
            );
            // These CPU frames deliberately do not install texture deltas in a
            // GPU renderer; drawing the preview must not require one.
            output.textures_delta.clear();
        }

        fn press(&mut self, pos: egui::Pos2) {
            self.frame(
                vec![egui::Event::PointerMoved(pos), button(pos, true)],
                true,
            );
        }

        fn release(&mut self, pos: egui::Pos2) {
            self.frame(
                vec![egui::Event::PointerMoved(pos), button(pos, false)],
                true,
            );
        }

        fn seed_redo(&mut self) {
            let pos = self.point(15, 11);
            self.press(pos);
            self.release(pos);
            assert_eq!(self.panel.history.undo.len(), 1);
            self.panel.undo();
            assert!(self.panel.history.undo.is_empty());
            assert_eq!(self.panel.history.redo.len(), 1);
            assert!(!self.panel.dirty());
        }
    }

    #[test]
    fn installed_rectangle_pointer_gesture_commits_once_with_captured_selection() {
        let mut h = Harness::new();
        let initial = h.panel.history_snapshot();
        let disk = std::fs::read(&h.panel.path).unwrap();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        assert_eq!(h.panel.history_snapshot(), initial);
        h.panel.selected = Some(11);
        h.frame(vec![egui::Event::PointerMoved(end)], true);
        let pending = h.panel.rectangle.as_ref().unwrap();
        assert_eq!(pending.start, CellCoord { column: 1, row: 1 });
        assert_eq!(pending.end, CellCoord { column: 3, row: 2 });
        assert_eq!(pending.region, Some(10));
        assert_eq!(h.panel.history_snapshot(), initial);
        assert_eq!(std::fs::read(&h.panel.path).unwrap(), disk);
        h.release(end);
        assert!(h.panel.rectangle.is_none());
        assert!(h.panel.error.is_none());
        let mut expected = vec![None; 192];
        // Literal 16-column row-major indices, not production rectangle logic.
        for index in [17, 18, 19, 33, 34, 35] {
            expected[index] = Some(10);
        }
        assert_eq!(h.panel.document().document().cells, expected);
        assert_eq!(h.panel.history.undo, vec![initial.0.clone()]);
        assert!(h.panel.history.redo.is_empty());
        assert_eq!(h.panel.history.saved, initial.1);
        assert!(h.panel.dirty());
        assert_eq!(std::fs::read(&h.panel.path).unwrap(), disk);
        let committed = h.panel.history_snapshot();
        h.frame(Vec::new(), true);
        assert_eq!(h.panel.history_snapshot(), committed);
        h.panel.undo();
        assert_eq!(h.panel.document(), &initial.0);
        h.panel.redo();
        assert_eq!(h.panel.history_snapshot(), committed);
    }

    #[test]
    fn installed_rectangle_escape_and_focus_loss_cancel_without_history_changes() {
        for lose_focus in [false, true] {
            let mut h = Harness::new();
            h.seed_redo();
            let initial = h.panel.history_snapshot();
            let start = h.point(1, 1);
            let end = h.point(3, 2);
            h.press(start);
            assert!(h.panel.rectangle.is_some());
            let events = if lose_focus {
                vec![egui::Event::WindowFocused(false)]
            } else {
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            };
            h.frame(events, !lose_focus);
            assert!(h.panel.rectangle.is_none());
            assert_eq!(h.panel.history_snapshot(), initial);
            // Releasing after cancellation must not turn into a fresh gesture.
            h.release(end);
            assert!(h.panel.rectangle.is_none());
            assert_eq!(h.panel.history_snapshot(), initial);
        }

        // A frame spent on another viewport/tab cannot silently keep capture:
        // returning with a release must cancel the stale rectangle, not commit.
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        let mut output = h.ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert_eq!(h.panel.history_snapshot(), initial);
        h.release(end);
        assert!(h.panel.rectangle.is_none());
        assert_eq!(h.panel.history_snapshot(), initial);
    }

    #[test]
    fn installed_rectangle_invalid_release_cancels_and_preserves_redo() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        for invalid in [
            viewport().max + egui::vec2(25.0, 25.0),
            viewport().min + egui::vec2(10.0, 10.0),
        ] {
            h.press(start);
            assert!(h.panel.rectangle.is_some());
            h.frame(vec![egui::Event::PointerMoved(end)], true);
            assert_eq!(h.panel.history_snapshot(), initial);
            h.release(invalid);
            assert!(h.panel.rectangle.is_none());
            assert_eq!(h.panel.history_snapshot(), initial);
            h.press(invalid);
            assert!(h.panel.rectangle.is_none());
            h.release(end);
            assert_eq!(h.panel.history_snapshot(), initial);
        }
        h.panel.selected = Some(u32::MAX);
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        h.release(end);
        assert!(h.panel.rectangle.is_none());
        assert!(h
            .panel
            .error
            .as_deref()
            .unwrap()
            .contains("nonexistent region"));
        assert_eq!(h.panel.history_snapshot(), initial);
    }

    #[test]
    fn installed_rectangle_source_stamp_change_cancels_without_overwriting_history() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        let mut paths = vec![
            h.panel.stamp.root.join("orr.project.json"),
            h.panel.stamp.root.join("orr.packages.lock.json"),
            h.panel.path.clone(),
        ];
        if let Some((path, _)) = &h.panel.stamp.actors {
            paths.push(path.clone());
        }
        for path in paths {
            h.press(start);
            assert!(h.panel.rectangle.is_some());
            let original = std::fs::read(&path).unwrap();
            let mut changed = original.clone();
            changed.push(b'\n');
            // Semantically identical JSON still invalidates the byte stamp.
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&original).unwrap(),
                serde_json::from_slice::<serde_json::Value>(&changed).unwrap()
            );
            std::fs::write(&path, &changed).unwrap();
            h.release(end);
            assert!(h.panel.rectangle.is_none());
            assert!(h
                .panel
                .error
                .as_deref()
                .unwrap()
                .contains("changed externally"));
            assert_eq!(h.panel.history_snapshot(), initial);
            assert_eq!(std::fs::read(&path).unwrap(), changed);
            assert!(h.panel.check_stamp().is_err());
            // Restore this test-owned fixture before probing the next source.
            std::fs::write(&path, original).unwrap();
            assert!(h.panel.check_stamp().is_ok());
            h.panel.error = None;
        }
    }

    #[test]
    fn installed_rectangle_transient_focus_loss_cancels_even_when_focus_returns() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        h.frame(
            vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
                button(end, false),
            ],
            true,
        );
        assert!(h.panel.rectangle.is_none());
        assert_eq!(h.panel.history_snapshot(), initial);
    }

    #[test]
    fn installed_rectangle_coalesced_button_positions_do_not_follow_later_moves() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        let outside = viewport().max + egui::vec2(25.0, 25.0);
        // A press outside followed by an inside move in the same input batch
        // must not be reinterpreted as an inside press.
        h.frame(
            vec![
                egui::Event::PointerMoved(outside),
                button(outside, true),
                egui::Event::PointerMoved(end),
            ],
            true,
        );
        assert!(h.panel.rectangle.is_none());
        h.release(end);
        assert_eq!(h.panel.history_snapshot(), initial);
        // Likewise, a release outside cannot be rescued by a later inside move.
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        h.frame(
            vec![button(outside, false), egui::Event::PointerMoved(end)],
            true,
        );
        assert!(h.panel.rectangle.is_none());
        assert_eq!(h.panel.history_snapshot(), initial);
        // An ambiguous release/re-press batch cancels rather than consuming two
        // transactions or keeping a fresh gesture alive after the old release.
        h.press(start);
        h.frame(vec![button(end, false), button(start, true)], true);
        assert!(h.panel.rectangle.is_none());
        h.release(end);
        assert_eq!(h.panel.history_snapshot(), initial);
        // A complete unambiguous click-drag-release batch is valid, but its
        // final pointer move must not stretch the captured release endpoint.
        h.frame(
            vec![
                button(start, true),
                egui::Event::PointerMoved(end),
                button(end, false),
                egui::Event::PointerMoved(h.point(8, 7)),
            ],
            true,
        );
        assert!(h.panel.rectangle.is_none());
        assert!(h.panel.error.is_none());
        let mut expected = vec![None; 192];
        for index in [17, 18, 19, 33, 34, 35] {
            expected[index] = Some(10);
        }
        assert_eq!(h.panel.document().document().cells, expected);
        assert_eq!(h.panel.history.undo.len(), 1);
        assert!(h.panel.history.redo.is_empty());
        assert_eq!(h.panel.history.saved, initial.1);
    }

    #[test]
    fn installed_rectangle_scene_byte_change_rejects_release_without_history_changes() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let tilemap_bytes = std::fs::read(&h.panel.path).unwrap();
        let scene_bytes = std::fs::read(&h.panel.stamp.scene).unwrap();
        let start = h.point(1, 1);
        let end = h.point(3, 2);
        h.press(start);
        assert_eq!(h.panel.rectangle.as_ref().unwrap().scene_bytes, scene_bytes);
        // A trailing newline changes bytes, not the YAML scene meaning.
        let mut changed = scene_bytes.clone();
        changed.push(b'\n');
        std::fs::write(&h.panel.stamp.scene, &changed).unwrap();
        h.release(end);
        assert!(h.panel.rectangle.is_none());
        assert!(h
            .panel
            .error
            .as_deref()
            .unwrap()
            .contains("scene source changed"));
        assert_eq!(h.panel.history_snapshot(), initial);
        assert_eq!(std::fs::read(&h.panel.path).unwrap(), tilemap_bytes);
        assert_eq!(std::fs::read(&h.panel.stamp.scene).unwrap(), changed);
        std::fs::write(&h.panel.stamp.scene, scene_bytes).unwrap();
    }
}
