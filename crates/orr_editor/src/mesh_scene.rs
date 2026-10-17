use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const MAX_SCENE_PATH_BYTES: usize = 4096;
const BLANK_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

pub(crate) fn create(path: &Path) -> Result<(), String> {
    let path_text = path.to_str().ok_or("scene path must be valid UTF-8")?;
    if path_text.len() > MAX_SCENE_PATH_BYTES {
        return Err("scene path exceeds the 4,096-byte limit".into());
    }
    if !path.is_absolute() {
        return Err("scene path must be absolute".into());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene path must have a UTF-8 file name")?;
    if !file_name.ends_with(".scene.yaml") || file_name.len() == ".scene.yaml".len() {
        return Err("scene file name must end in .scene.yaml".into());
    }
    if is_reserved_device_leaf(file_name) {
        return Err("scene file name cannot use a reserved Windows device name".into());
    }
    check_parent_chain(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => {
            return Err("scene path cannot be a link or reparse point".into());
        }
        Ok(_) => return Err("scene file already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(BLANK_SCENE.as_bytes())
        .map_err(|error| error.to_string())
}

fn check_parent_chain(path: &Path) -> Result<(), String> {
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        if let Component::Normal(value) = component {
            if value.to_str().is_some_and(|value| value.contains(':')) {
                return Err("scene path cannot contain alternate data stream separators".into());
            }
        }
        if index + 1 == components.len() {
            break;
        }
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err("scene path cannot contain parent-directory segments".into());
            }
            Component::Normal(value) => {
                current.push(value);
            }
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        if is_link_or_reparse(&metadata) {
            return Err("scene path cannot contain links or reparse points".into());
        }
        if !metadata.is_dir() {
            return Err("scene path parent components must be directories".into());
        }
    }
    Ok(())
}

fn is_reserved_device_leaf(file_name: &str) -> bool {
    #[cfg(windows)]
    {
        let stem = file_name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches([' ', '.'])
            .to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|number| {
                    matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                })
            })
    }
    #[cfg(not(windows))]
    {
        let _ = file_name;
        false
    }
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::{create, BLANK_SCENE};
    use std::{fs, path::PathBuf};

    #[test]
    fn creates_blank_scene_at_absolute_yaml_path() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("mesh.scene.yaml");

        create(&scene).unwrap();

        assert_eq!(fs::read_to_string(scene).unwrap(), BLANK_SCENE);
    }

    #[test]
    fn creates_blank_scene_below_a_canonicalized_parent() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("mesh.scene.yaml");

        create(&scene).unwrap();

        assert_eq!(fs::read_to_string(scene).unwrap(), BLANK_SCENE);
    }

    #[test]
    fn rejects_invalid_paths_without_creating_output() {
        let directory = tempfile::tempdir().unwrap();
        let invalid_extension = directory.path().join("mesh.yaml");
        let oversized = directory
            .path()
            .join(format!("{}.scene.yaml", "a".repeat(4096)));
        let relative = PathBuf::from(format!(
            "orr-mesh-scene-invalid-{}.scene.yaml",
            std::process::id()
        ));
        for path in [&invalid_extension, &oversized, &relative] {
            assert!(create(path).is_err());
            assert!(!path.exists());
        }
    }

    #[test]
    fn rejects_ads_path_without_changing_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("mesh");
        let stream_path = directory.path().join("mesh:stream.scene.yaml");
        fs::write(&base, b"preserve base file").unwrap();

        assert!(create(&stream_path).is_err());

        assert_eq!(fs::read(base).unwrap(), b"preserve base file");
    }

    #[test]
    fn refuses_to_replace_existing_scene_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("mesh.scene.yaml");
        fs::write(&scene, b"existing scene bytes").unwrap();

        assert!(create(&scene).is_err());

        assert_eq!(fs::read(scene).unwrap(), b"existing scene bytes");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_output_without_changing_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.scene.yaml");
        let link = directory.path().join("linked.scene.yaml");
        fs::write(&target, b"existing scene bytes").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(create(&link).is_err());

        assert_eq!(fs::read(target).unwrap(), b"existing scene bytes");
    }

    #[cfg(windows)]
    #[test]
    fn rejects_junction_parent_without_creating_scene() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let junction = directory.path().join("junction");
        fs::create_dir(&target).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "could not create test junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let scene = junction.join("mesh.scene.yaml");

        assert!(create(&scene).is_err());
        assert!(!target.join("mesh.scene.yaml").exists());
        fs::remove_dir(junction).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn rejects_reserved_device_leaf_without_touching_device() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("CON.scene.yaml");

        assert!(create(&scene).is_err());
        assert!(!directory.path().join("CON").exists());
    }
}
