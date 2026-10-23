use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, mesh_from_view, MeshScenePin};
use std::{
    fs::{self, Metadata, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_SCENE_BYTES: u64 = 1024 * 1024;
const MAX_ART_BYTES: u64 = orr_model::MAX_FILE_BYTES as u64;
const MAX_PROPS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CAMERA_BYTES: u64 = 16 * 1024;
const MAX_ANIMATION_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const SIDECAR_SUFFIXES: [&str; 6] = [
    ".mesh-art.json",
    ".mesh-props.json",
    ".mesh-camera.json",
    ".mesh-animation.json",
    ".mesh-lighting.json",
    ".mesh-hud.json",
];

struct SourceFile {
    path: PathBuf,
    name: String,
    limit: u64,
    bytes: Option<Vec<u8>>,
}

pub(crate) struct Plan {
    source_scene: PathBuf,
    destination: PathBuf,
    scene_name: String,
    source_checksum: u64,
    files: Vec<SourceFile>,
    stage: tempfile::TempDir,
}

impl Plan {
    pub(crate) fn prepare(
        source_scene: &Path,
        destination_directory: &Path,
    ) -> Result<Self, String> {
        let (source_scene, scene_name) = crate::mesh_art::checked_scene(source_scene)?;
        let (destination, destination_parent) = checked_destination(destination_directory)?;
        if destination == source_scene
            || destination == source_scene.parent().unwrap_or(Path::new(""))
        {
            return Err(
                "destination must be a new directory separate from the source scene".into(),
            );
        }
        refuse_existing(&destination)?;

        let mut files = vec![read_source_file(
            &source_scene,
            scene_name.clone(),
            MAX_SCENE_BYTES,
            true,
        )?];
        let mut total_bytes = files[0]
            .bytes
            .as_ref()
            .map_or(0, |bytes| bytes.len() as u64);
        for (suffix, limit) in SIDECAR_SUFFIXES.into_iter().zip([
            MAX_ART_BYTES,
            MAX_PROPS_BYTES,
            MAX_CAMERA_BYTES,
            MAX_ANIMATION_BYTES,
            crate::mesh_lighting::MAX_BYTES as u64,
            crate::mesh_hud::MAX_BYTES as u64,
        ]) {
            let name = format!("{scene_name}{suffix}");
            let path = source_scene.with_file_name(&name);
            let file = read_source_file(&path, name, limit, false)?;
            if let Some(bytes) = &file.bytes {
                total_bytes = total_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or("mesh workspace copy size overflow")?;
            }
            files.push(file);
        }
        if total_bytes > MAX_TOTAL_BYTES {
            return Err("mesh workspace scene and sidecars exceed the 128 MiB copy bound".into());
        }

        let scene_bytes = files[0]
            .bytes
            .as_deref()
            .ok_or("mesh workspace scene bytes are missing")?;
        let scene_text = std::str::from_utf8(scene_bytes)
            .map_err(|error| format!("mesh scene is not UTF-8: {error}"))?;
        let document = mesh_doc_from_yaml(scene_text)
            .map_err(|error| format!("mesh scene admission failed: {error}"))?;
        let source_checksum = document.frame().checksum();
        let frame = FrameView::of(document.frame());
        let pin = *frame.singleton::<MeshScenePin>();
        let graph_revision = if pin.is_empty() {
            if files.iter().skip(1).any(|file| file.bytes.is_some()) {
                return Err("orphan mesh presentation sidecars require a pinned graph".into());
            }
            None
        } else {
            Some(mesh_from_view(frame)?.graph.revision())
        };
        if let Some(graph_revision) = graph_revision {
            validate_sidecars(&source_scene, graph_revision, &files)?;
        }

        let stage = tempfile::Builder::new()
            .prefix(".orr-mesh-workspace-copy-")
            .tempdir_in(&destination_parent)
            .map_err(|error| format!("create mesh workspace staging directory: {error}"))?;
        for file in &files {
            let Some(bytes) = &file.bytes else {
                continue;
            };
            let output = stage.path().join(&file.name);
            let mut output_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .map_err(|error| format!("create staged mesh workspace file: {error}"))?;
            output_file
                .write_all(bytes)
                .and_then(|()| output_file.sync_all())
                .map_err(|error| format!("write staged mesh workspace file: {error}"))?;
        }
        if !regular_directory(
            &fs::symlink_metadata(stage.path()).map_err(|error| error.to_string())?,
        ) {
            return Err("mesh workspace staging directory changed before validation".into());
        }
        refuse_existing(&destination)?;
        Ok(Self {
            source_scene,
            destination,
            scene_name,
            source_checksum,
            files,
            stage,
        })
    }

    pub(crate) fn source_checksum(&self) -> u64 {
        self.source_checksum
    }

    pub(crate) fn publish(self) -> Result<PathBuf, String> {
        let (current_scene, current_name) = crate::mesh_art::checked_scene(&self.source_scene)?;
        if current_scene != self.source_scene || current_name != self.scene_name {
            return Err("source mesh workspace identity changed before publication".into());
        }
        for file in &self.files {
            let current = read_source_file(
                &file.path,
                file.name.clone(),
                file.limit,
                file.path == self.source_scene,
            )?;
            if current.bytes != file.bytes {
                return Err(format!(
                    "source mesh workspace file changed before publication: {}",
                    file.name
                ));
            }
        }
        let parent = self
            .destination
            .parent()
            .ok_or("mesh workspace destination parent is missing")?;
        if crate::mesh_art::checked_directory(parent, "mesh workspace destination parent")?
            != parent
        {
            return Err("mesh workspace destination parent changed before publication".into());
        }
        refuse_existing(&self.destination)?;
        let stage_metadata = fs::symlink_metadata(self.stage.path())
            .map_err(|error| format!("mesh workspace staging directory: {error}"))?;
        if !regular_directory(&stage_metadata) {
            return Err("mesh workspace staging directory is not a regular directory".into());
        }
        validate_staging(self.stage.path(), &self.files)?;
        publish_new_directory(self.stage.path(), &self.destination)?;
        let scene_path = self.destination.join(self.scene_name);
        let _preserved_directory = self.stage.keep();
        Ok(scene_path)
    }
}

fn validate_sidecars(
    scene: &Path,
    graph_revision: [u8; 32],
    files: &[SourceFile],
) -> Result<(), String> {
    if files[1].bytes.is_some() {
        let (_snapshot, observed_digest) = crate::mesh_art::load(scene, graph_revision)?
            .ok_or("mesh art sidecar disappeared during validation")?;
        require_captured_digest(&files[1], &observed_digest)?;
    }
    if files[2].bytes.is_some() {
        let (_document, observed_digest) = crate::mesh_props::load(scene, graph_revision)?
            .ok_or("mesh props sidecar disappeared during validation")?;
        require_captured_digest(&files[2], &observed_digest)?;
    }
    if files[3].bytes.is_some() {
        let (_document, observed_digest) =
            crate::mesh_camera_store::load(scene, graph_revision)?
                .ok_or("mesh camera sidecar disappeared during validation")?;
        require_captured_digest(&files[3], &observed_digest)?;
    }
    if files[4].bytes.is_some() {
        #[cfg(feature = "mesh-animated-models")]
        {
            let (_snapshot, observed_digest) =
                crate::mesh_animation_store::load(scene, graph_revision)?
                    .ok_or("mesh animation sidecar disappeared during validation")?;
            require_captured_digest(&files[4], &observed_digest)?;
        }
        #[cfg(not(feature = "mesh-animated-models"))]
        return Err("mesh animation sidecar requires the mesh-animated-models feature".into());
    }
    if files[5].bytes.is_some() {
        let (_document, observed_digest) = crate::mesh_lighting_store::load(scene, graph_revision)?
            .ok_or("mesh lighting sidecar disappeared during validation")?;
        require_captured_digest(&files[5], &observed_digest)?;
    }
    if files[6].bytes.is_some() {
        let (_document, observed_digest) = crate::mesh_hud_store::load(scene, graph_revision)?
            .ok_or("mesh HUD sidecar disappeared during validation")?;
        require_captured_digest(&files[6], &observed_digest)?;
    }
    Ok(())
}

fn require_captured_digest(file: &SourceFile, observed: &str) -> Result<(), String> {
    let captured = file
        .bytes
        .as_deref()
        .ok_or("validated mesh sidecar has no captured bytes")?;
    if crate::mesh_art::digest(captured) != observed {
        return Err(format!(
            "{} changed while its typed presentation data was being validated",
            file.name
        ));
    }
    Ok(())
}

fn validate_staging(stage: &Path, files: &[SourceFile]) -> Result<(), String> {
    let expected_names = files
        .iter()
        .filter(|file| file.bytes.is_some())
        .map(|file| file.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut observed_names = std::collections::BTreeSet::new();
    for entry in
        fs::read_dir(stage).map_err(|error| format!("inspect staged mesh copy: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read staged mesh entry: {error}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "staged mesh workspace filename is not Unicode")?;
        if !observed_names.insert(name.clone()) || !expected_names.contains(&name) {
            return Err("staged mesh workspace inventory contains an unexpected entry".into());
        }
    }
    if observed_names != expected_names {
        return Err("staged mesh workspace inventory is incomplete".into());
    }
    for file in files {
        let Some(expected) = &file.bytes else {
            continue;
        };
        let path = stage.join(&file.name);
        let observed =
            crate::mesh_art::read_optional_file(&path, file.limit, "staged mesh workspace file")?
                .ok_or("staged mesh workspace file disappeared")?;
        if &observed != expected {
            return Err(format!(
                "staged mesh workspace file changed before publication: {}",
                file.name
            ));
        }
    }
    Ok(())
}

fn read_source_file(
    path: &Path,
    name: String,
    limit: u64,
    required: bool,
) -> Result<SourceFile, String> {
    let bytes = match fs::symlink_metadata(path) {
        Ok(_) => crate::mesh_art::read_optional_file(path, limit, "mesh workspace source file")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => None,
        Err(error) => return Err(format!("mesh workspace source metadata: {error}")),
    };
    if required && bytes.is_none() {
        return Err("mesh workspace scene disappeared while reading".into());
    }
    Ok(SourceFile {
        path: path.to_path_buf(),
        name,
        limit,
        bytes,
    })
}

fn checked_destination(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let text = path
        .to_str()
        .ok_or("destination directory must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("destination directory must be bounded, absolute and normalized".into());
    }
    #[cfg(windows)]
    if !matches!(components.first(), Some(Component::Prefix(prefix)) if matches!(
        prefix.kind(),
        std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
    )) {
        return Err("destination directory must use an absolute local disk path".into());
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("destination directory needs a valid final name")?;
    validate_destination_leaf(name)?;
    let parent = path
        .parent()
        .ok_or("destination directory needs an existing parent")?;
    let canonical_parent = crate::mesh_art::checked_directory(parent, "destination parent")?;
    let destination = canonical_parent.join(name);
    refuse_existing(&destination)?;
    Ok((destination, canonical_parent))
}

fn validate_destination_leaf(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|character| character.is_control() || "<>:\"|?*/\\".contains(character))
        || reserved_device_name(name)
    {
        return Err("destination directory name is not a valid local directory name".into());
    }
    Ok(())
}

fn reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                    || matches!(suffix, "¹" | "²" | "³")
            })
        })
}

fn refuse_existing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("mesh workspace destination directory already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect mesh workspace destination: {error}")),
    }
}

fn publish_new_directory(stage: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let stage_wide: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_wide: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = unsafe { MoveFileExW(stage_wide.as_ptr(), destination_wide.as_ptr(), 0) };
        if result == 0 {
            return Err(format!(
                "native no-replace mesh workspace publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (stage, destination);
        Err("native no-replace mesh workspace publication is available only on Windows".into())
    }
}

#[cfg(windows)]
fn regular_directory(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(windows))]
fn regular_directory(metadata: &Metadata) -> bool {
    metadata.is_dir() && !metadata.file_type().is_symlink()
}

#[cfg(test)]
#[path = "copy/tests.rs"]
mod tests;
