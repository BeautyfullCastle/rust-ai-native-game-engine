//! Windows paths in the form people type and paste back into the tools.
use std::path::{Component, Path, Prefix};

/// Returns `path` as text without the `\\?\` verbatim prefix that
/// `fs::canonicalize` adds, so a printed path can be passed to the next command.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return text;
    };
    match prefix.kind() {
        Prefix::VerbatimDisk(_) => text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned(),
        Prefix::VerbatimUNC(..) => text
            .strip_prefix(r"\\?\UNC\")
            .map_or(text.clone(), |rest| format!(r"\\{rest}")),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::display;
    use std::path::Path;

    #[test]
    fn drops_verbatim_prefixes_and_keeps_ordinary_paths() {
        assert_eq!(display(Path::new(r"\\?\C:\Games\MyRoom")), r"C:\Games\MyRoom");
        assert_eq!(display(Path::new(r"\\?\UNC\server\share\room")), r"\\server\share\room");
        assert_eq!(display(Path::new(r"C:\Games\MyRoom")), r"C:\Games\MyRoom");
        assert_eq!(display(Path::new(r"\\server\share\room")), r"\\server\share\room");
    }

    #[test]
    fn canonical_paths_print_as_ordinary_paths() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let printed = display(&canonical);
        assert!(!printed.starts_with(r"\\?\"));
        assert_eq!(std::fs::canonicalize(&printed).unwrap(), canonical);
    }
}
