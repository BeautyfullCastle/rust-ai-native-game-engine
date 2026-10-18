use super::{decode_asset, mesh_doc_from_yaml, AgentSpec, MeshScenePin, PIN_NAME};
use orr_edit::{EditorDoc, Op, Origin};
use orr_navigation::mesh::StaticMeshGraph;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAX_ASSET_BYTES: u64 = 8_192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneReceipt {
    pub graph_revision: [u8; 32],
    pub checksum: u64,
    pub graph_bytes: u32,
    pub scene_bytes: u32,
}

pub fn create_mesh_scene(graph: &StaticMeshGraph, spec: AgentSpec) -> Result<EditorDoc, String> {
    let pin = MeshScenePin::new(graph, spec)?;
    let mut document = mesh_doc_from_yaml("schema: orr.scene/1\nsingletons: {}\nentities: {}\n")
        .map_err(|error| error.to_string())?;
    let descriptor = document
        .types()
        .get(PIN_NAME)
        .ok_or("mesh pin reflection is missing")?;
    let value = descriptor.read(bytemuck::bytes_of(&pin));
    document
        .apply(
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .map_err(|error| error.to_string())?;
    Ok(document)
}

pub fn read_graph(path: &Path) -> Result<StaticMeshGraph, String> {
    check_path(path)?;
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("orrmesh"))
    {
        return Err("mesh asset must use the .orrmesh extension".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_ASSET_BYTES {
        return Err("mesh asset must be a regular file of at most 8192 bytes".into());
    }
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.len() > MAX_ASSET_BYTES {
        return Err("mesh asset must be a regular file of at most 8192 bytes".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_ASSET_BYTES
        || bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("mesh asset exceeded its limit or changed while being read".into());
    }
    decode_asset(&bytes).map_err(|error| error.to_string())
}

pub fn create_scene_from_asset(
    asset: &Path,
    destination: &Path,
    spec: AgentSpec,
) -> Result<SceneReceipt, String> {
    check_path(destination)?;
    if !destination
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".scene.yaml"))
    {
        return Err("destination must end in .scene.yaml".into());
    }
    let graph = read_graph(asset)?;
    let mut document = create_mesh_scene(&graph, spec)?;
    let yaml = document.save_yaml();
    let reopened = mesh_doc_from_yaml(&yaml).map_err(|error| error.to_string())?;
    if reopened.frame().to_bytes() != document.frame().to_bytes() {
        return Err("saved mesh scene does not restore its admitted Frame".into());
    }
    let receipt = SceneReceipt {
        graph_revision: graph.revision(),
        checksum: document.frame().checksum(),
        graph_bytes: u32::try_from(graph.to_bytes().len()).map_err(|_| "graph size overflow")?,
        scene_bytes: u32::try_from(yaml.len()).map_err(|_| "scene size overflow")?,
    };
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("{}: {error}", destination.display()))?;
    output
        .write_all(yaml.as_bytes())
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("{}: {error}", destination.display()))?;
    Ok(receipt)
}

fn check_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.as_os_str().len() > 4_096 {
        return Err("path must be absolute and at most 4096 bytes".into());
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("parent traversal is not supported".into());
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || is_reparse(&metadata) {
                    return Err(format!(
                        "reparse/symlink path is not supported: {}",
                        ancestor.display()
                    ));
                }
                if ancestor != path && !metadata.is_dir() {
                    return Err(format!("parent is not a directory: {}", ancestor.display()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && ancestor == path => {}
            Err(error) => return Err(format!("{}: {error}", ancestor.display())),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool {
    false
}

/// Write caller-validated scene YAML to a new file, never replacing an existing
/// path. Callers must validate admission and exact save/reopen before calling.
/// This matches asset authoring's path checks and durability convention; a write
/// error may leave a partial new file and does not promise crash-atomic visibility.
pub fn create_new_scene_file(destination: &Path, yaml: &str) -> Result<(), String> {
    check_path(destination)?;
    if !destination
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".scene.yaml"))
    {
        return Err("destination must end in .scene.yaml".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("{}: {error}", destination.display()))?;
    output
        .write_all(yaml.as_bytes())
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("{}: {error}", destination.display()))
}
