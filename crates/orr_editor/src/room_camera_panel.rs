//! Editor-owned camera authoring. Camera history and saves are independent of
//! scene/model transactions and transient viewport navigation.
use orr_sample::{
    room_camera::{Document, Projection},
    room_project::PreparedRoomCamera,
};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
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
    manifest_path: PathBuf,
    root: Result<RootPin, String>,
    document: Document,
    candidate: Document,
    saved: Document,
    saved_bytes: Vec<u8>,
    manifest_bytes: Vec<u8>,
    undo: Vec<Document>,
    error: Option<String>,
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
            return Err("Camera project directory changed; reopen before saving".into());
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
    pub fn new(prepared: PreparedRoomCamera) -> Self {
        let root = RootPin::new(prepared.path.parent().unwrap_or(Path::new(".")));
        let error = root.as_ref().err().cloned();
        Self {
            path: prepared.path,
            manifest_path: prepared.manifest_path,
            root,
            candidate: prepared.document.clone(),
            saved: prepared.document.clone(),
            document: prepared.document,
            saved_bytes: prepared.bytes,
            manifest_bytes: prepared.manifest_bytes,
            undo: Vec::new(),
            error,
        }
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn apply_document(&mut self, candidate: Document) -> Result<(), String> {
        // Serialization performs shared validation before any mutation.
        candidate.to_bytes()?;
        if candidate != self.document {
            if self.undo.len() == MAX_UNDO {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
        }
        self.candidate = self.document.clone();
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.undo.pop() {
            self.document = previous;
            self.candidate = self.document.clone();
            true
        } else {
            false
        }
    }
    fn recheck_disk(&self) -> Result<(), String> {
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        pin.recheck(&self.path)?;
        pin.recheck(&self.manifest_path)?;
        unchanged_file(&self.path, &self.saved_bytes)?;
        unchanged_file(&self.manifest_path, &self.manifest_bytes)
    }
    pub fn save(&mut self) -> Result<(), String> {
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
        check(SaveStage::BeforePersist).map_err(|e| format!("Camera not saved: {e}"))?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        // Replacement already succeeded: advance the baseline even if syncing
        // directory metadata fails, so retry never mistakes our write for an edit.
        self.saved = self.document.clone();
        self.saved_bytes = bytes;
        #[cfg(unix)]
        check(SaveStage::BeforeDirectorySync)
            .and_then(|_| pin.directory.sync_all())
            .map_err(|e| {
                format!("Camera replaced, but directory sync failed; durability is uncertain: {e}")
            })?;
        Ok(())
    }
    /// True requests that the editor reset its live camera to this document.
    /// Navigation itself is never captured or silently persisted by this panel.
    pub fn show(&mut self, ui: &mut egui::Ui, editable: bool) -> bool {
        let mut reset_live = false;
        ui.collapsing("Room camera", |ui| {
            ui.label(if self.document != self.saved {
                "Camera (unsaved)"
            } else {
                "Camera saved"
            });
            if !editable {
                ui.label("Camera authoring is read-only during Play");
            }
            ui.add_enabled_ui(editable, |ui| {
                for (axis, value) in ["X", "Y", "Z"].into_iter().zip(&mut self.candidate.target) {
                    number(ui, &format!("Target {axis}"), value);
                }
                number(ui, "Yaw (radians)", &mut self.candidate.yaw);
                number(ui, "Pitch (radians)", &mut self.candidate.pitch);
                number(ui, "Distance", &mut self.candidate.distance);
                let mut orthographic =
                    matches!(self.candidate.projection, Projection::Orthographic { .. });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut orthographic, false, "Perspective");
                    ui.radio_value(&mut orthographic, true, "Orthographic");
                });
                if orthographic
                    != matches!(self.candidate.projection, Projection::Orthographic { .. })
                {
                    self.candidate.projection = if orthographic {
                        Projection::Orthographic { half_height: 12.0 }
                    } else {
                        Projection::Perspective {
                            fov_y_degrees: 60.0,
                        }
                    };
                }
                match &mut self.candidate.projection {
                    Projection::Perspective { fov_y_degrees } => {
                        number(ui, "Vertical FOV (degrees)", fov_y_degrees)
                    }
                    Projection::Orthographic { half_height } => {
                        number(ui, "Orthographic half-height", half_height)
                    }
                }
                if ui.button("Apply camera").clicked() {
                    match self.apply_document(self.candidate.clone()) {
                        Ok(()) => {
                            self.error = None;
                            reset_live = true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo camera edit"))
                    .clicked()
                {
                    reset_live = self.undo();
                    self.error = None;
                }
                if ui.button("Reset camera defaults").clicked() {
                    match self.apply_document(Document::readable_default()) {
                        Ok(()) => {
                            self.error = None;
                            reset_live = true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.button("Save camera").clicked() {
                    self.error = self.save().err();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
        reset_live
    }
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.05))
            .labelled_by(label.id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PreparedRoomCamera) {
        let dir = tempfile::tempdir().unwrap();
        let document = Document::readable_default();
        let bytes = document.to_bytes().unwrap();
        let path = dir.path().join("room.camera.json");
        let manifest_path = dir.path().join("orr.project.json");
        let manifest_bytes = b"{\"schema\":2,\"camera\":\"room.camera.json\"}\n".to_vec();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(&manifest_path, &manifest_bytes).unwrap();
        (
            dir,
            PreparedRoomCamera {
                path,
                document,
                bytes,
                manifest_path,
                manifest_bytes,
            },
        )
    }
    #[test]
    fn widgets_stage_apply_undo_save_reopen() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let path = prepared.path.clone();
        let manifest_path = prepared.manifest_path.clone();
        let mut h = Harness::builder()
            .with_size(egui::vec2(600.0, 800.0))
            .build_ui_state(
                |ui, state: &mut (Panel, usize)| {
                    state.1 += usize::from(state.0.show(ui, true));
                },
                (Panel::new(prepared), 0),
            );
        h.run_steps(3);
        h.get_by_label("Room camera").click();
        h.run_steps(3);
        let choice = if matches!(original.projection, Projection::Orthographic { .. }) {
            "Perspective"
        } else {
            "Orthographic"
        };
        h.get_by_label(choice).click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert_eq!(std::fs::read(&path).unwrap(), original.to_bytes().unwrap());
        h.get_by_label("Apply camera").click();
        h.run_steps(3);
        let edited = h.state().0.document().clone();
        assert_ne!(edited, original);
        assert_eq!(h.state().1, 1);
        h.get_by_label("Undo camera edit").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert_eq!(h.state().1, 2);
        h.get_by_label(choice).click();
        h.run_steps(3);
        h.get_by_label("Apply camera").click();
        h.run_steps(3);
        h.get_by_label("Save camera").click();
        h.run_steps(3);
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        assert_eq!(h.state().1, 3); // Save itself does not reset the live view.
        let bytes = std::fs::read(&path).unwrap();
        let reopened = Document::parse(&bytes).unwrap();
        assert_eq!(reopened, edited);
        let manifest_bytes = std::fs::read(&manifest_path).unwrap();
        let panel = Panel::new(PreparedRoomCamera {
            path,
            document: reopened,
            bytes,
            manifest_bytes,
            manifest_path,
        });
        assert_eq!(panel.document(), &edited);
        assert_eq!(panel.document, panel.saved);
    }
    #[test]
    fn invalid_widget_apply_preserves_document_and_reset_is_explicit() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let mut panel = Panel::new(prepared);
        panel.candidate.distance = 0.0;
        let mut h = Harness::builder()
            .with_size(egui::vec2(600.0, 800.0))
            .build_ui_state(
                |ui, state: &mut (Panel, usize)| {
                    state.1 += usize::from(state.0.show(ui, true));
                },
                (panel, 0),
            );
        h.run_steps(3);
        h.get_by_label("Room camera").click();
        h.run_steps(3);
        h.get_by_label("Apply camera").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert_eq!(h.state().1, 0);
        assert!(h.state().0.error.is_some());
        assert!(h.state().0.undo.is_empty());
        h.get_by_label("Reset camera defaults").click();
        h.run_steps(3);
        assert_eq!(h.state().1, 1);
        assert!(h.state().0.error.is_none());
        assert_eq!(h.state().0.candidate, original);
    }
    #[test]
    fn invalid_candidate_is_atomic_and_history_is_bounded() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let original = panel.document().clone();
        let mut invalid = original.clone();
        invalid.distance = f32::NAN;
        assert!(panel.apply_document(invalid).is_err());
        assert_eq!(panel.document(), &original);
        assert!(!panel.undo());
        for n in 0..40 {
            let mut candidate = panel.document().clone();
            candidate.target[0] = n as f32 * 0.01;
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
            changed.distance += 1.0;
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
        edited.distance += 1.0;
        panel.apply_document(edited.clone()).unwrap();
        let original_history = panel.undo.clone();
        let error = panel
            .save_with_check(|stage| {
                assert!(stage == SaveStage::BeforePersist);
                Err(std::io::Error::other("injected pre-persist failure"))
            })
            .unwrap_err();
        assert!(error.contains("Camera not saved"));
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
        edited.distance += 1.0;
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
        assert!(error.contains("Camera replaced"));
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
        changed.distance += 1.0;
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
        use egui_kittest::{Harness, kittest::Queryable};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let path = prepared.path.clone();
        let mut panel = Panel::new(prepared);
        panel.candidate.distance += 1.0;
        let mut h = Harness::builder()
            .with_size(egui::vec2(600.0, 800.0))
            .build_ui_state(
                |ui, panel: &mut Panel| {
                    assert!(!panel.show(ui, false));
                },
                panel,
            );
        h.run_steps(3);
        h.get_by_label("Room camera").click();
        h.run_steps(3);
        h.get_by_label("Apply camera").click();
        h.get_by_label("Save camera").click();
        h.run_steps(3);
        assert_eq!(h.state().document(), &original);
        assert!(h.state().undo.is_empty());
        assert_eq!(
            Document::parse(&std::fs::read(path).unwrap()).unwrap(),
            original
        );
    }
}
