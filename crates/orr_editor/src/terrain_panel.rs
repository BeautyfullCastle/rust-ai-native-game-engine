//! One scene-owned terrain authoring session, separate from the simulation/ERP.
//! All authoring numbers use decimal text and integer-only FP parsing. The
//! presentation session never installs collision. The explicit terrain-physics
//! game instead displays immutable terrain admitted by the host.
use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    model::Mode,
    terrain_document::{
        brush::{self, BrushOperation, TerrainBrush, MAX_BRUSH_RADIUS},
        format_fixed, parse_fixed, NewTerrain, TerrainSession, TerrainSource,
    },
    terrain_pick::{self, TerrainQuery, TerrainSelectionMode},
};
use egui::Ui;
use orr_fp::FP;
use orr_model::StaticModel;
use orr_render::Camera3D;
use orr_terrain::Edit;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SculptMode {
    #[default]
    RaiseLower,
    Flatten,
    Smooth,
}

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
    pub sculpt_mode: SculptMode,
    pub sculpt_radius: u32,
    pub sculpt_delta: String,
    pub sculpt_height: String,
    pub show_sculpt_footprint: bool,
    pub hole: bool,
    pub query_xz: [String; 2],
    confirm_discard: bool,
    error: Option<String>,
    stroke_context_lost: bool,
    sculpt_hover: Option<[u32; 2]>,
    stroke_cancelled_frame: bool,
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
            sculpt_mode: SculptMode::RaiseLower,
            sculpt_radius: 2,
            sculpt_delta: "0.5".into(),
            sculpt_height: "0".into(),
            show_sculpt_footprint: false,
            hole: false,
            query_xz: ["0".into(), "0".into()],
            confirm_discard: false,
            error: None,
            stroke_context_lost: false,
            sculpt_hover: None,
            stroke_cancelled_frame: false,
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
        if (editor.game() != EditorGame::Yard3D && !editor.game().is_navigation())
            || !editor.spec().is_local()
        {
            return Err("Terrain authoring requires a local Yard3D scene".into());
        }
        let expected: &std::path::Path = match editor.spec() {
            HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
            #[cfg(feature = "sprites")]
            HostSpec::PreparedArena { scene, .. } => scene.path(),
            #[cfg(feature = "collect-dodge")]
            HostSpec::PreparedCollect { .. } => {
                return Err("CollectDodge does not support local 3D asset authoring".into())
            }
            #[cfg(feature = "room-project")]
            HostSpec::PreparedRoom { scene, .. } => scene,
            #[cfg(feature = "navigation-project")]
            HostSpec::PreparedNavigation { scene, .. } => scene,
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
        if editor.game().is_navigation() && editor.admitted_navigation().admitted {
            return true;
        }
        #[cfg(feature = "terrain-physics")]
        if editor.game().is_terrain() {
            return editor.admitted_terrain().admitted;
        }
        self.scene_matches(editor) && self.document.terrain().is_some()
    }
    pub fn viewport_model(&self, editor: &Editor) -> Option<Arc<StaticModel>> {
        #[cfg(feature = "terrain-physics")]
        if editor.game().is_terrain() {
            return (editor.yard_rows_coherent() && editor.previewing().is_none())
                .then(|| editor.admitted_terrain().model.clone())
                .flatten();
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
        if self.stroke_context_lost && self.document.terrain().is_some() {
            return;
        }
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
            self.stroke_context_lost = false;
            self.confirm_discard = false;
            Ok(())
        })();
        self.report(result)
    }
    pub fn discard_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            Self::require_context(editor)?;
            self.document.discard();
            self.stroke_context_lost = false;
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
    /// One discrete stamp centered on the selected vertex. Existing viewport
    /// picking selects the center; applying never sends a host scene mutation.
    pub fn apply_sculpt_stamp(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            if self.selection_mode != TerrainSelectionMode::Vertex {
                return Err("Choose Vertex selection to sculpt terrain".into());
            }
            let operation = self.sculpt_operation()?;
            self.document.apply_brush(TerrainBrush {
                center: self.selected_vertex,
                radius: self.sculpt_radius,
                operation,
            })
        })();
        if result.is_ok() {
            self.refresh_selection();
        }
        self.report(result)
    }
    // Sample authoritative admitted data, never the editable height text. This
    // changes only the brush draft and must not create terrain/host history.
    fn sample_flatten_height(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            if self.sculpt_mode != SculptMode::Flatten
                || !matches!(
                    self.selection_mode,
                    TerrainSelectionMode::Vertex | TerrainSelectionMode::Sculpt
                )
            {
                return Err(
                    "Choose Vertex or Sculpt selection and Flatten to sample a height".into(),
                );
            }
            if self.document.read_only() || self.document.stroke_active() {
                return Err("Sampling requires writable terrain with no held stroke".into());
            }
            let terrain = self.document.terrain().ok_or("open a terrain first")?;
            let [x, z] = self.selected_vertex;
            if x >= terrain.width() || z >= terrain.depth() {
                return Err("Sample vertex is outside the terrain grid".into());
            }
            let height = terrain.heights()[(z * terrain.width() + x) as usize];
            self.sculpt_height = format_fixed(height);
            Ok(())
        })();
        self.report(result)
    }

    fn sculpt_operation(&self) -> Result<BrushOperation, String> {
        Ok(match self.sculpt_mode {
            SculptMode::RaiseLower => BrushOperation::RaiseLower {
                delta: parse_fixed(&self.sculpt_delta)?,
            },
            SculptMode::Flatten => BrushOperation::Flatten {
                height: parse_fixed(&self.sculpt_height)?,
            },
            SculptMode::Smooth => BrushOperation::Smooth,
        })
    }
    pub fn cancel_sculpt_stroke(&mut self, reason: &str) -> bool {
        if self.document.cancel_stroke() {
            self.refresh_selection();
            self.error = Some(reason.into());
            true
        } else {
            false
        }
    }
    /// Observe context before any hidden/collapsed viewport can miss cancellation.
    pub fn observe_sculpt_context(
        &mut self,
        editor: &Editor,
        ctx: &egui::Context,
        blocked: bool,
    ) -> bool {
        // Recomputed only by the current viewport; never retain a stale context hit.
        self.sculpt_hover = None;
        self.stroke_cancelled_frame = blocked
            || ctx.input(|i| {
                !i.focused
                    || i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::WindowFocused(false)))
                    || i.key_pressed(egui::Key::Escape)
                    || i.pointer.button_down(egui::PointerButton::Secondary)
                    || i.pointer.button_down(egui::PointerButton::Middle)
                    || i.pointer.button_released(egui::PointerButton::Secondary)
                    || i.pointer.button_released(egui::PointerButton::Middle)
            });
        if !self.document.stroke_active() {
            return false;
        }
        let scene_changed = !self.scene_matches(editor);
        if self.stroke_cancelled_frame
            || editor.game() != EditorGame::Yard3D
            || self.selection_mode != TerrainSelectionMode::Sculpt
            || self.require_attached(editor).is_err()
            || self.document.read_only()
        {
            self.stroke_context_lost |= scene_changed;
            self.stroke_cancelled_frame = true;
            return self.cancel_sculpt_stroke(
                "Terrain stroke cancelled; original terrain and history restored",
            );
        }
        false
    }
    /// Own primary input only in dedicated Sculpt mode; every supplied sample is
    /// processed in order against the frozen baseline. Gaps cancel the full stroke.
    pub fn sculpt_pointer(
        &mut self,
        editor: &Editor,
        response: &egui::Response,
        camera: &Camera3D,
        rect: egui::Rect,
        size: (u32, u32),
        frame_available: bool,
    ) -> bool {
        self.sculpt_hover = None;
        if editor.game() != EditorGame::Yard3D
            || self.selection_mode != TerrainSelectionMode::Sculpt
            || !self.authoring_active(editor)
            || !self.attached_for_editor(editor)
        {
            return false;
        }
        if self.stroke_cancelled_frame {
            return true;
        }
        let (events, down, at, hover) = response.ctx.input(|i| {
            (
                i.events.clone(),
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.interact_pos(),
                i.pointer.hover_pos(),
            )
        });
        let center_at = |panel: &Self, at: egui::Pos2| {
            if !frame_available || !rect.contains(at) {
                return None;
            }
            let pixel = [
                (at.x - rect.min.x) * size.0 as f32 / rect.width(),
                (at.y - rect.min.y) * size.1 as f32 / rect.height(),
            ];
            terrain_pick::pick_sculpt_center(
                panel.document.sculpt_pick_terrain()?,
                camera,
                pixel,
                size,
            )
        };
        if !self.document.stroke_active() && !down && !self.document.read_only() {
            self.sculpt_hover = hover
                .filter(|pos| response.ctx.layer_id_at(*pos) == Some(response.layer_id))
                .and_then(|pos| center_at(self, pos));
        }
        for event in events {
            let (position, starting, released) = match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if rect.contains(pos)
                    && response.ctx.layer_id_at(pos) == Some(response.layer_id) =>
                {
                    (pos, true, false)
                }
                egui::Event::PointerMoved(pos) if self.document.stroke_active() => {
                    (pos, false, false)
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } if self.document.stroke_active() => (pos, false, true),
                egui::Event::PointerGone
                | egui::Event::WindowFocused(false)
                | egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                }
                | egui::Event::PointerButton {
                    button: egui::PointerButton::Secondary | egui::PointerButton::Middle,
                    pressed: true,
                    ..
                } => {
                    self.stroke_cancelled_frame = true;
                    self.sculpt_hover = None;
                    self.cancel_sculpt_stroke("Terrain input interrupted; stroke cancelled");
                    break;
                }
                _ => continue,
            };
            let Some(center) = center_at(self, position) else {
                self.cancel_sculpt_stroke(
                    "Terrain stroke left the viewport surface or hit a hole; stroke cancelled",
                );
                continue;
            };
            let result = (|| {
                if starting {
                    let operation = self.sculpt_operation()?;
                    self.document.begin_stroke(TerrainBrush {
                        center,
                        radius: self.sculpt_radius,
                        operation,
                    })?;
                } else {
                    self.document.update_stroke(center)?;
                }
                self.selected_vertex = center;
                if released {
                    self.document.finish_stroke()?;
                }
                Ok(())
            })();
            if result.is_err() {
                self.document.cancel_stroke();
            }
            self.refresh_selection();
            let _ = self.report(result);
        }
        if self.document.stroke_active() {
            if let Some(center) = at.filter(|_| down).and_then(|at| center_at(self, at)) {
                let result = self.document.update_stroke(center);
                self.refresh_selection();
                let _ = self.report(result);
            } else {
                self.cancel_sculpt_stroke(
                    "Terrain pointer left its valid viewport or lost release; stroke cancelled",
                );
            }
        }
        true
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
        self.stroke_context_lost = false;
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
            TerrainSelectionMode::Entity | TerrainSelectionMode::Sculpt => {}
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
            TerrainSelectionMode::Vertex | TerrainSelectionMode::Sculpt => {
                if self.show_sculpt_footprint || self.selection_mode == TerrainSelectionMode::Sculpt
                {
                    let center = if self.selection_mode == TerrainSelectionMode::Sculpt
                        && !self.document.stroke_active()
                    {
                        self.sculpt_hover
                    } else {
                        Some(self.selected_vertex)
                    };
                    if let Some(Ok(vertices)) = center.map(|center| {
                        brush::footprint(
                            terrain,
                            center,
                            self.document
                                .active_stroke_brush()
                                .map_or(self.sculpt_radius, |b| b.radius),
                        )
                    }) {
                        for [x, z] in vertices {
                            if let Some(p) = terrain
                                .vertex_position(z * terrain.width() + x)
                                .and_then(screen)
                            {
                                painter.circle_filled(
                                    p,
                                    2.5,
                                    egui::Color32::from_rgb(110, 220, 240),
                                );
                            }
                        }
                    }
                }
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
        let writable = attached && !self.document.read_only() && !self.document.stroke_active();
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
                if editor.game() == EditorGame::Yard3D {
                    ui.selectable_value(
                        &mut self.selection_mode,
                        TerrainSelectionMode::Sculpt,
                        "Sculpt",
                    );
                }
            });
            ui.weak("Numeric X/Z indices are authoritative; holes remain selectable");
            if matches!(
                self.selection_mode,
                TerrainSelectionMode::Vertex | TerrainSelectionMode::Sculpt
            ) {
                let changed = index_field(ui, "Terrain vertex X", &mut self.selected_vertex[0])
                    | index_field(ui, "Terrain vertex Z", &mut self.selected_vertex[1]);
                if changed {
                    self.refresh_selection();
                }
                text_field(ui, "Terrain height", &mut self.height);
                if ui
                    .add_enabled(
                        writable && self.selection_mode == TerrainSelectionMode::Vertex,
                        egui::Button::new("Apply terrain height"),
                    )
                    .clicked()
                {
                    let _ = self.apply_height(editor);
                }
                ui.collapsing("Terrain sculpt stamp", |ui| {
                    ui.weak("Pick a vertex or enter X/Z above, then apply one undoable stamp");
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.sculpt_mode,
                            SculptMode::RaiseLower,
                            "Raise/lower",
                        );
                        ui.selectable_value(&mut self.sculpt_mode, SculptMode::Flatten, "Flatten");
                        ui.selectable_value(&mut self.sculpt_mode, SculptMode::Smooth, "Smooth");
                    });
                    ui.horizontal(|ui| {
                        let label = ui.label("Sculpt radius in grid steps");
                        ui.add(
                            egui::DragValue::new(&mut self.sculpt_radius)
                                .range(0..=MAX_BRUSH_RADIUS)
                                .speed(1),
                        )
                        .labelled_by(label.id);
                    });
                    match self.sculpt_mode {
                        SculptMode::RaiseLower => {
                            text_field(ui, "Sculpt height change", &mut self.sculpt_delta);
                            ui.weak("Use a negative change to lower terrain");
                        }
                        SculptMode::Flatten => {
                            text_field(ui, "Sculpt target height", &mut self.sculpt_height);
                            if ui
                                .add_enabled(
                                    writable,
                                    egui::Button::new("Sample selected vertex height"),
                                )
                                .clicked()
                            {
                                let _ = self.sample_flatten_height(editor);
                            }
                        }
                        SculptMode::Smooth => {
                            ui.weak(
                                "One 3×3 mean from the original heights; clipped at grid edges",
                            );
                        }
                    }
                    ui.checkbox(&mut self.show_sculpt_footprint, "Show sculpt footprint");
                    if let Some(terrain) = self.document.terrain() {
                        match brush::footprint(terrain, self.selected_vertex, self.sculpt_radius) {
                            Ok(vertices) => {
                                ui.label(format!("Stamp affects {} vertices", vertices.len()));
                            }
                            Err(error) => {
                                ui.colored_label(egui::Color32::YELLOW, error);
                            }
                        }
                    }
                    ui.weak("Holes keep their flags; hidden vertices participate");
                    if self.selection_mode == TerrainSelectionMode::Sculpt {
                        ui.weak(
                            "Drag the viewport to sculpt; release commits one Undo. Escape cancels",
                        );
                    }
                    if ui
                        .add_enabled(
                            writable && self.selection_mode == TerrainSelectionMode::Vertex,
                            egui::Button::new("Apply sculpt stamp"),
                        )
                        .clicked()
                    {
                        let _ = self.apply_sculpt_stamp(editor);
                    }
                });
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
