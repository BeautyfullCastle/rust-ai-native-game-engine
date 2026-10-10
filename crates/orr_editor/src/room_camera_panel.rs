//! Editor-owned camera authoring. Camera history and saves are independent of
//! scene/model transactions and transient viewport navigation.
use orr_sample::{
    room_camera::{Document, Follow, Projection},
    room_project::PreparedRoomCamera,
};
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
    /// A stale panel never silently binds to a later source, even if the camera
    /// document happens to return to the same value.
    Retired,
}

/// Narrow boundary for the production editor and focused widget tests.
trait CameraEditor {
    fn room_camera_document(&self) -> Option<&Document>;
    fn room_camera_source_token(&self) -> Arc<()>;
    fn validate_room_camera_document(&self, document: &Document) -> Result<(), String>;
    fn room_camera_player_guid(&self) -> Result<String, String>;
    fn install_room_camera(&mut self, document: Document) -> Result<(), String>;
    fn camera_authoring_editable(&self) -> bool;
}

impl CameraEditor for crate::editor::Editor {
    fn room_camera_document(&self) -> Option<&Document> {
        crate::editor::Editor::room_camera_document(self)
    }
    fn room_camera_source_token(&self) -> Arc<()> {
        crate::editor::Editor::room_camera_source_token(self)
    }
    fn validate_room_camera_document(&self, document: &Document) -> Result<(), String> {
        crate::editor::Editor::validate_room_camera_document(self, document)
    }
    fn room_camera_player_guid(&self) -> Result<String, String> {
        crate::editor::Editor::room_camera_player_guid(self)
    }
    fn install_room_camera(&mut self, document: Document) -> Result<(), String> {
        crate::editor::Editor::install_room_camera(self, document)
    }
    fn camera_authoring_editable(&self) -> bool {
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
            redo: Vec::new(),
            source: SourcePin::Unbound,
            error,
        }
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn apply_document(&mut self, candidate: Document) -> Result<(), String> {
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
    pub fn undo(&mut self) -> bool {
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
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            push_bounded(&mut self.undo, std::mem::replace(&mut self.document, next));
            self.candidate = self.document.clone();
            true
        } else {
            false
        }
    }
    fn ensure_source(&mut self, editor: &dyn CameraEditor) -> Result<(), String> {
        match &self.source {
            SourcePin::Retired => {
                return Err("Camera source changed; reopen the complete project".into());
            }
            SourcePin::Unbound => {
                let token = editor.room_camera_source_token();
                let matches = editor.room_camera_document() == Some(&self.document);
                if !matches {
                    self.source = SourcePin::Retired;
                    return Err(
                        "Camera panel does not match the installed Room camera; reopen the complete project".into(),
                    );
                }
                let current_token = editor.room_camera_source_token();
                if !Arc::ptr_eq(&token, &current_token)
                    || editor.room_camera_document() != Some(&self.document)
                {
                    self.source = SourcePin::Retired;
                    return Err("Camera source changed; reopen the complete project".into());
                }
                self.source = SourcePin::Pinned(token);
            }
            SourcePin::Pinned(pin) => {
                if !Arc::ptr_eq(pin, &editor.room_camera_source_token())
                    || editor.room_camera_document() != Some(&self.document)
                {
                    self.source = SourcePin::Retired;
                    return Err("Camera source changed; reopen the complete project".into());
                }
            }
        }

        let pin = match &self.source {
            SourcePin::Pinned(pin) => Arc::clone(pin),
            SourcePin::Unbound | SourcePin::Retired => unreachable!("source was bound above"),
        };
        editor.validate_room_camera_document(&self.document)?;
        if !Arc::ptr_eq(&pin, &editor.room_camera_source_token())
            || editor.room_camera_document() != Some(&self.document)
        {
            self.source = SourcePin::Retired;
            return Err("Camera source changed; reopen the complete project".into());
        }
        Ok(())
    }
    fn validate_candidate(
        &mut self,
        editor: &dyn CameraEditor,
        candidate: &Document,
    ) -> Result<(), String> {
        self.ensure_source(editor)?;
        candidate.to_bytes()?;
        editor.validate_room_camera_document(candidate)?;
        // The editor validates against a fresh coherent snapshot; this second
        // fence ensures that snapshot still belongs to the panel's source.
        self.ensure_source(editor)
    }
    fn apply_checked(
        &mut self,
        editor: &mut dyn CameraEditor,
        candidate: Document,
    ) -> Result<(), String> {
        self.validate_candidate(editor, &candidate)?;
        editor.install_room_camera(candidate.clone())?;
        self.accept_document(candidate);
        Ok(())
    }
    fn undo_checked(&mut self, editor: &mut dyn CameraEditor) -> Result<bool, String> {
        let Some(previous) = self.undo.last().cloned() else {
            return Ok(false);
        };
        self.validate_candidate(editor, &previous)?;
        editor.install_room_camera(previous.clone())?;
        let _ = self.undo.pop();
        push_bounded(
            &mut self.redo,
            std::mem::replace(&mut self.document, previous),
        );
        self.candidate = self.document.clone();
        Ok(true)
    }
    fn redo_checked(&mut self, editor: &mut dyn CameraEditor) -> Result<bool, String> {
        let Some(next) = self.redo.last().cloned() else {
            return Ok(false);
        };
        self.validate_candidate(editor, &next)?;
        editor.install_room_camera(next.clone())?;
        let _ = self.redo.pop();
        push_bounded(&mut self.undo, std::mem::replace(&mut self.document, next));
        self.candidate = self.document.clone();
        Ok(true)
    }
    fn save_checked(&mut self, editor: &dyn CameraEditor) -> Result<(), String> {
        self.ensure_source(editor)?;
        editor.validate_room_camera_document(&self.document)?;
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
        self.show_impl(ui, editable, None)
    }

    /// Production editor path. It binds this panel to one camera source lifetime,
    /// and every state-changing operation is validated against the editor's
    /// current coherent Room snapshot before it can update either side.
    pub fn show_for_editor(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut crate::editor::Editor,
    ) -> bool {
        let editable = editor.camera_authoring_editable();
        self.show_impl(ui, editable, Some(editor))
    }

    fn show_impl(
        &mut self,
        ui: &mut egui::Ui,
        editable: bool,
        mut editor: Option<&mut dyn CameraEditor>,
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
        ui.collapsing("Room camera", |ui| {
            ui.label(if self.document != self.saved {
                "Camera (unsaved)"
            } else {
                "Camera saved"
            });
            if !editable {
                ui.label("Camera authoring is read-only during Play or for a remote project");
            } else if !source_ok {
                ui.label("Camera source is no longer available for authoring; reopen the complete project");
            }
            ui.add_enabled_ui(can_edit, |ui| {
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

                let mut follow_player = self.candidate.follow.is_some();
                if ui.checkbox(&mut follow_player, "Follow PLAYER").changed() {
                    if follow_player {
                        let player = editor
                            .as_deref()
                            .ok_or_else(|| "Follow requires an active Room editor".to_string())
                            .and_then(|editor| editor.room_camera_player_guid());
                        match player {
                            Ok(player) => {
                                self.candidate.schema = 2;
                                self.candidate.follow = Some(Follow {
                                    player,
                                    offset: [0.0; 3],
                                });
                                self.error = None;
                            }
                            Err(error) => {
                                self.error = Some(error);
                            }
                        }
                    } else {
                        self.candidate.schema = 1;
                        self.candidate.follow = None;
                        self.error = None;
                    }
                }
                if let Some(follow) = &mut self.candidate.follow {
                    ui.label(format!("Following PLAYER {}", follow.player));
                    ui.label("Pan is disabled while following; orbit and zoom remain session-only");
                    for (axis, value) in ["X", "Y", "Z"].into_iter().zip(&mut follow.offset) {
                        number_bounded(ui, &format!("Follow offset {axis}"), value, -16.0, 16.0);
                    }
                }

                if ui.button("Apply camera").clicked() {
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
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo camera edit"))
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
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo camera edit"))
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
                if ui.button("Reset camera defaults").clicked() {
                    let defaults = Document::readable_default();
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
                if ui.button("Save camera").clicked() {
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

fn number(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.05))
            .labelled_by(label.id);
    });
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

    struct MockCameraEditor {
        document: Option<Document>,
        player: String,
        source: Arc<()>,
        fail_validation: bool,
        fail_install: bool,
    }

    impl MockCameraEditor {
        fn new(document: Document) -> Self {
            Self {
                document: Some(document),
                player: "e_0000002a".into(),
                source: Arc::new(()),
                fail_validation: false,
                fail_install: false,
            }
        }
    }

    impl CameraEditor for MockCameraEditor {
        fn room_camera_document(&self) -> Option<&Document> {
            self.document.as_ref()
        }
        fn room_camera_source_token(&self) -> Arc<()> {
            Arc::clone(&self.source)
        }
        fn validate_room_camera_document(&self, document: &Document) -> Result<(), String> {
            if self.fail_validation {
                return Err("mock coherent snapshot rejected camera".into());
            }
            document.to_bytes()?;
            if document
                .follow
                .as_ref()
                .is_some_and(|follow| follow.player != self.player)
            {
                return Err("mock PLAYER GUID mismatch".into());
            }
            Ok(())
        }
        fn room_camera_player_guid(&self) -> Result<String, String> {
            Ok(self.player.clone())
        }
        fn install_room_camera(&mut self, document: Document) -> Result<(), String> {
            if self.fail_install {
                return Err("mock installation rejected camera".into());
            }
            self.document = Some(document);
            Ok(())
        }
        fn camera_authoring_editable(&self) -> bool {
            true
        }
    }

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
        use egui_kittest::{kittest::Queryable, Harness};
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
    fn follow_widgets_apply_redo_save_and_reopen() {
        use egui::accesskit::Role;
        use egui_kittest::{kittest::Queryable, Harness};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let path = prepared.path.clone();
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 1100.0))
            .build_ui_state(
                |ui, state: &mut (Panel, MockCameraEditor, usize)| {
                    state.2 += usize::from(state.0.show_impl(ui, true, Some(&mut state.1)));
                },
                (
                    Panel::new(prepared),
                    MockCameraEditor::new(original.clone()),
                    0,
                ),
            );
        h.run_steps(3);
        h.get_by_label("Room camera").click();
        h.run_steps(3);
        h.get_by_label("Follow PLAYER").click();
        h.run_steps(3);
        assert_eq!(
            h.state().0.candidate.follow,
            Some(Follow {
                player: "e_0000002a".into(),
                offset: [0.0; 3],
            })
        );
        // Edit an actual bounded DragValue through its accessible spin button.
        h.get_by_role_and_label(Role::SpinButton, "Follow offset X")
            .focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        for character in "1.25".chars() {
            h.get_by_role_and_label(Role::SpinButton, "Follow offset X")
                .type_text(&character.to_string());
            h.run_steps(1);
        }
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(
            h.state().0.candidate.follow.as_ref().unwrap().offset[0],
            1.25
        );

        h.get_by_label("Apply camera").click();
        h.run_steps(3);
        let edited = h.state().0.document().clone();
        assert_eq!(edited.schema, 2);
        assert_eq!(
            edited.follow,
            Some(Follow {
                player: "e_0000002a".into(),
                offset: [1.25, 0.0, 0.0],
            })
        );
        assert_eq!(edited.target, original.target);
        assert_eq!(edited.yaw, original.yaw);
        assert_eq!(edited.pitch, original.pitch);
        assert_eq!(edited.distance, original.distance);
        assert_eq!(edited.projection, original.projection);
        assert_eq!(h.state().1.document.as_ref(), Some(&edited));
        assert_eq!(h.state().2, 1);

        h.get_by_label("Undo camera edit").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert_eq!(h.state().1.document.as_ref(), Some(&original));
        assert_eq!(h.state().2, 2);
        h.get_by_label("Redo camera edit").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &edited);
        assert_eq!(h.state().1.document.as_ref(), Some(&edited));
        assert_eq!(h.state().2, 3);

        h.get_by_label("Save camera").click();
        h.run_steps(3);
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        assert_eq!(h.state().2, 3, "Save does not reset the live view");
        let reopened = Document::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reopened, edited);
    }

    #[test]
    fn checked_apply_and_source_retirement_preserve_panel_history() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (_dir, prepared) = fixture();
        let original = prepared.document.clone();
        let mut panel = Panel::new(prepared);
        let mut editor = MockCameraEditor::new(original.clone());
        let mut candidate = original.clone();
        candidate.distance += 2.0;
        panel.candidate = candidate.clone();
        editor.fail_install = true;
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 900.0))
            .build_ui_state(
                |ui, state: &mut (Panel, MockCameraEditor)| {
                    state.0.show_impl(ui, true, Some(&mut state.1));
                },
                (panel, editor),
            );
        h.run_steps(3);
        h.get_by_label("Room camera").click();
        h.run_steps(3);
        h.get_by_label("Apply camera").click();
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert_eq!(h.state().1.document.as_ref(), Some(&original));
        assert!(h.state().0.undo.is_empty());
        assert!(h.state().0.redo.is_empty());
        assert!(h.state().0.error.is_some());

        // A retired source cannot be rebound, even if it later presents the
        // exact same document bytes again.
        h.state_mut().1.fail_install = false;
        h.state_mut().1.source = Arc::new(());
        h.run_steps(3);
        assert_eq!(h.state().0.document(), &original);
        assert!(h.state().0.undo.is_empty());
        assert!(h.state().0.redo.is_empty());
        assert!(h
            .state()
            .0
            .error
            .as_deref()
            .is_some_and(|e| e.contains("source changed")));
    }
    #[test]
    fn invalid_widget_apply_preserves_document_and_reset_is_explicit() {
        use egui_kittest::{kittest::Queryable, Harness};
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
        use egui_kittest::{kittest::Queryable, Harness};
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
