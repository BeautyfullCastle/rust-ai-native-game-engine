//! Read-only Windows snapshot. No manifest-directed paths, writes or device opens.
use super::{admit, AdmittedBank, CUES};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Read,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;

fn error(e: impl std::fmt::Display) -> String {
    format!("audio bank: {e}")
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut value = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: live File owns this handle; output has the exact Win32 layout.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut value) } == 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    Ok(value)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snapshot {
    volume: u32,
    high: u32,
    low: u32,
    size_high: u32,
    size_low: u32,
    written_high: u32,
    written_low: u32,
    links: u32,
    attributes: u32,
}
impl Snapshot {
    fn of(file: &File) -> Result<Self, String> {
        let info = info(file)?;
        Ok(Self {
            volume: info.dwVolumeSerialNumber,
            high: info.nFileIndexHigh,
            low: info.nFileIndexLow,
            size_high: info.nFileSizeHigh,
            size_low: info.nFileSizeLow,
            written_high: info.ftLastWriteTime.dwHighDateTime,
            written_low: info.ftLastWriteTime.dwLowDateTime,
            links: info.nNumberOfLinks,
            attributes: info.dwFileAttributes,
        })
    }
    fn regular(&self, file: &File, max: usize) -> Result<(), String> {
        // SAFETY: GetFileType only inspects the live handle.
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
            || self.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || self.links != 1
            || self.size_high != 0
            || self.size_low as usize > max
        {
            return Err(error(
                "expected a single-link bounded regular file without reparse points",
            ));
        }
        Ok(())
    }
}
fn input_parts(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
    let text = path
        .to_str()
        .ok_or_else(|| error("directory path must be Unicode"))?;
    let bytes = text.as_bytes();
    if bytes.len() < 4
        || bytes.len() > 240
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
    {
        return Err(error(
            "use a non-root absolute local drive directory of at most 240 bytes",
        ));
    }
    let mut parts = Vec::new();
    for part in text[3..].split('\\') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if part.is_empty()
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"/|?*".contains(c))
            || reserved
        {
            return Err(error(
                "directory path contains a reserved or ambiguous component",
            ));
        }
        parts.push(part.to_owned());
    }
    Ok((PathBuf::from(&text[..3]), parts))
}
fn open_directory(path: &Path) -> Result<File, String> {
    // List access is essential: metadata-only handles do not reserve sharing access.
    let file = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(error)?;
    let attributes = info(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(error("directory traversal cannot include reparse points"));
    }
    Ok(file)
}
fn volume_path(file: &File) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    // SAFETY: live handle and bounded writable UTF-16 output. GUID form avoids drive remapping.
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            VOLUME_NAME_GUID,
        )
    } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(error("could not resolve a stable local volume path"));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..count],
    )))
}
struct Directory {
    path: PathBuf,
    anchors: Vec<File>,
}
impl Directory {
    fn open(path: &Path) -> Result<Self, String> {
        let (root, parts) = input_parts(path)?;
        let root = open_directory(&root)?;
        let mut path = volume_path(&root)?;
        let root_name = wide(&path);
        // SAFETY: NUL-terminated volume GUID root points to the already opened root volume.
        if unsafe { GetDriveTypeW(root_name.as_ptr()) } != 3 {
            return Err(error("bank requires a fixed local NTFS drive"));
        }
        let mut filesystem = [0u16; 32];
        // SAFETY: live root handle, correctly sized output; unused outputs are null.
        let ok = unsafe {
            GetVolumeInformationByHandleW(
                root.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        };
        if ok == 0 || filesystem[..5] != [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0] {
            return Err(error("bank requires a fixed local NTFS drive"));
        }
        let mut anchors = vec![root];
        // From the root onwards even ancestry traversal uses the pinned volume, not a drive letter.
        for part in parts {
            path.push(part);
            anchors.push(open_directory(&path)?);
        }
        Ok(Self { path, anchors })
    }
    fn recheck(&self) -> Result<(), String> {
        for anchor in &self.anchors {
            let attributes = info(anchor)?.dwFileAttributes;
            if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || attributes & FILE_ATTRIBUTE_DIRECTORY == 0
            {
                return Err(error("directory attributes changed during admission"));
            }
        }
        Ok(())
    }
}
struct Input {
    file: File,
    snapshot: Snapshot,
}
fn read(directory: &Directory, name: &str, max: usize) -> Result<(Input, Vec<u8>), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory.path.join(name))
        .map_err(error)?;
    let snapshot = Snapshot::of(&file)?;
    snapshot.regular(&file, max)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > max
        || bytes.len() != snapshot.size_low as usize
        || Snapshot::of(&file)? != snapshot
    {
        return Err(error("input changed during bounded snapshot"));
    }
    Ok((Input { file, snapshot }, bytes))
}
pub(super) fn load(path: &Path) -> Result<AdmittedBank, String> {
    load_with(path, |_| Ok(()))
}
fn load_with(
    path: &Path,
    mut hook: impl FnMut(&str) -> Result<(), String>,
) -> Result<AdmittedBank, String> {
    let directory = Directory::open(path)?;
    let (manifest_file, manifest) = read(&directory, "bank.json", 128 * 1024)?;
    let mut pinned = vec![manifest_file];
    hook("bank.json")?;
    let mut objects = BTreeMap::new();
    for cue in CUES {
        let name = format!("{cue}.pcm");
        let (file, bytes) = read(&directory, &name, 96008)?;
        pinned.push(file);
        objects.insert(name.clone(), bytes);
        hook(&name)?;
    }
    directory.recheck()?;
    for input in &pinned {
        if Snapshot::of(&input.file)? != input.snapshot {
            return Err(error("input attributes changed before admission"));
        }
    }
    // Reservations remain live throughout the shared strict JSON/PCM/digest admission.
    let result = admit(&manifest, &objects);
    drop(pinned);
    drop(directory);
    result
}
#[cfg(test)]
#[path = "coop_audio_windows_tests.rs"]
mod tests;
