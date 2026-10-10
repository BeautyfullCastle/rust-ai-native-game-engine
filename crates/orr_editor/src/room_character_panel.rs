//! Editor-owned character authoring. Clip edits coordinate the searching binding
//! default; pair saves roll back ordinary second-file failures, not process crashes.
use orr_sample::{
    room_character::{Document, PlaybackRate},
    room_project::PreparedRoomCharacter,
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
    ModelPersist,
    CharacterPersist,
    #[cfg(unix)]
    DirectorySync,
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
    redo: Vec<Document>,
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
            return Err("Character project directory changed; reopen before saving".into());
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
    pub fn new(prepared: PreparedRoomCharacter) -> Self {
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
            redo: Vec::new(),
            error,
        }
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn apply_document(
        &mut self,
        candidate: Document,
        editor: &mut crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
    ) -> Result<(), String> {
        candidate.to_bytes()?;
        models.apply_character_mapping(editor, &candidate)?;
        if candidate != self.document {
            if self.undo.len() == MAX_UNDO {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
            self.redo.clear();
        }
        self.candidate = self.document.clone();
        Ok(())
    }
    pub fn undo(
        &mut self,
        editor: &mut crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
    ) -> Result<bool, String> {
        let Some(previous) = self.undo.last().cloned() else {
            return Ok(false);
        };
        models.apply_character_mapping(editor, &previous)?;
        self.undo.pop();
        self.redo
            .push(std::mem::replace(&mut self.document, previous));
        self.candidate = self.document.clone();
        Ok(true)
    }
    pub fn redo(
        &mut self,
        editor: &mut crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
    ) -> Result<bool, String> {
        let Some(next) = self.redo.last().cloned() else {
            return Ok(false);
        };
        models.apply_character_mapping(editor, &next)?;
        self.redo.pop();
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.candidate = self.document.clone();
        Ok(true)
    }
    fn recheck_disk(&self) -> Result<(), String> {
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        pin.recheck(&self.path)?;
        pin.recheck(&self.manifest_path)?;
        unchanged_file(&self.path, &self.saved_bytes)?;
        unchanged_file(&self.manifest_path, &self.manifest_bytes)
    }
    pub fn save_all(
        &mut self,
        editor: &crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
    ) -> Result<(), String> {
        self.save_with_check(editor, models, |_| Ok(()))
    }
    fn save_with_check(
        &mut self,
        editor: &crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
        mut check: impl FnMut(SaveStage) -> std::io::Result<()>,
    ) -> Result<(), String> {
        models.validate_save(editor)?;
        if editor.room_character() != Some(&self.document) {
            return Err("Character document detached from current editor".into());
        }
        let bindings = models
            .bindings
            .as_ref()
            .ok_or("Room model bindings missing")?;
        let model_path = bindings.path.clone();
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        pin.recheck(&model_path)?;
        let metadata = std::fs::symlink_metadata(&model_path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024
        {
            return Err("Model sidecar must remain a bounded regular file".into());
        }
        let mut original = Vec::new();
        File::open(&model_path)
            .map_err(|e| e.to_string())?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut original)
            .map_err(|e| e.to_string())?;
        if original.len() > 1024 * 1024 {
            return Err("Model sidecar exceeds limit".into());
        }
        let disk: crate::model_bindings::Document =
            serde_json::from_slice(&original).map_err(|e| e.to_string())?;
        if &disk != bindings.saved_document() {
            return Err("Model sidecar changed on disk; reopen before saving".into());
        }
        let model_bytes =
            serde_json::to_vec_pretty(bindings.document()).map_err(|e| e.to_string())?;
        let bytes = self.document.to_bytes()?;
        let write_temp = |bytes: &[u8]| -> Result<tempfile::NamedTempFile, String> {
            let mut file = tempfile::NamedTempFile::new_in(&pin.path).map_err(|e| e.to_string())?;
            file.write_all(bytes)
                .and_then(|()| file.as_file().sync_all())
                .map_err(|e| e.to_string())?;
            Ok(file)
        };
        let model_temp = write_temp(&model_bytes)?;
        let character_temp = write_temp(&bytes)?;
        let rollback = write_temp(&original)?;
        self.recheck_disk()?;
        unchanged_file(&model_path, &original)?;
        check(SaveStage::ModelPersist).map_err(|e| format!("Character not saved: {e}"))?;
        model_temp.persist(&model_path).map_err(|e| e.to_string())?;
        let second = check(SaveStage::CharacterPersist)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                character_temp
                    .persist(&self.path)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = second {
            return match rollback.persist(&model_path) {
                Ok(_) => {
                    let _ = pin.directory.sync_all();
                    Err(format!(
                        "Character save failed; model sidecar restored: {error}"
                    ))
                }
                Err(rollback) => Err(format!(
                    "Character save failed ({error}); model rollback failed ({rollback}); reopen and recover the sidecars before continuing"
                )),
            };
        }
        models.bindings.as_mut().unwrap().mark_current_saved();
        self.saved = self.document.clone();
        self.saved_bytes = bytes;
        #[cfg(unix)]
        check(SaveStage::DirectorySync).and_then(|()|pin.directory.sync_all()).map_err(|e|format!("Character and model sidecars replaced, but directory sync failed; durability is uncertain: {e}"))?;
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut crate::editor::Editor,
        models: &mut crate::model_panel::ModelPanel,
    ) {
        let editable = editor.mode() == crate::model::Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.yard_rows_coherent();
        ui.collapsing("Room character", |ui| {
            ui.label(format!("Player {}", self.document.player));
            ui.label(if self.document != self.saved {
                "Character (unsaved)"
            } else {
                "Character saved"
            });
            ui.label("Loop clips use absolute simulation ticks; Edit/Stop shows rest");
            ui.add_enabled_ui(editable, |ui| {
                for (label, value) in [
                    ("Searching clip", &mut self.candidate.searching),
                    ("Carrying key clip", &mut self.candidate.carrying),
                    ("Escaped clip", &mut self.candidate.escaped),
                ] {
                    egui::ComboBox::from_label(label)
                        .selected_text(format!("Clip {}", *value))
                        .show_ui(ui, |ui| {
                            for clip in 0..32 {
                                ui.selectable_value(value, clip, format!("{label} {clip}"));
                            }
                        });
                }
                let mut speeds = self.candidate.speeds.unwrap_or_default();
                let original_speeds = speeds;
                for (label, value) in [
                    ("Searching speed", &mut speeds.searching),
                    ("Carrying key speed", &mut speeds.carrying),
                    ("Escaped speed", &mut speeds.escaped),
                ] {
                    egui::ComboBox::from_label(label)
                        .selected_text(value.label())
                        .show_ui(ui, |ui| {
                            for rate in PlaybackRate::ALL {
                                ui.selectable_value(
                                    value,
                                    rate,
                                    format!("{label} {}", rate.label()),
                                );
                            }
                        });
                }
                if speeds != original_speeds {
                    self.candidate.schema = 2;
                    self.candidate.speeds = Some(speeds);
                }
                if ui.button("Apply character settings").clicked() {
                    self.error = self
                        .apply_document(self.candidate.clone(), editor, models)
                        .err();
                }
                if ui
                    .add_enabled(
                        !self.undo.is_empty(),
                        egui::Button::new("Undo character edit"),
                    )
                    .clicked()
                {
                    self.error = self.undo(editor, models).err();
                }
                if ui
                    .add_enabled(
                        !self.redo.is_empty(),
                        egui::Button::new("Redo character edit"),
                    )
                    .clicked()
                {
                    self.error = self.redo(editor, models).err();
                }
                if ui.button("Save character and model bindings").clicked() {
                    self.error = self.save_all(editor, models).err();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
    }
}

#[cfg(all(test, feature = "project-create", target_os = "linux"))]
mod tests {
    use super::*;
    use crate::{model_panel::ModelPanel, Editor, HostSpec};
    use orr_sample::{
        project_create::{create, CreateOptions, ROOM_CHARACTER_TEMPLATE},
        room_project::{CheckpointSupport, PreparedProject},
    };
    use std::{fs, time::Duration};

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        editor: Editor,
        models: ModelPanel,
        panel: Panel,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp
                .path()
                .canonicalize()
                .unwrap()
                .join("character project");
            create(&CreateOptions {
                output: root.clone(),
                template: ROOM_CHARACTER_TEMPLATE.into(),
                seed: "character-pair-save".into(),
            })
            .unwrap();
            let mut prepared = PreparedProject::open_with_capabilities(
                &root,
                false,
                CheckpointSupport::Disabled,
                true,
            )
            .unwrap();
            let character = prepared.take_character().unwrap();
            let (_, path, scene, prepared_models) = prepared.into_parts();
            let mut editor = Editor::start(&HostSpec::PreparedRoom {
                scene: path,
                text: scene.text().into(),
                listen: None,
                debug_hooks: false,
            })
            .unwrap();
            editor
                .install_room_character(character.document.clone())
                .unwrap();
            let mut models = ModelPanel::default();
            models.install_room(prepared_models).unwrap();
            for attempt in 0..1600 {
                editor.sync();
                if editor.yard_rows_coherent() && models.scene_matches(&editor) {
                    break;
                }
                assert!(
                    attempt < 1599,
                    "room character panel did not become coherent"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Self {
                _temp: temp,
                root,
                editor,
                models,
                panel: Panel::new(character),
            }
        }
        fn edit_searching(&mut self) -> Document {
            let mut candidate = self.panel.document().clone();
            std::mem::swap(&mut candidate.searching, &mut candidate.carrying);
            self.panel
                .apply_document(candidate.clone(), &mut self.editor, &mut self.models)
                .unwrap();
            assert!(self.models.bindings.as_ref().unwrap().dirty());
            assert_ne!(self.panel.document, self.panel.saved);
            candidate
        }
        fn model_path(&self) -> PathBuf {
            self.models.bindings.as_ref().unwrap().path.clone()
        }
        fn pair_bytes(&self) -> (Vec<u8>, Vec<u8>) {
            (
                fs::read(self.model_path()).unwrap(),
                fs::read(&self.panel.path).unwrap(),
            )
        }
        fn reopen(&self, expected: &Document) {
            let reopened = PreparedProject::open_with_capabilities(
                &self.root,
                false,
                CheckpointSupport::Disabled,
                true,
            )
            .unwrap();
            assert_eq!(&reopened.character().unwrap().document, expected);
            let binding = &reopened.models().document.bindings[&expected.player];
            assert_eq!(binding.animation.unwrap().clip_index, expected.searching);
            assert_eq!(
                binding.animation.unwrap().playback,
                crate::model_bindings::PlaybackMode::Loop
            );
        }
        fn assert_model_history_survives(
            &mut self,
            old: &crate::model_bindings::Document,
            edited: &crate::model_bindings::Document,
        ) {
            // Exercise the real model history, rather than inferring it from
            // dirty state. This permutation has one reversible binding edit.
            let bindings = self.models.bindings.as_mut().unwrap();
            bindings.undo();
            assert_eq!(bindings.document(), old);
            bindings.redo();
            assert_eq!(bindings.document(), edited);
        }
    }

    #[test]
    fn pair_save_second_file_failure_restores_both_files_and_dirty_history_then_retries() {
        for fail_stage in [SaveStage::ModelPersist, SaveStage::CharacterPersist] {
            let mut f = Fixture::new();
            let initial = f.pair_bytes();
            let original = f.panel.document.clone();
            let original_models = f.models.bindings.as_ref().unwrap().document().clone();
            let edited = f.edit_searching();
            let edited_models = f.models.bindings.as_ref().unwrap().document().clone();
            let history = f.panel.undo.clone();
            let checksum = f.editor.snapshot().unwrap().predicted().checksum();
            let mut visited = Vec::new();
            let error = f
                .panel
                .save_with_check(&f.editor, &mut f.models, |stage| {
                    visited.push(stage);
                    if stage == fail_stage {
                        Err(std::io::Error::other("injected pair-save failure"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert!(visited.last() == Some(&fail_stage));
            if fail_stage == SaveStage::CharacterPersist {
                assert!(error.contains("model sidecar restored"), "{error}");
            }
            assert_eq!(f.pair_bytes(), initial);
            assert_eq!(f.panel.saved, original);
            assert_eq!(f.panel.saved_bytes, initial.1);
            assert_eq!(f.panel.document, edited);
            assert_eq!(f.panel.candidate, edited);
            assert_eq!(f.panel.undo, history);
            assert!(f.panel.redo.is_empty());
            assert!(f.models.bindings.as_ref().unwrap().dirty());
            assert_eq!(
                f.models.bindings.as_ref().unwrap().saved_document(),
                &original_models
            );
            f.assert_model_history_survives(&original_models, &edited_models);
            assert_eq!(
                f.editor.snapshot().unwrap().predicted().checksum(),
                checksum
            );
            // Independent model Save must not publish half a searching remap.
            assert!(f.models.save(&f.editor).is_err());
            assert_eq!(f.pair_bytes(), initial);
            f.panel.save_all(&f.editor, &mut f.models).unwrap();
            assert_eq!(f.panel.document, f.panel.saved);
            assert!(!f.models.bindings.as_ref().unwrap().dirty());
            assert_eq!(f.panel.undo, history);
            f.reopen(&edited);
        }
    }

    #[test]
    fn external_manifest_character_and_model_edits_preserve_disk_and_history() {
        for target in ["manifest", "character", "models"] {
            let mut f = Fixture::new();
            let original = f.pair_bytes();
            let original_manifest = fs::read(&f.panel.manifest_path).unwrap();
            let original_models = f.models.bindings.as_ref().unwrap().document().clone();
            let edited = f.edit_searching();
            let edited_models = f.models.bindings.as_ref().unwrap().document().clone();
            let history = f.panel.undo.clone();
            let path = match target {
                "manifest" => f.panel.manifest_path.clone(),
                "character" => f.panel.path.clone(),
                _ => f.model_path(),
            };
            let mut external = fs::read(&path).unwrap();
            if target == "models" {
                let mut document: crate::model_bindings::Document =
                    serde_json::from_slice(&external).unwrap();
                document
                    .bindings
                    .get_mut(&edited.player)
                    .unwrap()
                    .transform
                    .translation[0] += 0.25;
                external = serde_json::to_vec_pretty(&document).unwrap();
            } else {
                // Even semantically identical external edits to the pinned
                // manifest/character bytes require reopening.
                external.push(b' ');
            }
            fs::write(&path, &external).unwrap();
            let before_pair = f.pair_bytes();
            let before_manifest = fs::read(&f.panel.manifest_path).unwrap();
            assert!(
                f.panel.save_all(&f.editor, &mut f.models).is_err(),
                "{target}"
            );
            assert_eq!(fs::read(&path).unwrap(), external);
            assert_eq!(f.pair_bytes(), before_pair);
            assert_eq!(fs::read(&f.panel.manifest_path).unwrap(), before_manifest);
            assert_eq!(f.panel.document, edited);
            assert_eq!(f.panel.undo, history);
            assert!(f.panel.redo.is_empty());
            assert!(f.models.bindings.as_ref().unwrap().dirty());
            assert_ne!(f.panel.document, f.panel.saved);
            f.assert_model_history_survives(&original_models, &edited_models);
            // Restoring the original external baseline lets the same retained
            // transaction retry without rebuilding its undo history.
            fs::write(f.model_path(), &original.0).unwrap();
            fs::write(&f.panel.path, &original.1).unwrap();
            fs::write(&f.panel.manifest_path, &original_manifest).unwrap();
            f.panel.save_all(&f.editor, &mut f.models).unwrap();
            f.reopen(&edited);
        }
    }

    #[test]
    fn directory_sync_failure_advances_both_baselines_and_retry_keeps_history() {
        let mut f = Fixture::new();
        let edited = f.edit_searching();
        let history = f.panel.undo.clone();
        let mut visited = Vec::new();
        let error = f
            .panel
            .save_with_check(&f.editor, &mut f.models, |stage| {
                visited.push(stage);
                if stage == SaveStage::DirectorySync {
                    Err(std::io::Error::other("injected directory sync failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(
            visited
                == [
                    SaveStage::ModelPersist,
                    SaveStage::CharacterPersist,
                    SaveStage::DirectorySync
                ]
        );
        assert!(error.contains("durability is uncertain"), "{error}");
        assert_eq!(f.panel.saved, edited);
        assert_eq!(f.panel.saved_bytes, fs::read(&f.panel.path).unwrap());
        assert!(!f.models.bindings.as_ref().unwrap().dirty());
        assert_eq!(f.panel.undo, history);
        f.reopen(&edited);
        f.panel.save_all(&f.editor, &mut f.models).unwrap();
        assert_eq!(f.panel.undo, history);
        assert!(f.panel.undo(&mut f.editor, &mut f.models).unwrap());
        assert_ne!(f.panel.document, f.panel.saved);
        assert!(f.models.bindings.as_ref().unwrap().dirty());
    }

    #[test]
    fn replaced_project_directory_refuses_both_sidecar_writes() {
        fn copy_tree(from: &Path, to: &Path) {
            fs::create_dir(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    assert!(entry.file_type().unwrap().is_file());
                    fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let mut f = Fixture::new();
        let edited = f.edit_searching();
        let history = f.panel.undo.clone();
        let before = f.pair_bytes();
        let moved = f.root.with_file_name("moved original");
        fs::rename(&f.root, &moved).unwrap();
        copy_tree(&moved, &f.root);
        // The replacement has byte-identical valid packages and documents,
        // so rejection must retain the pinned original directory identity.
        let error = f.panel.save_all(&f.editor, &mut f.models).unwrap_err();
        assert!(error.contains("directory changed"), "{error}");
        assert_eq!(f.pair_bytes(), before);
        assert_eq!(f.panel.document, edited);
        assert_eq!(f.panel.undo, history);
        assert!(f.models.bindings.as_ref().unwrap().dirty());
        assert_ne!(f.panel.document, f.panel.saved);
    }
}
