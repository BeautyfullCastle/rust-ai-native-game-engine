//! Bounded local staging primitive, not a validated exporter or smoke receipt.
//! Trusted NTFS directories only. No process execution or permission changes.
//! Failed/dropped stages are retained; this module never recursively removes files.
use crate::stormglass_windows_admission::{LAUNCHER, LAUNCHER_NAME, RUNTIME_NAME, SCENE_NAME};
use crate::stormglass_windows_snapshot::{Directory, ReservedInputs};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;

fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: valid owned handle and correctly sized output.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(info)
}
fn ordinary(file: &File, directory: bool) -> Result<(), String> {
    let i = info(file)?;
    if i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && i.nNumberOfLinks != 1)
    {
        return Err("stage requires ordinary single-link files and non-reparse directory".into());
    }
    Ok(())
}
/// Held local files, explicitly unvalidated for execution. Dropping only releases
/// handles. Source reservations are borrowed through this value's lifetime;
/// later source/output reopening is outside this contract.
pub struct UnvalidatedStage<'a> {
    inputs: &'a ReservedInputs,
    pin: String,
    parent: Directory,
    directory: File,
    path: PathBuf,
    files: Vec<(File, Vec<u8>)>,
}
impl<'a> UnvalidatedStage<'a> {
    /// Capture only admitted fixed runtime/scene/embedded launcher bytes into a
    /// unique retained stage. Parent must already exist in a trusted directory.
    /// Creation-to-open and writer-to-reader transitions are not hostile-directory
    /// transactions. Exact readback detects ordinary substitution/content changes.
    pub fn prepare(inputs: &'a ReservedInputs, pin: &str, parent: &Path) -> Result<Self, String> {
        Self::prepare_with(inputs, pin, parent, |_| Ok(()))
    }
    fn prepare_with(
        inputs: &'a ReservedInputs,
        pin: &str,
        parent: &Path,
        hook: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<Self, String> {
        let content = inputs.admit(pin)?;
        let parent = Directory::open(parent)?;
        // Keep immediately: no automatic cleanup can delete substituted contents.
        let path = tempfile::Builder::new()
            .prefix(".stormglass-unvalidated-")
            .tempdir_in(&parent.path)
            .map_err(|e| e.to_string())?
            .keep();
        let prepare = || -> Result<Self, String> {
            let directory = OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&path)
                .map_err(|e| e.to_string())?;
            ordinary(&directory, true)?;
            let mut result = Self {
                inputs,
                pin: pin.into(),
                parent,
                directory,
                path: path.clone(),
                files: Vec::new(),
            };
            hook(&result.path)?;
            for (name, bytes) in [
                (RUNTIME_NAME, content.runtime()),
                (SCENE_NAME, content.scene()),
                (LAUNCHER_NAME, LAUNCHER),
            ] {
                let path = result.path.join(name);
                let mut writer = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .share_mode(0)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                writer.write_all(bytes).map_err(|e| e.to_string())?;
                writer.sync_all().map_err(|e| e.to_string())?;
                drop(writer);
                let file = OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                result.files.push((file, bytes.to_vec()));
            }
            result.recheck()?;
            Ok(result)
        };
        prepare().map_err(|e| format!("{e}; retained stage: {}", path.display()))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Recheck reserved source/leaf bytes and the current exact three-name inventory.
    /// This is a point-in-time check: directory sharing does not prevent new children.
    /// Trusted/cooperating writers must leave the stage alone. No execution readiness
    /// or atomic inventory guarantee is implied.
    pub fn recheck(&mut self) -> Result<(), String> {
        self.parent.recheck()?;
        self.inputs.admit(&self.pin)?;
        ordinary(&self.directory, true)?;
        let actual: BTreeSet<_> = fs::read_dir(&self.path)
            .map_err(|e| e.to_string())?
            .take(4)
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let expected = [RUNTIME_NAME, SCENE_NAME, LAUNCHER_NAME]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect();
        if actual != expected {
            return Err("stage closure changed".into());
        }
        for (file, expected) in &mut self.files {
            ordinary(file, false)?;
            file.rewind().map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(expected.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if &bytes != expected {
                return Err("stage bytes changed".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "stormglass_windows_stage_tests.rs"]
mod tests;
