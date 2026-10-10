//! Editor-owned lighting authoring with independent history and exact source binding.
use orr_sample::{room_lighting::Document, room_project::PreparedRoomLighting};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_UNDO: usize = 32;

// Private failure-injection boundary. Checks may fail an operation but never
// replace it: production Save always executes the real persistence operations.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveStage {
    BeforePersist,
    #[cfg(unix)]
    BeforeDirectorySync,
}

pub struct Panel {
    path: PathBuf,
    scene_path: PathBuf,
    manifest_path: PathBuf,
    root: Result<RootPin, String>,
    document: Document,
    candidate: Document,
    saved: Document,
    saved_bytes: Vec<u8>,
    manifest_bytes: Vec<u8>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    source: SourcePin,
    error: Option<String>,
}

enum SourcePin {
    /// The panel has not yet been shown alongside its editor.
    Unbound,
    /// This panel belongs to one source lifetime, which is retired by the editor
    /// when a room source is replaced or a host restart succeeds.
    Pinned(Arc<()>),
    /// A stale panel never silently binds to a later source, even if the lighting
    /// document happens to return to the same value.
    Retired,
}

/// Narrow boundary for the production editor and focused widget tests.
trait LightingEditor {
    fn room_lighting_document(&self) -> Option<&Document>;
    fn room_lighting_source_token(&self) -> Arc<()>;
    fn validate_room_lighting_document(&self, document: &Document) -> Result<(), String>;
    fn room_lighting_scene_path(&self) -> Option<PathBuf>;
    fn install_room_lighting(&mut self, document: Document) -> Result<(), String>;
    fn lighting_authoring_editable(&self) -> bool;
}

impl LightingEditor for crate::editor::Editor {
    fn room_lighting_document(&self) -> Option<&Document> {
        crate::editor::Editor::room_lighting_document(self)
    }
    fn room_lighting_source_token(&self) -> Arc<()> {
        crate::editor::Editor::room_lighting_source_token(self)
    }
    fn validate_room_lighting_document(&self, document: &Document) -> Result<(), String> {
        crate::editor::Editor::validate_room_lighting_document(self, document)
    }
    fn room_lighting_scene_path(&self) -> Option<PathBuf> {
        self.path()
    }
    fn install_room_lighting(&mut self, document: Document) -> Result<(), String> {
        crate::editor::Editor::install_room_lighting(self, document)
    }
    fn lighting_authoring_editable(&self) -> bool {
        self.mode() == crate::editor::Mode::Edit
            && self.can_mutate()
            && self.previewing().is_none()
            && self.yard_rows_coherent()
            && self.spec().is_local()
    }
}

struct RootPin {
    path: PathBuf,
    directory: File,
}
impl RootPin {
    fn new(parent: &Path) -> Result<Self, String> {
        let path = checked_directory(parent)?;
        let directory = File::open(&path).map_err(|e| e.to_string())?;
        Ok(Self { path, directory })
    }
    fn recheck(&self, path: &Path) -> Result<(), String> {
        let current = checked_directory(path.parent().unwrap_or(Path::new(".")))?;
        let metadata = std::fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        let pinned = self.directory.metadata().map_err(|e| e.to_string())?;
        if current != self.path || !same_directory(&metadata, &pinned) {
            return Err("Lighting project directory changed; reopen before saving".into());
        }
        Ok(())
    }
}
#[cfg(unix)]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    matches!((a.created(), b.created()), (Ok(a), Ok(b)) if a == b)
}
fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    orr_model_bindings::model_bindings::resolve_project(path, ".")
        .and_then(|p| std::fs::canonicalize(p).map_err(|e| e.to_string()))
}
fn unchanged_file(path: &Path, expected: &[u8]) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{} must remain a regular non-symlink file",
            path.display()
        ));
    }
    if metadata.len() != expected.len() as u64 {
        return Err(format!(
            "{} changed on disk; reopen before saving",
            path.display()
        ));
    }
    // A changing file cannot cause unbounded allocation even after metadata.
    let mut current = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut current)
        .map_err(|e| e.to_string())?;
    if current != expected {
        return Err(format!(
            "{} changed on disk; reopen before saving",
            path.display()
        ));
    }
    Ok(())
}
impl Panel {
    /// Bind before the first UI frame; a replaced source may never reclaim this panel.
    pub fn new_for_editor(
        prepared: PreparedRoomLighting,
        editor: &crate::editor::Editor,
    ) -> Result<Self, String> {
        let mut panel = Self::new(prepared);
        panel.ensure_source(editor)?;
        Ok(panel)
    }
    // Unbound construction is private; production must bind immediately above.
    fn new(prepared: PreparedRoomLighting) -> Self {
        let root = RootPin::new(prepared.path.parent().unwrap_or(Path::new(".")));
        let error = root.as_ref().err().cloned();
        Self {
            path: prepared.path,
            scene_path: prepared.scene_path,
            manifest_path: prepared.manifest_path,
            root,
            candidate: prepared.document.clone(),
            saved: prepared.document.clone(),
            document: prepared.document,
            saved_bytes: prepared.bytes,
            manifest_bytes: prepared.manifest_bytes,
            undo: Vec::new(),
            redo: Vec::new(),
            source: SourcePin::Unbound,
            error,
        }
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    fn apply_document(&mut self, candidate: Document) -> Result<(), String> {
        // Serialization performs shared validation before any mutation.
        candidate.to_bytes()?;
        self.accept_document(candidate);
        Ok(())
    }
    fn accept_document(&mut self, candidate: Document) {
        if candidate != self.document {
            push_bounded(
                &mut self.undo,
                std::mem::replace(&mut self.document, candidate),
            );
            self.redo.clear();
        }
        self.candidate = self.document.clone();
    }
    fn undo(&mut self) -> bool {
        if let Some(previous) = self.undo.pop() {
            push_bounded(
                &mut self.redo,
                std::mem::replace(&mut self.document, previous),
            );
            self.candidate = self.document.clone();
            true
        } else {
            false
        }
    }
    fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            push_bounded(&mut self.undo, std::mem::replace(&mut self.document, next));
            self.candidate = self.document.clone();
            true
        } else {
            false
        }
    }
    fn ensure_source(&mut self, editor: &dyn LightingEditor) -> Result<(), String> {
        if editor.room_lighting_scene_path().as_deref() != Some(self.scene_path.as_path()) {
            self.source = SourcePin::Retired;
            return Err("Lighting scene changed; reopen the complete project".into());
        }
        match &self.source {
            SourcePin::Retired => {
                return Err("Lighting source changed; reopen the complete project".into());
            }
            SourcePin::Unbound => {
                let token = editor.room_lighting_source_token();
                let matches = editor.room_lighting_document() == Some(&self.document);
                if !matches {
                    self.source = SourcePin::Retired;
                    return Err(
                        "Lighting panel does not match the installed Room lighting; reopen the complete project".into(),
                    );
                }
                let current_token = editor.room_lighting_source_token();
                if !Arc::ptr_eq(&token, &current_token)
                    || editor.room_lighting_document() != Some(&self.document)
                {
                    self.source = SourcePin::Retired;
                    return Err("Lighting source changed; reopen the complete project".into());
                }
                self.source = SourcePin::Pinned(token);
            }
            SourcePin::Pinned(pin) => {
                if !Arc::ptr_eq(pin, &editor.room_lighting_source_token())
                    || editor.room_lighting_document() != Some(&self.document)
                {
                    self.source = SourcePin::Retired;
                    return Err("Lighting source changed; reopen the complete project".into());
                }
            }
        }

        let pin = match &self.source {
            SourcePin::Pinned(pin) => Arc::clone(pin),
            SourcePin::Unbound | SourcePin::Retired => unreachable!("source was bound above"),
        };
        editor.validate_room_lighting_document(&self.document)?;
        if !Arc::ptr_eq(&pin, &editor.room_lighting_source_token())
            || editor.room_lighting_document() != Some(&self.document)
        {
            self.source = SourcePin::Retired;
            return Err("Lighting source changed; reopen the complete project".into());
        }
        Ok(())
    }
    fn validate_candidate(
        &mut self,
        editor: &dyn LightingEditor,
        candidate: &Document,
    ) -> Result<(), String> {
        if !editor.lighting_authoring_editable() {
            return Err("Lighting authoring requires a coherent local Edit snapshot".into());
        }
        self.ensure_source(editor)?;
        candidate.to_bytes()?;
        editor.validate_room_lighting_document(candidate)?;
        // The editor validates against a fresh coherent snapshot; this second
        // fence ensures that snapshot still belongs to the panel's source.
        self.ensure_source(editor)
    }
    fn apply_checked(
        &mut self,
        editor: &mut dyn LightingEditor,
        candidate: Document,
    ) -> Result<(), String> {
        self.validate_candidate(editor, &candidate)?;
        editor.install_room_lighting(candidate.clone())?;
        self.accept_document(candidate);
        Ok(())
    }
    fn undo_checked(&mut self, editor: &mut dyn LightingEditor) -> Result<bool, String> {
        self.validate_candidate(editor, &self.document.clone())?;
        let Some(previous) = self.undo.last().cloned() else {
            return Ok(false);
        };
        self.validate_candidate(editor, &previous)?;
        editor.install_room_lighting(previous.clone())?;
        let _ = self.undo.pop();
        push_bounded(
            &mut self.redo,
            std::mem::replace(&mut self.document, previous),
        );
        self.candidate = self.document.clone();
        Ok(true)
    }
    fn redo_checked(&mut self, editor: &mut dyn LightingEditor) -> Result<bool, String> {
        self.validate_candidate(editor, &self.document.clone())?;
        let Some(next) = self.redo.last().cloned() else {
            return Ok(false);
        };
        self.validate_candidate(editor, &next)?;
        editor.install_room_lighting(next.clone())?;
        let _ = self.redo.pop();
        push_bounded(&mut self.undo, std::mem::replace(&mut self.document, next));
        self.candidate = self.document.clone();
        Ok(true)
    }
    fn save_checked(&mut self, editor: &dyn LightingEditor) -> Result<(), String> {
        if !editor.lighting_authoring_editable() {
            return Err("Lighting authoring requires a coherent local Edit snapshot".into());
        }
        self.ensure_source(editor)?;
        editor.validate_room_lighting_document(&self.document)?;
        self.ensure_source(editor)?;
        self.save()
    }
    fn recheck_disk(&self) -> Result<(), String> {
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        pin.recheck(&self.path)?;
        pin.recheck(&self.manifest_path)?;
        unchanged_file(&self.path, &self.saved_bytes)?;
        unchanged_file(&self.manifest_path, &self.manifest_bytes)
    }
    fn save(&mut self) -> Result<(), String> {
        self.save_with_check(|_| Ok(()))
    }
    fn save_with_check(
        &mut self,
        mut check: impl FnMut(SaveStage) -> std::io::Result<()>,
    ) -> Result<(), String> {
        let bytes = self.document.to_bytes()?;
        self.recheck_disk()?;
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        let mut temp = tempfile::NamedTempFile::new_in(&pin.path).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        // These ordinary change checks do not claim hostile-race isolation.
        self.recheck_disk()?;
        check(SaveStage::BeforePersist).map_err(|e| format!("Lighting not saved: {e}"))?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        // Replacement already succeeded: advance the baseline even if syncing
        // directory metadata fails, so retry never mistakes our write for an edit.
        self.saved = self.document.clone();
        self.saved_bytes = bytes;
        #[cfg(unix)]
        check(SaveStage::BeforeDirectorySync)
            .and_then(|_| pin.directory.sync_all())
            .map_err(|e| {
                format!(
                    "Lighting replaced, but directory sync failed; durability is uncertain: {e}"
                )
            })?;
        Ok(())
    }
    pub fn apply_for_editor(
        &mut self,
        editor: &mut crate::editor::Editor,
        document: Document,
    ) -> Result<(), String> {
        self.apply_checked(editor, document)
    }
    pub fn undo_for_editor(&mut self, editor: &mut crate::editor::Editor) -> Result<bool, String> {
        self.undo_checked(editor)
    }
    pub fn redo_for_editor(&mut self, editor: &mut crate::editor::Editor) -> Result<bool, String> {
        self.redo_checked(editor)
    }
    pub fn save_for_editor(&mut self, editor: &crate::editor::Editor) -> Result<(), String> {
        self.save_checked(editor)
    }

    /// Production editor path. It binds this panel to one lighting source lifetime,
    /// and every state-changing operation is validated against the editor's
    /// current coherent Room snapshot before it can update either side.
    pub fn show_for_editor(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut crate::editor::Editor,
    ) -> bool {
        let editable = editor.lighting_authoring_editable();
        self.show_impl(ui, editable, Some(editor))
    }

    fn show_impl(
        &mut self,
        ui: &mut egui::Ui,
        editable: bool,
        mut editor: Option<&mut dyn LightingEditor>,
    ) -> bool {
        let source_ok = match editor.as_deref() {
            Some(editor) => match self.ensure_source(editor) {
                Ok(()) => true,
                Err(error) => {
                    self.error = Some(error);
                    false
                }
            },
            None => true,
        };
        let can_edit = editable && source_ok;
        let mut reset_live = false;
        ui.collapsing("Room lighting", |ui| {
            ui.label(if self.document != self.saved {
                "Lighting (unsaved)"
            } else {
                "Lighting saved"
            });
            if !editable {
                ui.label("Lighting authoring is read-only during Play or for a remote project");
            } else if !source_ok {
                ui.label("Lighting source is no longer available for authoring; reopen the complete project");
            }
            ui.add_enabled_ui(can_edit, |ui| {
                let mut enabled = self.candidate.point_light.is_some();
                if ui.checkbox(&mut enabled, "Point light enabled").changed() {
                    self.candidate = if enabled { Document::enabled_default() } else { Document::default() };
                }
                if let Some(light) = &mut self.candidate.point_light {
                    for (axis, value) in ["X", "Y", "Z"].into_iter().zip(&mut light.position) {
                        number_bounded(ui, &format!("Position {axis}"), value, -1e9, 1e9);
                    }
                    for (channel, value) in ["R", "G", "B"].into_iter().zip(&mut light.color) {
                        number_bounded(ui, &format!("Linear {channel}"), value, 0.0, 1e4);
                    }
                    number_bounded(ui, "Intensity", &mut light.intensity, 0.0, 1e4);
                    number_bounded(ui, "Range", &mut light.range, 1e-6, 1e9);
                }

                if ui.button("Apply lighting").clicked() {
                    let result = match editor.as_deref_mut() {
                        Some(editor) => self.apply_checked(editor, self.candidate.clone()),
                        None => self.apply_document(self.candidate.clone()),
                    };
                    match result {
                        Ok(()) => {
                            self.error = None;
                            reset_live = true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo lighting edit"))
                    .clicked()
                {
                    match editor.as_deref_mut() {
                        Some(editor) => match self.undo_checked(editor) {
                            Ok(changed) => {
                                reset_live = changed;
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        },
                        None => {
                            reset_live = self.undo();
                            self.error = None;
                        }
                    }
                }
                if ui
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo lighting edit"))
                    .clicked()
                {
                    match editor.as_deref_mut() {
                        Some(editor) => match self.redo_checked(editor) {
                            Ok(changed) => {
                                reset_live = changed;
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        },
                        None => {
                            reset_live = self.redo();
                            self.error = None;
                        }
                    }
                }
                if ui.button("Reset lighting defaults").clicked() {
                    let defaults = Document::default();
                    let result = match editor.as_deref_mut() {
                        Some(editor) => self.apply_checked(editor, defaults),
                        None => self.apply_document(defaults),
                    };
                    match result {
                        Ok(()) => {
                            self.error = None;
                            reset_live = true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.button("Save lighting").clicked() {
                    let result = match editor.as_deref() {
                        Some(editor) => self.save_checked(editor),
                        None => self.save(),
                    };
                    self.error = result.err();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
        reset_live
    }
}

fn push_bounded(stack: &mut Vec<Document>, document: Document) {
    if stack.len() == MAX_UNDO {
        stack.remove(0);
    }
    stack.push(document);
}

fn number_bounded(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.05).range(min..=max))
            .labelled_by(label.id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MockLightingEditor {
        document: Option<Document>,
        scene: PathBuf,
        source: Arc<()>,
        fail_validation: bool,
        fail_install: bool,
    }
    impl MockLightingEditor {
        fn new(panel: &Panel) -> Self {
            Self {
                document: Some(panel.document.clone()),
                scene: panel.scene_path.clone(),
                source: Arc::new(()),
                fail_validation: false,
                fail_install: false,
            }
        }
    }
    impl LightingEditor for MockLightingEditor {
        fn room_lighting_document(&self) -> Option<&Document> {
            self.document.as_ref()
        }
        fn room_lighting_source_token(&self) -> Arc<()> {
            self.source.clone()
        }
        fn room_lighting_scene_path(&self) -> Option<PathBuf> {
            Some(self.scene.clone())
        }
        fn validate_room_lighting_document(&self, doc: &Document) -> Result<(), String> {
            if self.fail_validation {
                return Err("incoherent snapshot".into());
            }
            doc.validate()
        }
        fn install_room_lighting(&mut self, doc: Document) -> Result<(), String> {
            if self.fail_install {
                return Err("rejected installation".into());
            }
            self.document = Some(doc);
            Ok(())
        }
        fn lighting_authoring_editable(&self) -> bool {
            true
        }
    }
    fn fixture() -> (tempfile::TempDir, PreparedRoomLighting) {
        let dir = tempfile::tempdir().unwrap();
        let document = Document::enabled_default();
        let bytes = document.to_bytes().unwrap();
        let path = dir.path().join("room.lighting.json");
        let manifest_path = dir.path().join("orr.project.json");
        let manifest_bytes =
            b"{\"schema\":2,\"entry\":{\"lighting\":\"room.lighting.json\"}}\n".to_vec();
        let scene_path = dir.path().join("room.scene.yaml");
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(&manifest_path, &manifest_bytes).unwrap();
        (
            dir,
            PreparedRoomLighting {
                scene_path,
                path,
                document,
                bytes,
                manifest_path,
                manifest_bytes,
            },
        )
    }
    #[test]
    fn invalid_candidate_is_atomic_and_history_is_bounded() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let original = panel.document().clone();
        let mut invalid = original.clone();
        invalid.point_light.as_mut().unwrap().intensity = f32::NAN;
        assert!(panel.apply_document(invalid).is_err());
        assert_eq!(panel.document(), &original);
        assert!(!panel.undo());
        for n in 0..40 {
            let mut candidate = panel.document().clone();
            candidate.point_light.as_mut().unwrap().position[0] = n as f32 * 0.01;
            panel.apply_document(candidate).unwrap();
        }
        assert_eq!(panel.undo.len(), MAX_UNDO);
    }
    #[test]
    fn external_exact_byte_edits_and_nonregular_targets_refuse_save() {
        for manifest in [false, true] {
            let (_dir, prepared) = fixture();
            let target = if manifest {
                prepared.manifest_path.clone()
            } else {
                prepared.path.clone()
            };
            let mut panel = Panel::new(prepared);
            let mut changed = panel.document().clone();
            changed.point_light.as_mut().unwrap().intensity += 1.0;
            panel.apply_document(changed).unwrap();
            let mut bytes = std::fs::read(&target).unwrap();
            *bytes.last_mut().unwrap() = b' '; // Same size and meaning still counts as an external edit.
            std::fs::write(&target, &bytes).unwrap();
            assert!(panel.save().is_err());
            assert_eq!(std::fs::read(&target).unwrap(), bytes);
            assert_ne!(panel.document, panel.saved);
            assert_eq!(panel.undo.len(), 1);
            std::fs::remove_file(&target).unwrap();
            std::fs::create_dir(&target).unwrap();
            assert!(panel.save().is_err());
        }
    }
    #[test]
    fn failed_pre_persist_preserves_file_baseline_history_and_dirty_state() {
        let (dir, prepared) = fixture();
        let original_bytes = prepared.bytes.clone();
        let original_document = prepared.document.clone();
        let mut panel = Panel::new(prepared);
        let mut edited = panel.document().clone();
        edited.point_light.as_mut().unwrap().intensity += 1.0;
        panel.apply_document(edited.clone()).unwrap();
        let original_history = panel.undo.clone();
        let error = panel
            .save_with_check(|stage| {
                assert!(stage == SaveStage::BeforePersist);
                Err(std::io::Error::other("injected pre-persist failure"))
            })
            .unwrap_err();
        assert!(error.contains("Lighting not saved"));
        assert_eq!(std::fs::read(&panel.path).unwrap(), original_bytes);
        assert_eq!(panel.saved_bytes, original_bytes);
        assert_eq!(panel.saved, original_document);
        assert_eq!(panel.document, edited);
        assert_eq!(panel.candidate, edited);
        assert_eq!(panel.undo, original_history);
        assert_ne!(panel.document, panel.saved);
        // The failed transaction cleans up its same-directory temporary file.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        panel.save().unwrap();
        assert_eq!(panel.saved, edited);
        assert_eq!(panel.undo, original_history);
    }
    #[cfg(unix)]
    #[test]
    fn failed_directory_sync_reports_uncertainty_and_advances_baseline_for_retry() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let mut edited = panel.document().clone();
        edited.point_light.as_mut().unwrap().intensity += 1.0;
        panel.apply_document(edited.clone()).unwrap();
        let history = panel.undo.clone();
        let mut visited = Vec::new();
        let error = panel
            .save_with_check(|stage| {
                visited.push(stage);
                if stage == SaveStage::BeforeDirectorySync {
                    Err(std::io::Error::other("injected directory sync failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(visited == [SaveStage::BeforePersist, SaveStage::BeforeDirectorySync]);
        assert!(error.contains("Lighting replaced"));
        assert!(error.contains("durability is uncertain"));
        let bytes = edited.to_bytes().unwrap();
        assert_eq!(std::fs::read(&panel.path).unwrap(), bytes);
        assert_eq!(panel.saved_bytes, bytes);
        assert_eq!(panel.saved, edited);
        assert_eq!(panel.document, edited);
        assert_eq!(panel.undo, history);
        // Retrying a real Save must not reject our completed replacement as an
        // external edit, and the independent undo stack remains usable.
        panel.save().unwrap();
        assert_eq!(panel.saved_bytes, bytes);
        assert_eq!(panel.undo, history);
        assert!(panel.undo());
        assert_ne!(panel.document, panel.saved);
    }
    #[test]
    fn save_preserves_undo_and_tracks_saved_baseline() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let original = panel.document().clone();
        let mut changed = original.clone();
        changed.point_light.as_mut().unwrap().intensity += 1.0;
        panel.apply_document(changed).unwrap();
        panel.save().unwrap();
        assert_eq!(panel.document, panel.saved);
        assert!(panel.undo());
        assert_eq!(panel.document(), &original);
        assert_ne!(panel.document, panel.saved);
        panel.save().unwrap();
        assert_eq!(
            Document::parse(&std::fs::read(&panel.path).unwrap()).unwrap(),
            original
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_files_and_replaced_or_symlink_directory_refuse_save() {
        for manifest in [false, true] {
            let (dir, prepared) = fixture();
            let target = if manifest {
                prepared.manifest_path.clone()
            } else {
                prepared.path.clone()
            };
            let other = dir.path().join("original.json");
            let mut panel = Panel::new(prepared);
            std::fs::rename(&target, &other).unwrap();
            std::os::unix::fs::symlink(&other, &target).unwrap();
            assert!(panel.save().is_err());
        }
        let (dir, prepared) = fixture();
        let path = prepared.path.clone();
        let manifest_path = prepared.manifest_path.clone();
        let bytes = prepared.bytes.clone();
        let manifest_bytes = prepared.manifest_bytes.clone();
        let mut panel = Panel::new(prepared);
        let holder = tempfile::tempdir().unwrap();
        let moved = holder.path().join("moved");
        std::fs::rename(dir.path(), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, dir.path()).unwrap();
        assert!(panel.save().is_err());
        std::fs::remove_file(dir.path()).unwrap();
        std::fs::create_dir(dir.path()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(&manifest_path, manifest_bytes).unwrap();
        assert!(panel.save().is_err());
    }
    #[test]
    fn play_widgets_cannot_apply_or_save_staged_values() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let path = prepared.path.clone();
        let mut panel = Panel::new(prepared);
        panel.candidate.point_light.as_mut().unwrap().intensity += 1.0;
        let mut h = Harness::builder()
            .with_size(egui::vec2(600.0, 800.0))
            .build_ui_state(
                |ui, panel: &mut Panel| {
                    assert!(!panel.show_impl(ui, false, None));
                },
                panel,
            );
        h.run_steps(3);
        h.get_by_label("Room lighting").click();
        h.run_steps(3);
        h.get_by_label("Apply lighting").click();
        h.get_by_label("Save lighting").click();
        h.run_steps(3);
        assert_eq!(h.state().document(), &original);
        assert!(h.state().undo.is_empty());
        assert_eq!(
            Document::parse(&std::fs::read(path).unwrap()).unwrap(),
            original
        );
    }
    #[test]
    fn source_identity_and_failed_install_preserve_checked_history() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let mut editor = MockLightingEditor::new(&panel);
        panel.ensure_source(&editor).unwrap();
        let original = panel.document.clone();
        let candidate = Document::default();
        editor.fail_validation = true;
        assert!(panel.apply_checked(&mut editor, candidate.clone()).is_err());
        editor.fail_validation = false;
        editor.fail_install = true;
        assert!(panel.apply_checked(&mut editor, candidate.clone()).is_err());
        assert_eq!(panel.document, original);
        assert!(panel.undo.is_empty());
        assert!(panel.redo.is_empty());
        assert_eq!(editor.document.as_ref(), Some(&original));
        editor.fail_install = false;
        editor.source = Arc::new(());
        assert!(panel.apply_checked(&mut editor, candidate.clone()).is_err());
        // Even returning to an identical document and path cannot revive retirement.
        assert!(matches!(panel.source, SourcePin::Retired));
        assert!(panel.apply_checked(&mut editor, candidate).is_err());
        assert_eq!(panel.document, original);
        assert!(panel.undo.is_empty());
    }
    #[test]
    fn initial_wrong_scene_cannot_bind_identical_document() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let mut editor = MockLightingEditor::new(&panel);
        let expected = editor.scene.clone();
        editor.scene = expected.with_file_name("different.scene.yaml");
        assert!(panel.ensure_source(&editor).is_err());
        editor.scene = expected;
        assert!(panel.ensure_source(&editor).is_err());
        assert!(matches!(panel.source, SourcePin::Retired));
        assert!(panel.undo.is_empty());
        assert!(panel.redo.is_empty());
    }
}
