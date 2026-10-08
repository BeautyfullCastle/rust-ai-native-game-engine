//! Explicit metadata-only checkpoint authoring. Editor Play never opens a profile.
use orr_package::{ProgressProfile, ProjectManifest, ProjectProgress};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

pub struct Panel {
    path: PathBuf,
    directory: File,
    root: PathBuf,
    bytes: Vec<u8>,
    document: ProjectManifest,
    saved: ProjectManifest,
    undo: Vec<ProjectManifest>,
    identity: String,
    checkpoint_font: Vec<u8>,
    source: Arc<AtomicU64>,
    epoch: u64,
    error: Option<String>,
}
impl Panel {
    pub fn new(
        project: &orr_sample::room_project::PreparedProject,
        editor: &crate::Editor,
    ) -> Result<Self, String> {
        if !editor.game().is_room()
            || !editor.spec().is_local()
            || editor.path().as_deref() != Some(project.path())
        {
            return Err("Checkpoint authoring requires the admitted local Room scene".into());
        }
        let path = project.root().join("orr.project.json");
        let bytes = read_manifest(&path)?;
        let document: ProjectManifest =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if bytes != project.manifest_bytes() || document.progress.as_ref() != project.checkpoint() {
            return Err("Room manifest changed; reopen the project".into());
        }
        Self::from_parts(
            path,
            bytes,
            document,
            editor.room_ui_source_token(),
            project.ui().map_or_else(Vec::new, |ui| ui.font.clone()),
        )
    }
    fn from_parts(
        path: PathBuf,
        bytes: Vec<u8>,
        document: ProjectManifest,
        source: Arc<AtomicU64>,
        checkpoint_font: Vec<u8>,
    ) -> Result<Self, String> {
        let parent = path.parent().ok_or("Room manifest has no parent")?;
        let root = orr_model_bindings::model_bindings::resolve_project(parent, ".")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let directory = open_directory(&root)?;
        let epoch = source.load(Ordering::SeqCst);
        Ok(Self {
            path,
            directory,
            root,
            bytes,
            saved: document.clone(),
            identity: document
                .progress
                .as_ref()
                .map_or(String::new(), |p| p.game_id.clone()),
            document,
            checkpoint_font,
            source,
            epoch,
            undo: Vec::new(),
            error: None,
        })
    }
    fn active(&self) -> Result<(), String> {
        if self.epoch == u64::MAX || self.source.load(Ordering::SeqCst) != self.epoch {
            return Err("Room checkpoint source retired; reopen the project".into());
        }
        Ok(())
    }
    fn apply(&mut self, progress: Option<ProjectProgress>) -> Result<(), String> {
        self.active()?;
        if let Some(progress) = &progress {
            progress.validate()?;
            if self
                .document
                .entry
                .as_ref()
                .and_then(|e| e.ui.as_ref())
                .is_none()
            {
                return Err("Checkpoint requires an authored Room UI project; create room-escape-ui-3d-v1 first".into());
            }
            orr_sample::room_project::validate_checkpoint_font(&self.checkpoint_font)?;
        }
        let mut candidate = self.document.clone();
        candidate.schema = if progress.is_some() { 4 } else { 2 };
        candidate.progress = progress;
        if candidate != self.document {
            if self.undo.len() == 32 {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
        }
        Ok(())
    }
    fn recheck(&self) -> Result<(), String> {
        self.active()?;
        let root =
            orr_model_bindings::model_bindings::resolve_project(self.path.parent().unwrap(), ".")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
        let current = open_directory(&root)?;
        let metadata = current.metadata().map_err(|e| e.to_string())?;
        if root != self.root
            || !same_directory(
                &metadata,
                &self.directory.metadata().map_err(|e| e.to_string())?,
            )
        {
            return Err("Room project directory changed; reopen before saving".into());
        }
        if read_pinned_manifest(&self.directory)? != self.bytes {
            return Err("Room manifest changed; reopen before saving".into());
        }
        Ok(())
    }
    fn save(&mut self) -> Result<(), String> {
        self.recheck()?;
        let mut bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let staged = Staged::new(&self.directory, &bytes)?;
        self.recheck()?;
        // Descriptor-relative replacement cannot be redirected by parent renames.
        // A malicious writer holding this same directory can still race our stale check.
        staged.commit()?;
        self.bytes = bytes;
        self.saved = self.document.clone();
        #[cfg(unix)]
        self.directory.sync_all().map_err(|e| {
            format!("Manifest replaced but directory sync failed; durability uncertain: {e}")
        })?;
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editable: bool) {
        ui.collapsing("Room checkpoint", |ui| {
            ui.label(if self.document.progress.is_some() {"Key checkpoint enabled"} else {"Key checkpoint disabled"});
            ui.label("Editor Play never reads or writes player profiles. Save changes only project metadata; reopen the project before saving camera or HUD edits.");
            ui.label("Use a new canonical UUIDv4 for an independent game. Reusing an ID shares its checkpoint.");
            let active = self.active().is_ok();
            ui.add_enabled_ui(editable && active, |ui| {
                let response = ui.text_edit_singleline(&mut self.identity);
                response.labelled_by(ui.label("Checkpoint game UUID").id);
                if ui.button("Enable checkpoint").clicked() {
                    self.error = self.apply(Some(ProjectProgress { schema:1, game_id:self.identity.clone(), profile:ProgressProfile::RoomKeyCheckpointV1 })).err();
                }
                if ui.button("Disable checkpoint").clicked() {self.error=self.apply(None).err();}
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo checkpoint edit")).clicked() {
                    if let Some(previous)=self.undo.pop() { self.document=previous; }
                    self.error=None;
                }
                if ui.add_enabled(self.document != self.saved, egui::Button::new("Save checkpoint")).clicked() {self.error=self.save().err();}
            });
            if !editable { ui.label("Checkpoint authoring is read-only during Play"); }
            if !active {ui.label("Room checkpoint source retired; reopen the project");}
            if let Some(error)=&self.error {ui.colored_label(egui::Color32::RED,error);}
        });
    }
}
fn read_manifest(path: &Path) -> Result<Vec<u8>, String> {
    let directory = open_directory(path.parent().ok_or("Manifest has no parent")?)?;
    read_pinned_manifest(&directory)
}
#[cfg(target_os = "linux")]
fn open_directory(path: &Path) -> Result<File, String> {
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::path::Component;
    if !path.is_absolute() {
        return Err("Room directory must be absolute".into());
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(|e| e.to_string())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory =
                    openat(&directory, name, flags, Mode::empty()).map_err(|e| e.to_string())?
            }
            _ => return Err("Room directory contains traversal".into()),
        }
    }
    Ok(File::from(directory))
}
#[cfg(not(target_os = "linux"))]
fn open_directory(_: &Path) -> Result<File, String> {
    Err("Checkpoint authoring currently requires Linux".into())
}
#[cfg(target_os = "linux")]
fn read_pinned_manifest(directory: &File) -> Result<Vec<u8>, String> {
    use rustix::fs::{openat, Mode, OFlags};
    let fd = openat(
        directory,
        "orr.project.json",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| e.to_string())?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("Manifest must be a regular file within 1 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Manifest exceeds 1 MiB".into());
    }
    Ok(bytes)
}
#[cfg(not(target_os = "linux"))]
fn read_pinned_manifest(_: &File) -> Result<Vec<u8>, String> {
    Err("Checkpoint authoring currently requires Linux".into())
}
struct Staged<'a> {
    directory: &'a File,
    name: String,
    committed: bool,
}
impl<'a> Staged<'a> {
    #[cfg(target_os = "linux")]
    fn new(directory: &'a File, bytes: &[u8]) -> Result<Self, String> {
        use rustix::fs::{openat, Mode, OFlags};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let name = format!(
                ".orr-checkpoint-{}-{}.tmp",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let fd = match openat(
                directory,
                name.as_str(),
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let staged = Self {
                directory,
                name,
                committed: false,
            };
            let mut file = File::from(fd);
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            return Ok(staged);
        }
        Err("Checkpoint staging name collision limit exceeded".into())
    }
    #[cfg(not(target_os = "linux"))]
    fn new(_: &'a File, _: &[u8]) -> Result<Self, String> {
        Err("Checkpoint authoring currently requires Linux".into())
    }
    #[cfg(target_os = "linux")]
    fn commit(mut self) -> Result<(), String> {
        rustix::fs::renameat(
            self.directory,
            self.name.as_str(),
            self.directory,
            "orr.project.json",
        )
        .map_err(|e| e.to_string())?;
        self.committed = true;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    fn commit(self) -> Result<(), String> {
        Err("Checkpoint authoring currently requires Linux".into())
    }
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if !self.committed {
            let _ = rustix::fs::unlinkat(
                self.directory,
                self.name.as_str(),
                rustix::fs::AtFlags::empty(),
            );
        }
    }
}
#[cfg(unix)]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    matches!((a.created(),b.created()),(Ok(a),Ok(b)) if a==b)
}

#[cfg(all(test, target_os = "linux", feature = "project-create"))]
mod tests {
    use super::*;
    use egui_kittest::{kittest::Queryable, Harness};
    const UUID: &str = "12345678-1234-4234-9234-123456789abc";
    fn fixture() -> (tempfile::TempDir, Panel) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("room");
        orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
            output: root.clone(),
            template: orr_sample::project_create::ROOM_UI_TEMPLATE.into(),
            seed: "checkpoint-editor".into(),
        })
        .unwrap();
        let path = root.join("orr.project.json");
        let bytes = read_manifest(&path).unwrap();
        let document = serde_json::from_slice(&bytes).unwrap();
        let project = orr_sample::room_project::PreparedProject::open_with_ui(&root, true).unwrap();
        let panel = Panel::from_parts(
            path,
            bytes,
            document,
            Arc::new(AtomicU64::new(0)),
            project.ui().unwrap().font.clone(),
        )
        .unwrap();
        (dir, panel)
    }
    #[test]
    fn widgets_enable_save_undo_reopen_preserve_scene_and_sidecars() {
        let (_dir, mut panel) = fixture();
        let original = panel.bytes.clone();
        let root = panel.root.clone();
        let sidecars: [Vec<u8>; 4] = [
            "room.scene.yaml",
            "room.models.json",
            "room.camera.json",
            "room.ui.json",
        ]
        .map(|p| std::fs::read(root.join(p)).unwrap());
        panel.identity = UUID.into();
        let mut h = Harness::builder()
            .with_size([800.0, 1000.0])
            .build_ui_state(|ui, panel: &mut Panel| panel.show(ui, true), panel);
        h.run_steps(3);
        h.get_by_label("Room checkpoint").click();
        h.run_steps(3);
        h.get_by_label("Enable checkpoint").click();
        h.run_steps(3);
        assert_eq!(read_manifest(&h.state().path).unwrap(), original);
        assert_eq!(h.state().document.progress.as_ref().unwrap().game_id, UUID);
        h.get_by_label("Save checkpoint").click();
        h.run_steps(3);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        let project = orr_sample::room_project::PreparedProject::open_with_options(
            &root,
            true,
            orr_sample::room_project::CheckpointSupport::MetadataOnly,
        )
        .unwrap();
        assert_eq!(project.checkpoint().unwrap().game_id, UUID);
        h.get_by_label("Undo checkpoint edit").click();
        h.run_steps(3);
        assert!(h.state().document.progress.is_none());
        assert_eq!(project.checkpoint().unwrap().game_id, UUID);
        h.get_by_label("Save checkpoint").click();
        h.run_steps(3);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        let reopened =
            orr_sample::room_project::PreparedProject::open_with_ui(&root, true).unwrap();
        assert!(reopened.checkpoint().is_none());
        for (path, bytes) in [
            "room.scene.yaml",
            "room.models.json",
            "room.camera.json",
            "room.ui.json",
        ]
        .into_iter()
        .zip(sidecars)
        {
            assert_eq!(std::fs::read(root.join(path)).unwrap(), bytes);
        }
    }
    #[test]
    fn invalid_identity_stale_manifest_and_retired_source_fail_closed() {
        let (_dir, mut panel) = fixture();
        let original = panel.bytes.clone();
        assert!(panel
            .apply(Some(ProjectProgress {
                schema: 1,
                game_id: "bad".into(),
                profile: ProgressProfile::RoomKeyCheckpointV1
            }))
            .is_err());
        assert_eq!(panel.document, panel.saved);
        panel
            .apply(Some(ProjectProgress {
                schema: 1,
                game_id: UUID.into(),
                profile: ProgressProfile::RoomKeyCheckpointV1,
            }))
            .unwrap();
        std::fs::write(&panel.path, b"external edit").unwrap();
        assert!(panel.save().unwrap_err().contains("changed"));
        assert_eq!(std::fs::read(&panel.path).unwrap(), b"external edit");
        std::fs::write(&panel.path, &original).unwrap();
        panel.source.fetch_add(1, Ordering::SeqCst);
        assert!(panel.save().unwrap_err().contains("retired"));
        assert_eq!(std::fs::read(&panel.path).unwrap(), original);
    }
    #[test]
    fn symlink_and_replaced_directory_cannot_receive_save() {
        let (_dir, mut panel) = fixture();
        panel
            .apply(Some(ProjectProgress {
                schema: 1,
                game_id: UUID.into(),
                profile: ProgressProfile::RoomKeyCheckpointV1,
            }))
            .unwrap();
        let target = panel.root.join("outside.json");
        std::fs::write(&target, &panel.bytes).unwrap();
        std::fs::remove_file(&panel.path).unwrap();
        std::os::unix::fs::symlink(&target, &panel.path).unwrap();
        assert!(panel.save().is_err());
        assert_eq!(std::fs::read(&target).unwrap(), panel.bytes);
        std::fs::remove_file(&panel.path).unwrap();
        std::fs::write(&panel.path, &panel.bytes).unwrap();
        let moved = panel.root.with_extension("old");
        std::fs::rename(&panel.root, &moved).unwrap();
        std::fs::create_dir(&panel.root).unwrap();
        std::fs::write(&panel.path, &panel.bytes).unwrap();
        assert!(panel.save().unwrap_err().contains("directory changed"));
        assert_eq!(std::fs::read(&panel.path).unwrap(), panel.bytes);
    }
    #[test]
    fn fifo_manifest_rejected_without_blocking_and_staged_write_is_pinned() {
        let (_dir, panel) = fixture();
        std::fs::remove_file(&panel.path).unwrap();
        rustix::fs::mknodat(
            &panel.directory,
            "orr.project.json",
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        assert!(read_pinned_manifest(&panel.directory)
            .unwrap_err()
            .contains("regular"));
        std::fs::remove_file(&panel.path).unwrap();
        std::fs::write(&panel.path, &panel.bytes).unwrap();
        let staged = Staged::new(&panel.directory, b"pinned").unwrap();
        let moved = panel.root.with_extension("old");
        std::fs::rename(&panel.root, &moved).unwrap();
        std::fs::create_dir(&panel.root).unwrap();
        std::fs::write(&panel.path, b"replacement").unwrap();
        staged.commit().unwrap();
        assert_eq!(std::fs::read(&panel.path).unwrap(), b"replacement");
        assert_eq!(
            std::fs::read(moved.join("orr.project.json")).unwrap(),
            b"pinned"
        );
    }
    #[test]
    fn play_widgets_are_read_only_and_open_preserves_legacy_bytes() {
        let (_dir, mut panel) = fixture();
        let original = panel.bytes.clone();
        panel.identity = UUID.into();
        let mut h = Harness::builder()
            .with_size([800.0, 1000.0])
            .build_ui_state(|ui, panel: &mut Panel| panel.show(ui, false), panel);
        h.run_steps(3);
        h.get_by_label("Room checkpoint").click();
        h.run_steps(3);
        h.get_by_label("Enable checkpoint").click();
        h.get_by_label("Disable checkpoint").click();
        h.get_by_label("Save checkpoint").click();
        h.run_steps(3);
        assert_eq!(read_manifest(&h.state().path).unwrap(), original);
        assert!(h.state().document.progress.is_none());
    }
    #[test]
    fn unsupported_checkpoint_font_cannot_enable_or_rewrite_legacy_manifest() {
        let (_dir, mut panel) = fixture();
        let original = panel.bytes.clone();
        panel.checkpoint_font.clear();
        let error = panel
            .apply(Some(ProjectProgress {
                schema: 1,
                game_id: UUID.into(),
                profile: ProgressProfile::RoomKeyCheckpointV1,
            }))
            .unwrap_err();
        assert!(error.contains("font"));
        assert_eq!(panel.document, panel.saved);
        assert_eq!(read_manifest(&panel.path).unwrap(), original);
    }
}
