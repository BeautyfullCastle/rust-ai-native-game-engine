use super::*;
use crate::tilemap_panel::{History, TilemapPanel};
use orr_tilemap::{Document, SpriteRef};
fn atlas() -> SpriteDocument {
    SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1}],"clips":[]}"#).unwrap()
}
fn document(width: u32, height: u32, cells: Vec<Option<u32>>) -> ValidatedDocument {
    Document {
        version: 1,
        sprite: SpriteRef {
            package: "fixture".into(),
            document: "sprites.json".into(),
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
fn copy_all_corner_orders_literal_pattern_overlap_and_snapshot_reuse() {
    let source = document(
        5,
        3,
        vec![
            Some(0),
            None,
            Some(1),
            None,
            None,
            Some(1),
            Some(0),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    for (start, end) in [
        (cell(0, 0), cell(2, 1)),
        (cell(2, 1), cell(0, 0)),
        (cell(2, 0), cell(0, 1)),
        (cell(0, 1), cell(2, 0)),
    ] {
        let pattern = Pattern::capture(&source, start, end, 7, &atlas()).unwrap();
        assert_eq!((pattern.width, pattern.height), (3, 2));
        assert_eq!(
            pattern.cells,
            vec![Some(0), None, Some(1), Some(1), Some(0), None]
        );
        let overlapping = pattern.candidate(&source, cell(1, 1), 7, &atlas()).unwrap();
        assert_eq!(
            overlapping.document().cells,
            vec![
                Some(0),
                None,
                Some(1),
                None,
                None,
                Some(1),
                Some(0),
                None,
                Some(1),
                None,
                None,
                Some(1),
                Some(0),
                None,
                None
            ]
        );
        let repeated = pattern
            .candidate(&overlapping, cell(2, 0), 7, &atlas())
            .unwrap();
        assert_eq!(
            repeated.document().cells,
            vec![
                Some(0),
                None,
                Some(0),
                None,
                Some(1),
                Some(1),
                Some(0),
                Some(1),
                Some(0),
                None,
                None,
                Some(1),
                Some(0),
                None,
                None
            ]
        );
        assert_eq!(
            source.document().cells,
            vec![
                Some(0),
                None,
                Some(1),
                None,
                None,
                Some(1),
                Some(0),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None
            ]
        );
        let old = source.document();
        let new = repeated.document();
        assert_eq!(old.sprite, new.sprite);
        assert_eq!(old.origin.map(f32::to_bits), new.origin.map(f32::to_bits));
        assert_eq!(old.tile_size, new.tile_size);
        assert_eq!(old.units_per_pixel.to_bits(), new.units_per_pixel.to_bits());
    }
}
#[test]
fn copy_empty_zero_edges_overflow_fail_atomic_and_noop_redo() {
    let source = document(3, 2, vec![None, Some(0), Some(1), None, None, None]);
    let empty = Pattern::capture(&source, cell(0, 0), cell(0, 1), 4, &atlas()).unwrap();
    let mut history = History::new(source.clone()).unwrap();
    let erased = empty.candidate(&source, cell(1, 0), 4, &atlas()).unwrap();
    assert_eq!(
        erased.document().cells,
        vec![None, None, Some(1), None, None, None]
    );
    assert!(history.commit(erased.clone()));
    history.undo();
    assert!(!history.commit(empty.candidate(&source, cell(0, 0), 4, &atlas()).unwrap()));
    assert_eq!(history.redo, vec![erased]);
    let snapshot = (
        history.current.clone(),
        history.undo.clone(),
        history.redo.clone(),
        history.saved.clone(),
    );
    for anchor in [cell(0, 1), cell(3, 0), cell(u32::MAX, 0), cell(0, u32::MAX)] {
        assert!(empty.candidate(&source, anchor, 4, &atlas()).is_err());
    }
    for anchor in [cell(3, 0), cell(u32::MAX, 0), cell(0, u32::MAX)] {
        assert!(Pattern::capture(&source, anchor, cell(0, 0), 4, &atlas()).is_err());
    }
    let reversed = Pattern::capture(&source, cell(0, 1), cell(0, 0), 4, &atlas()).unwrap();
    assert_eq!(
        (reversed.width, reversed.height, reversed.cells),
        (empty.width, empty.height, empty.cells)
    );
    assert_eq!(
        (history.current, history.undo, history.redo, history.saved),
        snapshot
    );
}
#[test]
fn copy_maximum_budget_epoch_geometry_atlas_and_local_edit_identity() {
    let source = document(64, 64, vec![Some(0); 4096]);
    let pattern = Pattern::capture(&source, cell(63, 63), cell(0, 0), 2, &atlas()).unwrap();
    assert_eq!(pattern.cells.len(), 4096);
    let next = pattern.candidate(&source, cell(0, 0), 2, &atlas()).unwrap();
    TilemapPanel::budget(&next, 0).unwrap();
    assert!(TilemapPanel::budget(&next, 1).is_err());
    assert!(TilemapPanel::budget(&next, usize::MAX).is_err());
    assert!(pattern.candidate(&source, cell(1, 0), 2, &atlas()).is_err());
    assert!(pattern.candidate(&source, cell(0, 0), 3, &atlas()).is_err());
    let edited = source.with_cell(cell(1, 1), None, &atlas()).unwrap();
    assert!(pattern.matches(&edited, 2, &atlas()));
    let mut changed = source.document().clone();
    changed.origin[0] = 0.0;
    assert!(!pattern.geometry_matches(&changed.validate(&atlas()).unwrap()));
    changed = source.document().clone();
    changed.units_per_pixel = 1.0;
    assert!(!pattern.geometry_matches(&changed.validate(&atlas()).unwrap()));
    let other =
        SpriteDocument::from_json(&atlas().to_json().unwrap().replace("atlas.png", "other.png"))
            .unwrap();
    assert!(!pattern.matches(&source, 2, &other));
}

#[cfg(all(test, feature = "project-create", target_os = "linux"))]
mod installed_copy_gestures {
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
            panel.mode = ToolMode::Copy;
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
            self.panel.mode = ToolMode::Rectangle;
            let pos = self.point(15, 11);
            self.press(pos);
            self.release(pos);
            assert_eq!(self.panel.history.undo.len(), 1);
            self.panel.undo();
            assert!(self.panel.history.undo.is_empty());
            assert_eq!(self.panel.history.redo.len(), 1);
            assert!(!self.panel.dirty());
            self.panel.mode = ToolMode::Copy;
        }
    }

    #[test]
    fn copy_capture_is_read_only_stamp_once_and_resize_undo_expires() {
        let mut h = Harness::new();
        h.seed_redo();
        let initial = h.panel.history_snapshot();
        let start = h.point(0, 0);
        let end = h.point(2, 1);
        h.press(start);
        h.release(end);
        assert_eq!(h.panel.history_snapshot(), initial);
        assert!(h.panel.pattern.is_some());
        h.panel.mode = ToolMode::Stamp;
        let target = h.point(5, 5);
        h.press(target);
        h.release(target);
        assert_eq!(h.panel.history_snapshot(), initial); // all empty stamp preserves redo
        h.panel.redo(); // ordinary local edit keeps the immutable empty pattern
        assert!(h.panel.pattern.is_some());
        let occupied = h.point(15, 11);
        h.press(occupied);
        h.release(occupied); // out-of-bounds whole footprint refuses
        assert!(h.panel.test_error().is_some());
        h.panel.draft_width = "17".into();
        h.panel.draft_height = "12".into();
        h.panel.resize(&h.editor, &h.sprites).unwrap();
        assert!(h.panel.pattern.is_none());
        h.panel.undo();
        assert!(h.panel.pattern.is_none());
    }
    #[test]
    fn copy_cancellation_and_source_guard_preserve_history_and_pattern() {
        let mut h = Harness::new();
        let start = h.point(0, 0);
        let end = h.point(2, 1);
        let initial = h.panel.history_snapshot();
        h.press(start);
        h.frame(vec![], false);
        h.release(end);
        assert!(h.panel.pattern.is_none());
        assert_eq!(h.panel.history_snapshot(), initial);
        h.press(start);
        h.panel.cancel_gesture();
        h.release(end);
        assert!(h.panel.pattern.is_none());
        assert_eq!(h.panel.history_snapshot(), initial);
        h.press(start);
        std::fs::write(&h.panel.path, b"external change").unwrap();
        h.release(end);
        assert!(h.panel.pattern.is_none());
        assert!(h.panel.test_error().is_some());
        assert_eq!(h.panel.history_snapshot(), initial);
    }
    #[test]
    fn copy_stamp_one_press_no_drag_and_epoch_invalidation() {
        let mut h = Harness::new();
        h.panel.mode = ToolMode::Rectangle;
        let seed = h.point(1, 1);
        h.press(seed);
        h.release(seed);
        h.panel.mode = ToolMode::Copy;
        h.press(seed);
        h.release(seed);
        let captured = h.panel.history_snapshot();
        h.panel.mode = ToolMode::Stamp;
        let target = h.point(4, 4);
        let drag = h.point(5, 4);
        h.press(target);
        assert_eq!(h.panel.history.undo.len(), captured.2.len() + 1);
        assert_eq!(
            h.panel
                .document()
                .cell(CellCoord { column: 4, row: 4 })
                .unwrap(),
            Some(10)
        );
        let stamped = h.panel.history_snapshot();
        h.frame(vec![egui::Event::PointerMoved(drag)], true);
        h.release(drag);
        assert_eq!(h.panel.history_snapshot(), stamped);
        h.panel.undo();
        assert_eq!(h.panel.document(), &captured.0);
        assert!(h.panel.pattern.is_some());
        // Coalesced outside press cannot be rescued by the final inside pointer.
        let outside = viewport().max + egui::vec2(20.0, 20.0);
        let before = h.panel.history_snapshot();
        h.frame(
            vec![button(outside, true), egui::Event::PointerMoved(target)],
            true,
        );
        h.release(target);
        assert_eq!(h.panel.history_snapshot(), before);
        h.panel.editor_epoch = None;
        h.press(target);
        h.release(target);
        assert!(h.panel.pattern.is_none());
        assert_eq!(h.panel.history_snapshot(), before);
    }
    #[test]
    fn copy_scene_bytes_skipped_frames_and_escape_are_cancelled() {
        let mut h = Harness::new();
        let start = h.point(1, 1);
        let end = h.point(2, 2);
        let before = h.panel.history_snapshot();
        h.press(start);
        let mut output = h.ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        h.release(end);
        assert!(h.panel.pattern.is_none());
        h.press(start);
        h.frame(
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        h.release(end);
        assert!(h.panel.pattern.is_none());
        h.frame(
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        h.press(start);
        let mut bytes = std::fs::read(&h.panel.stamp.scene).unwrap();
        bytes.push(b'\n');
        std::fs::write(&h.panel.stamp.scene, bytes).unwrap();
        h.release(end);
        assert!(h.panel.pattern.is_none());
        assert!(h
            .panel
            .test_error()
            .unwrap()
            .contains("scene source changed"));
        assert_eq!(h.panel.history_snapshot(), before);
    }
    #[test]
    fn copy_direct_stamp_refuses_active_gestures_atomically() {
        let mut h = Harness::new();
        let start = h.point(0, 0);
        h.press(start);
        h.release(start);
        let before = h.panel.history_snapshot();
        h.panel.painting = true;
        assert!(h
            .panel
            .stamp_pattern(CellCoord { column: 1, row: 1 }, &h.editor, &h.sprites)
            .unwrap_err()
            .contains("current tile gesture"));
        assert_eq!(h.panel.history_snapshot(), before);
        assert!(h.panel.pattern.is_some());
        h.panel.painting = false;
        h.press(start);
        assert!(h.panel.rectangle.is_some());
        assert!(h
            .panel
            .stamp_pattern(CellCoord { column: 1, row: 1 }, &h.editor, &h.sprites)
            .unwrap_err()
            .contains("current tile gesture"));
        assert_eq!(h.panel.history_snapshot(), before);
        assert!(h.panel.pattern.is_some());
    }
    #[test]
    fn copy_and_stamp_reject_installed_atlas_tamper_atomically() {
        let mut h = Harness::new();
        h.panel.mode = ToolMode::Rectangle;
        let start = h.point(0, 0);
        h.press(start);
        h.release(start);
        h.panel.mode = ToolMode::Copy;
        h.press(start);
        h.release(start);
        let before = h.panel.history_snapshot();
        let pattern = h.panel.pattern.as_ref().unwrap().clone();
        let texture = h.panel.pattern_texture;
        let key = h.panel.key();
        let image = h
            .sprites
            .tilemap_asset(&key)
            .unwrap()
            .0
            .document
            .atlas()
            .image
            .clone();
        let root = h.panel.stamp.root.clone();
        let project =
            orr_package::Project::open(&root, crate::sprite_bindings::compiled_runtime()).unwrap();
        let lock = project.verify().unwrap();
        let original = project.read_asset(&key.0, &image).unwrap();
        let path = root
            .join(".orr/packages/objects")
            .join(&lock.packages[&key.0].digest)
            .join(&image);
        let mut files = vec![
            h.panel.path.clone(),
            h.panel.stamp.scene.clone(),
            root.join("orr.project.json"),
            root.join("orr.packages.lock.json"),
        ];
        if let Some((path, _)) = &h.panel.stamp.actors {
            files.push(path.clone());
        }
        let bytes: Vec<_> = files
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        h.press(start);
        std::fs::write(&path, b"tampered installed atlas").unwrap();
        let end = h.point(2, 1);
        h.release(end);
        assert!(h
            .panel
            .test_error()
            .unwrap()
            .contains("installed tilemap assets"));
        assert!(h
            .panel
            .test_error()
            .unwrap()
            .contains("installed content changed"));
        assert_eq!(h.panel.history_snapshot(), before);
        let copied = h.panel.pattern.as_ref().unwrap();
        assert_eq!(
            (
                copied.width,
                copied.height,
                &copied.cells,
                &copied.geometry,
                copied.epoch,
                &copied.atlas
            ),
            (
                pattern.width,
                pattern.height,
                &pattern.cells,
                &pattern.geometry,
                pattern.epoch,
                &pattern.atlas
            )
        );
        assert_eq!(h.panel.pattern_texture, texture);
        assert!(h.panel.rectangle.is_none());
        let error = h
            .panel
            .stamp_pattern(CellCoord { column: 4, row: 4 }, &h.editor, &h.sprites)
            .unwrap_err();
        assert!(error.contains("installed tilemap assets"));
        assert!(error.contains("installed content changed"));
        assert_eq!(h.panel.history_snapshot(), before);
        let copied = h.panel.pattern.as_ref().unwrap();
        assert_eq!(
            (
                copied.width,
                copied.height,
                &copied.cells,
                &copied.geometry,
                copied.epoch,
                &copied.atlas
            ),
            (
                pattern.width,
                pattern.height,
                &pattern.cells,
                &pattern.geometry,
                pattern.epoch,
                &pattern.atlas
            )
        );
        assert_eq!(h.panel.pattern_texture, texture);
        assert_eq!(std::fs::read(&path).unwrap(), b"tampered installed atlas");
        for (file, original) in files.iter().zip(&bytes) {
            assert_eq!(&std::fs::read(file).unwrap(), original);
        }
        assert!(h.sprites.tilemap_asset(&key).is_some());
        std::fs::write(&path, original).unwrap();
        assert!(h
            .panel
            .stamp_pattern(CellCoord { column: 4, row: 4 }, &h.editor, &h.sprites)
            .unwrap());
        assert_eq!(
            h.panel
                .document()
                .cell(CellCoord { column: 4, row: 4 })
                .unwrap(),
            Some(10)
        );
        assert_eq!(h.panel.history.undo.len(), before.2.len() + 1);
    }
}
