use crate::{editor::Editor, game::EditorGame};
use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, mesh_from_view, MeshScenePin};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_SCENE_PATH_BYTES: usize = 4096;
const MAX_SCENE_COMPONENTS: usize = 64;
const MAX_SCENE_BYTES: u64 = 1024 * 1024;
const MAX_SCENE_LEAF_BYTES: usize = 180;
const SIDECAR_SUFFIX: &str = ".mesh-art.json";
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const BLANK_SCENE: &[u8] = b"schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Create,
    Open,
}

pub(crate) fn prepare(path: &Path, intent: Intent) -> Result<Editor, String> {
    let path = checked_scene_path(path, intent == Intent::Open)?;
    if intent == Intent::Create {
        refuse_existing_sidecar(&path)?;
        crate::mesh_scene::create(&path)
            .map_err(|error| format!("cannot create mesh scene {}: {error}", path.display()))?;
    }
    let source_bytes = read_scene(&path)?;
    if intent == Intent::Create && source_bytes != BLANK_SCENE {
        return Err("new mesh scene differs from the expected blank scene".into());
    }
    let source_text = std::str::from_utf8(&source_bytes)
        .map_err(|error| format!("mesh scene is not UTF-8: {error}"))?;
    let admitted = mesh_doc_from_yaml(source_text)
        .map_err(|error| format!("mesh scene admission failed: {error}"))?;
    let expected_checksum = admitted.frame().checksum();
    validate_admitted_frame(FrameView::of(admitted.frame()))?;

    let mut editor = Editor::open_game(&path, EditorGame::MeshNavigation)?;
    editor.sync();
    if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
        return Err("mesh workspace opened an unexpected host or game".into());
    }
    let snapshot = editor
        .snapshot()
        .ok_or("mesh workspace host did not publish an admitted Frame")?;
    let frame = snapshot.predicted();
    if frame.checksum() != expected_checksum {
        return Err("mesh workspace host Frame differs from admitted scene bytes".into());
    }
    validate_admitted_frame(frame)?;
    if read_scene(&path)? != source_bytes {
        return Err("mesh scene changed while its host was starting".into());
    }
    Ok(editor)
}

fn validate_admitted_frame(frame: FrameView<'_>) -> Result<(), String> {
    let pin = *frame.singleton::<MeshScenePin>();
    if !pin.is_empty() {
        mesh_from_view(frame)?;
    }
    Ok(())
}

fn checked_scene_path(path: &Path, must_exist: bool) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("mesh scene path must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_SCENE_PATH_BYTES
        || components.len() > MAX_SCENE_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("mesh scene path must be bounded, absolute and normalized".into());
    }
    #[cfg(windows)]
    {
        if !matches!(components.first(), Some(Component::Prefix(prefix)) if matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        )) {
            return Err("mesh scene path must use an absolute local disk path".into());
        }
    }
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mesh scene filename must be Unicode")?;
    validate_leaf(filename)?;
    if !filename.ends_with(".scene.yaml")
        || filename.len() == ".scene.yaml".len()
        || filename.len() > MAX_SCENE_LEAF_BYTES
    {
        return Err("mesh scene filename must be a bounded *.scene.yaml name".into());
    }
    let parent = path
        .parent()
        .ok_or("mesh scene path needs a parent directory")?;
    check_parent_chain(parent)?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize mesh scene parent: {error}"))?;
    let canonical_path = canonical_parent.join(filename);
    match fs::symlink_metadata(&canonical_path) {
        Ok(metadata) => {
            if !regular_non_reparse(&metadata) {
                return Err("mesh scene must be a regular non-reparse file".into());
            }
            if !must_exist {
                return Err("mesh scene already exists".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !must_exist => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("mesh scene does not exist".into())
        }
        Err(error) => return Err(format!("mesh scene metadata: {error}")),
    }
    Ok(canonical_path)
}

fn check_parent_chain(parent: &Path) -> Result<(), String> {
    let components: Vec<_> = parent.components().collect();
    let mut current = PathBuf::new();
    for component in components {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => {
                validate_path_component(
                    part.to_str()
                        .ok_or("mesh scene path components must be Unicode")?,
                )?;
                current.push(part);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("mesh scene parent {}: {error}", current.display()))?;
                if !regular_directory(&metadata) {
                    return Err(
                        "mesh scene ancestry must contain only non-reparse directories".into(),
                    );
                }
            }
            Component::CurDir | Component::ParentDir => {
                return Err("mesh scene ancestry is not normalized".into())
            }
        }
    }
    Ok(())
}

fn refuse_existing_sidecar(scene: &Path) -> Result<(), String> {
    let filename = scene
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mesh scene filename must be Unicode")?;
    for suffix in [SIDECAR_SUFFIX, ".mesh-props.json", ".mesh-camera.json"] {
        let sidecar_name = format!("{filename}{suffix}");
        if sidecar_name.len() > 255 {
            return Err("mesh scene name is too long for its sidecars".into());
        }
        let sidecar = scene.with_file_name(sidecar_name);
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                return Err(format!(
                    "new mesh scene refused because its {suffix} sidecar path already exists"
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("mesh sidecar metadata: {error}")),
        }
    }
    Ok(())
}

fn read_scene(path: &Path) -> Result<Vec<u8>, String> {
    recheck_parent(path)?;
    let before =
        fs::symlink_metadata(path).map_err(|error| format!("mesh scene metadata: {error}"))?;
    if !regular_non_reparse(&before) || before.len() > MAX_SCENE_BYTES {
        return Err("mesh scene must be a regular non-reparse file of at most 1 MiB".into());
    }
    let mut file = open_without_following(path)
        .map_err(|error| format!("open mesh scene without following reparse points: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("mesh scene handle metadata: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() > MAX_SCENE_BYTES {
        return Err("mesh scene changed to a non-regular or oversized file".into());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_SCENE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read mesh scene: {error}"))?;
    let after =
        fs::symlink_metadata(path).map_err(|error| format!("mesh scene recheck: {error}"))?;
    let handle_after = file
        .metadata()
        .map_err(|error| format!("mesh scene handle recheck: {error}"))?;
    recheck_parent(path)?;
    if bytes.len() as u64 > MAX_SCENE_BYTES
        || bytes.len() as u64 != opened.len()
        || !regular_non_reparse(&after)
        || !regular_non_reparse(&handle_after)
        || !same_observation(&before, &opened)
        || !same_observation(&opened, &handle_after)
        || !same_observation(&before, &after)
    {
        return Err("mesh scene changed while being read".into());
    }
    Ok(bytes)
}

fn recheck_parent(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("mesh scene parent missing")?;
    check_parent_chain(parent)?;
    if fs::canonicalize(parent).map_err(|error| format!("mesh scene parent recheck: {error}"))?
        != parent
    {
        return Err("mesh scene parent changed while being read".into());
    }
    Ok(())
}

fn validate_leaf(filename: &str) -> Result<(), String> {
    if filename.len() > MAX_SCENE_LEAF_BYTES {
        return Err("mesh scene filename is too long".into());
    }
    validate_path_component(filename)
}

fn validate_path_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 255
        || value.ends_with('.')
        || value.ends_with(' ')
        || value.contains([':', '\\', '/', '\0'])
        || value.chars().any(char::is_control)
        || reserved_device_name(value)
    {
        return Err("mesh scene path contains an invalid Windows filename component".into());
    }
    Ok(())
}

fn reserved_device_name(filename: &str) -> bool {
    let stem = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
}

fn open_without_following(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn regular_non_reparse(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn regular_directory(metadata: &Metadata) -> bool {
    if !metadata.is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_time), Ok(second_time)) => first_time == second_time,
            _ => true,
        }
}

#[cfg(test)]
#[path = "mesh_workspace/tests.rs"]
mod tests;
