//! Private presentation history. Neither terrain bytes nor host Frames are edited.
use crate::{
    editor::Editor, model::Mode, terrain_document::TerrainSource, terrain_panel::TerrainPanel,
};
use orr_model::StaticModel;
use orr_terrain::Terrain;
use orr_terrain_view::surface::{self, Surface};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
const MAX_BYTES: usize = 8 * 1024 * 1024;
const HISTORY_BYTES: usize = 16 * 1024 * 1024;
const HISTORY_ENTRIES: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    scene: PathBuf,
    epoch: u64,
    relative: String,
    asset: String,
}
#[derive(Clone)]
struct Context {
    binding: Binding,
    terrain: Terrain,
    marker: Option<orr_terrain_view::QueryMarker>,
}
impl Context {
    fn get(editor: &Editor, panel: &TerrainPanel) -> Result<Self, String> {
        if !editor.spec().is_local()
            || !editor.yard_rows_coherent()
            || editor.previewing().is_some()
        {
            return Err(
                "Appearance requires a coherent local terrain scene without a proposal preview"
                    .into(),
            );
        }
        let scene = editor
            .path()
            .ok_or("Save the scene before authoring appearance")?;
        let (relative, terrain) =
            if editor.mode() == Mode::Edit && same_scene(panel.document.scene(), &scene) {
                match (panel.document.source(), panel.document.terrain()) {
                    (Some(TerrainSource::Local { relative_path }), Some(terrain)) => {
                        (relative_path.clone(), terrain.clone())
                    }
                    _ => Self::host(editor)?,
                }
            } else {
                Self::host(editor)?
            };
        let marker = if editor.mode() == Mode::Edit && same_scene(panel.document.scene(), &scene) {
            panel.document.marker()
        } else {
            None
        };
        Ok(Self {
            binding: Binding {
                scene,
                epoch: editor.scene_load_epoch(),
                relative,
                asset: terrain.asset_id().into(),
            },
            terrain,
            marker,
        })
    }
    fn host(editor: &Editor) -> Result<(String, Terrain), String> {
        if !editor.game().is_navigation() {
            return Err("Open local terrain before authoring appearance".into());
        }
        let frame = editor
            .snapshot()
            .ok_or("Wait for the navigation snapshot")?
            .predicted();
        let view = orr_remote::navigation_yard3d::navigation_from_view(frame)?;
        Ok((editor.admitted_navigation().source.clone(), view.terrain))
    }
    fn identity(editor: &Editor, panel: &TerrainPanel) -> Result<(Binding, [u8; 32]), String> {
        if !editor.spec().is_local()
            || !editor.yard_rows_coherent()
            || editor.previewing().is_some()
        {
            return Err("Wait for a coherent local terrain scene".into());
        }
        let scene = editor.path().ok_or("No scene")?;
        let (relative, asset, revision) =
            if editor.mode() == Mode::Edit && same_scene(panel.document.scene(), &scene) {
                match (panel.document.source(), panel.document.terrain()) {
                    (Some(TerrainSource::Local { relative_path }), Some(terrain)) => (
                        relative_path.clone(),
                        terrain.asset_id().to_owned(),
                        panel.document.revision().ok_or("No terrain revision")?,
                    ),
                    _ => Self::host_identity(editor)?,
                }
            } else {
                Self::host_identity(editor)?
            };
        Ok((
            Binding {
                scene,
                epoch: editor.scene_load_epoch(),
                relative,
                asset,
            },
            revision,
        ))
    }
    fn host_identity(editor: &Editor) -> Result<(String, String, [u8; 32]), String> {
        if !editor.game().is_navigation() {
            return Err("No local terrain".into());
        }
        let view = editor.admitted_navigation();
        if !view.admitted {
            return Err("Navigation terrain not admitted".into());
        }
        Ok((
            view.source.clone(),
            view.identity.clone(),
            view.revision.ok_or("No terrain revision")?,
        ))
    }
    fn terrain_path(&self) -> Result<PathBuf, String> {
        joined(&self.binding.scene, &self.binding.relative)
    }
    fn sidecar_path(&self) -> Result<PathBuf, String> {
        joined(
            &self.binding.scene,
            &surface::sidecar_relative(&self.binding.relative).map_err(|e| e.to_string())?,
        )
    }
}
struct Pending {
    context: Context,
    backing: Option<Vec<u8>>,
    scene_bytes: Vec<u8>,
    host_epoch: u64,
    png_path: PathBuf,
    png: Vec<u8>,
    surface: Arc<Surface>,
}
#[derive(Default)]
pub(crate) struct State {
    binding: Option<Binding>,
    current: Option<Arc<Surface>>,
    saved: Option<Vec<u8>>,
    saved_digest: Option<[u8; 32]>,
    undo: Vec<Option<Arc<Surface>>>,
    redo: Vec<Option<Arc<Surface>>>,
    pending: Option<Pending>,
    cache_marker: Option<orr_terrain_view::QueryMarker>,
    cache: Option<([u8; 32], [u8; 32], Arc<StaticModel>)>,
    pub(crate) png_path: String,
    pub(crate) status: String,
    load_error: Option<String>,
    uncertain: bool,
    pub(crate) operation_blocked: bool,
}
impl State {
    pub(crate) fn dirty(&self) -> bool {
        self.current.as_ref().map(|s| s.digest()) != self.saved_digest
    }
    pub(crate) fn busy(&self) -> bool {
        self.dirty() || self.pending.is_some() || self.uncertain
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        feature = "navigation-export"
    ))]
    pub(crate) fn v1_export_guard(
        &self,
        editor: &Editor,
        panel: &TerrainPanel,
    ) -> Result<(), String> {
        let context = Context::get(editor, panel)?;
        if self.busy()
            || self.has_surface()
            || read_optional(&context.sidecar_path()?, MAX_BYTES)?.is_some()
        {
            return Err("Terrain appearance cannot be exported by the v1 exporter. Save appearance and use navigation_surface_export instead.".into());
        }
        Ok(())
    }
    pub(crate) fn playback_blocker(&self, editor: &Editor, panel: &TerrainPanel) -> Option<String> {
        if let Some(error) = &self.load_error {
            return Some(format!(
                "Repair the terrain appearance sidecar and Reload before Play: {error}"
            ));
        }
        if self.has_surface() || self.busy() {
            match Context::identity(editor, panel) {
                Ok((binding, _)) if self.binding.as_ref() == Some(&binding) => {},
                _ => return Some("Terrain appearance belongs to another scene or terrain; save or explicitly discard its work before Play".into()),
            }
        }
        None
    }
    pub(crate) fn has_surface(&self) -> bool {
        self.current.is_some() || self.load_error.is_some()
    }
    pub(crate) fn preview_surface(&self) -> Option<&Surface> {
        self.pending.as_ref().map(|p| p.surface.as_ref())
    }
    pub(crate) fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub(crate) fn editable(editor: &Editor) -> Result<(), String> {
        if editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.sim().in_tx
            || editor.history().in_tx
            || editor.previewing().is_some()
        {
            Err("Stop Play before editing terrain appearance".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn sync(&mut self, editor: &Editor, panel: &TerrainPanel) {
        let Ok((binding, _)) = Context::identity(editor, panel) else {
            return;
        };
        if self.binding.as_ref() == Some(&binding) {
            return;
        }
        if self.busy() {
            self.status = "Appearance belongs to the previous terrain or scene load. Work is retained; return to its owner or explicitly discard appearance work.".into();
            return;
        }
        if let Ok(context) = Context::get(editor, panel) {
            self.install_loaded(&context);
        }
    }
    fn install_loaded(&mut self, context: &Context) {
        let result = (|| {
            let bytes = read_optional(&context.sidecar_path()?, MAX_BYTES)?;
            let surface = bytes
                .as_ref()
                .map(|bytes| Surface::from_bytes(bytes).map_err(|e| e.to_string()))
                .transpose()?;
            if let Some(surface) = &surface {
                surface
                    .validate_terrain(&context.binding.relative, &context.terrain)
                    .map_err(|e| e.to_string())?;
                model(context, surface)?;
            }
            Ok::<_, String>((bytes, surface.map(Arc::new)))
        })();
        self.binding = Some(context.binding.clone());
        self.current = None;
        self.saved = None;
        self.saved_digest = None;
        self.undo.clear();
        self.redo.clear();
        self.cache = None;
        match result {
            Ok((bytes, surface)) => {
                self.saved = bytes;
                self.saved_digest = surface.as_ref().map(|s| s.digest());
                self.current = surface;
                self.load_error = None;
                self.status = "Appearance loaded (missing sidecar uses checker).".into();
            }
            Err(error) => {
                self.status = format!("Appearance refused: {error}");
                self.load_error = Some(error);
            }
        }
    }
    pub(crate) fn reload(&mut self, editor: &Editor, panel: &TerrainPanel) -> Result<(), String> {
        Self::editable(editor)?;
        if self.busy() {
            return Err("Save or explicitly discard appearance work before Reload".into());
        }
        let context = Context::get(editor, panel)?;
        self.install_loaded(&context);
        self.load_error.clone().map_or(Ok(()), Err)
    }
    fn context(&self, editor: &Editor, panel: &TerrainPanel) -> Result<Context, String> {
        Self::editable(editor)?;
        #[cfg(feature = "terrain-raw16")]
        if panel.raw16.has_preview() {
            return Err("Finish or cancel the RAW16 preview before appearance edits".into());
        }
        #[cfg(feature = "terrain-png16")]
        if panel.png16.has_preview() {
            return Err("Finish or cancel the PNG16 preview before appearance edits".into());
        }
        if self.operation_blocked || panel.stamp.has_preview() {
            return Err(
                "Finish or cancel the active terrain gesture before appearance edits".into(),
            );
        }
        let context = Context::get(editor, panel)?;
        if self.binding.as_ref() != Some(&context.binding) {
            return Err(
                "Appearance ownership changed; Reload or return to the original scene".into(),
            );
        }
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Repair the malformed or mismatched sidecar and Reload: {error}"
            ));
        }
        Ok(context)
    }
    pub(crate) fn preview(&mut self, editor: &Editor, panel: &TerrainPanel) -> Result<(), String> {
        self.pending = None;
        let context = self.context(editor, panel)?;
        if read_optional(&context.sidecar_path()?, MAX_BYTES)? != self.saved {
            return Err("Sidecar changed on disk; Reload before Preview".into());
        }
        let png_path = PathBuf::from(self.png_path.trim());
        let png =
            read_optional(&png_path, surface::MAX_PNG_BYTES)?.ok_or("PNG source is missing")?;
        let surface = Arc::new(
            Surface::from_png(&context.binding.relative, &context.binding.asset, &png)
                .map_err(|e| e.to_string())?,
        );
        model(&context, &surface)?;
        let backing = read_optional(&context.terrain_path()?, orr_terrain::MAX_FILE_BYTES)?;
        let scene_bytes =
            read_optional(&context.binding.scene, MAX_BYTES)?.ok_or("Scene is missing")?;
        self.pending = Some(Pending {
            context,
            backing,
            scene_bytes,
            host_epoch: editor.sim().epoch,
            png_path,
            png,
            surface,
        });
        Ok(())
    }
    pub(crate) fn apply(&mut self, editor: &Editor, panel: &TerrainPanel) -> Result<(), String> {
        let context = self.context(editor, panel)?;
        let pending = self.pending.as_ref().ok_or("Preview a PNG first")?;
        if editor.sim().epoch != pending.host_epoch
            || read_optional(&context.binding.scene, MAX_BYTES)?.as_ref()
                != Some(&pending.scene_bytes)
            || context.binding != pending.context.binding
            || context.terrain.cook() != pending.context.terrain.cook()
            || Path::new(self.png_path.trim()) != pending.png_path.as_path()
            || read_optional(&pending.png_path, surface::MAX_PNG_BYTES)?.as_ref()
                != Some(&pending.png)
            || read_optional(&context.terrain_path()?, orr_terrain::MAX_FILE_BYTES)?
                != pending.backing
        {
            return Err(
                "Terrain, source PNG, settings or scene changed after Preview; preview again"
                    .into(),
            );
        }
        if read_optional(&context.sidecar_path()?, MAX_BYTES)? != self.saved {
            return Err("Sidecar changed after Preview".into());
        }
        let candidate = pending.surface.clone();
        let rendered = model(&context, &candidate)?;
        if self.current.as_ref().map(|s| s.digest()) != Some(candidate.digest()) {
            self.undo.push(self.current.clone());
            self.redo.clear();
            self.current = Some(candidate.clone());
            self.trim_history();
        }
        self.cache_marker = context.marker;
        self.cache =
            rendered.map(|rendered| (context.terrain.revision(), candidate.digest(), rendered));
        self.pending = None;
        Ok(())
    }
    fn trim_history(&mut self) {
        while self.undo.len() + self.redo.len() > HISTORY_ENTRIES
            || self
                .undo
                .iter()
                .chain(&self.redo)
                .filter_map(|s| s.as_ref())
                .map(|s| s.to_bytes().map_or(MAX_BYTES, |b| b.len()))
                .sum::<usize>()
                > HISTORY_BYTES
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }
    pub(crate) fn history(
        &mut self,
        editor: &Editor,
        panel: &TerrainPanel,
        redo: bool,
    ) -> Result<(), String> {
        let context = self.context(editor, panel)?;
        if self.pending.is_some() {
            return Err("Cancel Preview before appearance Undo or Redo".into());
        }
        let stack = if redo { &self.redo } else { &self.undo };
        let candidate = stack.last().ok_or("Appearance history is empty")?.clone();
        if let Some(surface) = &candidate {
            model(&context, surface)?;
        }
        if redo {
            self.redo.pop();
            self.undo.push(self.current.clone());
        } else {
            self.undo.pop();
            self.redo.push(self.current.clone());
        }
        self.current = candidate;
        self.trim_history();
        self.cache = None;
        Ok(())
    }
    pub(crate) fn cancel_preview(&mut self) {
        self.pending = None;
    }
    pub(crate) fn discard(&mut self) {
        // Explicitly discard ownership as well; the next sync reloads disk truth.
        *self = Self {
            png_path: self.png_path.clone(),
            ..Self::default()
        };
    }
    pub(crate) fn save(&mut self, editor: &Editor, panel: &TerrainPanel) -> Result<(), String> {
        self.save_with_checkpoint(editor, panel, || {})
    }
    fn save_with_checkpoint(
        &mut self,
        editor: &Editor,
        panel: &TerrainPanel,
        checkpoint: impl FnOnce(),
    ) -> Result<(), String> {
        let context = self.context(editor, panel)?;
        let host_epoch = editor.sim().epoch;
        let scene_bytes = read_optional(&context.binding.scene, MAX_BYTES)?
            .ok_or("Save requires an existing scene file")?;
        let backing_path = context.terrain_path()?;
        let backing_bytes = read_optional(&backing_path, orr_terrain::MAX_FILE_BYTES)?
            .ok_or("Save terrain before saving its appearance")?;
        let backing = Terrain::load(&backing_bytes)
            .map_err(|e| format!("Appearance backing terrain refused: {e}"))?;
        if backing.asset_id() != context.binding.asset {
            return Err("Backing terrain identity changed; refusing appearance Save".into());
        }
        if let Some(surface) = &self.current {
            surface.validate_terrain(&context.binding.relative, &backing)?;
        }
        let recheck = || -> Result<(), String> {
            if editor.sim().epoch != host_epoch
                || Context::identity(editor, panel)?.0 != context.binding
                || read_optional(&context.binding.scene, MAX_BYTES)?.as_ref() != Some(&scene_bytes)
                || read_optional(&backing_path, orr_terrain::MAX_FILE_BYTES)?.as_ref()
                    != Some(&backing_bytes)
            {
                return Err("Scene or backing terrain changed before appearance publication; nothing was published".into());
            }
            Ok(())
        };
        if self.pending.is_some() {
            return Err("Apply or cancel Preview before Save appearance".into());
        }
        let bytes = self.current.as_ref().map(|s| s.to_bytes()).transpose()?;
        let path = context.sidecar_path()?;
        if read_optional(&path, MAX_BYTES)? != self.saved {
            return Err("Sidecar changed on disk; refusing to overwrite unexpected bytes".into());
        }
        let parent = path.parent().ok_or("Sidecar has no parent")?;
        checked_path(parent)?;
        if bytes.is_none() {
            checkpoint();
            recheck()?;
            if self.saved.is_some() {
                if read_optional(&path, MAX_BYTES)? != self.saved {
                    return Err("Sidecar changed before removal".into());
                }
                fs::remove_file(&path).map_err(|e| e.to_string())?;
                self.saved = None;
                self.saved_digest = None;
                self.uncertain = true;
                #[cfg(unix)]
                fs::File::open(parent)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| format!("Sidecar removed; directory durability uncertain: {e}"))?;
            }
            self.uncertain = false;
            return Ok(());
        }
        let bytes = bytes.expect("checked appearance bytes");
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        checkpoint();
        checked_path(parent)?;
        recheck()?;
        if read_optional(&path, MAX_BYTES)? != self.saved {
            return Err("Sidecar changed before publication".into());
        }
        if self.saved.is_some() {
            temp.persist(&path).map_err(|e| e.to_string())?;
        } else {
            temp.persist_noclobber(&path).map_err(|e| e.to_string())?;
        }
        // Rename has happened: report uncertainty rather than pretending rollback.
        self.saved = Some(bytes);
        self.saved_digest = self.current.as_ref().map(|s| s.digest());
        self.uncertain = true;
        #[cfg(unix)]
        fs::File::open(parent).and_then(|f| f.sync_all()).map_err(|e| format!("Appearance published, but directory durability is uncertain: {e}. Inspect the saved sidecar before closing."))?;
        self.uncertain = false;
        Ok(())
    }
    /// Returns None only when checker is allowed; Some(None) refuses rendering.
    pub(crate) fn viewport(
        &mut self,
        editor: &Editor,
        panel: &TerrainPanel,
    ) -> Option<Option<Arc<StaticModel>>> {
        let (binding, revision) = match Context::identity(editor, panel) {
            Ok(c) => c,
            Err(_) => return self.has_surface().then_some(None),
        };
        if self.binding.as_ref() != Some(&binding) {
            return self.has_surface().then_some(None);
        }
        if self.load_error.is_some() {
            return Some(None);
        }
        let surface = self.current.as_ref()?;
        let marker =
            if editor.mode() == Mode::Edit && same_scene(panel.document.scene(), &binding.scene) {
                panel.document.marker()
            } else {
                None
            };
        let key = (revision, surface.digest());
        if let Some((revision, digest, cached)) = &self.cache {
            if (*revision, *digest) == key && self.cache_marker == marker {
                return Some(Some(cached.clone()));
            }
        }
        let context = match Context::get(editor, panel) {
            Ok(c) => c,
            Err(error) => {
                self.status = error;
                return Some(None);
            }
        };
        match model(&context, surface) {
            Ok(rendered) => {
                self.cache_marker = marker;
                self.cache = rendered.as_ref().map(|m| (key.0, key.1, m.clone()));
                Some(rendered)
            }
            Err(error) => {
                self.status = format!("Appearance rendering refused: {error}");
                Some(None)
            }
        }
    }
}
fn model(context: &Context, surface: &Surface) -> Result<Option<Arc<StaticModel>>, String> {
    surface.validate_terrain(&context.binding.relative, &context.terrain)?;
    surface::to_static_model(
        &context.terrain,
        &context.marker.into_iter().collect::<Vec<_>>(),
        surface,
    )
    .map(|model| model.map(Arc::new))
    .map_err(|e| e.to_string())
}
// Lexical identity only. Actual file operations separately reject symlinks.
fn same_scene(document: Option<&Path>, scene: &Path) -> bool {
    let Some(document) = document else {
        return false;
    };
    let path = if scene.is_absolute() {
        scene.to_path_buf()
    } else {
        let Ok(cwd) = std::env::current_dir() else {
            return false;
        };
        cwd.join(scene)
    };
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            part => normal.push(part),
        }
    }
    document == normal
}
fn joined(scene: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty()
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || relative.contains('\\')
        || relative.split('/').any(|c| c.eq_ignore_ascii_case(".orr"))
    {
        return Err(
            "Appearance path must be a normal scene-relative path outside package stores".into(),
        );
    }
    let path = scene.parent().ok_or("Scene has no parent")?.join(relative);
    checked_path(path.parent().ok_or("Path has no parent")?)?;
    Ok(path)
}
fn checked_path(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut cursor = PathBuf::new();
    for component in absolute.components() {
        if matches!(component, Component::ParentDir) {
            return Err("Parent traversal is not allowed".into());
        }
        cursor.push(component);
        let meta = fs::symlink_metadata(&cursor).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("Symlink paths are not allowed for appearance files".into());
        }
    }
    Ok(())
}
fn read_optional(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    checked_path(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("Appearance input must be a bounded regular file, not a symlink".into());
    }
    let mut bytes = Vec::new();
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() > limit as u64 {
        return Err("Appearance input changed while opening".into());
    }
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Appearance input byte limit".into());
    }
    checked_path(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let named = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.dev() != opened.dev()
            || metadata.ino() != opened.ino()
            || named.dev() != opened.dev()
            || named.ino() != opened.ino()
        {
            return Err("Appearance input replaced while reading".into());
        }
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
