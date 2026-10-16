use std::{
    fs::{self, File, Metadata, OpenOptions},
    io,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf, Prefix},
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_UNITS: usize = 32_760;
const MAX_COMPONENTS: usize = 64;
const MAX_LEAF_BYTES: usize = 200;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
}

pub(super) fn destination(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("output path must be Unicode")?;
    if text.len() > MAX_PATH_BYTES
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || !ordinary_disk_absolute(path)
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("output must be a bounded absolute disk path without traversal".into());
    }

    let components: Vec<_> = path.components().collect();
    if components.len() > MAX_COMPONENTS {
        return Err("output path has too many components".into());
    }
    let leaf = path
        .file_name()
        .ok_or("output needs a new directory leaf")?;
    let leaf_text = leaf.to_str().ok_or("output leaf must be Unicode")?;
    if leaf_text.len() > MAX_LEAF_BYTES {
        return Err("output leaf exceeds 200 bytes".into());
    }
    let parent = path.parent().ok_or("output requires an existing parent")?;
    for component in components.iter().skip(2) {
        if let Component::Normal(name) = component {
            let name = name.to_str().ok_or("output component must be Unicode")?;
            validate_component(name)?;
        }
    }

    let mut current = PathBuf::new();
    for component in parent.components() {
        if !matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::Normal(_)
        ) {
            return Err("invalid output parent component".into());
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("output parent {}: {error}", current.display()))?;
        if !is_directory(&metadata) {
            return Err("output parent must contain only existing non-reparse directories".into());
        }
    }
    let canonical_parent = fs::canonicalize(parent).map_err(error)?;
    let output = canonical_parent.join(leaf);
    match fs::symlink_metadata(&output) {
        Ok(_) => Err("output destination already exists".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(output),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn publish(stage: &Path, output: &Path) -> Result<(), String> {
    let stage = wide_path(stage)?;
    let output = wide_path(output)?;
    let result = unsafe { MoveFileExW(stage.as_ptr(), output.as_ptr(), 0) };
    if result == 0 {
        return Err(format!(
            "native no-replace publication failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub(super) fn open_read(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(error)?;
    if !is_regular(&file.metadata().map_err(error)?) {
        return Err("expected a regular non-reparse file".into());
    }
    Ok(file)
}

pub(super) fn is_regular(metadata: &Metadata) -> bool {
    metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

pub(super) fn is_directory(metadata: &Metadata) -> bool {
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn ordinary_disk_absolute(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        components.next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ) && matches!(components.next(), Some(Component::RootDir))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn validate_component(component: &str) -> Result<(), String> {
    if component.is_empty()
        || component.ends_with('.')
        || component.ends_with(' ')
        || component.contains(':')
        || component.chars().any(char::is_control)
        || reserved_device_name(component)
    {
        return Err("output path contains a reserved or non-portable component".into());
    }
    Ok(())
}

fn reserved_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' ']);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        let Some(suffix) = upper.strip_prefix(prefix) else {
            return false;
        };
        suffix.chars().count() == 1
            && suffix
                .chars()
                .next()
                .is_some_and(|character| matches!(character, '1'..='9' | '¹' | '²' | '³'))
    })
}

fn wide_path(path: &Path) -> Result<Vec<u16>, String> {
    let text = path.to_str().ok_or("filesystem path must be Unicode")?;
    if text.contains('\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("filesystem path must be absolute and normalized".into());
    }
    let mut components = path.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => return Err("filesystem path must be an absolute disk path".into()),
    };
    if !matches!(components.next(), Some(Component::RootDir))
        || !components.all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("filesystem path must be an absolute disk path".into());
    }
    let normalized = match prefix {
        Prefix::Disk(_) => format!(r"\\?\{}", text.replace('/', "\\")),
        Prefix::VerbatimDisk(_) => text.to_owned(),
        _ => return Err("filesystem path must use a disk prefix".into()),
    };
    let mut wide: Vec<u16> = normalized.encode_utf16().collect();
    if wide.is_empty() || wide.len() >= MAX_PATH_UNITS || wide.contains(&0) {
        return Err("filesystem path exceeds Windows path limits".into());
    }
    wide.push(0);
    Ok(wide)
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_path(path: &Path) -> PathBuf {
        let text = path.to_str().unwrap();
        let ordinary = text.strip_prefix(r"\\?\").unwrap_or(text);
        PathBuf::from(ordinary)
    }

    #[test]
    fn destination_accepts_unicode_space_paths_and_rejects_unsafe_forms() {
        let temp = tempfile::tempdir().unwrap();
        let root = user_path(temp.path());
        assert!(destination(&root.join("New Room Ω")).is_ok());

        for invalid in [
            PathBuf::from(r"\\server\share\room"),
            PathBuf::from(r"\\?\C:\room"),
            PathBuf::from(r"\\.\C:\room"),
            PathBuf::from(r"C:room"),
            PathBuf::from(r"\room"),
            root.join("..\room"),
            root.join("name:stream"),
            root.join("CON.txt"),
            root.join("COM¹.log"),
            root.join("LPT².log"),
            root.join("room. "),
        ] {
            assert!(
                destination(&invalid).is_err(),
                "accepted {}",
                invalid.display()
            );
        }
    }

    #[test]
    fn publisher_normalizes_disk_paths_to_extended_utf16_without_filesystem_access() {
        let ordinary = Path::new(r"C:\room Ω\staged");
        let verbatim = Path::new(r"\\?\C:\room Ω\staged");
        assert_eq!(wide_path(ordinary).unwrap(), wide_path(verbatim).unwrap());
        assert!(wide_path(Path::new(r"\\server\share\room")).is_err());
        assert!(wide_path(Path::new(r"\room")).is_err());
        assert!(wide_path(Path::new(r"C:\room\..\staged")).is_err());
    }

    #[test]
    fn publication_never_replaces_existing_file_or_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = user_path(temp.path());
        let stage = temp.path().join("staged-room");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("payload"), b"staged").unwrap();

        let file = root.join("existing-file");
        fs::write(&file, b"winner").unwrap();
        assert!(publish(&stage, &file).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"winner");
        assert_eq!(fs::read(stage.join("payload")).unwrap(), b"staged");

        for (name, contents) in [
            ("empty-dir", None),
            ("nonempty-dir", Some(b"winner".as_slice())),
        ] {
            let output = root.join(name);
            fs::create_dir(&output).unwrap();
            if let Some(contents) = contents {
                fs::write(output.join("payload"), contents).unwrap();
            }
            assert!(publish(&stage, &output).is_err());
            assert!(output.is_dir());
            assert_eq!(fs::read(stage.join("payload")).unwrap(), b"staged");
            if let Some(contents) = contents {
                assert_eq!(fs::read(output.join("payload")).unwrap(), contents);
            }
        }
    }

    #[test]
    fn publication_moves_a_new_stage_without_copy_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let root = user_path(temp.path());
        let stage = temp.path().join("new-stage");
        let output = root.join("published-room");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("payload"), b"complete").unwrap();

        publish(&stage, &output).unwrap();
        assert!(!stage.exists());
        assert_eq!(fs::read(output.join("payload")).unwrap(), b"complete");
    }

    #[test]
    fn open_read_rejects_directories() {
        let temp = tempfile::tempdir().unwrap();
        assert!(open_read(temp.path()).is_err());
        let file = temp.path().join("plain-file");
        fs::write(&file, b"file").unwrap();
        assert_eq!(open_read(&file).unwrap().metadata().unwrap().len(), 4);
    }
}
