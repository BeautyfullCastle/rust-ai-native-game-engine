//! Offline creation of closed Arena, Collect and Room starters on Linux.
//!
//! Package installation and runtime admission remain the existing authorities.
//! Publication never replaces a destination. Ordinary errors clean only the owned
//! transaction; hostile filesystem races and power-loss durability are not claimed.
#[cfg(feature = "collect-dodge")]
mod collect_template;
#[cfg(feature = "room-project")]
mod room_template;
mod template;
#[cfg(test)]
mod tests;
#[cfg(feature = "collect-ui")]
mod ui_template;

use crate::{
    project_publish::publish_no_replace,
    project_runtime::{compiled_runtime, PreparedRuntime},
};
use orr_reflect::Guid;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

pub const TEMPLATE: &str = template::ID;
pub const COLLECT_TEMPLATE: &str = "collect-dodge-2d-v1";
pub const COLLECT_UI_TEMPLATE: &str = "collect-dodge-ui-2d-v1";
pub const ROOM_TEMPLATE: &str = "room-escape-3d-v1";
pub const MAX_SEED_BYTES: usize = 128;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 16;
const MANIFEST: &[u8] = b"{\n  \"schema\": 2,\n  \"engine\": \"^0.0.1\",\n  \"entry\": {\n    \"game\": \"arena\",\n    \"scene\": \"arena.scene.yaml\",\n    \"sprites\": \"arena.sprites.json\"\n  }\n}\n";

#[derive(Clone, Debug)]
pub struct CreateOptions {
    pub output: PathBuf,
    pub template: String,
    /// Nonempty ASCII letters, digits, hyphens, underscores or dots, at most 128 bytes.
    /// Same seed and template/tool version intentionally reproduce the namespace.
    pub seed: String,
}
#[derive(Debug)]
pub struct CreateReport {
    pub output: PathBuf,
    pub template: String,
    pub seed: String,
    pub initial_checksum: u64,
    pub entity_guids: Vec<Guid>,
}

/// Create a new playable starter without executing code or consulting source paths.
/// The output must be absolute, absent, and have an existing nonsymlink parent.
pub fn create(options: &CreateOptions) -> Result<CreateReport, String> {
    create_with(options, |_, _| Ok(()), publish_no_replace)
}

/// Create the closed CollectDodge starter with an explicitly supplied UUIDv4.
/// The seed never determines this identity; reusing an identity shares progress.
#[cfg(feature = "collect-dodge")]
pub fn create_collect(options: &CreateOptions, game_id: &str) -> Result<CreateReport, String> {
    let progress = orr_package::ProjectProgress {
        schema: 1,
        game_id: game_id.into(),
        profile: orr_package::ProgressProfile::CollectDodgeHighscoreV1,
    };
    progress.validate()?;
    if options.template != COLLECT_TEMPLATE && options.template != COLLECT_UI_TEMPLATE {
        return Err("expected collect-dodge-2d-v1 or collect-dodge-ui-2d-v1 template".into());
    }
    create_transaction(options, Some(&progress), |_, _| Ok(()), publish_no_replace)
}

fn create_with(
    options: &CreateOptions,
    checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    publish: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<CreateReport, String> {
    create_transaction(options, None, checkpoint, publish)
}

fn create_transaction(
    options: &CreateOptions,
    progress: Option<&orr_package::ProjectProgress>,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    publish: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<CreateReport, String> {
    let with_ui = options.template == COLLECT_UI_TEMPLATE;
    let with_room = options.template == ROOM_TEMPLATE;
    if with_room && !cfg!(feature = "room-project") {
        return Err("Room template requires room-project feature".into());
    }
    if (progress.is_some() && options.template != COLLECT_TEMPLATE && !with_ui)
        || (progress.is_none() && options.template != TEMPLATE && !with_room)
    {
        return Err(format!("unsupported template; expected {TEMPLATE}"));
    }
    if with_ui && !cfg!(feature = "collect-ui") {
        return Err("Collect UI template requires collect-ui feature".into());
    }
    if options.seed.is_empty()
        || options.seed.len() > MAX_SEED_BYTES
        || !options
            .seed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(format!("seed must contain 1..{MAX_SEED_BYTES} ASCII letters, digits, dots, underscores or hyphens"));
    }
    let output = destination(&options.output)?;
    let parent = output
        .parent()
        .ok_or("output requires an existing parent")?;
    let transaction = tempfile::Builder::new()
        .prefix(".orr-create-")
        .tempdir_in(parent)
        .map_err(error)?;
    let project_root = transaction.path().join("project");
    let source_root = transaction.path().join("source");
    fs::create_dir(&project_root).map_err(error)?;
    fs::create_dir(&source_root).map_err(error)?;
    checkpoint("stage-created", transaction.path())?;

    let (scene, sidecar_bytes, manifest, scene_file, sidecar_file, readme) = if let Some(progress) =
        progress
    {
        #[cfg(feature = "collect-dodge")]
        {
            let (scene, sprites) =
                collect_template::documents_for(&options.seed, &options.template)?;
            let manifest = serde_json::json!({"schema":3,"engine":"^0.0.1",
                "entry":{"game":"collect-dodge-v1","scene":"level.scene.yaml","sprites":"level.sprites.json"},
                "progress":progress});
            #[cfg(feature = "collect-ui")]
            let manifest = if with_ui {
                let mut manifest = manifest;
                manifest["entry"]["ui"] = ui_template::descriptor();
                manifest
            } else {
                manifest
            };
            let manifest = json_line(&manifest)?;
            (
                scene,
                json_line(&sprites)?,
                manifest,
                "level.scene.yaml",
                "level.sprites.json",
                collect_template::readme_for(&options.seed, &progress.game_id, &options.template),
            )
        }
        #[cfg(not(feature = "collect-dodge"))]
        {
            let _ = progress;
            return Err("CollectDodge template requires collect-dodge feature".into());
        }
    } else if with_room {
        #[cfg(feature = "room-project")]
        {
            let scene = room_template::scene(&options.seed)?;
            let manifest = json_line(&serde_json::json!({"schema":2,"engine":"^0.0.1",
                "entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}}))?;
            (
                scene,
                Vec::new(),
                manifest,
                "room.scene.yaml",
                "room.models.json",
                room_template::readme(&options.seed),
            )
        }
        #[cfg(not(feature = "room-project"))]
        {
            return Err("Room template requires room-project feature".into());
        }
    } else {
        let (scene, sprites) = template::documents(&options.seed)?;
        (
            scene,
            json_line(&sprites)?,
            MANIFEST.to_vec(),
            "arena.scene.yaml",
            "arena.sprites.json",
            readme(&options.seed),
        )
    };
    let entity_guids = scene.entities.keys().cloned().collect();
    let mut expected = BTreeMap::from([
        ("orr.project.json".into(), manifest),
        (scene_file.into(), scene.to_yaml().into_bytes()),
        ("README.md".into(), readme.into_bytes()),
    ]);
    if !with_room {
        expected.insert(sidecar_file.into(), sidecar_bytes);
    }
    #[cfg(feature = "collect-ui")]
    if with_ui {
        expected.insert(
            "level.ui.json".into(),
            json_line(&crate::authored_ui::Document::default_collect())?,
        );
    }
    for (relative, bytes) in &expected {
        write_new(&project_root.join(relative), bytes)?;
        checkpoint("project-file-written", transaction.path())?;
    }
    let sources = vec![(template::PACKAGE, template::SOURCE)];
    #[cfg(feature = "room-project")]
    let sources = if with_room {
        vec![(room_template::PACKAGE, room_template::SOURCE)]
    } else {
        sources
    };
    #[cfg(feature = "collect-ui")]
    let sources = if with_ui {
        let mut sources = sources;
        sources.push((ui_template::PACKAGE, ui_template::SOURCE));
        sources
    } else {
        sources
    };
    let mut roots = Vec::new();
    for (name, source) in &sources {
        let root = source_root.join(name);
        fs::create_dir(&root).map_err(error)?;
        for (relative, bytes) in *source {
            write_profile_file(&root.join(relative), bytes, with_ui)?;
            checkpoint("source-file-written", transaction.path())?;
        }
        roots.push(root);
    }
    checkpoint("before-install", transaction.path())?;
    let runtime = compiled_runtime();
    #[cfg(feature = "room-project")]
    let runtime = if with_room {
        crate::room_project::compiled_runtime()
    } else {
        runtime
    };
    let project = orr_package::Project::open(&project_root, runtime).map_err(error)?;
    let lock = project
        .install(&roots)
        .map_err(|e| format!("starter package installation: {e}"))?;
    checkpoint("installed", transaction.path())?;
    let verified = project
        .verify()
        .map_err(|e| format!("starter package verification: {e}"))?;
    if verified != lock
        || lock.packages.len() != sources.len()
        || lock.direct.len() != sources.len()
    {
        return Err("starter active lock differs from the closed bundle".into());
    }
    expected.insert("orr.packages.lock.json".into(), json_line(&lock)?);
    for (name, source) in sources {
        let package = lock
            .packages
            .get(name)
            .ok_or("starter package missing after installation")?;
        let bundled_manifest: orr_package::Manifest =
            serde_json::from_slice(source[0].1).map_err(error)?;
        let declared: BTreeSet<_> = source
            .iter()
            .skip(1)
            .map(|(name, _)| (*name).to_owned())
            .collect();
        if lock.direct.get(name) != Some(&bundled_manifest.version)
            || package.manifest != bundled_manifest
            || package.manifest.files != declared
        {
            return Err("installed starter package differs from the closed bundle".into());
        }
        let object = format!(".orr/packages/objects/{}", package.digest);
        expected.insert(
            format!("{object}/orr.package.json"),
            serde_json::to_vec_pretty(&package.manifest).map_err(error)?,
        );
        for (relative, bytes) in source.iter().skip(1) {
            expected.insert(format!("{object}/{relative}"), bytes.to_vec());
        }
    }
    #[cfg(feature = "room-project")]
    if with_room {
        let models = room_template::models(&project, &scene)?;
        let bytes = json_line(&models)?;
        write_new(&project_root.join(sidecar_file), &bytes)?;
        expected.insert(sidecar_file.into(), bytes);
        checkpoint("room-models-written", transaction.path())?;
    }
    fs::remove_dir_all(&source_root).map_err(error)?;
    checkpoint("before-validation", transaction.path())?;
    verify_profile_stage(&project_root, &expected, with_ui)?;
    let initial_checksum = if progress.is_some() {
        #[cfg(feature = "collect-dodge")]
        {
            use crate::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
            PreparedProject::open_with_ui(
                &project_root,
                ProgressSupport::MetadataOnly,
                SpriteSupport::Supported,
                with_ui,
            )?
            .scene()
            .frame()
            .checksum()
        }
        #[cfg(not(feature = "collect-dodge"))]
        {
            return Err("CollectDodge template requires collect-dodge feature".into());
        }
    } else if with_room {
        #[cfg(feature = "room-project")]
        {
            crate::room_project::PreparedProject::open(&project_root)?
                .scene()
                .frame()
                .checksum()
        }
        #[cfg(not(feature = "room-project"))]
        {
            return Err("Room template requires room-project feature".into());
        }
    } else {
        PreparedRuntime::open(&project_root)?
            .initial_frame()
            .checksum()
    };
    checkpoint("before-publish", transaction.path())?;
    verify_profile_stage(&project_root, &expected, with_ui)?;
    // Recheck ordinary parent changes. RENAME_NOREPLACE itself closes concurrent
    // destination creation, including an empty directory or dangling symlink.
    if destination(&options.output)? != output {
        return Err("output parent changed during creation".into());
    }
    publish(&project_root, &output)?;
    // Never clean the published destination. TempDir owns only its unique parent,
    // which now contains no project subtree and was never a caller-owned path.
    drop(transaction);
    Ok(CreateReport {
        output,
        template: options.template.clone(),
        seed: options.seed.clone(),
        initial_checksum,
        entity_guids,
    })
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn json_line(value: &impl serde::Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(error)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_profile_file(path, bytes, false)
}
fn file_limit(path: &Path, bytes: &[u8], with_ui: bool) -> usize {
    #[cfg(feature = "collect-ui")]
    if with_ui
        && path
            .file_name()
            .is_some_and(|name| name == "OrreryKoreanUI.otf")
        && ui_template::is_bundled_font(bytes)
    {
        return ui_template::FONT_BYTES;
    }
    let _ = (path, bytes, with_ui);
    MAX_FILE_BYTES
}
fn write_profile_file(path: &Path, bytes: &[u8], with_ui: bool) -> Result<(), String> {
    if bytes.len() > file_limit(path, bytes, with_ui) {
        return Err("starter file exceeds byte limit".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(error)?;
    file.sync_all().map_err(error)
}

fn destination(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("output path must be UTF-8")?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| matches!(part, "." | ".."))
        || path.components().count() > 64
    {
        return Err(
            "output must be an absolute bounded path without dot or parent traversal".into(),
        );
    }
    let leaf = path
        .file_name()
        .ok_or("output needs a new directory leaf")?;
    if leaf.len() > 200 {
        return Err("output leaf exceeds 200 bytes".into());
    }
    let parent = path.parent().ok_or("output requires an existing parent")?;
    let mut current = PathBuf::new();
    for component in parent.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("invalid output parent component".into());
        }
        current.push(component);
        let meta = fs::symlink_metadata(&current)
            .map_err(|e| format!("output parent {}: {e}", current.display()))?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err("output parent must contain only existing nonsymlink directories".into());
        }
    }
    let output = fs::canonicalize(parent).map_err(error)?.join(leaf);
    match fs::symlink_metadata(&output) {
        Ok(_) => Err("output destination already exists".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Err(e) => Err(error(e)),
    }
}

fn verify_profile_stage(
    root: &Path,
    expected: &BTreeMap<String, Vec<u8>>,
    with_ui: bool,
) -> Result<(), String> {
    let root_metadata = fs::symlink_metadata(root).map_err(error)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("starter stage root must be a nonsymlink directory".into());
    }
    if expected.len() > if with_ui { 17 } else { MAX_FILES }
        || expected.values().map(Vec::len).sum::<usize>() > MAX_TOTAL_BYTES
    {
        return Err("starter payload exceeds transaction limits".into());
    }
    let mut expected_dirs = BTreeSet::new();
    for relative in expected.keys() {
        let mut parent = Path::new(relative).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            expected_dirs.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut seen_files = BTreeSet::new();
    let mut seen_dirs = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(error)? {
            let entry = entry.map_err(error)?;
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(error)?;
            let kind = entry.file_type().map_err(error)?;
            if kind.is_dir() {
                if !expected_dirs.contains(relative) || !seen_dirs.insert(relative.to_path_buf()) {
                    return Err("unexpected starter stage directory".into());
                }
                pending.push(path);
            } else if kind.is_file() {
                let name = relative.to_str().ok_or("stage path must be UTF-8")?;
                let bytes = expected.get(name).ok_or("unexpected starter stage file")?;
                let metadata = fs::symlink_metadata(&path).map_err(error)?;
                if metadata.len() != bytes.len() as u64
                    || bytes.len() > file_limit(&path, bytes, with_ui)
                {
                    return Err("starter stage file size changed".into());
                }
                let mut actual = Vec::new();
                OpenOptions::new()
                    .read(true)
                    .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                    .open(&path)
                    .map_err(error)?
                    .take(file_limit(&path, bytes, with_ui) as u64 + 1)
                    .read_to_end(&mut actual)
                    .map_err(error)?;
                if actual != *bytes || !seen_files.insert(name.to_owned()) {
                    return Err("starter stage bytes changed".into());
                }
            } else {
                return Err("starter stage contains a symlink or special file".into());
            }
        }
    }
    if seen_files != expected.keys().cloned().collect() || seen_dirs != expected_dirs {
        return Err("starter stage file closure changed".into());
    }
    Ok(())
}

fn readme(seed: &str) -> String {
    format!("# New Arena project\n\nTemplate: {TEMPLATE}\nGenerator: {}\nAuthoring seed: {seed}\n\nThis is the existing two-player Arena simulation with sprite idle/walk bindings,\nzero scores and camera follow. It is an editable starter, not a finished collect/dodge game.\n\nOpen with a sprites-enabled `orr_editor --project /absolute/path/to/this-project`.\nRun with a project-enabled `arena --project /absolute/path/to/this-project`.\nUse the existing `orr_export_arena` tool with a trusted prebuilt runtime to export.\n\nThe same seed/template/tool version reproduces these bytes; choose another seed\nfor different authored entity GUIDs. This does not create a new game, network,\nsave or score identity. Runtime remains Arena, seed 42, 60 Hz, two player slots.\nPlayer preferences intentionally remain shared under arena-controls-v1.\n\nThe MIT notice is retained in arena.scene.yaml header comments and the installed\nsample-sprites LICENSE.txt. No assets were regenerated, downloaded or executed.\nNo HOME/XDG settings, caches, captures, replays or editor state were copied.\n", env!("CARGO_PKG_VERSION"))
}
