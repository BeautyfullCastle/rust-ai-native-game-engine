//! One scene-owned terrain authoring session, separate from the simulation/ERP.
//! All authoring numbers use decimal text and integer-only FP parsing. The
//! presentation session never installs collision. The explicit terrain-physics
//! game instead displays immutable terrain admitted by the host.
use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    model::Mode,
    terrain_document::{format_fixed, parse_fixed, NewTerrain, TerrainSession, TerrainSource},
    terrain_pick::{self, TerrainQuery, TerrainSelectionMode},
};
use egui::Ui;
use orr_fp::FP;
use orr_model::StaticModel;
use orr_render::Camera3D;
use orr_terrain::Edit;
use std::{path::PathBuf, sync::Arc};

pub struct TerrainPanel {
    pub document: TerrainSession,
    pub local_path: String,
    pub asset_id: String,
    pub dimensions: [u32; 2],
    pub origin: [String; 2],
    pub spacing: String,
    pub initial_height: String,
    pub project: String,
    pub package: String,
    pub asset: String,
    pub selection_mode: TerrainSelectionMode,
    pub selected_vertex: [u32; 2],
    pub selected_cell: [u32; 2],
    pub height: String,
    pub hole: bool,
    pub query_xz: [String; 2],
    confirm_discard: bool,
    error: Option<String>,
}
impl Default for TerrainPanel {
    fn default() -> Self {
        Self {
            document: TerrainSession::default(),
            local_path: "terrain.orrt".into(),
            asset_id: "terrain/local.orrt".into(),
            dimensions: [9, 9],
            origin: ["-4".into(), "-4".into()],
            spacing: "1".into(),
            initial_height: "0".into(),
            project: ".".into(),
            package: String::new(),
            asset: String::new(),
            selection_mode: TerrainSelectionMode::Entity,
            selected_vertex: [0; 2],
            selected_cell: [0; 2],
            height: "0".into(),
            hole: false,
            query_xz: ["0".into(), "0".into()],
            confirm_discard: false,
            error: None,
        }
    }
}
impl TerrainPanel {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn report<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        self.error = result.as_ref().err().cloned();
        result
    }
    fn local_scene(editor: &Editor) -> Result<PathBuf, String> {
        if (editor.game() != EditorGame::Yard3D && !editor.game().is_navigation()) || !editor.spec().is_local() {
            return Err("Terrain authoring requires a local Yard3D scene".into());
        }
        let expected: &std::path::Path = match editor.spec() {
            HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
            #[cfg(feature = "sprites")]
            HostSpec::PreparedArena { scene, .. } => scene.path(),
            #[cfg(feature = "collect-dodge")]
            HostSpec::PreparedCollect { .. } => return Err("CollectDodge does not support local 3D asset authoring".into()),
            HostSpec::Remote { .. } => {
                return Err("Remote paths are not local terrain authority".into())
            }
        };
        let reported = editor
            .sim()
            .scene_path
            .as_ref()
            .map(PathBuf::from)
            .ok_or("Save the local scene before opening terrain")?;
        if reported.as_path() != expected {
            return Err("Wait for the local scene path to synchronize".into());
        }
        if !expected.is_file() {
            return Err("Save the local scene to a regular file before opening terrain".into());
        }
        // The document validates the lexical path and rejects symlinks itself.
        Ok(expected.to_path_buf())
    }
    fn require_context(editor: &Editor) -> Result<PathBuf, String> {
        let scene = Self::local_scene(editor)?;
        if editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.previewing().is_some()
            || !editor.yard_rows_coherent()
        {
            return Err(
                "Terrain requires coherent local Yard3D Edit mode without a preview".into(),
            );
        }
        Ok(scene)
    }
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        Self::local_scene(editor)
            .ok()
            .is_some_and(|scene| self.document.scene_matches(Some(&scene)))
    }
    pub fn authoring_active(&self, editor: &Editor) -> bool {
        Self::require_context(editor).is_ok() && self.scene_matches(editor)
    }
    /// Distinct from model presence: an all-hole terrain still participates in
    /// render compatibility and must disable stale baked lighting.
    pub fn attached_for_editor(&self, editor: &Editor) -> bool {
        #[cfg(feature = "navigation")]
        if editor.game().is_navigation() && editor.admitted_navigation().admitted { return true; }
        #[cfg(feature = "terrain-physics")]
        if editor.game().is_terrain() { return editor.admitted_terrain().admitted; }
        self.scene_matches(editor) && self.document.terrain().is_some()
    }
    pub fn viewport_model(&self, editor: &Editor) -> Option<Arc<StaticModel>> {
        #[cfg(feature = "terrain-physics")]
        if editor.game().is_terrain() {
            return (editor.yard_rows_coherent() && editor.previewing().is_none()).then(|| editor.admitted_terrain().model.clone()).flatten();
        }
        (self.attached_for_editor(editor)
            && editor.yard_rows_coherent()
            && editor.previewing().is_none())
        .then(|| self.document.model().cloned())
        .flatten()
    }
    fn require_attached(&self, editor: &Editor) -> Result<(), String> {
        Self::require_context(editor)?;
        if !self.scene_matches(editor) {
            return Err(
                "Terrain belongs to another scene; save or discard it before switching".into(),
            );
        }
        if self.document.terrain().is_none() {
            return Err("Create or open terrain first".into());
        }
        Ok(())
    }
    fn prepare_scene(&mut self, editor: &Editor) -> Result<(), String> {
        let scene = Self::require_context(editor)?;
        // A failed open must not replace the previous terrain or its scene.
        if self.document.terrain().is_some() && !self.scene_matches(editor) {
            return Err("Close the previous scene's terrain before opening another terrain".into());
        }
        self.document.set_scene(Some(&scene))
    }
    /// Called before viewport drawing even when the panel is collapsed. Dirty
    /// state survives external scene changes, while its render/query is hidden.
    pub fn sync_for_editor(&mut self, editor: &Editor) {
        let Ok(scene) = Self::require_context(editor) else {
            return;
        };
        if self.document.scene_matches(Some(&scene)) {
            return;
        }
        if let Err(error) = self.document.set_scene(Some(&scene)) {
            self.error = Some(error);
        } else {
            self.confirm_discard = false;
            self.error = None;
            self.selected_vertex = [0; 2];
            self.selected_cell = [0; 2];
        }
    }
    pub fn create_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            // Parse before touching association or any existing terrain.
            let config = NewTerrain {
                asset_id: self.asset_id.clone(),
                width: self.dimensions[0],
                depth: self.dimensions[1],
                origin: [parse_fixed(&self.origin[0])?, parse_fixed(&self.origin[1])?],
                spacing: parse_fixed(&self.spacing)?,
                height: parse_fixed(&self.initial_height)?,
            };
            self.prepare_scene(editor)?;
            self.document.new_local(&self.local_path, config)?;
            self.refresh_loaded();
            Ok(())
        })();
        self.report(result)
    }
    pub fn open_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.prepare_scene(editor)?;
            self.document.open_local(&self.local_path)?;
            self.refresh_loaded();
            Ok(())
        })();
        self.report(result)
    }
    pub fn open_package_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.prepare_scene(editor)?;
            let scene = self.document.scene().ok_or("Save the local scene first")?;
            let path = PathBuf::from(&self.project);
            let project = if path.is_absolute() {
                path
            } else {
                scene
                    .parent()
                    .ok_or("Scene has no parent directory")?
                    .join(path)
            };
            self.document
                .open_package(&project, &self.package, &self.asset)?;
            self.refresh_loaded();
            Ok(())
        })();
        self.report(result)
    }
    pub fn copy_to_scene(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            self.document.copy_to_scene(&self.local_path)
        })();
        self.report(result)
    }
    pub fn save_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            // Save is intentionally to the original explicit scene-owned path,
            // even after an external host scene switch; edits remain disabled.
            Self::require_context(editor)?;
            self.document.save()
        })();
        self.report(result)
    }
    pub fn close_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            Self::require_context(editor)?;
            self.document.close()?;
            self.confirm_discard = false;
            Ok(())
        })();
        self.report(result)
    }
    pub fn discard_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            Self::require_context(editor)?;
            self.document.discard();
            self.confirm_discard = false;
            Ok(())
        })();
        self.report(result)
    }
    pub fn apply_height(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            if self.selection_mode != TerrainSelectionMode::Vertex {
                return Err("Choose Vertex selection to apply a height".into());
            }
            let height = parse_fixed(&self.height)?;
            let [x, z] = self.selected_vertex;
            self.document.apply(&[Edit::SetHeight { x, z, height }])
        })();
        self.report(result)
    }
    pub fn apply_hole(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            if self.selection_mode != TerrainSelectionMode::Cell {
                return Err("Choose Cell selection to change a hole".into());
            }
            let [x, z] = self.selected_cell;
            self.document.apply(&[Edit::SetHole {
                x,
                z,
                hole: self.hole,
            }])
        })();
        self.report(result)
    }
    pub fn undo(&mut self, editor: &Editor) -> Result<bool, String> {
        let result = (|| {
            self.require_attached(editor)?;
            self.document.undo()
        })();
        if matches!(result, Ok(true)) {
            self.refresh_selection();
        }
        self.report(result)
    }
    pub fn redo(&mut self, editor: &Editor) -> Result<bool, String> {
        let result = (|| {
            self.require_attached(editor)?;
            self.document.redo()
        })();
        if matches!(result, Ok(true)) {
            self.refresh_selection();
        }
        self.report(result)
    }
    pub fn query_for_editor(&mut self, editor: &Editor) -> Result<TerrainQuery, String> {
        let result = (|| {
            self.require_attached(editor)?;
            let xz = [
                parse_fixed(&self.query_xz[0])?,
                parse_fixed(&self.query_xz[1])?,
            ];
            self.document.set_query(Some(xz))?;
            Ok(terrain_pick::query(
                self.document.terrain().ok_or("Open terrain first")?,
                xz[0],
                xz[1],
            ))
        })();
        self.report(result)
    }
    pub fn query_result(&self) -> Option<TerrainQuery> {
        let [x, z] = self.document.query()?;
        Some(terrain_pick::query(self.document.terrain()?, x, z))
    }
    fn refresh_loaded(&mut self) {
        self.confirm_discard = false;
        self.selected_vertex = [0; 2];
        self.selected_cell = [0; 2];
        if let Some(terrain) = self.document.terrain() {
            self.asset_id = terrain.asset_id().into();
            self.dimensions = [terrain.width(), terrain.depth()];
            self.origin = terrain.origin().map(format_fixed);
            self.spacing = format_fixed(terrain.spacing());
        }
        self.refresh_selection();
    }
    fn refresh_selection(&mut self) {
        let Some(terrain) = self.document.terrain() else {
            return;
        };
        let [x, z] = self.selected_vertex;
        if x < terrain.width() && z < terrain.depth() {
            self.height = format_fixed(terrain.heights()[(z * terrain.width() + x) as usize]);
        }
        let [x, z] = self.selected_cell;
        if x < terrain.width() - 1 && z < terrain.depth() - 1 {
            self.hole = terrain.holes()[(z * (terrain.width() - 1) + x) as usize];
        }
    }
    /// Returns whether this click belongs to terrain mode, including a miss.
    /// Entity mode and unavailable terrain leave the existing entity route alone.
    pub fn pick(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        pixel: [f32; 2],
        size: (u32, u32),
    ) -> bool {
        if !self.authoring_active(editor)
            || !self.attached_for_editor(editor)
            || self.selection_mode == TerrainSelectionMode::Entity
        {
            return false;
        }
        let Some(terrain) = self.document.terrain() else {
            return false;
        };
        match self.selection_mode {
            TerrainSelectionMode::Vertex => {
                if let Some(vertex) = terrain_pick::pick_vertex(terrain, camera, pixel, size, 12.0)
                {
                    self.selected_vertex = vertex;
                    self.refresh_selection();
                }
            }
            TerrainSelectionMode::Cell => {
                if let Some(cell) = terrain_pick::pick_cell(terrain, camera, pixel, size) {
                    self.selected_cell = cell;
                    self.refresh_selection();
                }
            }
            TerrainSelectionMode::Entity => {}
        }
        true
    }
    pub fn paint_selection(
        &self,
        editor: &Editor,
        painter: &egui::Painter,
        camera: &Camera3D,
        rect: egui::Rect,
    ) {
        if !self.authoring_active(editor) || !self.attached_for_editor(editor) {
            return;
        }
        let Some(terrain) = self.document.terrain() else {
            return;
        };
        let size = (rect.width().max(1.0) as u32, rect.height().max(1.0) as u32);
        let screen = |point: [FP; 3]| {
            camera
                .world_to_screen(point.map(FP::to_f32), size)
                .map(|p| rect.min + egui::vec2(p[0], p[1]))
        };
        let painter = painter.with_clip_rect(rect);
        let color = egui::Color32::from_rgb(255, 220, 70);
        match self.selection_mode {
            TerrainSelectionMode::Vertex => {
                let [x, z] = self.selected_vertex;
                if x < terrain.width() && z < terrain.depth() {
                    if let Some(p) = terrain
                        .vertex_position(z * terrain.width() + x)
                        .and_then(screen)
                    {
                        painter.circle_stroke(p, 7.0, egui::Stroke::new(2.0, color));
                    }
                }
            }
            TerrainSelectionMode::Cell => {
                if let Some(points) = terrain_pick::cell_vertices(terrain, self.selected_cell) {
                    for i in 0..4 {
                        if let (Some(a), Some(b)) = (screen(points[i]), screen(points[(i + 1) % 4]))
                        {
                            painter.line_segment([a, b], egui::Stroke::new(2.0, color));
                        }
                    }
                }
            }
            TerrainSelectionMode::Entity => {}
        }
    }

    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        #[cfg(feature = "terrain-physics")]
        if editor.game().is_terrain() {
            ui.collapsing("Terrain collision", |ui| {
                let admitted = editor.admitted_terrain();
                ui.label("Admitted Frame-owned terrain · dynamic spheres only");
                ui.weak("No hidden floor, mixed rain or input spawning. Terrain and body edits are blocked during Play");
                ui.label(format!("Asset: {}", admitted.identity));
                ui.label(format!("Scene-relative source: {}", admitted.source));
                ui.label(format!("SHA-256: {}", admitted.revision_text()));
                if let Some(error) = &admitted.error { ui.colored_label(egui::Color32::RED, error); }
                ui.weak("To change terrain, stop play, save the source asset, and update its full scene pin. Every scene edit revalidates the source before admission");
            });
            return;
        }

        if editor.game() != EditorGame::Yard3D && !editor.game().is_navigation() {
            return;
        }
        self.sync_for_editor(editor);
        ui.collapsing("Terrain authoring", |ui| {
            let height = ui.available_height().clamp(120.0, 420.0);
            egui::ScrollArea::vertical()
                .id_salt("terrain-authoring-scroll")
                .max_height(height)
                .show(ui, |ui| {
                    ui.weak("Presentation only · terrain Save/Undo are separate from the scene");
                    let context = Self::require_context(editor);
                    if let Err(reason) = &context {
                        ui.label(reason);
                    }
                    ui.add_enabled_ui(context.is_ok(), |ui| {
                        text_field(ui, "Terrain scene-relative path", &mut self.local_path);
                        ui.collapsing("New terrain settings", |ui| {
                            text_field(ui, "Terrain asset ID", &mut self.asset_id);
                            index_field(ui, "Terrain width", &mut self.dimensions[0]);
                            index_field(ui, "Terrain depth", &mut self.dimensions[1]);
                            text_field(ui, "Terrain origin X", &mut self.origin[0]);
                            text_field(ui, "Terrain origin Z", &mut self.origin[1]);
                            text_field(ui, "Terrain spacing", &mut self.spacing);
                            text_field(ui, "Terrain initial height", &mut self.initial_height);
                            ui.weak(format!(
                                "Dimensions: 2–{} vertices per side",
                                orr_terrain::MAX_SIDE
                            ));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Create terrain").clicked() {
                                let _ = self.create_for_editor(editor);
                            }
                            if ui.button("Open terrain").clicked() {
                                let _ = self.open_for_editor(editor);
                            }
                        });
                        ui.collapsing("Installed terrain package", |ui| {
                            text_field(ui, "Terrain project root", &mut self.project);
                            text_field(ui, "Terrain package name", &mut self.package);
                            text_field(ui, "Terrain asset path in package", &mut self.asset);
                            if ui.button("Open verified terrain package").clicked() {
                                let _ = self.open_package_for_editor(editor);
                            }
                        });
                        if self.document.terrain().is_some() {
                            self.show_attached(ui, editor);
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                });
        });
    }
    fn show_attached(&mut self, ui: &mut Ui, editor: &Editor) {
        if let Some(terrain) = self.document.terrain() {
            ui.label(format!(
                "{} · {} × {} vertices",
                terrain.asset_id(),
                terrain.width(),
                terrain.depth()
            ));
        }
        if let Some(source) = self.document.source() {
            match source {
                TerrainSource::Local { relative_path } => {
                    ui.label(format!("Scene terrain: {relative_path}"));
                }
                TerrainSource::Package {
                    package,
                    asset,
                    package_digest,
                    ..
                } => {
                    ui.label(format!("Read-only installed terrain: {package}/{asset}"));
                    ui.small(format!("Verified package: {package_digest}"));
                }
            }
        }
        if !self.scene_matches(editor) {
            ui.colored_label(egui::Color32::YELLOW, "Terrain belongs to the previous scene and is hidden here. Save writes its original path; close or discard before opening another terrain.");
        }
        let attached = self.authoring_active(editor) && self.attached_for_editor(editor);
        let writable = attached && !self.document.read_only();
        ui.add_enabled_ui(attached, |ui| {
            ui.horizontal(|ui| {
                ui.label("Selection");
                ui.selectable_value(
                    &mut self.selection_mode,
                    TerrainSelectionMode::Entity,
                    "Entity",
                );
                ui.selectable_value(
                    &mut self.selection_mode,
                    TerrainSelectionMode::Vertex,
                    "Vertex",
                );
                ui.selectable_value(&mut self.selection_mode, TerrainSelectionMode::Cell, "Cell");
            });
            ui.weak("Numeric X/Z indices are authoritative; holes remain selectable");
            if self.selection_mode == TerrainSelectionMode::Vertex {
                let changed = index_field(ui, "Terrain vertex X", &mut self.selected_vertex[0])
                    | index_field(ui, "Terrain vertex Z", &mut self.selected_vertex[1]);
                if changed {
                    self.refresh_selection();
                }
                text_field(ui, "Terrain height", &mut self.height);
                if ui
                    .add_enabled(writable, egui::Button::new("Apply terrain height"))
                    .clicked()
                {
                    let _ = self.apply_height(editor);
                }
            }
            if self.selection_mode == TerrainSelectionMode::Cell {
                let changed = index_field(ui, "Terrain cell X", &mut self.selected_cell[0])
                    | index_field(ui, "Terrain cell Z", &mut self.selected_cell[1]);
                if changed {
                    self.refresh_selection();
                }
                ui.checkbox(&mut self.hole, "Terrain cell is a hole");
                if ui
                    .add_enabled(writable, egui::Button::new("Apply terrain hole"))
                    .clicked()
                {
                    let _ = self.apply_hole(editor);
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        writable && self.document.can_undo(),
                        egui::Button::new("Terrain Undo"),
                    )
                    .clicked()
                {
                    let _ = self.undo(editor);
                }
                if ui
                    .add_enabled(
                        writable && self.document.can_redo(),
                        egui::Button::new("Terrain Redo"),
                    )
                    .clicked()
                {
                    let _ = self.redo(editor);
                }
            });
            text_field(ui, "Terrain query X", &mut self.query_xz[0]);
            text_field(ui, "Terrain query Z", &mut self.query_xz[1]);
            ui.horizontal(|ui| {
                if ui.button("Query terrain surface").clicked() {
                    let _ = self.query_for_editor(editor);
                }
                if ui.button("Clear terrain query").clicked() {
                    let result = self.document.set_query(None);
                    let _ = self.report(result);
                }
            });
            if let Some([x, z]) = self.document.query() {
                ui.label(format!(
                    "Last query X/Z: {}, {}",
                    format_fixed(x),
                    format_fixed(z)
                ));
            }
            if let Some(result) = self.query_result() {
                match result {
                    TerrainQuery::Outside => {
                        ui.label("Query: outside terrain bounds");
                    }
                    TerrainQuery::Hole { cell } => {
                        ui.label(format!("Query: hole at cell {}, {}", cell[0], cell[1]));
                    }
                    TerrainQuery::Surface { cell, surface } => {
                        ui.label(format!(
                            "Query height: {} · cell {}, {}",
                            format_fixed(surface.height),
                            cell[0],
                            cell[1]
                        ));
                        ui.label(format!(
                            "Normal: [{}, {}, {}]",
                            format_fixed(surface.normal[0]),
                            format_fixed(surface.normal[1]),
                            format_fixed(surface.normal[2])
                        ));
                        ui.label(format!(
                            "Slope (rise/run): {}",
                            surface
                                .slope
                                .map(format_fixed)
                                .unwrap_or_else(|| "outside FP range".into())
                        ));
                    }
                }
            }
            if self.document.read_only() && ui.button("Copy terrain to scene").clicked() {
                let _ = self.copy_to_scene(editor);
            }
        });
        if self.document.dirty() {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Terrain has unsaved changes; scene Save does not save terrain",
            );
        }
        if ui
            .add_enabled(
                !self.document.read_only(),
                egui::Button::new("Save terrain"),
            )
            .clicked()
        {
            let _ = self.save_for_editor(editor);
        }
        if self.confirm_discard {
            ui.colored_label(egui::Color32::YELLOW, "Discard unsaved terrain changes?");
            ui.horizontal(|ui| {
                if ui.button("Discard terrain changes and close").clicked() {
                    let _ = self.discard_for_editor(editor);
                }
                if ui.button("Keep terrain open").clicked() {
                    self.confirm_discard = false;
                }
            });
        } else if ui.button("Close terrain").clicked() {
            if self.document.dirty() {
                self.confirm_discard = true;
            } else {
                let _ = self.close_for_editor(editor);
            }
        }
    }
}
fn text_field(ui: &mut Ui, label: &str, value: &mut String) {
    let label = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label.id);
}
fn index_field(ui: &mut Ui, label: &str, value: &mut u32) -> bool {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(1))
            .labelled_by(label.id)
            .changed()
    })
    .inner
}
