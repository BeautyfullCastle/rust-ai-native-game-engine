//! Bounded Windows x86-64 export of an already-authored, admitted project.
//!
//! The operator supplies known trusted, already-built code. A hash, PE header,
//! declared revision and successful smoke output do not establish trust or
//! attest source/capabilities. Inputs are read-only and ordinary source changes
//! are detected; hostile filesystem races are not prevented.
#[path = "project_export/admission.rs"]
pub(crate) mod admission;
mod io;
#[path = "project_export/profile.rs"]
pub(crate) mod profile;

use admission::{hash, ProjectSnapshot, SnapshotFile, MAX_BINARY_BYTES};
use profile::{Prepared, Profile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const MANIFEST_NAME: &str = "orr.export.json";
pub const LAUNCHER_NAME: &str = "run-arena";
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SMOKE_OUTPUT: usize = 1024 * 1024;
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(10);
const DOMAIN: &[u8] = b"orrery.arena.export.content.v1\0";
const LAUNCHER: &[u8] = b"#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P)\nexec \"$root/bin/arena\" --project \"$root/project\" \"$@\"\n";
const WINDOWS_TARGET: &str = "windows-x86_64";

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub project: PathBuf,
    pub runtime: PathBuf,
    pub runtime_sha256: String,
    pub output: PathBuf,
    pub trusted_runtime: bool,
    pub source_revision: Option<String>,
}

#[derive(Debug)]
pub struct ExportReport {
    pub output: PathBuf,
    pub content_digest: String,
    pub project_files: usize,
    pub manifest: ExportManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportFile {
    pub path: String,
    pub role: String,
    pub mode: u32,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MeasuredRuntime {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclaredProvenance {
    pub target: String,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestPayload {
    pub exporter_version: String,
    pub profile: String,
    pub entry: orr_package::ProjectEntry,
    pub packages: BTreeMap<String, String>,
    pub files: Vec<ExportFile>,
    pub runtime: MeasuredRuntime,
    pub declared: DeclaredProvenance,
    pub initial_checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportManifest {
    pub schema: u32,
    pub payload: ManifestPayload,
    pub content_digest: String,
}

#[derive(Clone)]
struct WindowsProfile {
    binary: String,
    launcher_name: String,
    name: String,
    launcher: Vec<u8>,
}

/// Export an admitted Arena or Room project (and Collect when compiled).
/// Compilation, installation, and runtime trust decisions are not performed.
pub fn export(options: &ExportOptions) -> Result<ExportReport, String> {
    export_with(
        options,
        |_| Ok(()),
        |command| io::run_bounded(command, SMOKE_TIMEOUT, MAX_SMOKE_OUTPUT),
    )
}

/// Export only a Room project through the same guarded transaction pipeline.
#[cfg(feature = "room-project")]
pub fn export_room(options: &ExportOptions) -> Result<ExportReport, String> {
    export_profile_with(
        options,
        Profile::Room,
        |_| Ok(()),
        |command| io::run_bounded(command, SMOKE_TIMEOUT, MAX_SMOKE_OUTPUT),
    )
}

/// Export the currently pinned Room scene only when its path and checksum match.
#[cfg(feature = "room-project")]
pub fn export_room_for_scene(
    options: &ExportOptions,
    scene: &Path,
    checksum: u64,
) -> Result<ExportReport, String> {
    export_profile_with_scene(
        options,
        Profile::Room,
        scene,
        checksum,
        |_| Ok(()),
        |command| io::run_bounded(command, SMOKE_TIMEOUT, MAX_SMOKE_OUTPUT),
    )
}

/// Measure a bounded regular PE runtime without asserting that it is trusted.
pub fn measure_runtime(path: &Path) -> Result<MeasuredRuntime, String> {
    let runtime_path = io::checked_path(path, false)?;
    let runtime = SnapshotFile::read(&runtime_path, "trusted-runtime.exe", MAX_BINARY_BYTES)?;
    io::validate_pe(&runtime.bytes)?;
    let measured = MeasuredRuntime {
        sha256: runtime.sha256.clone(),
        bytes: runtime.bytes.len() as u64,
    };
    runtime.recheck()?;
    Ok(measured)
}

fn export_with(
    options: &ExportOptions,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    export_auto_profile_with(options, checkpoint, smoke)
}

fn export_auto_profile_with(
    options: &ExportOptions,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    let project_root = io::checked_path(&options.project, true)?;
    let manifest_path = project_root.join("orr.project.json");
    let manifest_path = io::checked_path(&manifest_path, false)?;
    let manifest_file = SnapshotFile::read(
        &manifest_path,
        "orr.project.json",
        admission::MAX_JSON_BYTES,
    )?;
    let manifest: orr_package::ProjectManifest =
        serde_json::from_slice(&manifest_file.bytes).map_err(error)?;
    let entry = manifest
        .entry
        .as_ref()
        .ok_or("export requires an authored game entry")?;
        if entry.tilemap.is_some() { return Err("tilemap projects are not supported by export yet".into()); }
    let profile = profile_for_game(entry.game)?;
    let expected_game = entry.game;
    manifest_file.recheck()?;
    export_profile_with_expected(
        options,
        profile,
        Some(expected_game),
        None,
        checkpoint,
        smoke,
    )
}

#[cfg(feature = "room-project")]
fn export_profile_with(
    options: &ExportOptions,
    profile: Profile,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    export_profile_with_expected(options, profile, None, None, checkpoint, smoke)
}

#[cfg(feature = "room-project")]
fn export_profile_with_scene(
    options: &ExportOptions,
    profile: Profile,
    scene: &Path,
    checksum: u64,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    export_profile_with_expected(
        options,
        profile,
        None,
        Some((scene, checksum)),
        checkpoint,
        smoke,
    )
}

fn export_profile_with_expected(
    options: &ExportOptions,
    profile: Profile,
    expected_game: Option<orr_package::ProjectGame>,
    expected_scene: Option<(&Path, u64)>,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    if !options.trusted_runtime {
        return Err("export requires --trusted-runtime: supply known trusted already-built Windows code; its SHA256 does not establish trust".into());
    }
    if options.runtime_sha256.len() != 64
        || !options
            .runtime_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("runtime SHA256 must be exactly 64 lowercase hexadecimal characters".into());
    }
    validate_source_revision(&options.source_revision)?;

    let project_root = io::checked_path(&options.project, true)?;
    let project = open_project_snapshot(&project_root, profile)?;
    validate_project_snapshot_paths(&project)?;
    if expected_game.is_some_and(|game| game != project.entry.game) {
        return Err("project game changed during Windows export admission".into());
    }
    if !profile_accepts_game(profile, project.entry.game) {
        return Err("project game does not match the selected Windows export profile".into());
    }
    if let Some((scene, checksum)) = expected_scene {
        let requested_scene = io::checked_path(scene, false)?;
        let admitted_scene = project.root.join(&project.entry.scene);
        if requested_scene != admitted_scene || project.initial_checksum != checksum {
            return Err("current editor scene no longer matches the admitted Room project".into());
        }
    }
    let target = windows_profile(profile)?;
    let runtime_path = io::checked_path(&options.runtime, false)?;
    let binary = SnapshotFile::read(&runtime_path, "trusted-runtime.exe", MAX_BINARY_BYTES)?;
    if binary.sha256 != options.runtime_sha256 {
        return Err("trusted runtime SHA256 does not match expected hash".into());
    }
    io::validate_pe(&binary.bytes)?;
    let output = io::destination(&options.output, &project.root, &runtime_path)?;
    checkpoint("admitted")?;

    let parent = output.parent().ok_or("output parent missing")?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-win-export-")
        .tempdir_in(parent)
        .map_err(error)?;
    let mut files = Vec::new();
    write_payload(
        stage.path(),
        &target.binary,
        "runtime",
        &binary.bytes,
        &mut files,
    )?;
    checkpoint("binary-copied")?;
    for file in &project.files {
        write_payload(
            stage.path(),
            &format!("project/{}", file.relative),
            file.role,
            &file.bytes,
            &mut files,
        )?;
        checkpoint("content-copied")?;
    }
    write_payload(
        stage.path(),
        &target.launcher_name,
        "launcher",
        &target.launcher,
        &mut files,
    )?;
    checkpoint("before-validation")?;

    let staged_root = stage.path().join("project");
    let staged_project = open_project_snapshot(&staged_root, profile)?;
    let staged_prepared = Prepared::open(&staged_root, profile)?;
    if staged_project.initial_checksum != project.initial_checksum
        || staged_prepared.checksum() != project.initial_checksum
        || staged_prepared.root() != staged_project.root
        || staged_prepared.scene_path() != staged_project.root.join(&project.entry.scene)
        || staged_prepared.scene_text().as_bytes()
            != project
                .files
                .iter()
                .find(|file| file.relative == project.entry.scene)
                .ok_or("admitted project scene is missing from its closure")?
                .bytes
                .as_slice()
        || !same_project_closure(&project, &staged_project)
    {
        return Err("staged project differs from its admitted source".into());
    }

    let smoke_cwd = stage.path().join(".smoke-cwd");
    fs::create_dir(&smoke_cwd).map_err(error)?;
    let staged_binary = stage.path().join(&target.binary);
    verify_staged_file(&staged_binary, binary.bytes.len() as u64, &binary.sha256)?;
    let expected_stdout = staged_prepared.smoke()?;
    let mut command = Command::new(&staged_binary);
    command
        .env_clear()
        .arg("--project")
        .arg(&staged_root)
        .args(["--headless", "--ticks", "0"])
        .current_dir(&smoke_cwd);
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    if let Some(windir) = std::env::var_os("WINDIR") {
        command.env("WINDIR", windir);
    }
    let stdout = smoke(&mut command)?;
    if stdout != expected_stdout {
        return Err(
            "trusted Windows runtime smoke output does not match the staged project".into(),
        );
    }
    fs::remove_dir(&smoke_cwd)
        .map_err(|error| format!("smoke working directory must remain empty: {error}"))?;
    checkpoint("after-validation")?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    let payload = ManifestPayload {
        exporter_version: env!("CARGO_PKG_VERSION").into(),
        profile: target.name.clone(),
        entry: project.entry.clone(),
        packages: project.package_identities.clone(),
        files,
        runtime: MeasuredRuntime {
            sha256: binary.sha256.clone(),
            bytes: binary.bytes.len() as u64,
        },
        declared: DeclaredProvenance {
            target: WINDOWS_TARGET.into(),
            source_revision: options.source_revision.clone(),
        },
        initial_checksum: format!("0x{:016x}", project.initial_checksum),
    };
    let canonical = serde_json::to_vec(&payload).map_err(error)?;
    let mut digest_bytes =
        Vec::with_capacity(profile.domain().len() + WINDOWS_TARGET.len() + canonical.len());
    digest_bytes.extend_from_slice(profile.domain());
    digest_bytes.extend_from_slice(WINDOWS_TARGET.as_bytes());
    digest_bytes.push(0);
    digest_bytes.extend_from_slice(&canonical);
    let manifest = ExportManifest {
        schema: 1,
        payload,
        content_digest: hash(&digest_bytes),
    };
    let mut manifest_bytes = serde_json::to_vec(&manifest).map_err(error)?;
    manifest_bytes.push(b'\n');
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("export manifest exceeds size limit".into());
    }
    io::write_payload(stage.path(), MANIFEST_NAME, &manifest_bytes)?;
    checkpoint("before-publish")?;

    project.recheck()?;
    validate_project_snapshot_paths(&project)?;
    project.recheck()?;
    validate_project_snapshot_paths(&project)?;
    staged_project.recheck()?;
    binary.recheck()?;
    verify_stage(stage.path(), &manifest, &manifest_bytes)?;
    io::publish(stage.path(), &output)?;
    drop(stage);
    Ok(ExportReport {
        output,
        content_digest: manifest.content_digest.clone(),
        project_files: project.files.len(),
        manifest,
    })
}

fn profile_for_game(game: orr_package::ProjectGame) -> Result<Profile, String> {
    match game {
        orr_package::ProjectGame::Arena => Ok(Profile::Arena),
        #[cfg(feature = "room-project")]
        orr_package::ProjectGame::RoomEscapeV1 => Ok(Profile::Room),
        #[cfg(not(feature = "room-project"))]
        orr_package::ProjectGame::RoomEscapeV1 => {
            Err("Room export requires the room-project feature".into())
        }
        #[cfg(feature = "collect-dodge")]
        orr_package::ProjectGame::CollectDodgeV1 => Ok(Profile::Collect),
        #[cfg(not(feature = "collect-dodge"))]
        orr_package::ProjectGame::CollectDodgeV1 => {
            Err("Collect export requires the collect-dodge feature".into())
        }
    }
}

fn profile_accepts_game(profile: Profile, game: orr_package::ProjectGame) -> bool {
    match profile {
        Profile::Arena => game == orr_package::ProjectGame::Arena,
        #[cfg(feature = "room-project")]
        Profile::Room => game == orr_package::ProjectGame::RoomEscapeV1,
        #[cfg(feature = "collect-dodge")]
        Profile::Collect => game == orr_package::ProjectGame::CollectDodgeV1,
    }
}

fn open_project_snapshot(root: &Path, profile: Profile) -> Result<ProjectSnapshot, String> {
    match profile {
        Profile::Arena => ProjectSnapshot::open(root),
        #[cfg(feature = "room-project")]
        Profile::Room => ProjectSnapshot::open_profile(root, profile),
        #[cfg(feature = "collect-dodge")]
        Profile::Collect => ProjectSnapshot::open_profile(root, profile),
    }
}

fn windows_profile(profile: Profile) -> Result<WindowsProfile, String> {
    let binary = profile.binary();
    let launcher_name = profile.launcher_name();
    let source_launcher = std::str::from_utf8(profile.launcher())
        .map_err(|_| "closed source launcher is not UTF-8")?;
    let expected_binary = format!("$root/{binary}");
    if !source_launcher.contains(&expected_binary) || !source_launcher.contains("--project") {
        return Err("closed source profile launcher does not match its binary contract".into());
    }
    let name = profile.name().replace("linux-x86_64", "windows-x86_64");
    if name == profile.name() {
        return Err("closed source profile has no Windows target mapping".into());
    }
    let binary = format!("{binary}.exe");
    let launcher_name = format!("{launcher_name}.cmd");
    let launcher = windows_launcher(&binary);
    Ok(WindowsProfile {
        binary,
        launcher_name,
        name,
        launcher,
    })
}

fn windows_launcher(binary: &str) -> Vec<u8> {
    let executable = binary.replace('/', "\\");
    format!(
        "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\"%~dp0{executable}\" --project \"%~dp0project\"\r\nexit /b %ERRORLEVEL%\r\n"
    )
    .into_bytes()
}

fn same_project_closure(source: &ProjectSnapshot, staged: &ProjectSnapshot) -> bool {
    source.entry == staged.entry
        && source.package_identities == staged.package_identities
        && source.initial_checksum == staged.initial_checksum
        && source.files.len() == staged.files.len()
        && source.files.iter().zip(&staged.files).all(|(left, right)| {
            left.relative == right.relative
                && left.role == right.role
                && left.sha256 == right.sha256
                && left.bytes == right.bytes
        })
}

fn validate_project_snapshot_paths(project: &ProjectSnapshot) -> Result<(), String> {
    for file in &project.files {
        let path = io::checked_path(&project.root.join(&file.relative), false)?;
        if path == project.root || !path.starts_with(&project.root) {
            return Err("admitted project file resolves outside its project root".into());
        }
    }
    Ok(())
}

fn write_payload(
    root: &Path,
    relative: &str,
    role: &str,
    bytes: &[u8],
    files: &mut Vec<ExportFile>,
) -> Result<(), String> {
    admission::validate_relative(relative)?;
    io::write_payload(root, relative, bytes)?;
    files.push(ExportFile {
        path: relative.into(),
        role: role.into(),
        mode: 0,
        bytes: bytes.len() as u64,
        sha256: hash(bytes),
    });
    Ok(())
}

fn verify_stage(
    root: &Path,
    manifest: &ExportManifest,
    manifest_bytes: &[u8],
) -> Result<(), String> {
    let mut expected: BTreeMap<String, (u64, String)> = manifest
        .payload
        .files
        .iter()
        .map(|file| (file.path.clone(), (file.bytes, file.sha256.clone())))
        .collect();
    expected.insert(
        MANIFEST_NAME.into(),
        (manifest_bytes.len() as u64, hash(manifest_bytes)),
    );
    let mut directories = BTreeSet::new();
    for path in expected.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent.filter(|value| !value.as_os_str().is_empty()) {
            directories.insert(directory.to_path_buf());
            parent = directory.parent();
        }
    }
    let mut pending = vec![PathBuf::new()];
    while let Some(directory) = pending.pop() {
        let directory_path = root.join(&directory);
        io::checked_path(&directory_path, true)?;
        for entry in fs::read_dir(&directory_path).map_err(error)? {
            let entry = entry.map_err(error)?;
            let child = directory.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
            if metadata.is_dir() {
                if !directories.remove(&child) {
                    return Err("unexpected staged directory".into());
                }
                io::checked_path(&entry.path(), true)?;
                pending.push(child);
            } else if metadata.is_file() {
                let relative = child
                    .to_str()
                    .ok_or("non-UTF8 staged path")?
                    .replace('\\', "/");
                let (length, digest) =
                    expected.remove(&relative).ok_or("unexpected staged file")?;
                verify_staged_file(&entry.path(), length, &digest)?;
            } else {
                return Err("staged special file rejected".into());
            }
        }
    }
    if !expected.is_empty() || !directories.is_empty() {
        return Err("staged payload missing".into());
    }
    Ok(())
}

fn verify_staged_file(path: &Path, length: u64, digest: &str) -> Result<(), String> {
    let path = io::checked_path(path, false)?;
    let before = fs::metadata(&path).map_err(error)?;
    if before.len() != length || !before.is_file() {
        return Err("staged file length or type changed".into());
    }
    let mut file = File::open(&path).map_err(error)?;
    let mut hasher = Sha256::new();
    let mut bytes_read = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        bytes_read = bytes_read
            .checked_add(count as u64)
            .ok_or("staged byte count overflow")?;
        if bytes_read > length {
            return Err("staged file grew".into());
        }
        hasher.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(error)?;
    let actual_digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes_read != length
        || actual_digest != digest
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err("staged file content or metadata changed".into());
    }
    Ok(())
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn validate_source_revision(revision: &Option<String>) -> Result<(), String> {
    if let Some(revision) = revision {
        if revision.is_empty()
            || revision.starts_with('/')
            || revision.len() > 128
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._/-".contains(&byte))
        {
            return Err(
                "declared source revision must be a nonempty ASCII token of at most 128 bytes"
                    .into(),
            );
        }
    }
    Ok(())
}

fn expected_smoke(prepared: &crate::project_runtime::PreparedRuntime) -> Result<Vec<u8>, String> {
    let checksum = prepared.initial_frame().checksum();
    let mut expected = format!("project initial checksum: 0x{checksum:016x}\nproject tick: 0 checksum: 0x{checksum:016x}\n");
    for (guid, entity) in prepared.index().iter() {
        if let Some(pos) = prepared
            .initial_frame()
            .get::<orr_testgame::Position>(entity)
        {
            use std::fmt::Write;
            writeln!(
                expected,
                "project entity: {guid} handle:{}v{} position:{},{}",
                entity.index,
                entity.version,
                pos.pos.x.raw(),
                pos.pos.y.raw()
            )
            .map_err(error)?;
        }
    }
    Ok(expected.into_bytes())
}

#[cfg(all(test, feature = "room-project"))]
mod tests;
