//! Exact saved-source assertions for the editor's bounded Arena export route.
//!
//! These values are neither trust credentials nor serializable deployment data.
//! Package assets remain governed by the exporter's entire active-lock admission.
use super::{
    admission::{validate_relative, ProjectSnapshot, MAX_JSON_BYTES, MAX_SCENE_BYTES},
    export_profile_for_source_with, run_bounded, ExportOptions, ExportReport, Profile,
    MAX_SMOKE_OUTPUT, SMOKE_TIMEOUT,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

const PROJECT_MANIFEST: &str = "orr.project.json";
const PACKAGE_LOCK: &str = "orr.packages.lock.json";
const MAX_SOURCE_FILES: usize = 4;
const MAX_SOURCE_BYTES: u64 = MAX_SCENE_BYTES + 3 * MAX_JSON_BYTES;
const MAX_CANONICAL_PATH_BYTES: usize = 4096;

/// Exact bytes of one saved authority file, not authentication of its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSourceDigest {
    bytes: u64,
    sha256: String,
}
impl ArenaSourceDigest {
    pub fn new(bytes: u64, sha256: String) -> Result<Self, String> {
        let value = Self { bytes, sha256 };
        value.validate()?;
        Ok(value)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn validate(&self) -> Result<(), String> {
        if self.bytes > MAX_SCENE_BYTES {
            return Err("Arena export source file exceeds its byte bound".into());
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "Arena export source SHA256 must be 64 lowercase hexadecimal characters".into(),
            );
        }
        Ok(())
    }
}

/// A portable project-relative authority path and its captured presence.
/// A complete [`ArenaExportSource`] permits only the lock to be absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSourceFile {
    path: String,
    present: Option<ArenaSourceDigest>,
}
impl ArenaSourceFile {
    pub fn new(path: String, present: Option<ArenaSourceDigest>) -> Result<Self, String> {
        let value = Self { path, present };
        value.validate()?;
        Ok(value)
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn present(&self) -> Option<&ArenaSourceDigest> {
        self.present.as_ref()
    }

    fn validate(&self) -> Result<(), String> {
        validate_relative(&self.path)?;
        if let Some(digest) = &self.present {
            digest.validate()?;
            if matches!(self.path.as_str(), PROJECT_MANIFEST | PACKAGE_LOCK)
                && digest.bytes > MAX_JSON_BYTES
            {
                return Err("Arena export authority exceeds its JSON byte bound".into());
            }
        }
        Ok(())
    }
}

/// An immutable, bounded assertion of the saved Arena selected by the caller.
///
/// Inventory must contain exactly the manifest, an explicit present/absent lock,
/// entry scene and, if declared, sprite sidecar. It does not replace active-lock
/// asset verification, runtime trust, source rechecks or publication safeguards.
/// Constructors only check shape; export compares against its one admitted
/// snapshot before reading supplied executable code or creating a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaExportSource {
    canonical_scene: PathBuf,
    initial_checksum: u64,
    inventory: Vec<ArenaSourceFile>,
}
impl ArenaExportSource {
    pub fn new(
        canonical_scene: PathBuf,
        initial_checksum: u64,
        inventory: Vec<ArenaSourceFile>,
    ) -> Result<Self, String> {
        let value = Self {
            canonical_scene,
            initial_checksum,
            inventory,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn canonical_scene(&self) -> &Path {
        &self.canonical_scene
    }

    pub fn initial_checksum(&self) -> u64 {
        self.initial_checksum
    }

    pub fn inventory(&self) -> &[ArenaSourceFile] {
        &self.inventory
    }

    fn validate(&self) -> Result<(), String> {
        let scene = &self.canonical_scene;
        if !scene.is_absolute()
            || scene.file_name().is_none()
            || scene.as_os_str().as_encoded_bytes().len() > MAX_CANONICAL_PATH_BYTES
            || scene
                .to_str()
                .is_none_or(|text| text.chars().any(char::is_control))
            || scene
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            || scene.components().collect::<PathBuf>().as_os_str() != scene.as_os_str()
        {
            return Err("Arena export scene path must be bounded, absolute and canonical".into());
        }
        if !(3..=MAX_SOURCE_FILES).contains(&self.inventory.len()) {
            return Err("Arena export source inventory must contain 3 or 4 authority files".into());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.inventory {
            file.validate()?;
            if file.present.is_none() && file.path != PACKAGE_LOCK {
                return Err("only the Arena export package lock may be absent".into());
            }
            let folded = file.path.to_ascii_lowercase();
            if !paths.insert(folded) {
                return Err("duplicate or case-colliding Arena export source path".into());
            }
            if let Some(digest) = &file.present {
                total = total
                    .checked_add(digest.bytes)
                    .ok_or("Arena export source byte count overflow")?;
            }
        }
        if total > MAX_SOURCE_BYTES {
            return Err("Arena export source inventory exceeds its total byte bound".into());
        }
        for path in &paths {
            for (index, _) in path.match_indices('/') {
                if paths.contains(&path[..index]) {
                    return Err("Arena export source file/directory collision".into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_snapshot(&self, project: &ProjectSnapshot) -> Result<(), String> {
        self.validate()?;
        let entry = &project.entry;
        if entry.game != orr_package::ProjectGame::Arena
            || entry.tilemap.is_some()
            || entry.ui.is_some()
            || entry.models.is_some()
            || entry.camera.is_some()
        {
            return Err(
                "editor Arena export supports only Arena scene and sprites, without tilemap or UI"
                    .into(),
            );
        }
        // root and entry paths have already been validated, read without symlink
        // ancestors and retained by this snapshot. Do not reopen a second project.
        if self.canonical_scene != project.root.join(&entry.scene) {
            return Err("Arena export source canonical scene does not match admitted entry".into());
        }
        if self.initial_checksum != project.initial_checksum {
            return Err(
                "Arena export source initial checksum does not match admitted entry".into(),
            );
        }
        let mut required = BTreeMap::from([
            (PROJECT_MANIFEST, MAX_JSON_BYTES),
            (PACKAGE_LOCK, MAX_JSON_BYTES),
            (entry.scene.as_str(), MAX_SCENE_BYTES),
        ]);
        if let Some(sprites) = entry.sprites.as_deref() {
            required.insert(sprites, MAX_JSON_BYTES);
        }
        if self.inventory.len() != required.len() {
            return Err(
                "Arena export source inventory is incomplete or contains extra entries".into(),
            );
        }
        for expected in &self.inventory {
            let limit = required
                .remove(expected.path.as_str())
                .ok_or("Arena export source inventory contains an unexpected entry")?;
            let admitted = project
                .files
                .iter()
                .find(|file| file.relative == expected.path);
            match (&expected.present, admitted) {
                (Some(digest), Some(actual))
                    if digest.bytes <= limit
                        && digest.bytes == actual.bytes.len() as u64
                        && digest.sha256 == actual.sha256 => {}
                (None, None) if expected.path == PACKAGE_LOCK => {}
                _ => {
                    return Err(format!(
                        "Arena export source bytes or presence changed: {}",
                        expected.path
                    ));
                }
            }
        }
        if !required.is_empty() {
            return Err("Arena export source inventory is missing a required entry".into());
        }
        Ok(())
    }
}

/// Export the exact saved Arena asserted by the caller into a new directory.
///
/// This uses the existing exporter transaction, including full active-lock
/// verification, explicit runtime trust, smoke, final rechecks and no-replace
/// publication. Source assertions never enter the deployment manifest.
pub fn export_arena_for_source(
    options: &ExportOptions,
    source: &ArenaExportSource,
) -> Result<ExportReport, String> {
    export_profile_for_source_with(
        options,
        Profile::Arena,
        Some(source),
        |_| Ok(()),
        |command| run_bounded(command, SMOKE_TIMEOUT, MAX_SMOKE_OUTPUT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_export::{admission::hash, expected_smoke, MANIFEST_NAME};
    use std::{
        cell::Cell,
        fs,
        os::unix::fs::{symlink, PermissionsExt},
        process::Command,
    };

    const SCENE: &str = "arena.scene.yaml";
    const SPRITES: &str = "arena.sprites.json";

    struct Fixture {
        temp: tempfile::TempDir,
        options: ExportOptions,
    }
    impl Fixture {
        fn new(sprites: bool, lock: bool) -> Self {
            let temp =
                tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let project = temp.path().join("source");
            fs::create_dir(&project).unwrap();
            let mut manifest = serde_json::json!({
                "schema": 2, "engine": "*", "entry": {"game": "arena", "scene": SCENE}
            });
            if sprites {
                manifest["entry"]["sprites"] = SPRITES.into();
                fs::write(
                    project.join(SPRITES),
                    br#"{"version":1,"scene":"arena.scene.yaml","project":".","bindings":{}}"#,
                )
                .unwrap();
            }
            fs::write(
                project.join(PROJECT_MANIFEST),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::write(
                project.join(SCENE),
                include_bytes!("../../../../assets/saved_arena_project/arena.scene.yaml"),
            )
            .unwrap();
            if lock {
                fs::write(
                    project.join(PACKAGE_LOCK),
                    serde_json::to_vec(&orr_package::Lock::default()).unwrap(),
                )
                .unwrap();
            }
            // The injected smoke never executes this bounded ELF header fixture.
            // Actual already-built player execution remains an integration test.
            let mut binary = vec![0; 64];
            binary[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
            binary[16] = 2;
            binary[18] = 62;
            binary[52] = 64;
            let runtime = temp.path().join("trusted-runtime");
            fs::write(&runtime, &binary).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                options: ExportOptions {
                    project,
                    runtime,
                    runtime_sha256: hash(&binary),
                    output: temp.path().join("export"),
                    trusted_runtime: true,
                    source_revision: None,
                },
                temp,
            }
        }

        fn capture(&self) -> ArenaExportSource {
            let snapshot = ProjectSnapshot::open(&self.options.project).unwrap();
            capture(&snapshot)
        }

        fn smoke(&self) -> Vec<u8> {
            expected_smoke(
                &crate::project_runtime::PreparedRuntime::open(&self.options.project).unwrap(),
            )
            .unwrap()
        }

        fn run(
            &self,
            source: &ArenaExportSource,
            checkpoint: impl FnMut(&str) -> Result<(), String>,
            smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
        ) -> Result<ExportReport, String> {
            export_profile_for_source_with(
                &self.options,
                Profile::Arena,
                Some(source),
                checkpoint,
                smoke,
            )
        }

        fn no_stage(&self) {
            assert!(fs::read_dir(self.temp.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-export-")));
        }

        fn stage(&self) -> PathBuf {
            fs::read_dir(self.temp.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".orr-export-")
                })
                .unwrap()
        }

        fn reject_before_runtime(&mut self, source: &ArenaExportSource, message: &str) {
            let original_runtime = self.options.runtime.clone();
            // A missing executable is a sentinel: source errors must win before
            // even runtime-path inspection, not merely before smoke invocation.
            self.options.runtime = self.temp.path().join("must-not-be-inspected");
            let expected_error = export_arena_for_source(&self.options, source).unwrap_err();
            assert!(
                expected_error.contains(message),
                "expected {message:?}, got {expected_error:?}"
            );
            let checkpoints = Cell::new(0);
            let smoke_calls = Cell::new(0);
            let error = self
                .run(
                    source,
                    |_| {
                        checkpoints.set(checkpoints.get() + 1);
                        Ok(())
                    },
                    |_| {
                        smoke_calls.set(smoke_calls.get() + 1);
                        Ok(Vec::new())
                    },
                )
                .unwrap_err();
            assert!(error.contains(message), "{error}");
            assert_eq!(
                checkpoints.get(),
                0,
                "no runtime-admitted or stage boundary"
            );
            assert_eq!(smoke_calls.get(), 0, "must not invoke supplied code");
            assert!(!self.options.output.exists());
            self.no_stage();
            self.options.runtime = original_runtime;
        }
    }

    fn capture(snapshot: &ProjectSnapshot) -> ArenaExportSource {
        let mut paths = vec![
            PROJECT_MANIFEST,
            PACKAGE_LOCK,
            snapshot.entry.scene.as_str(),
        ];
        if let Some(path) = snapshot.entry.sprites.as_deref() {
            paths.push(path);
        }
        let inventory = paths
            .into_iter()
            .map(|path| {
                ArenaSourceFile::new(
                    path.into(),
                    snapshot
                        .files
                        .iter()
                        .find(|file| file.relative == path)
                        .map(|file| {
                            ArenaSourceDigest::new(file.bytes.len() as u64, file.sha256.clone())
                                .unwrap()
                        }),
                )
                .unwrap()
            })
            .collect();
        ArenaExportSource::new(
            snapshot.root.join(&snapshot.entry.scene),
            snapshot.initial_checksum,
            inventory,
        )
        .unwrap()
    }

    fn digest(bytes: u64) -> ArenaSourceDigest {
        ArenaSourceDigest::new(bytes, "a".repeat(64)).unwrap()
    }

    #[test]
    fn constructors_bound_hash_bytes_paths_count_and_presence() {
        for hash in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            assert!(ArenaSourceDigest::new(1, hash).is_err());
        }
        assert!(ArenaSourceDigest::new(MAX_SCENE_BYTES + 1, "a".repeat(64)).is_err());
        assert!(ArenaSourceDigest::new(u64::MAX, "a".repeat(64)).is_err());
        assert!(
            ArenaSourceFile::new(PROJECT_MANIFEST.into(), Some(digest(MAX_JSON_BYTES + 1)))
                .is_err()
        );
        assert!(
            ArenaSourceFile::new(PACKAGE_LOCK.into(), Some(digest(MAX_JSON_BYTES + 1))).is_err()
        );
        for path in [
            "",
            ".",
            "..",
            "/scene",
            "../scene",
            "a/../b",
            "a/./b",
            "a//b",
            "a\\b",
            "C:scene",
            "CON",
            "a/NUL.txt",
            "bad\npath",
            "é.yaml",
            "trailing.",
        ] {
            assert!(
                ArenaSourceFile::new(path.into(), Some(digest(1))).is_err(),
                "{path:?}"
            );
        }
        for path in [
            "a".repeat(101),
            ["a"; 33].join("/"),
            vec!["a".repeat(100); 6].join("/"),
        ] {
            assert!(ArenaSourceFile::new(path, Some(digest(1))).is_err());
        }
        assert!(ArenaSourceFile::new(SCENE.into(), None).is_ok());
        assert!(ArenaSourceFile::new(PROJECT_MANIFEST.into(), None).is_ok());
        assert!(ArenaSourceFile::new(PACKAGE_LOCK.into(), None).is_ok());
        let fixture = Fixture::new(false, false);
        let source = fixture.capture();
        for path in [PROJECT_MANIFEST, SCENE] {
            let mut inventory = source.inventory.clone();
            inventory
                .iter_mut()
                .find(|file| file.path == path)
                .unwrap()
                .present = None;
            assert!(ArenaExportSource::new(source.canonical_scene.clone(), 0, inventory).is_err());
        }
        for path in [
            PathBuf::from("relative.yaml"),
            PathBuf::from("/"),
            PathBuf::from("/tmp/../scene.yaml"),
            PathBuf::from("/tmp/./scene.yaml"),
            PathBuf::from("/tmp//scene.yaml"),
            PathBuf::from("/tmp/scene.yaml/"),
            PathBuf::from("/tmp/scene\n.yaml"),
            PathBuf::from(format!("/{}", "a".repeat(MAX_CANONICAL_PATH_BYTES))),
        ] {
            assert!(ArenaExportSource::new(path, 0, source.inventory.clone()).is_err());
        }
        for count in [0, 1, 2, 5] {
            let inventory = (0..count)
                .map(|index| ArenaSourceFile::new(format!("file{index}"), Some(digest(1))).unwrap())
                .collect();
            assert!(ArenaExportSource::new(source.canonical_scene.clone(), 0, inventory).is_err());
        }
        assert_eq!(
            source.canonical_scene(),
            fixture.options.project.join(SCENE)
        );
        assert_eq!(source.initial_checksum(), source.initial_checksum);
        assert_eq!(source.inventory().len(), 3);
        assert_eq!(source.inventory()[0].path(), PROJECT_MANIFEST);
        let first = source.inventory()[0].present().unwrap();
        assert_eq!(
            first.bytes(),
            fs::metadata(fixture.options.project.join(PROJECT_MANIFEST))
                .unwrap()
                .len()
        );
        assert_eq!(
            first.sha256(),
            hash(&fs::read(fixture.options.project.join(PROJECT_MANIFEST)).unwrap())
        );
    }

    #[test]
    fn constructors_reject_duplicates_collisions_and_total_overflow_bound() {
        let fixture = Fixture::new(false, false);
        let source = fixture.capture();
        for extra in [
            source.inventory[0].clone(),
            ArenaSourceFile::new("ORR.PROJECT.JSON".into(), Some(digest(1))).unwrap(),
        ] {
            let mut inventory = source.inventory.clone();
            inventory.push(extra);
            assert!(ArenaExportSource::new(source.canonical_scene.clone(), 0, inventory).is_err());
        }
        let directory_collision = ["a", "a/b", "c"]
            .map(|path| ArenaSourceFile::new(path.into(), Some(digest(1))).unwrap());
        assert!(ArenaExportSource::new(
            source.canonical_scene.clone(),
            0,
            directory_collision.to_vec()
        )
        .is_err());
        let too_large = ["a", "b", "c"]
            .map(|path| ArenaSourceFile::new(path.into(), Some(digest(MAX_SCENE_BYTES))).unwrap());
        assert!(ArenaExportSource::new(source.canonical_scene, 0, too_large.to_vec()).is_err());
    }

    #[test]
    fn exact_complete_inventory_exports_with_and_without_sprites_or_lock() {
        for sprites in [false, true] {
            for lock in [false, true] {
                let fixture = Fixture::new(sprites, lock);
                let mut source = fixture.capture();
                // Complete inventory is a set, not an order-sensitive spelling.
                source.inventory.reverse();
                let before = fs::read(fixture.options.project.join(SCENE)).unwrap();
                let expected = fixture.smoke();
                let smoke_calls = Cell::new(0);
                let report = fixture
                    .run(
                        &source,
                        |_| Ok(()),
                        |_| {
                            smoke_calls.set(smoke_calls.get() + 1);
                            Ok(expected)
                        },
                    )
                    .unwrap();
                assert_eq!(smoke_calls.get(), 1);
                assert_eq!(
                    report.project_files,
                    2 + usize::from(sprites) + usize::from(lock)
                );
                assert_eq!(
                    report.manifest.payload.profile,
                    "arena-authored-linux-x86_64-v1"
                );
                assert_eq!(
                    fs::read(report.output.join("project").join(SCENE)).unwrap(),
                    before
                );
                assert_eq!(
                    fs::read(fixture.options.project.join(SCENE)).unwrap(),
                    before
                );
                let manifest = fs::read_to_string(report.output.join(MANIFEST_NAME)).unwrap();
                assert!(!manifest.contains(source.canonical_scene().to_str().unwrap()));
                let previous = fs::read(report.output.join(MANIFEST_NAME)).unwrap();
                assert!(fixture
                    .run(
                        &source,
                        |_| Ok(()),
                        |_| panic!("existing output must not smoke")
                    )
                    .is_err());
                assert_eq!(
                    fs::read(report.output.join(MANIFEST_NAME)).unwrap(),
                    previous
                );
                fixture.no_stage();
            }
        }
    }

    #[test]
    fn canonical_scene_and_initial_checksum_are_independent_requirements() {
        let mut fixture = Fixture::new(false, false);
        let mut source = fixture.capture();
        let original = source.clone();
        let other = fixture.options.project.join("other.scene.yaml");
        fs::copy(&source.canonical_scene, &other).unwrap();
        source.canonical_scene = other.clone();
        fixture.reject_before_runtime(&source, "canonical scene");
        let alias = fixture.temp.path().join("scene-alias");
        symlink(&original.canonical_scene, &alias).unwrap();
        source.canonical_scene = alias;
        fixture.reject_before_runtime(&source, "canonical scene");
        source = original;
        source.initial_checksum ^= 1;
        fixture.reject_before_runtime(&source, "initial checksum");
    }

    #[test]
    fn unchanged_checksum_does_not_hide_different_raw_scene_bytes() {
        let mut fixture = Fixture::new(false, false);
        let path = fixture.options.project.join(SCENE);
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, [b"# a\n".as_slice(), &bytes].concat()).unwrap();
        let source = fixture.capture();
        fs::write(&path, [b"# b\n".as_slice(), &bytes].concat()).unwrap();
        let current = fixture.capture();
        assert_eq!(source.initial_checksum(), current.initial_checksum());
        assert_eq!(
            source.inventory()[2].present().unwrap().bytes(),
            current.inventory()[2].present().unwrap().bytes()
        );
        assert_ne!(
            source.inventory()[2].present().unwrap().sha256(),
            current.inventory()[2].present().unwrap().sha256()
        );
        fixture.reject_before_runtime(&source, "bytes or presence changed");
    }

    #[test]
    fn raw_manifest_lock_and_sprite_changes_reject_before_runtime_or_stage() {
        for relative in [PROJECT_MANIFEST, PACKAGE_LOCK, SPRITES] {
            let mut fixture = Fixture::new(true, true);
            let source = fixture.capture();
            let path = fixture.options.project.join(relative);
            let original = fs::read(&path).unwrap();
            let changed = [original.as_slice(), b"\n"].concat();
            fs::write(&path, &changed).unwrap();
            assert_eq!(
                fixture.capture().initial_checksum(),
                source.initial_checksum()
            );
            fixture.reject_before_runtime(&source, "bytes or presence changed");
            assert_eq!(
                fs::read(path).unwrap(),
                changed,
                "export never saves or repairs source"
            );
        }
    }

    #[test]
    fn missing_extra_and_wrong_presence_inventory_reject_before_runtime() {
        for omitted in [PROJECT_MANIFEST, PACKAGE_LOCK, SCENE, SPRITES] {
            let mut fixture = Fixture::new(true, true);
            let mut source = fixture.capture();
            source.inventory.retain(|file| file.path() != omitted);
            fixture.reject_before_runtime(&source, "inventory");
        }
        let mut fixture = Fixture::new(false, false);
        let mut source = fixture.capture();
        source
            .inventory
            .push(ArenaSourceFile::new("unrelated.json".into(), Some(digest(1))).unwrap());
        fixture.reject_before_runtime(&source, "inventory");
        let mut source = fixture.capture();
        source.inventory[2] =
            ArenaSourceFile::new("unrelated.yaml".into(), Some(digest(1))).unwrap();
        fixture.reject_before_runtime(&source, "unexpected entry");
        for present in [false, true] {
            let mut fixture = Fixture::new(false, present);
            let mut source = fixture.capture();
            source.inventory[1].present = if present { None } else { Some(digest(1)) };
            fixture.reject_before_runtime(&source, "bytes or presence changed");
        }
    }

    #[test]
    fn lock_appearance_and_disappearance_are_not_equal_to_empty_lock() {
        for present in [false, true] {
            let mut fixture = Fixture::new(false, present);
            let source = fixture.capture();
            let lock = fixture.options.project.join(PACKAGE_LOCK);
            if present {
                fs::remove_file(lock).unwrap();
            } else {
                fs::write(
                    lock,
                    serde_json::to_vec(&orr_package::Lock::default()).unwrap(),
                )
                .unwrap();
            }
            assert_eq!(
                source.initial_checksum(),
                fixture.capture().initial_checksum()
            );
            fixture.reject_before_runtime(&source, "bytes or presence changed");
        }
    }

    #[test]
    fn byte_lengths_are_compared_as_well_as_digests() {
        let mut fixture = Fixture::new(true, true);
        for index in 0..4 {
            let mut source = fixture.capture();
            source.inventory[index].present.as_mut().unwrap().bytes += 1;
            fixture.reject_before_runtime(&source, "bytes or presence changed");
        }
    }

    #[test]
    fn expected_source_is_revalidated_at_transaction_boundary() {
        let mut fixture = Fixture::new(false, false);
        let source = fixture.capture();
        let mut invalid = source.clone();
        invalid.inventory[0].present.as_mut().unwrap().sha256 = "A".repeat(64);
        fixture.reject_before_runtime(&invalid, "lowercase hexadecimal");
        let mut invalid = source.clone();
        invalid.inventory[0].path = "../manifest".into();
        fixture.reject_before_runtime(&invalid, "invalid relative");
        let mut invalid = source;
        invalid.inventory[2] = invalid.inventory[0].clone();
        fixture.reject_before_runtime(&invalid, "duplicate");
    }

    #[test]
    fn source_shape_rejects_other_games_and_all_unsupported_entries() {
        let fixture = Fixture::new(false, false);
        let source = fixture.capture();
        let mut snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
        let original = snapshot.entry.clone();
        for game in [
            orr_package::ProjectGame::CollectDodgeV1,
            orr_package::ProjectGame::RoomEscapeV1,
        ] {
            snapshot.entry.game = game;
            assert!(source
                .check_snapshot(&snapshot)
                .unwrap_err()
                .contains("only Arena"));
        }
        for field in ["tilemap", "ui", "models", "camera"] {
            snapshot.entry = original.clone();
            match field {
                "tilemap" => snapshot.entry.tilemap = Some("arena.tiles.json".into()),
                "models" => snapshot.entry.models = Some("arena.models.json".into()),
                "camera" => snapshot.entry.camera = Some("arena.camera.json".into()),
                "ui" => {
                    snapshot.entry.ui = Some(
                        serde_json::from_value(serde_json::json!({
                            "profile": "arena-korean-v1",
                            "font": {"package": "font", "asset": "font.otf"}
                        }))
                        .unwrap(),
                    )
                }
                _ => unreachable!(),
            }
            assert!(
                source
                    .check_snapshot(&snapshot)
                    .unwrap_err()
                    .contains("only Arena"),
                "{field}"
            );
        }
    }

    #[test]
    fn declared_unsupported_shapes_fail_before_runtime_or_staging() {
        for declaration in [
            serde_json::json!({"game": "collect-dodge-v1", "scene": SCENE}),
            serde_json::json!({"game": "room-escape-v1", "scene": SCENE, "models": "room.models.json"}),
            serde_json::json!({"game": "arena", "scene": SCENE, "tilemap": "arena.tiles.json"}),
            serde_json::json!({"game": "arena", "scene": SCENE, "ui": {
                "profile": "arena-korean-v1", "font": {"package": "font", "asset": "font.otf"}
            }}),
        ] {
            let mut fixture = Fixture::new(false, false);
            let source = fixture.capture();
            fs::write(
                fixture.options.project.join(PROJECT_MANIFEST),
                serde_json::to_vec(
                    &serde_json::json!({"schema": 2, "engine": "*", "entry": declaration}),
                )
                .unwrap(),
            )
            .unwrap();
            fixture.options.runtime = fixture.temp.path().join("must-not-be-inspected");
            let error = fixture
                .run(
                    &source,
                    |_| panic!("no admitted boundary"),
                    |_| panic!("no smoke"),
                )
                .unwrap_err();
            assert!(!error.contains("must-not-be-inspected"), "{error}");
            assert!(!fixture.options.output.exists());
            fixture.no_stage();
        }
    }

    #[test]
    fn source_pin_preserves_trust_hash_and_elf_requirements() {
        let mut fixture = Fixture::new(false, false);
        let source = fixture.capture();
        fixture.options.trusted_runtime = false;
        assert!(export_arena_for_source(&fixture.options, &source)
            .unwrap_err()
            .contains("trusted-runtime"));
        fixture.options.trusted_runtime = true;
        fixture.options.runtime_sha256 = "A".repeat(64);
        assert!(export_arena_for_source(&fixture.options, &source)
            .unwrap_err()
            .contains("lowercase hexadecimal"));
        fixture.options.runtime_sha256 = "0".repeat(64);
        assert!(export_arena_for_source(&fixture.options, &source)
            .unwrap_err()
            .contains("does not match expected hash"));
        fs::write(&fixture.options.runtime, b"not an executable").unwrap();
        fixture.options.runtime_sha256 = hash(b"not an executable");
        assert!(export_arena_for_source(&fixture.options, &source)
            .unwrap_err()
            .contains("ELF"));
        fixture.no_stage();
    }

    #[test]
    fn same_snapshot_stages_pinned_bytes_then_final_recheck_rejects_changes() {
        for relative in [PROJECT_MANIFEST, PACKAGE_LOCK, SCENE, SPRITES] {
            let fixture = Fixture::new(true, true);
            let source = fixture.capture();
            let expected = fixture.smoke();
            let path = fixture.options.project.join(relative);
            let original = fs::read(&path).unwrap();
            let changed = [original.as_slice(), b"\n"].concat();
            let saw_stage = Cell::new(false);
            let saw_smoke = Cell::new(false);
            let error = fixture
                .run(
                    &source,
                    |point| {
                        if point == "admitted" {
                            fs::write(&path, &changed).unwrap();
                        } else if point == "before-validation" {
                            assert_eq!(
                                fs::read(fixture.stage().join("project").join(relative)).unwrap(),
                                original
                            );
                            saw_stage.set(true);
                        }
                        Ok(())
                    },
                    |_| {
                        saw_smoke.set(true);
                        Ok(expected)
                    },
                )
                .unwrap_err();
            assert!(error.contains("changed"), "{error}");
            assert!(saw_stage.get());
            assert!(saw_smoke.get());
            assert_eq!(fs::read(path).unwrap(), changed);
            assert!(!fixture.options.output.exists());
            fixture.no_stage();
        }
    }

    #[test]
    fn exact_source_retains_transaction_failpoints_and_previous_output() {
        let fixture = Fixture::new(true, true);
        let source = fixture.capture();
        let previous = fixture.temp.path().join("previous");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("keep"), b"previous-good").unwrap();
        let original = fs::read(fixture.options.project.join(SCENE)).unwrap();
        for failpoint in [
            "admitted",
            "binary-copied",
            "content-copied",
            "before-validation",
            "after-validation",
            "before-publish",
        ] {
            let expected = fixture.smoke();
            let error = fixture
                .run(
                    &source,
                    |point| {
                        if point == failpoint {
                            Err(format!("injected {point}"))
                        } else {
                            Ok(())
                        }
                    },
                    |_| Ok(expected),
                )
                .unwrap_err();
            assert!(error.contains("injected"), "{failpoint}: {error}");
            assert_eq!(
                fs::read(fixture.options.project.join(SCENE)).unwrap(),
                original
            );
            assert_eq!(fs::read(previous.join("keep")).unwrap(), b"previous-good");
            assert!(!fixture.options.output.exists());
            fixture.no_stage();
        }
    }

    #[test]
    fn exact_source_does_not_bypass_whole_active_lock_asset_validation() {
        fn copy_tree(source: &Path, destination: &Path) {
            fs::create_dir_all(destination).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let mut fixture = Fixture::new(false, false);
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/saved_arena_project"),
            &fixture.options.project,
        );
        let snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
        let source = capture(&snapshot);
        assert_eq!(
            source.inventory().len(),
            4,
            "packages are validated by active-lock closure, not caller subsets"
        );
        let asset = snapshot
            .files
            .iter()
            .find(|file| file.role == "package_asset")
            .unwrap();
        let mut changed = asset.bytes.clone();
        changed[0] ^= 1;
        fs::write(fixture.options.project.join(&asset.relative), changed).unwrap();
        fixture.options.runtime = fixture.temp.path().join("must-not-be-inspected");
        let error = fixture
            .run(
                &source,
                |_| panic!("no admitted boundary"),
                |_| panic!("no smoke"),
            )
            .unwrap_err();
        assert!(!error.contains("must-not-be-inspected"), "{error}");
        assert!(!fixture.options.output.exists());
        fixture.no_stage();
    }
}
