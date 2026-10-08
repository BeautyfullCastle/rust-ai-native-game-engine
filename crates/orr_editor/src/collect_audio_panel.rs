//! One bounded, source-pinned Collect pickup editor. Preview owns a separate mixer.
use orr_sample::collect_audio::{Document, PreparedAudio};
use orr_sample::collect_audio_output::{AudioMode, Playback};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("audio source must remain a bounded regular non-symlink file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("audio source exceeds its byte limit".into());
    }
    Ok(bytes)
}

fn pin_manifest(parent: &Path, audio: &Path, scene: &Path) -> Result<Vec<u8>, String> {
    let path = parent.join("orr.project.json");
    let bytes = read_regular(&path, MAX_MANIFEST_BYTES)?;
    // Reuse the closed schema/path admission rather than accepting arbitrary
    // JSON claims about an unrelated scene or sidecar.
    let project = orr_package::Project::open(parent, orr_package::Runtime::content_only())
        .map_err(|e| e.to_string())?;
    let manifest = project.manifest().ok_or("audio project metadata missing")?;
    let entry = manifest
        .entry
        .as_ref()
        .ok_or("audio project entry missing")?;
    if !matches!(manifest.schema, 2 | 3)
        || entry.game != orr_package::ProjectGame::CollectDodgeV1
        || entry
            .audio
            .as_ref()
            .map(|relative| project.root().join(relative))
            .as_deref()
            != Some(audio)
        || project.root().join(&entry.scene) != scene
        || read_regular(&path, MAX_MANIFEST_BYTES)? != bytes
    {
        return Err("audio project entry changed or does not own this scene and sidecar".into());
    }
    Ok(bytes)
}

pub struct Panel {
    prepared: PreparedAudio,
    saved: Vec<u8>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    preview: Option<Playback>,
    preview_id: u64,
    token: Arc<AtomicU64>,
    generation: u64,
    scene: PathBuf,
    parent: File,
    manifest: Vec<u8>,
    retired: bool,
    error: Option<String>,
}
impl Panel {
    pub fn new(prepared: PreparedAudio, editor: &mut crate::Editor) -> Result<Self, String> {
        let scene = editor.path().ok_or("audio needs its saved local scene")?;
        let parent_path = prepared.path.parent().ok_or("audio parent missing")?;
        crate::sprite_bindings::resolve_project(parent_path, ".")?;
        let parent = File::open(parent_path).map_err(|e| e.to_string())?;
        let manifest = pin_manifest(parent_path, &prepared.path, &scene)?;
        if read_regular(&prepared.path, orr_sample::collect_audio::MAX_BYTES)? != prepared.bytes {
            return Err("audio sidecar changed since admission; reopen project".into());
        }
        let token = editor.collect_audio_source_token();
        let generation = token.load(Ordering::SeqCst);
        if generation == u64::MAX {
            return Err("audio source generation exhausted".into());
        }
        editor.install_collect_audio(prepared.clone(), AudioMode::Auto)?;
        Ok(Self {
            saved: prepared.bytes.clone(),
            prepared,
            undo: Vec::new(),
            redo: Vec::new(),
            preview: None,
            preview_id: 0,
            token,
            generation,
            scene,
            parent,
            manifest,
            retired: false,
            error: None,
        })
    }
    fn active(&mut self, editor: &crate::Editor) -> Result<(), String> {
        if self.retired
            || !Arc::ptr_eq(&self.token, &editor.collect_audio_source_token())
            || self.token.load(Ordering::SeqCst) != self.generation
            || !editor.game().is_collect()
            || !editor.spec().is_local()
            || editor.path().as_ref() != Some(&self.scene)
        {
            self.retired = true;
            self.preview = None;
            return Err("audio source changed; reopen its project".into());
        }
        Ok(())
    }
    pub fn document(&self) -> &Document {
        &self.prepared.document
    }
    pub fn change(&mut self, next: Document, editor: &mut crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        if editor.is_playing_mode() {
            return Err("stop Play before editing pickup audio".into());
        }
        let next = self.prepared.with_document(next)?;
        if next.document == self.prepared.document {
            return Ok(());
        }
        editor
            .collect_audio_mut()
            .ok_or("audio owner retired")?
            .set_document(next.document.clone())?;
        self.preview = None;
        if self.undo.len() == 32 {
            self.undo.remove(0);
        }
        self.undo.push(self.prepared.document.clone());
        self.redo.clear();
        self.prepared = next;
        Ok(())
    }
    pub fn undo(&mut self, editor: &mut crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        if editor.is_playing_mode() {
            return Err("stop Play before Undo".into());
        }
        if let Some(previous) = self.undo.last().cloned() {
            let next = self.prepared.with_document(previous)?;
            editor
                .collect_audio_mut()
                .ok_or("audio owner retired")?
                .set_document(next.document.clone())?;
            self.undo.pop();
            self.redo.push(self.prepared.document.clone());
            self.prepared = next;
            self.preview = None;
        }
        Ok(())
    }
    pub fn redo(&mut self, editor: &mut crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        if editor.is_playing_mode() {
            return Err("stop Play before Redo".into());
        }
        if let Some(next) = self.redo.last().cloned() {
            let next = self.prepared.with_document(next)?;
            editor
                .collect_audio_mut()
                .ok_or("audio owner retired")?
                .set_document(next.document.clone())?;
            self.redo.pop();
            self.undo.push(self.prepared.document.clone());
            self.prepared = next;
            self.preview = None;
        }
        Ok(())
    }
    fn check_save_source(&self) -> Result<PathBuf, String> {
        let parent = self
            .prepared
            .path
            .parent()
            .ok_or("audio parent missing")?
            .to_path_buf();
        crate::sprite_bindings::resolve_project(&parent, ".")?;
        let metadata = std::fs::symlink_metadata(&parent).map_err(|e| e.to_string())?;
        let pinned = self.parent.metadata().map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != pinned.dev() || metadata.ino() != pinned.ino() {
                return Err("audio directory replaced; reopen project".into());
            }
        }
        #[cfg(not(unix))]
        if !matches!((metadata.created(),pinned.created()),(Ok(a),Ok(b)) if a==b) {
            return Err("cannot verify audio directory identity".into());
        }
        if read_regular(&parent.join("orr.project.json"), MAX_MANIFEST_BYTES)? != self.manifest {
            return Err("audio project metadata changed; reopen before saving".into());
        }
        // An ordinary package-manager update can replace the admitted bank while
        // leaving this sidecar unchanged. Never save old-bank assignments into it.
        let mut runtime = crate::sprite_bindings::compiled_runtime();
        runtime.capabilities.insert("collect-audio".into());
        let project = orr_package::Project::open(&parent, runtime).map_err(|e| e.to_string())?;
        let package = project
            .read_package_bounded(
                &self.prepared.document.pickup.package,
                orr_sample::collect_audio::MAX_PACKAGE_FILES,
                2 * 1024 * 1024,
                orr_sample::collect_audio::MAX_PACKAGE_BYTES,
            )
            .map_err(|e| e.to_string())?;
        if package != self.prepared.package {
            return Err("audio package changed; reopen before saving".into());
        }
        let current = read_regular(&self.prepared.path, orr_sample::collect_audio::MAX_BYTES)?;
        if current != self.saved {
            return Err("audio sidecar changed externally; reopen before saving".into());
        }
        Ok(parent)
    }
    pub fn save(&mut self, editor: &crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        if editor.is_playing_mode() {
            return Err("stop Play before saving audio".into());
        }
        let parent = self.check_save_source()?;
        let bytes = self.prepared.document.to_bytes()?;
        let mut temporary = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
        temporary.write_all(&bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        self.active(editor)?;
        // Recheck after writing the temporary file. These checks detect stale
        // paths and edits; they do not claim hostile-race filesystem isolation.
        self.check_save_source()?;
        temporary
            .persist(&self.prepared.path)
            .map_err(|e| e.to_string())?;
        self.saved = bytes.clone();
        self.prepared.bytes = bytes;
        self.parent
            .sync_all()
            .map_err(|e| format!("audio saved but directory durability uncertain: {e}"))?;
        Ok(())
    }
    /// Use the real Kira offline renderer for device-independent acceptance.
    /// Subsequent Preview button events use this same explicit offline owner.
    pub fn preview_offline(&mut self, editor: &crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        self.preview = Some(Playback::offline(self.prepared.clone())?);
        self.preview(editor)
    }
    pub fn render_preview(&mut self, stereo: &mut [f32]) -> Result<(), String> {
        self.preview
            .as_mut()
            .ok_or("audio preview is not active")?
            .render(stereo)
    }
    pub fn preview(&mut self, editor: &crate::Editor) -> Result<(), String> {
        self.active(editor)?;
        if self.preview.is_none() {
            self.preview = Some(Playback::open(self.prepared.clone(), AudioMode::Auto)?);
        }
        self.preview_id = self
            .preview_id
            .checked_add(1)
            .ok_or("preview identity exhausted")?;
        let update = orr_bridge::ViewUpdate {
            snapshot: None,
            resync: None,
            events: vec![orr_bridge::BridgeEvent::Sim {
                key: orr_bridge::EventKey::new(self.preview_id, 0, 0),
                status: orr_bridge::EventStatus::Verified(()),
            }],
        };
        self.preview.as_mut().unwrap().update(1, &update, |_| true)
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut crate::Editor) {
        if let Err(error) = self.active(editor) {
            ui.label(error);
            return;
        }
        ui.collapsing("Collect pickup audio", |ui| {
            ui.label("Installed PCM16 • one SFX cue");
            let mut next = self.prepared.document.clone();
            ui.add_enabled_ui(!editor.is_playing_mode(), |ui| {
                egui::ComboBox::from_label("Pickup clip")
                    .selected_text(&next.pickup.asset)
                    .show_ui(ui, |ui| {
                        for available in &self.prepared.available {
                            ui.selectable_value(
                                &mut next.pickup.asset,
                                available.asset.clone(),
                                &available.asset,
                            );
                        }
                    });
                ui.add(egui::Slider::new(&mut next.gain, 0..=1000).text("Gain (0–1000)"));
                ui.checkbox(&mut next.mute, "Mute pickup");
            });
            if next != self.prepared.document {
                if let Err(error) = self.change(next, editor) {
                    self.error = Some(error);
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Preview pickup").clicked() {
                    if let Err(error) = self.preview(editor) {
                        self.error = Some(error);
                    }
                }
                if ui.button("Stop preview").clicked() {
                    self.preview = None;
                }
            });
            ui.add_enabled_ui(!editor.is_playing_mode(), |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo audio"))
                        .clicked()
                    {
                        if let Err(error) = self.undo(editor) {
                            self.error = Some(error);
                        }
                    }
                    if ui
                        .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo audio"))
                        .clicked()
                    {
                        if let Err(error) = self.redo(editor) {
                            self.error = Some(error);
                        }
                    }
                    if ui.button("Save audio").clicked() {
                        match self.save(editor) {
                            Ok(()) => self.error = None,
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
            });
            if let Some(preview) = &self.preview {
                ui.label(preview.status());
            }
            if let Some(playback) = editor.collect_audio_mut() {
                ui.label(format!("Play: {}", playback.status()));
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
    }
}
