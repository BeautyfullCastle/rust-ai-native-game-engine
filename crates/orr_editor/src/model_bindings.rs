//! Editor-owned model binding editing, history, and persistence.
//! Read-only descriptors and loading are shared with runtime consumers.

pub use orr_model_bindings::model_bindings::*;
use orr_reflect::Guid;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_HISTORY: usize = 128;

/// A validated candidate tied to one exact document revision. Preparing never
/// mutates state; commit cannot overwrite edits made after preparation.
pub struct PreparedAssignment {
    generation: Arc<()>,
    path: PathBuf,
    next: Document,
}

pub struct Bindings {
    pub path: PathBuf,
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    generation: Arc<()>,
}
impl Bindings {
    /// Restore already validated project bytes; no filesystem reread.
    pub fn from_document(path: PathBuf, mut document: Document) -> Result<Self,String> {
        document.validate()?;
        document.version=2;
        Ok(Self { path, saved:document.clone(),document,undo:Vec::new(),redo:Vec::new(),generation:Arc::new(()) })
    }
    pub fn create(path: PathBuf, scene: String, project: String) -> Result<Self, String> {
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err("sidecar exists; use Open bindings".into());
        }
        let document = Document {
            version: 2,
            scene,
            project,
            bindings: BTreeMap::new(),
        };
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            generation: Arc::new(()),
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let initial_metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if initial_metadata.file_type().is_symlink() || !initial_metadata.is_file() {
            return Err(
                "model binding sidecar must be a regular file, not a symlink or special file"
                    .into(),
            );
        }
        if initial_metadata.len() > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let opened_metadata = file.metadata().map_err(|e| e.to_string())?;
        if !opened_metadata.is_file() {
            return Err("model binding sidecar changed to a special file while opening".into());
        }
        if opened_metadata.len() > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let mut document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        // Legacy v1 sidecars contain only explicitly static bindings. Keep the
        // in-memory document at the current version; the next save writes v2.
        document.version = 2;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            generation: Arc::new(()),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    #[cfg(feature="room-character")]
    pub(crate) fn saved_document(&self) -> &Document { &self.saved }
    #[cfg(feature="room-character")]
    pub(crate) fn mark_current_saved(&mut self) { self.saved = self.document.clone(); }
    pub fn dirty(&self) -> bool {
        self.document != self.saved || !self.path.exists()
    }

    pub fn base(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    /// Only compare the sidecar's local scene hint with a caller-provided local path.
    /// The editor must not pass a path received from a remote host here.
    pub fn matches_scene(&self, scene: &str) -> bool {
        match (
            std::fs::canonicalize(self.base().join(&self.document.scene)),
            std::fs::canonicalize(scene),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Resolve only after matching the sidecar to a trusted local scene path.
    /// A remote host's scene path is never authority for local package reads.
    pub fn project_root(&self) -> Result<PathBuf, String> {
        resolve_project(self.base(), &self.document.project)
    }

    /// Validate a whole selection without mutating document, history, or dirtiness.
    /// The caller remains responsible for confirming GUIDs exist in its local scene.
    pub fn prepare_assignment(
        &self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<PreparedAssignment, String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        binding.validate(loaded)?;
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.insert(guid.to_string(), binding.clone());
        }
        next.validate()?;
        Ok(PreparedAssignment {
            generation: Arc::clone(&self.generation),
            path: self.path.clone(),
            next,
        })
    }

    pub fn commit_assignment(&mut self, prepared: PreparedAssignment) -> Result<(), String> {
        if !Arc::ptr_eq(&self.generation, &prepared.generation) || self.path != prepared.path {
            return Err(
                "model bindings changed after preparation; prepare the assignment again".into(),
            );
        }
        self.commit(prepared.next);
        Ok(())
    }

    /// Assignment is one validated transaction using immutable verified model data.
    pub fn assign_validated(
        &mut self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<(), String> {
        let prepared = self.prepare_assignment(guids, binding, loaded)?;
        self.commit_assignment(prepared)
    }

    fn commit(&mut self, next: Document) {
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
            self.generation = Arc::new(());
        }
    }

    /// Remove a selection in one transactional undo step.
    pub fn remove(&mut self, guids: &[Guid]) -> Result<(), String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.remove(&guid.to_string());
        }
        self.commit(next);
        Ok(())
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.generation = Arc::new(());
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.generation = Arc::new(());
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }

    /// Atomic sidecar replacement. State/history become clean only after persist succeeds.
    pub fn save(&mut self) -> Result<(), String> {
        self.document.validate()?;
        let bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(self.base()).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        self.saved = self.document.clone();
        Ok(())
    }
}
