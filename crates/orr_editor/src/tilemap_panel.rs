//! Optional single-layer authoring. CPU history is independent of scene undo.
//! Saves are per-file atomic under a trusted, quiescent filesystem assumption.
use crate::{
    editor::Editor, game::EditorGame, model::Mode, sprite_panel::SpritePanel, viewport::GpuViewport,
};
use orr_render::{orr_rhi::Wgpu, Camera, RenderList};
use orr_sample::{
    project_compositor::ProjectCompositor, project_sprites::AssetKey,
    project_tilemap::PreparedTilemap,
};
use orr_tilemap::{CellCoord, ValidatedDocument};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
const HISTORY: usize = 128;
const STAMP_LIMIT: usize = 2 * 1024 * 1024;

/// File bytes are captured before replacing the active project. They are never
/// refreshed implicitly on a failed save, so external edits cannot be adopted.
struct Stamp {
    root: PathBuf,
    root_identity: RootIdentity,
    scene: PathBuf,
    manifest: Vec<u8>,
    lock: Vec<u8>,
    sidecar: Vec<u8>,
    actors: Option<(PathBuf, Vec<u8>)>,
}
#[derive(PartialEq, Eq)]
struct RootIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: std::time::SystemTime,
}
fn root_identity(path: &Path) -> Result<RootIdentity, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("project root must remain a regular directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(RootIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(RootIdentity {
            created: metadata
                .created()
                .map_err(|e| format!("root identity unavailable: {e}"))?,
        })
    }
}
struct History {
    current: ValidatedDocument,
    saved: Vec<u8>,
    undo: Vec<ValidatedDocument>,
    redo: Vec<ValidatedDocument>,
}
impl History {
    fn new(current: ValidatedDocument) -> Result<Self, String> {
        let saved = encode(&current)?;
        Ok(Self {
            current,
            saved,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    fn dirty(&self) -> bool {
        encode(&self.current).map_or(true, |bytes| bytes != self.saved)
    }
    fn commit(&mut self, next: ValidatedDocument) -> bool {
        if next == self.current {
            return false;
        }
        if self.undo.len() == HISTORY {
            self.undo.remove(0);
        }
        self.undo.push(std::mem::replace(&mut self.current, next));
        self.redo.clear();
        true
    }
    fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.current, previous));
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.current, next));
        }
    }
}
fn encode(document: &ValidatedDocument) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(document.document()).map_err(|e| e.to_string())?;
    if bytes.len() > orr_tilemap::MAX_BYTES {
        return Err("tilemap exceeds 64 KiB".into());
    }
    Ok(bytes)
}
fn regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("tilemap save input must be a bounded regular file, not a symlink".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("tilemap save input exceeds byte limit".into());
    }
    Ok(bytes)
}

pub struct TilemapPanel {
    prepared: PreparedTilemap,
    path: PathBuf,
    stamp: Stamp,
    history: History,
    editor_epoch: Option<u64>,
    tool: bool,
    painting: bool,
    selected: Option<u32>,
    error: Option<String>,
    compositor: Option<ProjectCompositor<Wgpu>>,
}
impl TilemapPanel {
    pub fn prepare(root: &Path, scene: &Path, prepared: PreparedTilemap) -> Result<Self, String> {
        let root = crate::sprite_bindings::resolve_project(root, ".")?;
        let scene = std::fs::canonicalize(scene).map_err(|e| e.to_string())?;
        if prepared.path.parent() != Some(root.as_path()) || !scene.starts_with(&root) {
            return Err("tilemap and scene must belong to the admitted project root".into());
        }
        let sidecar = regular(&prepared.path, orr_tilemap::MAX_BYTES)?;
        if sidecar != prepared.original_bytes()
            || orr_tilemap::Document::from_bytes(&sidecar)? != *prepared.document.document()
        {
            return Err("tilemap changed after admission".into());
        }
        let manifest = regular(&root.join("orr.project.json"), STAMP_LIMIT)?;
        let lock = regular(&root.join("orr.packages.lock.json"), STAMP_LIMIT)?;
        let value: serde_json::Value =
            serde_json::from_slice(&manifest).map_err(|e| e.to_string())?;
        let entry = value.get("entry").ok_or("missing project entry")?;
        if entry
            .get("tilemap")
            .and_then(|v| v.as_str())
            .map(|p| root.join(p))
            != Some(prepared.path.clone())
            || entry
                .get("scene")
                .and_then(|v| v.as_str())
                .map(|p| root.join(p))
                != Some(scene.clone())
        {
            return Err("tilemap/scene project association changed".into());
        }
        let actors = entry
            .get("sprites")
            .and_then(|v| v.as_str())
            .map(|relative| {
                if !orr_sample::project_sprites::relative(relative) {
                    return Err("invalid actor sidecar path".into());
                }
                let path = root.join(relative);
                let parent = path.parent().ok_or("actor sidecar parent missing")?;
                let canonical = crate::sprite_bindings::resolve_project(parent, ".")?;
                if !canonical.starts_with(&root) {
                    return Err("actor sidecar escaped project".into());
                }
                let bytes = regular(&path, orr_sample::project_sprites::MAX_BYTES as usize)?;
                Ok::<_, String>((path, bytes))
            })
            .transpose()?;
        Ok(Self {
            prepared: prepared.clone(),
            path: prepared.path,
            stamp: Stamp {
                root_identity: root_identity(&root)?,
                root,
                scene,
                manifest,
                lock,
                sidecar,
                actors,
            },
            history: History::new(prepared.document)?,
            editor_epoch: None,
            tool: false,
            painting: false,
            selected: None,
            error: None,
            compositor: None,
        })
    }
    pub fn install_renderer(
        &mut self,
        state: Option<&egui_wgpu::RenderState>,
        sprites: &SpritePanel,
    ) -> Result<(), String> {
        match (&self.stamp.actors, &sprites.bindings) {
            (Some((path, bytes)), Some(bindings))
                if path == &bindings.path
                    && orr_sample::project_sprites::Document::from_bytes(bytes)?
                        == *bindings.document() => {}
            (None, None) => {}
            _ => return Err("actor sprite sidecar changed after admission".into()),
        }
        if sprites.tilemap_asset(&self.key()).is_none() {
            return Err("tilemap installed atlas is missing from shared cache".into());
        }
        if let Some(state) = state {
            let rhi = Wgpu::from_parts(
                state.instance.clone(),
                state.adapter.clone(),
                state.device.clone(),
                state.queue.clone(),
            );
            self.compositor = Some(ProjectCompositor::new(
                rhi,
                crate::viewport::TARGET_FORMAT,
                &sprites.tilemap_assets(),
            )?);
        }
        Ok(())
    }
    fn key(&self) -> AssetKey {
        let s = &self.history.current.document().sprite;
        (s.package.clone(), s.document.clone())
    }
    pub fn document(&self) -> &ValidatedDocument {
        &self.history.current
    }
    pub fn dirty(&self) -> bool {
        self.history.dirty()
    }
    pub fn active_tool(&self) -> bool {
        self.tool
    }
    pub(crate) fn install_editor_identity(&mut self, editor: &Editor) -> Result<(), String> {
        if self.editor_epoch.is_some() {
            return Err("tilemap lifecycle cannot be rebound; reopen full project".into());
        }
        if editor.game() != EditorGame::Arena
            || editor
                .sim()
                .scene_path
                .as_ref()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .as_ref()
                != Some(&self.stamp.scene)
        {
            return Err("tilemap requires its admitted Arena scene".into());
        }
        self.editor_epoch = Some(editor.scene_load_epoch());
        Ok(())
    }
    fn epoch_matches(&self, epoch: u64) -> bool {
        self.editor_epoch == Some(epoch)
    }
    pub(crate) fn scene_matches(&self, editor: &Editor) -> bool {
        self.epoch_matches(editor.scene_load_epoch())
            && editor.game() == EditorGame::Arena
            && editor.down().is_none()
            && editor
                .sim()
                .scene_path
                .as_ref()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .as_ref()
                == Some(&self.stamp.scene)
            && root_identity(&self.stamp.root).ok().as_ref() == Some(&self.stamp.root_identity)
            && crate::sprite_bindings::resolve_project(&self.stamp.root, ".")
                .ok()
                .as_ref()
                == Some(&self.stamp.root)
    }
    fn editable(&self, editor: &Editor) -> bool {
        self.scene_matches(editor)
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.mode() == Mode::Edit
            && editor.yard_rows_coherent()
            && editor.snapshot().is_some_and(|s| s.timeline().is_none())
    }
    fn actor_count(sprites: &SpritePanel) -> usize {
        sprites
            .bindings
            .as_ref()
            .map_or(0, |b| b.document().bindings.len())
    }
    fn budget(document: &ValidatedDocument, actors: usize) -> Result<(), String> {
        if actors
            .checked_add(document.occupied_count())
            .is_none_or(|n| n > 4096)
        {
            return Err("tiles plus actor bindings exceed 4096".into());
        }
        Ok(())
    }
    fn change(
        &mut self,
        cell: CellCoord,
        region: Option<u32>,
        sprites: &SpritePanel,
    ) -> Result<bool, String> {
        let (asset, _) = sprites
            .tilemap_asset(&self.key())
            .ok_or("shared atlas missing")?;
        let next = self
            .history
            .current
            .with_cell(cell, region, &asset.document)?;
        Self::budget(&next, Self::actor_count(sprites))?;
        Ok(self.history.commit(next))
    }
    fn check_stamp(&self) -> Result<(), String> {
        if root_identity(&self.stamp.root)? != self.stamp.root_identity {
            return Err("project root was replaced; reopen the project".into());
        }
        if crate::sprite_bindings::resolve_project(&self.stamp.root, ".")? != self.stamp.root
            || std::fs::canonicalize(&self.stamp.scene).map_err(|e| e.to_string())?
                != self.stamp.scene
        {
            return Err("project/scene identity changed".into());
        }
        regular(
            &self.stamp.scene,
            orr_sample::project::MAX_SCENE_BYTES as usize,
        )?;
        if let Some((path, bytes)) = &self.stamp.actors {
            let parent = crate::sprite_bindings::resolve_project(
                path.parent().ok_or("missing actor sidecar parent")?,
                ".",
            )?;
            if !parent.starts_with(&self.stamp.root)
                || regular(path, orr_sample::project_sprites::MAX_BYTES as usize)? != *bytes
            {
                return Err("actor sprite sidecar changed externally; reopen full project".into());
            }
        }
        if regular(&self.stamp.root.join("orr.project.json"), STAMP_LIMIT)? != self.stamp.manifest
            || regular(&self.stamp.root.join("orr.packages.lock.json"), STAMP_LIMIT)?
                != self.stamp.lock
            || regular(&self.path, orr_tilemap::MAX_BYTES)? != self.stamp.sidecar
        {
            return Err("project manifest, lock or tilemap changed externally; reopen project without overwriting".into());
        }
        Ok(())
    }
    fn save_document(&mut self) -> Result<(), String> {
        let bytes = encode(&self.history.current)?;
        self.check_stamp()?;
        // Hash actual installed bytes against the pinned authoritative lock;
        // neither a decoded cache nor an unchanged lock proves intact assets.
        orr_package::Project::open(&self.stamp.root, crate::sprite_bindings::compiled_runtime())
            .and_then(|project| project.verify())
            .map_err(|e| format!("installed tilemap assets: {e}"))?;
        self.check_stamp()?;
        if !self.dirty() {
            return Ok(());
        }
        let mut temp =
            tempfile::NamedTempFile::new_in(&self.stamp.root).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        self.check_stamp()?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        self.stamp.sidecar = bytes.clone();
        self.history.saved = bytes;
        Ok(())
    }
    pub fn save(&mut self, editor: &Editor, sprites: &SpritePanel) -> Result<(), String> {
        if !self.editable(editor) {
            return Err("tilemap Save requires the matching editable Arena project".into());
        }
        let (asset, _) = sprites
            .tilemap_asset(&self.key())
            .ok_or("shared atlas missing")?;
        self.history.current.document().validate(&asset.document)?;
        Self::budget(&self.history.current, Self::actor_count(sprites))?;
        self.save_document()
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &Editor, sprites: &SpritePanel) {
        ui.heading("Tile background");
        ui.label("Replaces Arena floor presentation only. Empty / transparent cells show clear background, not collision or navmesh.");
        ui.label("Scene Save / Save As does not save or retarget this tilemap. Package mutation and export are unsupported in v1.");
        ui.checkbox(&mut self.tool, "Tile tool (owns viewport clicks)");
        ui.label(if self.dirty() {
            "Unsaved tiles: Save or Undo before Play, switching or closing."
        } else {
            "Saved tilemap"
        });
        let editable = self.editable(editor);
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.history.undo.is_empty(),
                        egui::Button::new("Undo tile"),
                    )
                    .clicked()
                {
                    self.history.undo();
                }
                if ui
                    .add_enabled(
                        !self.history.redo.is_empty(),
                        egui::Button::new("Redo tile"),
                    )
                    .clicked()
                {
                    self.history.redo();
                }
                if ui.button("Save tilemap").clicked() {
                    self.error = self.save(editor, sprites).err();
                }
                ui.selectable_value(&mut self.selected, None, "Erase");
            });
            if let Some((asset, texture)) = sprites.tilemap_asset(&self.key()) {
                let size = self.history.current.document().tile_size;
                ui.horizontal_wrapped(|ui| {
                    for region in asset
                        .document
                        .regions()
                        .iter()
                        .filter(|r| [r.width, r.height] == size)
                    {
                        let uv = asset.document.uv_rect(region.id).expect("validated region");
                        let image = egui::Image::new((texture, egui::vec2(32., 32.))).uv(
                            egui::Rect::from_min_max(
                                egui::pos2(uv[0], uv[1]),
                                egui::pos2(uv[2], uv[3]),
                            ),
                        );
                        if ui
                            .add(
                                egui::Button::image_and_text(
                                    image,
                                    format!("Static region {}", region.id),
                                )
                                .selected(self.selected == Some(region.id)),
                            )
                            .on_hover_text(format!("Static region {}", region.id))
                            .clicked()
                        {
                            self.selected = Some(region.id);
                        }
                    }
                });
            }
        });
        if !editable {
            ui.label("Paint and Save require matching Edit viewport, without proposal, replay or disconnected host.");
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
    /// True reserves the pointer even when editing is temporarily disallowed.
    pub fn paint_viewport(
        &mut self,
        ui: &egui::Ui,
        editor: &Editor,
        sprites: &SpritePanel,
        rect: egui::Rect,
        pixels: (u32, u32),
    ) -> bool {
        if !self.tool {
            self.painting = false;
            return false;
        }
        if !self.editable(editor)
            || pixels.0 == 0
            || pixels.1 == 0
            || rect.width() <= 0.
            || rect.height() <= 0.
        {
            self.painting = false;
            return true;
        }
        let (pos, pressed, down) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
            )
        });
        if !down {
            self.painting = false;
        }
        if pressed {
            self.painting = pos.is_some_and(|p| rect.contains(p)) && ui.rect_contains_pointer(rect);
        }
        if let (Some(pos), true) = (pos, down && self.painting) {
            if rect.contains(pos) && ui.rect_contains_pointer(rect) {
                let screen = [
                    (pos.x - rect.min.x) * pixels.0 as f32 / rect.width(),
                    (pos.y - rect.min.y) * pixels.1 as f32 / rect.height(),
                ];
                let world = editor.camera.screen_to_world(screen, pixels);
                if let Some(cell) = self.cell_at(world) {
                    self.error = self.change(cell, self.selected, sprites).err();
                }
            }
        }
        true
    }
    /// Draw after the viewport image, so the authoring grid stays visible.
    pub fn paint_grid(&self, ui: &egui::Ui, editor: &Editor, rect: egui::Rect, pixels: (u32, u32)) {
        if !self.tool || !self.editable(editor) || pixels.0 == 0 || pixels.1 == 0 {
            return;
        }
        let d = self.history.current.document();
        let size = d.tile_size.map(|v| v as f32 * d.units_per_pixel);
        let to_pos = |world| {
            let p = editor.camera.world_to_screen(world, pixels);
            rect.min
                + egui::vec2(
                    p[0] * rect.width() / pixels.0 as f32,
                    p[1] * rect.height() / pixels.1 as f32,
                )
        };
        let painter = ui.painter().with_clip_rect(rect);
        let stroke = egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(90, 180, 200, 100),
        );
        for column in 0..=d.width {
            let x = d.origin[0] + column as f32 * size[0];
            painter.line_segment(
                [
                    to_pos([x, d.origin[1]]),
                    to_pos([x, d.origin[1] - d.height as f32 * size[1]]),
                ],
                stroke,
            );
        }
        for row in 0..=d.height {
            let y = d.origin[1] - row as f32 * size[1];
            painter.line_segment(
                [
                    to_pos([d.origin[0], y]),
                    to_pos([d.origin[0] + d.width as f32 * size[0], y]),
                ],
                stroke,
            );
        }
    }
    pub fn cell_at(&self, world: [f32; 2]) -> Option<CellCoord> {
        let d = self.history.current.document();
        let x = ((world[0] - d.origin[0]) / (d.tile_size[0] as f32 * d.units_per_pixel)).floor();
        let y = ((d.origin[1] - world[1]) / (d.tile_size[1] as f32 * d.units_per_pixel)).floor();
        (x.is_finite()
            && y.is_finite()
            && x >= 0.
            && y >= 0.
            && x < d.width as f32
            && y < d.height as f32)
            .then_some(CellCoord {
                column: x as u32,
                row: y as u32,
            })
    }
    pub fn render(
        &mut self,
        viewport: &mut GpuViewport,
        size: (u32, u32),
        list: &RenderList,
        camera: &Camera,
        actor_count: usize,
        editor: &Editor,
    ) -> Result<egui::TextureId, String> {
        if !self.scene_matches(editor) {
            return Err("stale tilemap lifecycle; reopen complete project".into());
        }
        Self::budget(&self.history.current, actor_count)?;
        let prepared = self.prepared.with_document(self.history.current.clone());
        let draws = orr_sample::project_tilemap::draws(&prepared, actor_count)?;
        viewport.render_composed(
            size,
            list,
            camera,
            self.compositor
                .as_mut()
                .ok_or("tilemap GPU renderer unavailable")?,
            &draws,
        )
    }
}
#[cfg(test)]
mod tests;
