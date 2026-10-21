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
mod fill;
mod rectangle;
mod resize;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolMode {
    Paint,
    Fill,
    Rectangle,
}

/// An uncommitted gesture: selection is fixed at press, not at release.
struct RectangleGesture {
    start: CellCoord,
    end: CellCoord,
    region: Option<u32>,
    source: ValidatedDocument,
    scene_bytes: Vec<u8>,
    frame: u64,
}

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
    mode: ToolMode,
    painting: bool,
    rectangle: Option<RectangleGesture>,
    selected: Option<u32>,
    draft_width: String,
    draft_height: String,
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
            draft_width: prepared.document.document().width.to_string(),
            draft_height: prepared.document.document().height.to_string(),
            history: History::new(prepared.document)?,
            editor_epoch: None,
            tool: false,
            mode: ToolMode::Paint,
            painting: false,
            rectangle: None,
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
        let next = match self.mode {
            ToolMode::Paint => self
                .history
                .current
                .with_cell(cell, region, &asset.document)?,
            ToolMode::Fill => {
                fill::candidate(&self.history.current, cell, region, &asset.document)?.0
            }
            ToolMode::Rectangle => return Err("Rectangle requires a complete gesture".into()),
        };
        Self::budget(&next, Self::actor_count(sprites))?;
        Ok(self.history.commit(next))
    }
    fn cancel_gesture(&mut self) {
        self.painting = false;
        self.rectangle = None;
    }
    fn sync_dimensions(&mut self) {
        self.draft_width = self.history.current.document().width.to_string();
        self.draft_height = self.history.current.document().height.to_string();
        self.cancel_gesture();
    }
    fn undo(&mut self) {
        self.history.undo();
        self.sync_dimensions();
    }
    fn redo(&mut self) {
        self.history.redo();
        self.sync_dimensions();
    }
    /// Explicit apply only: editing a draft never mutates the document.
    fn resize(&mut self, editor: &Editor, sprites: &SpritePanel) -> Result<bool, String> {
        self.cancel_gesture();
        if !self.editable(editor) {
            return Err("tilemap Resize requires the matching editable Arena project".into());
        }
        let dimension = |text: &str| -> Result<u32, String> {
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Width and Height must be whole numbers from 1 to 64".into());
            }
            text.parse()
                .map_err(|_| "Width and Height must be whole numbers from 1 to 64".into())
        };
        let width = dimension(&self.draft_width)?;
        let height = dimension(&self.draft_height)?;
        let (asset, _) = sprites
            .tilemap_asset(&self.key())
            .ok_or("shared atlas missing")?;
        let next = resize::candidate(&self.history.current, width, height, &asset.document)?;
        Self::budget(&next, Self::actor_count(sprites))?;
        let changed = self.history.commit(next);
        self.sync_dimensions();
        Ok(changed)
    }
    #[cfg(all(test, feature = "project-create", target_os = "linux"))]
    pub(crate) fn history_snapshot(
        &self,
    ) -> (
        ValidatedDocument,
        Vec<u8>,
        Vec<ValidatedDocument>,
        Vec<ValidatedDocument>,
    ) {
        (
            self.history.current.clone(),
            self.history.saved.clone(),
            self.history.undo.clone(),
            self.history.redo.clone(),
        )
    }
    #[cfg(all(test, feature = "project-create", target_os = "linux"))]
    pub(crate) fn test_error(&self) -> Option<&str> {
        self.error.as_deref()
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
        if ui
            .checkbox(&mut self.tool, "Tile tool (owns viewport clicks)")
            .changed()
        {
            self.cancel_gesture();
        }
        ui.label(if self.dirty() {
            "Unsaved tiles: Save or Undo before Play, switching or closing."
        } else {
            "Saved tilemap"
        });
        let editable = self.editable(editor);
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                let paint = ui.selectable_value(&mut self.mode, ToolMode::Paint, "Paint");
                let fill = ui.selectable_value(&mut self.mode, ToolMode::Fill, "Fill");
                let rectangle = ui.selectable_value(&mut self.mode, ToolMode::Rectangle, "Rectangle");
                if paint.changed() || fill.changed() || rectangle.changed() {
                    self.cancel_gesture();
                }
            });
            ui.label("Paint: click or drag. Fill: click once to replace connected matching cells. Rectangle: drag, then release to apply once.");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.history.undo.is_empty(),
                        egui::Button::new("Undo tile"),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        !self.history.redo.is_empty(),
                        egui::Button::new("Redo tile"),
                    )
                    .clicked()
                {
                    self.redo();
                }
                if ui.button("Save tilemap").clicked() {
                    self.error = self.save(editor, sprites).err();
                }
                ui.selectable_value(&mut self.selected, None, "Erase");
            });
            ui.horizontal(|ui| {
                let width = ui.label("Width");
                ui.add(egui::TextEdit::singleline(&mut self.draft_width).desired_width(38.0))
                    .labelled_by(width.id);
                let height = ui.label("Height");
                ui.add(egui::TextEdit::singleline(&mut self.draft_height).desired_width(38.0))
                    .labelled_by(height.id);
                if ui.button("Resize").clicked() {
                    self.error = self.resize(editor, sprites).err();
                }
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
            ui.label("Paint, Fill, Rectangle, Resize and Save require matching Edit viewport, without proposal, replay or disconnected host.");
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
            self.cancel_gesture();
            return false;
        }
        if !self.editable(editor)
            || pixels.0 == 0
            || pixels.1 == 0
            || rect.width() <= 0.
            || rect.height() <= 0.
        {
            self.cancel_gesture();
            return true;
        }
        if self.mode == ToolMode::Rectangle {
            self.rectangle_viewport(ui, editor, sprites, rect, pixels);
            return true;
        }
        self.rectangle = None;
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
        // Fill is press-only: holding or dragging must not start another fill,
        // including after an invalid seed or a rejected candidate.
        let apply = down && self.painting && (self.mode == ToolMode::Paint || pressed);
        if let (Some(pos), true) = (pos, apply) {
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
    /// No document mutation occurs until an actual primary release passes every guard.
    fn rectangle_viewport(
        &mut self,
        ui: &egui::Ui,
        editor: &Editor,
        sprites: &SpritePanel,
        rect: egui::Rect,
        pixels: (u32, u32),
    ) {
        self.painting = false;
        let (pos, down, press_pos, release_pos, cancel) = ui.input(|i| {
            let mut press = None;
            let mut release = None;
            let mut ambiguous = false;
            for event in &i.events {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } = event
                {
                    if *pressed {
                        ambiguous |= press.is_some() || release.is_some();
                        press = Some(*pos);
                    } else {
                        ambiguous |= release.is_some();
                        release = Some(*pos);
                    }
                }
            }
            (
                i.pointer.interact_pos(),
                i.pointer.primary_down(),
                press,
                release,
                ambiguous
                    || !i.focused
                    || i.key_pressed(egui::Key::Escape)
                    || i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::WindowFocused(false))),
            )
        });
        let pressed = press_pos.is_some();
        let released = release_pos.is_some();
        if cancel || !ui.is_enabled() {
            self.rectangle = None;
            return;
        }
        let frame = ui.ctx().cumulative_frame_nr();
        if self
            .rectangle
            .as_ref()
            .is_some_and(|g| g.source != self.history.current || frame > g.frame.saturating_add(1))
        {
            self.rectangle = None;
            return;
        }
        // Source changes invalidate a pending gesture, not merely its later Save.
        // Read only while starting/continuing a gesture; never refresh the stamp.
        if pressed || self.rectangle.is_some() {
            if let Err(error) = self.check_stamp() {
                self.rectangle = None;
                self.error = Some(error);
                return;
            }
        }
        // Button coordinates belong to the event, not the final pointer location:
        // a later move in the same frame must not turn an outside edge into an edit.
        let cell_at_pointer = |pos: Option<egui::Pos2>| {
            pos.filter(|p| {
                rect.contains(*p)
                    && ui.clip_rect().contains(*p)
                    && ui.ctx().layer_id_at(*p) == Some(ui.layer_id())
            })
            .and_then(|pos| {
                let screen = [
                    (pos.x - rect.min.x) * pixels.0 as f32 / rect.width(),
                    (pos.y - rect.min.y) * pixels.1 as f32 / rect.height(),
                ];
                self.cell_at(editor.camera.screen_to_world(screen, pixels))
            })
        };
        let cell = cell_at_pointer(pos);
        let start_cell = cell_at_pointer(press_pos);
        let release_cell = cell_at_pointer(release_pos);
        if pressed {
            self.rectangle = None;
            if let Some(start) = start_cell {
                match regular(
                    &self.stamp.scene,
                    orr_sample::project::MAX_SCENE_BYTES as usize,
                ) {
                    Ok(scene_bytes) => {
                        self.rectangle = Some(RectangleGesture {
                            start,
                            end: start,
                            region: self.selected,
                            source: self.history.current.clone(),
                            scene_bytes,
                            frame,
                        });
                    }
                    Err(error) => {
                        self.error = Some(error);
                        return;
                    }
                }
            }
        }
        if let Some(gesture) = &mut self.rectangle {
            gesture.frame = frame;
            if let Some(end) = cell {
                gesture.end = end;
            }
        }
        if released {
            if let (Some(gesture), Some(end)) = (self.rectangle.take(), release_cell) {
                self.error = (|| {
                    if regular(
                        &self.stamp.scene,
                        orr_sample::project::MAX_SCENE_BYTES as usize,
                    )? != gesture.scene_bytes
                    {
                        return Err(
                            "scene source changed during Rectangle; reopen full project".into()
                        );
                    }
                    let (asset, _) = sprites
                        .tilemap_asset(&self.key())
                        .ok_or("shared atlas missing")?;
                    let next = rectangle::candidate(
                        &self.history.current,
                        gesture.start,
                        end,
                        gesture.region,
                        &asset.document,
                    )?;
                    Self::budget(&next, Self::actor_count(sprites))?;
                    self.history.commit(next);
                    Ok::<_, String>(())
                })()
                .err();
            }
        } else if !down {
            // Lost capture/focus is cancellation, never an implicit release.
            self.rectangle = None;
        }
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
        if let Some(gesture) = &self.rectangle {
            let left = gesture.start.column.min(gesture.end.column);
            let right = gesture.start.column.max(gesture.end.column) + 1;
            let top = gesture.start.row.min(gesture.end.row);
            let bottom = gesture.start.row.max(gesture.end.row) + 1;
            let preview = egui::Rect::from_two_pos(
                to_pos([
                    d.origin[0] + left as f32 * size[0],
                    d.origin[1] - top as f32 * size[1],
                ]),
                to_pos([
                    d.origin[0] + right as f32 * size[0],
                    d.origin[1] - bottom as f32 * size[1],
                ]),
            );
            let color = if gesture.region.is_some() {
                egui::Color32::from_rgba_unmultiplied(70, 190, 230, 55)
            } else {
                egui::Color32::from_rgba_unmultiplied(230, 100, 70, 55)
            };
            painter.rect_filled(preview, 0.0, color);
            painter.rect_stroke(
                preview,
                0.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
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

#[cfg(test)]
mod fill_history_tests {
    use super::*;

    #[cfg(all(feature = "project-create", target_os = "linux"))]
    #[test]
    fn installed_fill_dispatch_is_atomic_for_budget_error_and_noop() {
        use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fill-panel");
        orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
            output: root.clone(),
            template: "arena-tilemap-2d-v1".into(),
            seed: "fill-panel".into(),
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
        panel.mode = ToolMode::Fill;
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
        let before = snapshot(&panel);
        assert!(panel
            .change(seed, Some(10), &sprites)
            .unwrap_err()
            .contains("4096"));
        assert_eq!(snapshot(&panel), before);
    }

    #[test]
    fn grouped_fill_undo_redo_noop_and_budget_failure_preserve_history() {
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
        let (filled, changed) = fill::candidate(&history.current, seed, Some(0), &atlas).unwrap();
        assert_eq!(changed, 6);
        TilemapPanel::budget(&filled, 4090).unwrap();
        assert!(history.commit(filled.clone()));
        assert_eq!(history.undo.len(), 1);
        assert!(history.dirty());
        history.undo();
        assert_eq!(history.current, document);
        assert!(!history.dirty());
        let saved = history.saved.clone();
        let (same, changed) = fill::candidate(&history.current, seed, None, &atlas).unwrap();
        assert_eq!(changed, 0);
        assert!(!history.commit(same));
        assert_eq!(history.redo, vec![filled.clone()]);
        assert_eq!(history.saved, saved);

        // Candidate and budget failures precede commit, preserving every marker.
        let undo = history.undo.clone();
        let redo = history.redo.clone();
        assert!(fill::candidate(&history.current, seed, Some(99), &atlas).is_err());
        let (over_budget, _) = fill::candidate(&history.current, seed, Some(0), &atlas).unwrap();
        assert!(TilemapPanel::budget(&over_budget, 4091).is_err());
        assert!(TilemapPanel::budget(&over_budget, usize::MAX).is_err());
        assert_eq!(history.current, document);
        assert_eq!(history.undo, undo);
        assert_eq!(history.redo, redo);
        assert_eq!(history.saved, saved);
        assert!(!history.dirty());
        history.redo();
        assert_eq!(history.current, filled);
        assert_eq!(history.undo.len(), 1);
        let (erased, changed) = fill::candidate(&history.current, seed, None, &atlas).unwrap();
        assert_eq!(changed, 6);
        assert!(history.commit(erased));
        assert!(!history.dirty());
    }
}

#[cfg(all(test, feature = "project-create", target_os = "linux"))]
mod resize_history_tests {
    use super::*;
    use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};

    fn fixture() -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("resize-panel");
        orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
            output: root.clone(),
            template: "arena-tilemap-2d-v1".into(),
            seed: "resize-panel".into(),
        })
        .unwrap();
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
        panel.install_editor_identity(&editor).unwrap();
        assert!(panel.editable(&editor));
        (temp, panel, sprites, editor)
    }
    fn draft(panel: &mut TilemapPanel, width: &str, height: &str) {
        panel.draft_width = width.into();
        panel.draft_height = height.into();
    }
    #[test]
    fn installed_resize_dispatch_grouped_history_noop_and_crop_are_atomic() {
        let (_temp, mut panel, sprites, editor) = fixture();
        let initial = panel.history_snapshot();
        let file = std::fs::read(&panel.path).unwrap();
        draft(&mut panel, "17", "13");
        // Drafting is not applying, and a resize ends a pending paint stroke.
        assert_eq!(panel.history_snapshot(), initial);
        panel.painting = true;
        assert!(panel.resize(&editor, &sprites).unwrap());
        assert!(!panel.painting);
        assert_eq!(panel.history.undo.len(), 1);
        assert_eq!(panel.document().document().cells, vec![None; 221]);
        let grown = panel.document().clone();
        panel.undo();
        assert_eq!(panel.document(), &initial.0);
        assert_eq!((&*panel.draft_width, &*panel.draft_height), ("16", "12"));
        assert!(!panel.dirty());
        let undone = panel.history_snapshot();
        assert!(!panel.resize(&editor, &sprites).unwrap());
        assert_eq!(panel.history_snapshot(), undone);
        panel.redo();
        assert_eq!(panel.document(), &grown);
        assert_eq!((&*panel.draft_width, &*panel.draft_height), ("17", "13"));
        let outside = CellCoord {
            column: 16,
            row: 12,
        };
        assert!(panel.change(outside, Some(10), &sprites).unwrap());
        let occupied = panel.history_snapshot();
        draft(&mut panel, "16", "12");
        assert!(panel.resize(&editor, &sprites).is_err());
        assert_eq!(panel.history_snapshot(), occupied);
        assert!(panel.change(outside, None, &sprites).unwrap());
        assert!(panel.resize(&editor, &sprites).unwrap());
        assert_eq!(panel.document(), &initial.0);
        assert_eq!(panel.history.saved, initial.1);
        assert!(!panel.dirty());
        assert_eq!(std::fs::read(&panel.path).unwrap(), file);
    }
    #[test]
    fn installed_resize_rejects_invalid_guard_atlas_and_budget_without_commit() {
        let (_temp, mut panel, sprites, editor) = fixture();
        let initial = panel.history_snapshot();
        for (width, height) in [
            ("", "12"),
            ("1.5", "12"),
            ("-1", "12"),
            ("0", "12"),
            ("65", "12"),
            ("16", "0"),
            ("16", "65"),
            ("4294967296", "12"),
        ] {
            draft(&mut panel, width, height);
            assert!(panel.resize(&editor, &sprites).is_err());
            assert_eq!(panel.history_snapshot(), initial);
        }
        draft(&mut panel, "17", "13");
        let epoch = panel.editor_epoch.take();
        assert!(panel
            .resize(&editor, &sprites)
            .unwrap_err()
            .contains("editable"));
        assert_eq!(panel.history_snapshot(), initial);
        panel.editor_epoch = epoch;
        assert!(panel
            .resize(&editor, &SpritePanel::default())
            .unwrap_err()
            .contains("atlas"));
        assert_eq!(panel.history_snapshot(), initial);

        // Resize retains occupied count, but must still enforce the combined
        // budget even for same-size candidates and an unexpectedly full input.
        assert!(TilemapPanel::actor_count(&sprites) > 0);
        let (asset, _) = sprites.tilemap_asset(&panel.key()).unwrap();
        let mut large = panel.document().document().clone();
        large.width = 64;
        large.height = 64;
        large.cells = vec![Some(10); 4096];
        panel.history = History::new(large.validate(&asset.document).unwrap()).unwrap();
        draft(&mut panel, "64", "64");
        let full = panel.history_snapshot();
        assert!(panel
            .resize(&editor, &sprites)
            .unwrap_err()
            .contains("4096"));
        assert_eq!(panel.history_snapshot(), full);
    }
}
