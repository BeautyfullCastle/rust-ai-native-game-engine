use crate::{mesh_art::Snapshot, viewport3d::SceneModel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(test)]
#[path = "mesh_props/tests.rs"]
mod tests;

pub(crate) const MAX_PROPS: usize = 128;
const MAX_PROP_ASSETS: usize = 6;
const MAX_PROP_DRAWS: usize = 3584;
const SCHEMA: u32 = 1;
const MAX_NAME_BYTES: usize = 64;
const MAX_SIDECAR_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const SIDECAR_SUFFIX: &str = ".mesh-props.json";
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Clone)]
pub(crate) struct Prop {
    pub id: u64,
    pub name: String,
    pub art: Snapshot,
}

#[derive(Clone, Default)]
pub(crate) struct Document {
    pub props: Vec<Prop>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sidecar {
    schema: u32,
    scene_filename: String,
    graph_revision: [u8; 32],
    props: Vec<PropRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PropRecord {
    id: u64,
    name: String,
    art: serde_json::Value,
}

impl Document {
    pub(crate) fn validate(
        &self,
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<(), String> {
        self.encode(scene_filename, graph_revision)?;
        Ok(())
    }

    pub(crate) fn placements(&self) -> Result<Vec<SceneModel>, String> {
        validate_prop_collection(&self.props)?;
        let mut models = BTreeMap::new();
        let mut placements = Vec::with_capacity(self.props.len());
        for prop in &self.props {
            let identity = prop.art.render_identity();
            let mut placement = prop.art.placement([orr_fp::FP::ZERO; 3])?;
            if let Some(model) = models.get(&identity) {
                placement.model = std::sync::Arc::clone(model);
            } else {
                models.insert(identity, std::sync::Arc::clone(&placement.model));
            }
            placements.push(placement);
        }
        Ok(placements)
    }

    fn encode(&self, scene_filename: &str, graph_revision: [u8; 32]) -> Result<Vec<u8>, String> {
        validate_scene_filename(scene_filename)?;
        validate_prop_collection(&self.props)?;
        let mut embedded_bytes = 0usize;
        let mut props = Vec::with_capacity(self.props.len());
        for prop in &self.props {
            if prop.art.graph_revision() != graph_revision {
                return Err("mesh prop art belongs to a different graph revision".into());
            }
            prop.art.placement([orr_fp::FP::ZERO; 3])?;
            let art_bytes = prop.art.encode_embedded(scene_filename)?;
            embedded_bytes = embedded_bytes
                .checked_add(art_bytes.len())
                .ok_or("mesh props embedded byte count overflow")?;
            if embedded_bytes > MAX_SIDECAR_BYTES as usize {
                return Err("mesh props document exceeds the 16 MiB embedded limit".into());
            }
            props.push(PropRecord {
                id: prop.id,
                name: prop.name.clone(),
                art: serde_json::from_slice(&art_bytes)
                    .map_err(|error| format!("serialize embedded mesh art: {error}"))?,
            });
        }
        let bytes = serde_json::to_vec(&Sidecar {
            schema: SCHEMA,
            scene_filename: scene_filename.to_owned(),
            graph_revision,
            props,
        })
        .map_err(|error| format!("serialize mesh props sidecar: {error}"))?;
        if bytes.len() as u64 > MAX_SIDECAR_BYTES {
            return Err("mesh props sidecar exceeds the 16 MiB limit".into());
        }
        Ok(bytes)
    }

    fn decode(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_SIDECAR_BYTES {
            return Err("mesh props sidecar exceeds the 16 MiB limit".into());
        }
        let sidecar: Sidecar = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid mesh props sidecar: {error}"))?;
        if sidecar.schema != SCHEMA
            || sidecar.scene_filename != scene_filename
            || sidecar.graph_revision != graph_revision
        {
            return Err("mesh props sidecar belongs to another scene or graph revision".into());
        }
        if sidecar.props.len() > MAX_PROPS {
            return Err("mesh props sidecar exceeds the 128-prop limit".into());
        }
        let props = sidecar
            .props
            .into_iter()
            .map(|record| {
                validate_prop_name(&record.name)?;
                let bytes = serde_json::to_vec(&record.art)
                    .map_err(|error| format!("decode embedded mesh art: {error}"))?;
                let art = Snapshot::decode_embedded(&bytes, scene_filename, graph_revision)?;
                Ok(Prop {
                    id: record.id,
                    name: record.name,
                    art,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let document = Self { props };
        document.validate(scene_filename, graph_revision)?;
        Ok(document)
    }
}

pub(crate) fn sidecar_path(scene: &Path) -> Result<PathBuf, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = canonical_scene.with_file_name(format!("{filename}{SIDECAR_SUFFIX}"));
    if sidecar.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err("mesh props sidecar path exceeds the path bound".into());
    }
    Ok(sidecar)
}

pub(crate) fn load(
    scene: &Path,
    graph_revision: [u8; 32],
) -> Result<Option<(Document, String)>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path(&canonical_scene)?;
    let Some(bytes) = read_optional_bounded(&sidecar, "mesh props sidecar")? else {
        return Ok(None);
    };
    let document = Document::decode(&bytes, &filename, graph_revision)?;
    Ok(Some((document, digest(&bytes))))
}

pub(crate) fn sidecar_stamp(scene: &Path) -> Result<Option<String>, String> {
    let sidecar = sidecar_path(scene)?;
    Ok(read_optional_bounded(&sidecar, "mesh props sidecar")?
        .as_deref()
        .map(digest))
}

pub(crate) fn save(
    scene: &Path,
    document: Option<&Document>,
    expected: Option<&str>,
    graph_revision: [u8; 32],
) -> Result<Option<String>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path(&canonical_scene)?;
    validate_expected(expected)?;
    let current = read_optional_bounded(&sidecar, "mesh props sidecar")?;
    match (current.as_deref().map(digest), expected) {
        (None, None) => {}
        (Some(current), Some(expected)) if current == expected => {}
        (None, Some(_)) => return Err("mesh props sidecar was removed by another editor".into()),
        (Some(_), None) => {
            return Err("mesh props sidecar already exists; reload before saving".into())
        }
        (Some(_), Some(_)) => return Err("mesh props sidecar changed; reload before saving".into()),
    }
    let Some(document) = document else {
        if current.is_some() {
            recheck_scene(&canonical_scene)?;
            let final_current = read_optional_bounded(&sidecar, "mesh props sidecar")?;
            if final_current.as_deref().map(digest).as_deref() != expected {
                return Err("mesh props sidecar changed before removal".into());
            }
            fs::remove_file(&sidecar)
                .map_err(|error| format!("remove mesh props sidecar: {error}"))?;
        }
        return Ok(None);
    };
    let bytes = document.encode(&filename, graph_revision)?;
    let parent = canonical_scene.parent().ok_or("scene parent missing")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create mesh props staging file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh props sidecar: {error}"))?;
    recheck_scene(&canonical_scene)?;
    let final_current = read_optional_bounded(&sidecar, "mesh props sidecar")?;
    if final_current.as_deref().map(digest).as_deref() != expected {
        return Err("mesh props sidecar changed before save; reload before saving".into());
    }
    if expected.is_some() {
        temporary
            .persist(&sidecar)
            .map_err(|error| format!("publish mesh props sidecar: {}", error.error))?;
    } else {
        temporary.persist_noclobber(&sidecar).map_err(|error| {
            format!(
                "publish mesh props sidecar without replacement: {}",
                error.error
            )
        })?;
    }
    Ok(Some(digest(&bytes)))
}

fn validate_prop_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.len() > MAX_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err("mesh prop names must be nonempty bounded printable text".into());
    }
    Ok(())
}

fn validate_prop_collection(props: &[Prop]) -> Result<(), String> {
    if props.len() > MAX_PROPS {
        return Err("mesh props document exceeds the 128-prop limit".into());
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut draws = 0usize;
    for prop in props {
        validate_prop_name(&prop.name)?;
        if !names.insert(prop.name.as_str()) {
            return Err("mesh prop names must be unique".into());
        }
        if prop.id == 0 || !ids.insert(prop.id) {
            return Err("mesh prop IDs must be nonzero and unique".into());
        }
        identities.insert(prop.art.render_identity());
        draws = draws
            .checked_add(prop.art.source_model_arc().source().primitives.len())
            .ok_or("mesh props primitive-instance draw count overflow")?;
    }
    validate_render_budget(identities.len(), draws)
}

fn validate_render_budget(distinct_assets: usize, draws: usize) -> Result<(), String> {
    if distinct_assets > MAX_PROP_ASSETS {
        return Err("mesh props document exceeds the six-appearance limit".into());
    }
    if draws > MAX_PROP_DRAWS {
        return Err("mesh props document exceeds the 3584 primitive-instance draw limit".into());
    }
    Ok(())
}

fn validate_scene_filename(filename: &str) -> Result<(), String> {
    validate_leaf(filename, "scene filename")?;
    if filename.len() > 180 {
        return Err("scene filename is too long for its mesh props sidecar".into());
    }
    Ok(())
}

fn checked_scene(scene: &Path) -> Result<(PathBuf, String), String> {
    validate_path_shape(scene, "scene")?;
    let parent = scene.parent().ok_or("scene needs a parent directory")?;
    let parent = checked_directory(parent, "scene parent")?;
    let filename = scene
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or("scene filename must be Unicode")?;
    validate_scene_filename(filename)?;
    let canonical_scene = parent.join(filename);
    let metadata = fs::symlink_metadata(&canonical_scene)
        .map_err(|error| format!("scene metadata: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err("scene must be a regular non-reparse file".into());
    }
    Ok((canonical_scene, filename.to_owned()))
}

fn recheck_scene(scene: &Path) -> Result<(), String> {
    let (current, _) = checked_scene(scene)?;
    if current != scene {
        return Err("scene path identity changed while saving mesh props".into());
    }
    Ok(())
}

fn validate_path_shape(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
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
        return Err(format!(
            "{label} path must be bounded, absolute and normalized"
        ));
    }
    for component in components {
        if let Component::Normal(name) = component {
            validate_leaf(
                name.to_str()
                    .ok_or_else(|| format!("{label} path must be Unicode"))?,
                label,
            )?;
        }
    }
    Ok(())
}

fn validate_leaf(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || matches!(name.chars().last(), Some('.' | ' '))
        || name.contains([':', '\\', '/', '\0'])
        || name.chars().any(char::is_control)
        || reserved_device_name(name)
    {
        return Err(format!("{label} path contains an invalid filename"));
    }
    Ok(())
}

fn reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
            })
        })
}

fn checked_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_path_shape(path, label)?;
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    #[cfg(windows)]
    {
        let mut components = components.iter().copied();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ) =>
            {
                prefix
            }
            _ => return Err(format!("{label} must use an absolute local disk path")),
        };
        current.push(prefix.as_os_str());
        match components.next() {
            Some(Component::RootDir) => current.push("\\"),
            _ => return Err(format!("{label} must be rooted")),
        }
        ensure_directory(&current, label)?;
        for component in components {
            let Component::Normal(name) = component else {
                return Err(format!("{label} path is not normalized"));
            };
            validate_leaf(
                name.to_str().ok_or("path component must be Unicode")?,
                label,
            )?;
            current.push(name);
            ensure_directory(&current, label)?;
        }
    }
    #[cfg(not(windows))]
    {
        for component in components {
            match component {
                Component::RootDir => current.push(component.as_os_str()),
                Component::Normal(name) => {
                    validate_leaf(
                        name.to_str().ok_or("path component must be Unicode")?,
                        label,
                    )?;
                    current.push(name);
                    ensure_directory(&current, label)?;
                }
                _ => return Err(format!("{label} path is not normalized")),
            }
        }
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("canonicalize {label}: {error}"))?;
    if canonical.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(format!("canonical {label} path exceeds the path bound"));
    }
    Ok(canonical)
}

fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{label} path {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{label} path must contain directories only"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("{label} path cannot contain reparse points"));
        }
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} path cannot contain symlinks"));
    }
    Ok(())
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

fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_modified), Ok(second_modified)) => first_modified == second_modified,
            _ => true,
        }
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

fn read_optional_bounded(path: &Path, label: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_bounded(path, label).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect {label}: {error}")),
    }
}

fn read_bounded(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} parent missing"))?;
    let parent = checked_directory(parent, label)?;
    let filename = path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or_else(|| format!("{label} filename must be Unicode"))?;
    validate_leaf(filename, label)?;
    let canonical = parent.join(filename);
    let before =
        fs::symlink_metadata(&canonical).map_err(|error| format!("inspect {label}: {error}"))?;
    if !regular_non_reparse(&before) || before.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(format!(
            "{label} must be a bounded regular non-reparse file"
        ));
    }
    let mut file = open_without_following(&canonical)
        .map_err(|error| format!("open {label} without following links: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened {label}: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(format!("{label} changed before reading"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label}: {error}"))?;
    let opened_after = file
        .metadata()
        .map_err(|error| format!("recheck opened {label}: {error}"))?;
    let current =
        fs::symlink_metadata(&canonical).map_err(|error| format!("recheck {label}: {error}"))?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES
        || bytes.len() as u64 != opened.len()
        || !regular_non_reparse(&opened_after)
        || !regular_non_reparse(&current)
        || !same_observation(&before, &opened)
        || !same_observation(&opened, &opened_after)
        || !same_observation(&before, &current)
        || checked_directory(parent.as_path(), label)?.as_path() != parent
    {
        return Err(format!("{label} changed while it was read"));
    }
    Ok(bytes)
}

fn validate_expected(expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err("expected mesh props digest must be lowercase SHA-256".into());
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
