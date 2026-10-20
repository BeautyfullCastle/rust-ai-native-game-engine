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
const MAX_UI_FONT_BYTES: u64 = 16 * 1024 * 1024;

/// Exact descriptor and bytes handed to the caller's Arena UI installation.
///
/// This is bounded caller evidence, not authentication or deployment metadata.
/// The exporter's admitted active-lock snapshot remains the authority for assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaUiSource {
    descriptor: orr_package::ProjectUi,
    font_bytes: u64,
    font_sha256: String,
}
impl ArenaUiSource {
    pub fn new(
        descriptor: orr_package::ProjectUi,
        font_bytes: u64,
        font_sha256: String,
    ) -> Result<Self, String> {
        let value = Self {
            descriptor,
            font_bytes,
            font_sha256,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn descriptor(&self) -> &orr_package::ProjectUi {
        &self.descriptor
    }

    pub fn font_bytes(&self) -> u64 {
        self.font_bytes
    }

    pub fn font_sha256(&self) -> &str {
        &self.font_sha256
    }

    fn validate(&self) -> Result<(), String> {
        if self.descriptor.profile != orr_package::ProjectUiProfile::ArenaKoreanV1
            || self.descriptor.document.is_some()
        {
            return Err("Arena export UI requires the document-free Arena Korean preset".into());
        }
        let package = &self.descriptor.font.package;
        if package.is_empty()
            || package.len() > 64
            || !package.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte)
            })
        {
            return Err("Arena export UI font package must be a valid package name".into());
        }
        let asset = &self.descriptor.font.asset;
        // Package assets have stricter total length/depth than their prefixed
        // export paths; component grammar and DOS-name rejection are identical.
        if asset.len() > 240 || asset.split('/').count() > 16 {
            return Err("Arena export UI font asset exceeds its portable path bound".into());
        }
        validate_relative(asset)?;
        // Fonts deliberately do not use ArenaSourceDigest's 4 MiB scene bound.
        if !(1..=MAX_UI_FONT_BYTES).contains(&self.font_bytes) {
            return Err("Arena export UI font must be 1 byte to 16 MiB".into());
        }
        if self.font_sha256.len() != 64
            || !self
                .font_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "Arena export UI font SHA256 must be 64 lowercase hexadecimal characters".into(),
            );
        }
        Ok(())
    }
}

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
    ui: Option<ArenaUiSource>,
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
            ui: None,
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

    /// Attach the exact loaded UI assertion without changing authority inventory.
    pub fn with_ui(mut self, ui: ArenaUiSource) -> Result<Self, String> {
        self.ui = Some(ui);
        self.validate()?;
        Ok(self)
    }

    pub fn ui(&self) -> Option<&ArenaUiSource> {
        self.ui.as_ref()
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
        if let Some(ui) = &self.ui {
            ui.validate()?;
        }
        Ok(())
    }

    pub(super) fn check_snapshot(&self, project: &ProjectSnapshot) -> Result<(), String> {
        self.validate()?;
        let entry = &project.entry;
        if entry.game != orr_package::ProjectGame::Arena
            || entry.tilemap.is_some()
            || entry.models.is_some()
            || entry.camera.is_some()
        {
            return Err(
                "editor Arena export supports only Arena scene, sprites and the Korean UI preset, without tilemap, models or camera"
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
        match (&entry.ui, &self.ui) {
            (None, None) => {}
            (Some(descriptor), Some(ui)) => {
                if descriptor != ui.descriptor() {
                    return Err("Arena export UI descriptor does not match admitted entry".into());
                }
                let digest = project
                    .package_identities
                    .get(&descriptor.font.package)
                    .ok_or("Arena export UI font package is absent from admitted closure")?;
                let relative = format!(".orr/packages/objects/{digest}/{}", descriptor.font.asset);
                let font = project
                    .files
                    .iter()
                    .find(|file| file.relative == relative)
                    .ok_or("Arena export UI font asset is absent from admitted closure")?;
                if font.role != "package_asset"
                    || font.bytes.is_empty()
                    || font.bytes.len() as u64 != ui.font_bytes()
                    || font.sha256 != ui.font_sha256()
                {
                    return Err(
                        "Arena export UI font bytes do not match admitted package asset".into(),
                    );
                }
            }
            _ => {
                return Err(
                    "editor export accepts only Arena UI with matching explicit installation evidence; UI presence does not match admitted entry".into(),
                );
            }
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

    fn ui_descriptor() -> orr_package::ProjectUi {
        orr_package::ProjectUi {
            profile: orr_package::ProjectUiProfile::ArenaKoreanV1,
            font: orr_package::ProjectFont {
                package: "korean-game-ui".into(),
                asset: "OrreryKoreanUI.otf".into(),
            },
            document: None,
        }
    }

    #[cfg(feature = "game-ui")]
    fn capture_ui(snapshot: &ProjectSnapshot) -> ArenaUiSource {
        let descriptor = snapshot.entry.ui.as_ref().unwrap();
        let relative = format!(
            ".orr/packages/objects/{}/{}",
            snapshot.package_identities[&descriptor.font.package], descriptor.font.asset
        );
        let font = snapshot
            .files
            .iter()
            .find(|file| file.relative == relative)
            .unwrap();
        ArenaUiSource::new(
            descriptor.clone(),
            font.bytes.len() as u64,
            font.sha256.clone(),
        )
        .unwrap()
    }

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

        #[cfg(feature = "game-ui")]
        fn install_ui(&self) {
            let package = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/game_ui_font")
                .canonicalize()
                .unwrap();
            // Keep an unrelated active package as well as the selected font.
            // Neither the empty sprite sidecar nor this UI references its assets.
            let sprites = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/sprite_demo")
                .canonicalize()
                .unwrap();
            orr_package::Project::open_for_install(
                &self.options.project,
                orr_package::Runtime::content_only().engine_version,
            )
            .unwrap()
            .install(&[package, sprites])
            .unwrap();
            let manifest_path = self.options.project.join(PROJECT_MANIFEST);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            manifest["entry"]["ui"] = serde_json::to_value(ui_descriptor()).unwrap();
            fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
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
    fn ui_constructor_bounds_closed_descriptor_font_length_and_digest() {
        let descriptor = ui_descriptor();
        let sha256 = "a".repeat(64);
        for bytes in [1, MAX_SCENE_BYTES + 1, MAX_UI_FONT_BYTES] {
            let source = ArenaUiSource::new(descriptor.clone(), bytes, sha256.clone()).unwrap();
            assert_eq!(source.descriptor(), &descriptor);
            assert_eq!(source.font_bytes(), bytes);
            assert_eq!(source.font_sha256(), sha256);
        }
        for bytes in [0, MAX_UI_FONT_BYTES + 1, u64::MAX] {
            assert!(ArenaUiSource::new(descriptor.clone(), bytes, sha256.clone()).is_err());
        }
        for hash in [
            String::new(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            assert!(ArenaUiSource::new(descriptor.clone(), 1, hash).is_err());
        }
        let mut wrong_profile = descriptor.clone();
        wrong_profile.profile = orr_package::ProjectUiProfile::CollectAuthoredV1;
        assert!(ArenaUiSource::new(wrong_profile, 1, sha256.clone()).is_err());
        for document in ["", "arena.ui.json"] {
            let mut with_document = descriptor.clone();
            with_document.document = Some(document.into());
            assert!(ArenaUiSource::new(with_document, 1, sha256.clone()).is_err());
        }
        let fixture = Fixture::new(false, false);
        let source = fixture.capture();
        assert!(
            source.ui().is_none(),
            "legacy constructor has no UI authority"
        );
        let ui = ArenaUiSource::new(descriptor, 1, sha256).unwrap();
        let with_ui = source.clone().with_ui(ui.clone()).unwrap();
        assert_eq!(with_ui.ui(), Some(&ui));
        assert_eq!(source.inventory(), with_ui.inventory());
        assert_eq!(source.canonical_scene(), with_ui.canonical_scene());
        assert_eq!(source.initial_checksum(), with_ui.initial_checksum());
        let mut invalid = ui;
        invalid.font_bytes = 0;
        assert!(source.with_ui(invalid).is_err());
    }

    #[test]
    fn ui_constructor_uses_exact_package_name_and_portable_asset_grammar() {
        for package in ["a".into(), "a_0-9".into(), "z".repeat(64)] {
            let mut descriptor = ui_descriptor();
            descriptor.font.package = package;
            assert!(ArenaUiSource::new(descriptor, 1, "0".repeat(64)).is_ok());
        }
        for package in [
            "".into(),
            "a".repeat(65),
            "Font".into(),
            "font.name".into(),
            "font/name".into(),
            "font name".into(),
            "é".into(),
        ] {
            let mut descriptor = ui_descriptor();
            descriptor.font.package = package;
            assert!(ArenaUiSource::new(descriptor, 1, "0".repeat(64)).is_err());
        }
        for asset in [
            "font.otf".into(),
            ".font/a_B-1.OTF".into(),
            "a".repeat(100),
            ["a"; 16].join("/"),
            ["a".repeat(100), "b".repeat(100), "c".repeat(38)].join("/"),
            "COM10/LPTx.otf".into(),
        ] {
            let mut descriptor = ui_descriptor();
            descriptor.font.asset = asset.clone();
            assert!(
                ArenaUiSource::new(descriptor, 1, "0".repeat(64)).is_ok(),
                "{asset:?}"
            );
        }
        for asset in [
            "".into(),
            ".".into(),
            "..".into(),
            "/font.otf".into(),
            "../font.otf".into(),
            "a/./font.otf".into(),
            "a//font.otf".into(),
            "a/font.otf/".into(),
            "a\\font.otf".into(),
            "C:font.otf".into(),
            "한글.otf".into(),
            "font\n.otf".into(),
            "font .otf".into(),
            "a/font.".into(),
            "a".repeat(101),
            ["a"; 17].join("/"),
            ["a".repeat(100), "b".repeat(100), "c".repeat(39)].join("/"),
        ] {
            let mut descriptor = ui_descriptor();
            descriptor.font.asset = asset.clone();
            assert!(
                ArenaUiSource::new(descriptor, 1, "0".repeat(64)).is_err(),
                "{asset:?}"
            );
        }
        let mut reserved = vec!["CON".into(), "PRN".into(), "AUX".into(), "NUL".into()];
        for digit in 0..=9 {
            reserved.push(format!("COM{digit}"));
            reserved.push(format!("LPT{digit}"));
        }
        for stem in reserved {
            for asset in [
                stem.clone(),
                format!("fonts/{}.otf", stem.to_ascii_lowercase()),
            ] {
                let mut descriptor = ui_descriptor();
                descriptor.font.asset = asset.clone();
                assert!(
                    ArenaUiSource::new(descriptor, 1, "0".repeat(64)).is_err(),
                    "{asset:?}"
                );
            }
        }
    }

    #[test]
    fn extra_ui_assertion_rejects_before_runtime_stage_and_smoke() {
        let mut fixture = Fixture::new(false, false);
        let source = fixture
            .capture()
            .with_ui(ArenaUiSource::new(ui_descriptor(), 1, "a".repeat(64)).unwrap())
            .unwrap();
        fixture.reject_before_runtime(&source, "UI presence");
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn ui_constructor_accepts_parser_and_corpus_valid_font_above_scene_bound() {
        let mut font =
            include_bytes!("../../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
        font.resize(MAX_SCENE_BYTES as usize + 1, 0);
        crate::game_ui::GameUi::validate_font(&font).unwrap();
        let source = ArenaUiSource::new(ui_descriptor(), font.len() as u64, hash(&font)).unwrap();
        assert!(source.font_bytes() > MAX_SCENE_BYTES);
        assert!(ArenaSourceDigest::new(source.font_bytes(), source.font_sha256().into()).is_err());
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn exact_ui_assertion_exports_full_closure_without_expanding_inventory() {
        for sprites in [false, true] {
            let fixture = Fixture::new(sprites, false);
            fixture.install_ui();
            let snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
            let source = capture(&snapshot).with_ui(capture_ui(&snapshot)).unwrap();
            assert_eq!(source.inventory().len(), 3 + usize::from(sprites));
            let smoke_calls = Cell::new(0);
            let expected = fixture.smoke();
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
            assert_eq!(report.project_files, snapshot.files.len());
            assert_eq!(report.manifest.payload.entry.ui, snapshot.entry.ui);
            assert_eq!(
                report.manifest.payload.packages,
                snapshot.package_identities
            );
            for file in &snapshot.files {
                assert_eq!(
                    fs::read(report.output.join("project").join(&file.relative)).unwrap(),
                    file.bytes
                );
                assert_eq!(
                    fs::read(fixture.options.project.join(&file.relative)).unwrap(),
                    file.bytes
                );
            }
            let manifest = fs::read_to_string(report.output.join(MANIFEST_NAME)).unwrap();
            assert!(!manifest.contains("font_sha256"));
            assert!(!manifest.contains(source.canonical_scene().to_str().unwrap()));
            fixture.no_stage();
        }
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn ui_assertion_presence_descriptor_bytes_and_shape_reject_before_runtime() {
        let mut fixture = Fixture::new(false, false);
        fixture.install_ui();
        let snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
        let legacy = capture(&snapshot);
        fixture.reject_before_runtime(&legacy, "UI presence");
        let original = legacy.with_ui(capture_ui(&snapshot)).unwrap();
        for field in [
            "package", "asset", "length", "hash", "zero", "profile", "document", "digest",
        ] {
            let mut source = original.clone();
            let ui = source.ui.as_mut().unwrap();
            let expected = match field {
                "package" => {
                    ui.descriptor.font.package = "other-font".into();
                    "UI descriptor"
                }
                "asset" => {
                    ui.descriptor.font.asset = "other.otf".into();
                    "UI descriptor"
                }
                "length" => {
                    ui.font_bytes += 1;
                    "UI font bytes"
                }
                "hash" => {
                    ui.font_sha256 = "0".repeat(64);
                    "UI font bytes"
                }
                "zero" => {
                    ui.font_bytes = 0;
                    "1 byte to 16 MiB"
                }
                "profile" => {
                    ui.descriptor.profile = orr_package::ProjectUiProfile::CollectAuthoredV1;
                    "document-free"
                }
                "document" => {
                    ui.descriptor.document = Some("arena.ui.json".into());
                    "document-free"
                }
                "digest" => {
                    ui.font_sha256 = "A".repeat(64);
                    "lowercase hexadecimal"
                }
                _ => unreachable!(),
            };
            fixture.reject_before_runtime(&source, expected);
        }
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn ui_snapshot_requires_selected_package_asset_role_nonempty_length_and_hash() {
        let fixture = Fixture::new(false, false);
        fixture.install_ui();
        for mutation in [
            "package",
            "identity",
            "missing",
            "wrong-path",
            "role",
            "empty",
            "length",
            "hash",
        ] {
            let mut snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
            let source = capture(&snapshot).with_ui(capture_ui(&snapshot)).unwrap();
            source.check_snapshot(&snapshot).unwrap();
            let descriptor = source.ui().unwrap().descriptor();
            let relative = format!(
                ".orr/packages/objects/{}/{}",
                snapshot.package_identities[&descriptor.font.package], descriptor.font.asset
            );
            let index = snapshot
                .files
                .iter()
                .position(|file| file.relative == relative)
                .unwrap();
            match mutation {
                "package" => {
                    snapshot.package_identities.remove(&descriptor.font.package);
                }
                "identity" => {
                    snapshot
                        .package_identities
                        .insert(descriptor.font.package.clone(), "0".repeat(64));
                }
                "missing" => {
                    snapshot.files.remove(index);
                }
                "wrong-path" => {
                    snapshot.files[index].relative.push_str(".other");
                }
                "role" => {
                    snapshot.files[index].role = "package_manifest";
                }
                "empty" => {
                    snapshot.files[index].bytes.clear();
                }
                "length" => {
                    snapshot.files[index].bytes.push(0);
                }
                "hash" => {
                    snapshot.files[index].sha256 = "0".repeat(64);
                }
                _ => unreachable!(),
            }
            assert!(
                source
                    .check_snapshot(&snapshot)
                    .unwrap_err()
                    .contains("Arena export UI font"),
                "{mutation}"
            );
        }
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn ui_assertion_keeps_selected_asset_and_whole_active_lock_admission() {
        for mutation in [
            "missing-font",
            "empty-font",
            "corrupt-font",
            "unrelated-package",
        ] {
            let mut fixture = Fixture::new(false, false);
            fixture.install_ui();
            let snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
            let source = capture(&snapshot).with_ui(capture_ui(&snapshot)).unwrap();
            let ui = source.ui().unwrap();
            let relative = if mutation == "unrelated-package" {
                format!(
                    ".orr/packages/objects/{}/LICENSE.txt",
                    snapshot.package_identities["sample-sprites"]
                )
            } else {
                format!(
                    ".orr/packages/objects/{}/{}",
                    snapshot.package_identities[&ui.descriptor().font.package],
                    ui.descriptor().font.asset
                )
            };
            let path = fixture.options.project.join(relative);
            let error = match mutation {
                "missing-font" => {
                    fs::remove_file(path).unwrap();
                    "OrreryKoreanUI.otf"
                }
                "empty-font" => {
                    fs::write(path, []).unwrap();
                    "installed content changed"
                }
                _ => {
                    let mut changed = fs::read(&path).unwrap();
                    changed[0] ^= 1;
                    fs::write(path, changed).unwrap();
                    "installed content changed"
                }
            };
            fixture.reject_before_runtime(&source, error);
        }
    }

    #[cfg(feature = "game-ui")]
    #[test]
    fn ui_font_snapshot_is_staged_exactly_and_rechecked_before_publication() {
        let fixture = Fixture::new(false, false);
        fixture.install_ui();
        let snapshot = ProjectSnapshot::open(&fixture.options.project).unwrap();
        let source = capture(&snapshot).with_ui(capture_ui(&snapshot)).unwrap();
        let ui = source.ui().unwrap();
        let relative = format!(
            ".orr/packages/objects/{}/{}",
            snapshot.package_identities[&ui.descriptor().font.package],
            ui.descriptor().font.asset
        );
        let path = fixture.options.project.join(&relative);
        let original = fs::read(&path).unwrap();
        let mut changed = original.clone();
        changed[0] ^= 1;
        let expected = fixture.smoke();
        let stage_checks = Cell::new(0);
        let smoke_calls = Cell::new(0);
        let error = fixture
            .run(
                &source,
                |point| {
                    if point == "admitted" {
                        fs::write(&path, &changed).unwrap();
                    } else if point == "before-validation" {
                        assert_eq!(
                            fs::read(fixture.stage().join("project").join(&relative)).unwrap(),
                            original
                        );
                        stage_checks.set(stage_checks.get() + 1);
                    }
                    Ok(())
                },
                |_| {
                    smoke_calls.set(smoke_calls.get() + 1);
                    Ok(expected)
                },
            )
            .unwrap_err();
        assert!(error.contains("changed"), "{error}");
        assert_eq!(stage_checks.get(), 1);
        assert_eq!(smoke_calls.get(), 1);
        assert_eq!(fs::read(path).unwrap(), changed);
        assert!(!fixture.options.output.exists());
        fixture.no_stage();
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
