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

// CPU egui input frames exercise the production Pick query without a host,
// socket, native window or GPU. Host lifecycle admission stays in paint_viewport.
struct PickHarness {
    ctx: egui::Context,
    camera: Camera,
}
impl PickHarness {
    const PIXELS: (u32, u32) = (1200, 800);
    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(80., 60.), egui::vec2(600., 400.))
    }
    fn new(document: &ValidatedDocument) -> Self {
        let h = Self {
            ctx: egui::Context::default(),
            camera: Camera::new([1.5, 1.], 2.),
        };
        h.frame(document, Vec::new(), true, true, Self::rect(), false);
        h
    }
    fn point(&self, column: u32, row: u32) -> egui::Pos2 {
        let p = self
            .camera
            .world_to_screen([column as f32 + 0.5, 1.5 - row as f32], Self::PIXELS);
        Self::rect().min + egui::vec2(p[0] / 2., p[1] / 2.)
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn frame(
        &self,
        document: &ValidatedDocument,
        events: Vec<egui::Event>,
        focused: bool,
        enabled: bool,
        clip: egui::Rect,
        covered: bool,
    ) -> Option<Option<u32>> {
        let mut picked = None;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                focused,
                ..Default::default()
            },
            |ui| {
                if covered {
                    egui::Area::new(egui::Id::new("pick obstruction"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(Self::rect().min)
                        .show(ui.ctx(), |ui| {
                            ui.set_min_size(Self::rect().size());
                        });
                }
                ui.set_clip_rect(clip);
                ui.add_enabled_ui(enabled, |ui| {
                    picked = TilemapPanel::picked_region(
                        ui,
                        &self.camera,
                        Self::rect(),
                        Self::PIXELS,
                        document,
                    );
                });
            },
        );
        output.textures_delta.clear();
        picked
    }
    fn press(&self, document: &ValidatedDocument, pos: egui::Pos2) -> Option<Option<u32>> {
        self.frame(
            document,
            vec![egui::Event::PointerMoved(pos), Self::button(pos, true)],
            true,
            true,
            Self::rect(),
            false,
        )
    }
    fn release(&self, document: &ValidatedDocument, pos: egui::Pos2) {
        assert_eq!(
            self.frame(
                document,
                vec![Self::button(pos, false)],
                true,
                true,
                Self::rect(),
                false
            ),
            None
        );
    }
}

#[test]
fn pick_distinguishes_zero_empty_and_other_region_without_history_changes() {
    let mut history = History::new(document()).unwrap();
    for (column, region) in [(0, 0), (1, 1), (2, 1)] {
        history.commit(
            history
                .current
                .with_cell(CellCoord { column, row: 0 }, Some(region), &atlas())
                .unwrap(),
        );
    }
    history.undo(); // Retain both undo and redo while picking.
    let before = (
        history.current.clone(),
        history.saved.clone(),
        history.undo.clone(),
        history.redo.clone(),
        history.dirty(),
    );
    let h = PickHarness::new(&history.current);
    for (column, expected) in [(0, Some(0)), (1, Some(1)), (2, None)] {
        let pos = h.point(column, 0);
        assert_eq!(h.press(&history.current, pos), Some(expected));
        h.release(&history.current, pos);
        assert_eq!(
            (
                history.current.clone(),
                history.saved.clone(),
                history.undo.clone(),
                history.redo.clone(),
                history.dirty()
            ),
            before
        );
    }
}

#[test]
fn pick_uses_actual_press_position_and_never_drag_entry_or_held_input() {
    let doc = document()
        .with_cell(CellCoord { column: 0, row: 0 }, Some(0), &atlas())
        .unwrap();
    let h = PickHarness::new(&doc);
    let inside = h.point(0, 0);
    let empty = h.point(2, 0);
    let outside = egui::pos2(20., 20.);
    assert_eq!(
        h.frame(
            &doc,
            vec![
                PickHarness::button(outside, true),
                egui::Event::PointerMoved(inside)
            ],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        None
    );
    assert_eq!(
        h.frame(
            &doc,
            vec![egui::Event::PointerMoved(inside)],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        None
    );
    h.release(&doc, inside);
    // Final cursor position differs from the actual press in the same frame.
    assert_eq!(
        h.frame(
            &doc,
            vec![
                PickHarness::button(inside, true),
                egui::Event::PointerMoved(empty)
            ],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        Some(Some(0))
    );
    assert_eq!(
        h.frame(&doc, Vec::new(), true, true, PickHarness::rect(), false),
        None
    );
    h.release(&doc, empty);
    assert_eq!(
        h.frame(
            &doc,
            vec![
                PickHarness::button(outside, true),
                PickHarness::button(outside, false),
                egui::Event::PointerMoved(inside)
            ],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        None
    );
    assert_eq!(
        h.frame(
            &doc,
            vec![
                PickHarness::button(inside, true),
                PickHarness::button(inside, false),
                PickHarness::button(empty, true)
            ],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        None
    );
}

#[test]
fn pick_rejects_disabled_unfocused_clipped_and_covered_presses() {
    let doc = document();
    for (focused, enabled, clipped, covered) in [
        (false, true, false, false),
        (true, false, false, false),
        (true, true, true, false),
        (true, true, false, true),
    ] {
        let h = PickHarness::new(&doc);
        let clip = if clipped {
            egui::Rect::from_min_max(egui::pos2(80., 60.), egui::pos2(90., 70.))
        } else {
            PickHarness::rect()
        };
        h.frame(&doc, Vec::new(), focused, enabled, clip, covered);
        let pos = h.point(0, 0);
        assert_eq!(
            h.frame(
                &doc,
                vec![PickHarness::button(pos, true)],
                focused,
                enabled,
                clip,
                covered
            ),
            None
        );
    }
    let h = PickHarness::new(&doc);
    let pos = h.point(0, 0);
    assert_eq!(
        h.frame(
            &doc,
            vec![
                PickHarness::button(pos, true),
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true)
            ],
            true,
            true,
            PickHarness::rect(),
            false
        ),
        None
    );
}

#[test]
fn pick_rejects_outside_grid_and_degenerate_viewports() {
    let doc = document();
    let h = PickHarness::new(&doc);
    assert_eq!(h.press(&doc, h.point(3, 0)), None);
    h.release(&doc, h.point(3, 0));
    for world in [
        [f32::NAN, 0.],
        [f32::INFINITY, 0.],
        [-0.1, 1.5],
        [0.5, 2.1],
        [3., 1.5],
        [0.5, 0.],
    ] {
        assert_eq!(TilemapPanel::document_cell_at(&doc, world), None);
    }
    let pos = h.point(0, 0);
    let _ = h.ctx.run_ui(
        egui::RawInput {
            events: vec![PickHarness::button(pos, true)],
            focused: true,
            ..Default::default()
        },
        |ui| {
            for pixels in [(0, 800), (1200, 0)] {
                assert_eq!(
                    TilemapPanel::picked_region(ui, &h.camera, PickHarness::rect(), pixels, &doc),
                    None
                );
            }
            assert_eq!(
                TilemapPanel::picked_region(
                    ui,
                    &h.camera,
                    egui::Rect::from_min_size(pos, egui::Vec2::ZERO),
                    PickHarness::PIXELS,
                    &doc
                ),
                None
            );
        },
    );
}

#[test]
fn pick_viewport_dispatch_changes_only_selection_in_admitted_project() {
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};

    fn copy_fixture(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                copy_fixture(&entry.path(), &target.join(entry.file_name()));
            } else {
                assert!(kind.is_file());
                std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            }
        }
    }
    const PIXELS: (u32, u32) = (1200, 800);
    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(80., 60.), egui::vec2(600., 400.))
    }
    fn frame(
        ctx: &egui::Context,
        panel: &mut TilemapPanel,
        editor: &Editor,
        sprites: &SpritePanel,
        events: Vec<egui::Event>,
    ) -> bool {
        let mut reserved = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                reserved = panel.paint_viewport(ui, editor, sprites, rect(), PIXELS);
            },
        );
        output.textures_delta.clear();
        reserved
    }
    fn point(panel: &TilemapPanel, editor: &Editor, column: u32, row: u32) -> egui::Pos2 {
        let d = panel.document().document();
        let world = [
            d.origin[0] + (column as f32 + 0.5) * d.tile_size[0] as f32 * d.units_per_pixel,
            d.origin[1] - (row as f32 + 0.5) * d.tile_size[1] as f32 * d.units_per_pixel,
        ];
        let screen = editor.camera.world_to_screen(world, PIXELS);
        rect().min + egui::vec2(screen[0] / 2., screen[1] / 2.)
    }

    // Copy only the checked-in installed fixture; no Linux creator, installation,
    // arbitrary user directory, GPU, native window or network listener is used.
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let root = temp.path().join("pick-viewport");
    copy_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/saved_arena_project"),
        &root,
    );
    let manifest_path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["tilemap"] = serde_json::json!("arena.tilemap.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let map = orr_tilemap::Document {
        version: 1,
        sprite: orr_tilemap::SpriteRef {
            package: "sample-sprites".into(),
            document: "sprites.json".into(),
        },
        width: 3,
        height: 2,
        tile_size: [16, 16],
        origin: [0., 32.],
        units_per_pixel: 1.,
        cells: vec![Some(10), Some(11), None, None, None, None],
    };
    std::fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec_pretty(&map).unwrap(),
    )
    .unwrap();
    let admitted = crate::project::PreparedProject::open(&root).unwrap();
    let spec = admitted.host_spec();
    assert!(matches!(
        spec,
        crate::backend::HostSpec::PreparedArena { listen: None, .. }
    ));
    let mut editor = Editor::start(&spec).unwrap();
    editor.sync();
    editor.camera = Camera::new([24., 16.], 40.);
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
    let ctx = egui::Context::default();
    let mut sprites = SpritePanel::default();
    if let Some(prepared) = parts.sprites {
        sprites.install_prepared(
            &ctx,
            crate::sprite_bindings::Bindings::from_document(prepared.path, prepared.document),
            prepared.assets,
        );
    }
    sprites.install_tilemap_asset(&ctx, parts.tilemap_asset);
    panel.install_editor_identity(&editor).unwrap();
    panel.install_renderer(None, &sprites).unwrap();
    assert!(panel.editable(&editor));

    // A dirty unsaved cell and a retained redo ensure Pick samples the current
    // document without accidentally saving, committing or consuming history.
    panel
        .change(CellCoord { column: 0, row: 1 }, Some(10), &sprites)
        .unwrap();
    panel
        .change(CellCoord { column: 1, row: 1 }, Some(11), &sprites)
        .unwrap();
    panel.undo();
    assert!(panel.dirty());
    assert!(!panel.history.undo.is_empty() && !panel.history.redo.is_empty());
    panel.tool = true;
    panel.mode = ToolMode::Pick;
    panel.selected = Some(20);
    let state = |panel: &TilemapPanel, editor: &Editor| {
        (
            panel.document().clone(),
            panel.history.saved.clone(),
            panel.history.undo.clone(),
            panel.history.redo.clone(),
            panel.dirty(),
            editor.checksum(),
            editor.history().clone(),
            [
                "orr.project.json",
                "orr.packages.lock.json",
                "arena.scene.yaml",
                "arena.sprites.json",
                "arena.tilemap.json",
            ]
            .map(|name| std::fs::read(root.join(name)).unwrap()),
        )
    };
    let before = state(&panel, &editor);
    assert!(frame(&ctx, &mut panel, &editor, &sprites, Vec::new()));
    for (column, row, expected) in [(0, 1, Some(10)), (1, 0, Some(11)), (1, 1, None)] {
        let pos = point(&panel, &editor, column, row);
        assert!(frame(
            &ctx,
            &mut panel,
            &editor,
            &sprites,
            vec![
                egui::Event::PointerMoved(pos),
                PickHarness::button(pos, true)
            ]
        ));
        assert_eq!(panel.selected, expected);
        assert!(panel.mode == ToolMode::Pick);
        assert!(!panel.painting && panel.rectangle.is_none() && panel.error.is_none());
        let elsewhere = point(&panel, &editor, 2, 0);
        assert!(frame(
            &ctx,
            &mut panel,
            &editor,
            &sprites,
            vec![egui::Event::PointerMoved(elsewhere)]
        ));
        assert_eq!(
            panel.selected, expected,
            "holding/dragging never picks again"
        );
        assert!(frame(
            &ctx,
            &mut panel,
            &editor,
            &sprites,
            vec![PickHarness::button(elsewhere, false)]
        ));
        editor.sync();
        assert_eq!(state(&panel, &editor), before);
    }

    let inside = point(&panel, &editor, 0, 1);
    let outside = egui::pos2(20., 20.);
    assert!(frame(
        &ctx,
        &mut panel,
        &editor,
        &sprites,
        vec![
            PickHarness::button(outside, true),
            egui::Event::PointerMoved(inside)
        ]
    ));
    assert_eq!(panel.selected, None, "outside-origin drag is a no-op");
    frame(
        &ctx,
        &mut panel,
        &editor,
        &sprites,
        vec![PickHarness::button(inside, false)],
    );
    let epoch = panel.editor_epoch.take();
    assert!(frame(
        &ctx,
        &mut panel,
        &editor,
        &sprites,
        vec![PickHarness::button(inside, true)]
    ));
    assert_eq!(panel.selected, None, "missing admission epoch refuses Pick");
    panel.editor_epoch = epoch;
    frame(
        &ctx,
        &mut panel,
        &editor,
        &sprites,
        vec![PickHarness::button(inside, false)],
    );
    panel.tool = false;
    assert!(!frame(
        &ctx,
        &mut panel,
        &editor,
        &sprites,
        vec![PickHarness::button(inside, true)]
    ));
    assert_eq!(panel.selected, None);
    editor.sync();
    assert_eq!(state(&panel, &editor), before);
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
