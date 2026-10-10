//! Editor-owned model binding editing, history, and persistence.
//! Read-only descriptors and loading are shared with runtime consumers.

pub use orr_model_bindings::animated_bindings::*;
use orr_reflect::Guid;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_HISTORY: usize = 128;

pub struct Bindings {
    pub path: PathBuf,
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
}
impl Bindings {
    pub fn create(path: PathBuf, scene: String, project: String) -> Result<Self, String> {
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err("sidecar exists; use Open bindings".into());
        }
        let document = Document {
            version: 1,
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
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let initial_metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if initial_metadata.file_type().is_symlink() || !initial_metadata.is_file() {
            return Err(
                "animated binding sidecar must be a regular file, not a symlink or special file"
                    .into(),
            );
        }
        if initial_metadata.len() > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let opened_metadata = file.metadata().map_err(|e| e.to_string())?;
        if !opened_metadata.is_file() {
            return Err("animated binding sidecar changed to a special file while opening".into());
        }
        if opened_metadata.len() > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

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

    /// Assign or remove a selection in one transaction. Assignment requires the exact
    /// immutable verified asset used to build the binding, preventing arbitrary clip slots.
    pub fn assign_validated(
        &mut self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<(), String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        binding.validate(loaded)?;
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.insert(guid.to_string(), binding.clone());
        }
        next.validate()?;
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
        }
        Ok(())
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
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
        }
        Ok(())
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }

    /// Atomic sidecar replacement. State/history become clean only after persist succeeds.
    pub fn save(&mut self) -> Result<(), String> {
        self.document.validate()?;
        let bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
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
