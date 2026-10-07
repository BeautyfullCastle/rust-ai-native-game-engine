//! Shared read-only saved Arena project admission, before host or GPU creation.
//! This bounds ordinary file opening, not concurrent hostile filesystem replacement.
//! The package manager remains the sole installation/activation/lock authority.
use crate::project_sprites::{self, Asset, AssetKey, Document};
use orr_reflect::{Guid, Scene, TypeRegistry, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
pub const MAX_SCENE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_SCENE_ENTITIES: usize = 20_000;

pub fn arena_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    crate::arena_game::register_reflect(&mut types);
    types
}

/// Opaque admitted scene. Startup consumers never reread its path.
#[derive(Clone, Debug)]
pub struct PreparedArenaScene {
    path: PathBuf,
    text: Arc<str>,
    scene: Arc<Scene>,
}
impl PreparedArenaScene {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
}
pub struct PreparedSprites {
    pub path: PathBuf,
    pub document: Document,
    pub assets: BTreeMap<AssetKey, Asset>,
}
/// Whether this launch path compiled and deliberately enabled authored game UI.
/// This remains explicit even in builds which happen to include UI dependencies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSupport {
    Unsupported,
    Supported,
}

/// UI presentation data admitted through the project's verified active lock.
/// The font is owned by the prepared result, so startup never reopens the package.
#[derive(Clone, Debug)]
pub struct PreparedUi {
    pub descriptor: orr_package::ProjectUi,
    pub font: Vec<u8>,
}

pub struct PreparedProject {
    root: PathBuf,
    scene: PreparedArenaScene,
    sprites: Option<PreparedSprites>,
    ui: Option<PreparedUi>,
}
impl PreparedProject {
    /// Validate the entire active lock using the caller's real compiled inventory.
    pub fn open(root: impl AsRef<Path>, runtime: orr_package::Runtime) -> Result<Self, String> {
        Self::open_with_ui(root, runtime, UiSupport::Unsupported)
    }

    /// Validate the active lock and prepare the declared UI only when this launch
    /// path explicitly opts in. A feature-enabled unified build alone is not support.
    pub fn open_with_ui(
        root: impl AsRef<Path>,
        runtime: orr_package::Runtime,
        ui_support: UiSupport,
    ) -> Result<Self, String> {
        let project = orr_package::Project::open(root.as_ref(), runtime)
            .map_err(|e| format!("project: {e}"))?;
        let lock = project
            .verify()
            .map_err(|e| format!("package verification: {e}"))?;
        let manifest = project
            .manifest()
            .ok_or("saved project requires orr.project.json schema 2 with an entry")?;
        let entry = manifest
            .entry
            .as_ref()
            .ok_or("saved project requires schema 2 with an entry; schema 1 is metadata-only")?;
        if manifest.schema != 2 || entry.game != orr_package::ProjectGame::Arena {
            return Err("saved project entry must be schema 2 and game arena".into());
        }
        // This check intentionally follows full active-lock verification but precedes
        // scene, sidecar, and font decoding, so unsupported hosts fail closed early.
        if entry.ui.is_some() && ui_support == UiSupport::Unsupported {
            return Err("saved project declares UI but this host does not support it".into());
        }
        #[cfg(not(feature = "game-ui"))]
        if entry.ui.is_some() {
            return Err("saved project UI requires a game-ui-enabled host".into());
        }
        #[cfg(feature = "game-ui")]
        let ui = match (&entry.ui, ui_support) {
            (Some(descriptor), UiSupport::Supported) => {
                Some(prepare_ui(&project, &lock, descriptor.clone())?)
            }
            _ => None,
        };
        #[cfg(not(feature = "game-ui"))]
        let ui = None;
        let root = project.root().to_path_buf();
        let scene_path = entry_file(&root, &entry.scene)?;
        let bytes = read_regular(&scene_path, MAX_SCENE_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|e| format!("scene must be UTF-8: {e}"))?;
        let scene = Scene::parse(&text, &arena_types())
            .map_err(|e| format!("Arena scene {}: {e}", scene_path.display()))?;
        if scene.entities.len() > MAX_SCENE_ENTITIES {
            return Err(format!("Arena scene exceeds {MAX_SCENE_ENTITIES} entities"));
        }
        let sprites = if let Some(relative) = &entry.sprites {
            let path = entry_file(&root, relative)?;
            let bytes = read_regular(&path, project_sprites::MAX_BYTES)?;
            let document =
                Document::from_bytes(&bytes).map_err(|e| format!("sprite sidecar: {e}"))?;
            let base = path.parent().expect("admitted file parent");
            let sidecar_project = resolve_inside(&root, base, &document.project, true)?;
            if sidecar_project != root {
                return Err("sprite sidecar project must resolve to the same project root".into());
            }
            let sidecar_scene = resolve_inside(&root, base, &document.scene, false)?;
            if sidecar_scene != scene_path {
                return Err("sprite sidecar scene must resolve to the project entry scene".into());
            }
            for (guid, binding) in &document.bindings {
                validate_target(&scene, guid, "sprite binding")?;
                let package = lock.packages.get(&binding.package).ok_or_else(|| {
                    format!("sprite binding {guid}: missing package {}", binding.package)
                })?;
                if !package.manifest.capabilities.contains("sprite") {
                    return Err(format!(
                        "sprite package {} must declare the sprite capability",
                        binding.package
                    ));
                }
            }
            if let Some(guid) = &document.camera_follow {
                validate_target(&scene, guid, "camera follow")?;
            }
            let assets = project_sprites::load_project_assets(&document, &project)?;
            Some(PreparedSprites {
                path,
                document,
                assets,
            })
        } else {
            None
        };
        Ok(Self {
            root,
            scene: PreparedArenaScene {
                path: scene_path,
                text: text.into(),
                scene: Arc::new(scene),
            },
            sprites,
            ui,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn scene(&self) -> &PreparedArenaScene {
        &self.scene
    }
    pub fn sprites(&self) -> Option<&PreparedSprites> {
        self.sprites.as_ref()
    }
    pub fn ui(&self) -> Option<&PreparedUi> {
        self.ui.as_ref()
    }
    pub fn into_parts(
        self,
    ) -> (
        PathBuf,
        PreparedArenaScene,
        Option<PreparedSprites>,
        Option<PreparedUi>,
    ) {
        (self.root, self.scene, self.sprites, self.ui)
    }
}

#[cfg(feature = "game-ui")]
fn prepare_ui(
    project: &orr_package::Project,
    lock: &orr_package::Lock,
    descriptor: orr_package::ProjectUi,
) -> Result<PreparedUi, String> {
    let font_ref = &descriptor.font;
    let package = lock.packages.get(&font_ref.package).ok_or_else(|| {
        format!(
            "game UI font package is not declared in the active lock: {}",
            font_ref.package
        )
    })?;
    if !package.manifest.files.contains(&font_ref.asset) {
        return Err(format!(
            "game UI font asset is not declared by package {}: {}",
            font_ref.package, font_ref.asset
        ));
    }
    let font = project
        .read_asset(&font_ref.package, &font_ref.asset)
        .map_err(|e| format!("game UI font asset: {e}"))?;

    crate::game_ui::GameUi::validate_font(&font)?;

    Ok(PreparedUi { descriptor, font })
}

#[cfg(all(test, feature = "game-ui"))]
mod ui_admission_tests {
    use super::*;
    use orr_package::Project;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        temp: PathBuf,
        root: PathBuf,
    }

    impl Fixture {
        fn new(ui: Option<(&str, &str)>, sprites: bool, valid_scene: bool) -> Self {
            let mut temp = std::env::temp_dir();
            #[cfg(unix)]
            {
                temp = fs::canonicalize(temp).unwrap();
            }
            temp.push(format!(
                "orr-project-ui-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&temp).unwrap();
            let root = temp.join("project");
            fs::create_dir(&root).unwrap();
            let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
            let saved = assets.join("saved_arena_project");
            let scene = if valid_scene {
                fs::read(saved.join("arena.scene.yaml")).unwrap()
            } else {
                b"this is not an Arena scene".to_vec()
            };
            fs::write(root.join("arena.scene.yaml"), scene).unwrap();
            let mut entry = serde_json::json!({
                "game": "arena",
                "scene": "arena.scene.yaml"
            });
            if sprites {
                fs::copy(
                    saved.join("arena.sprites.json"),
                    root.join("arena.sprites.json"),
                )
                .unwrap();
                entry["sprites"] = "arena.sprites.json".into();
            }
            if let Some((package, asset)) = ui {
                entry["ui"] = serde_json::json!({
                    "profile": "arena-korean-v1",
                    "font": { "package": package, "asset": asset }
                });
            }
            fs::write(
                root.join("orr.project.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema": 2,
                    "engine": "*",
                    "entry": entry
                }))
                .unwrap(),
            )
            .unwrap();
            Self { temp, root }
        }

        fn install_packages(&self, font_override: Option<&[u8]>, sprites: bool) {
            let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
            let mut sources = Vec::new();
            let font_source = self.temp.join("font-source");
            copy_package(&assets.join("game_ui_font"), &font_source);
            if let Some(bytes) = font_override {
                fs::write(font_source.join("OrreryKoreanUI.otf"), bytes).unwrap();
            }
            sources.push(font_source);
            if sprites {
                let sprite_source = self.temp.join("sprite-source");
                copy_package(&assets.join("sprite_demo"), &sprite_source);
                sources.push(sprite_source);
            }
            Project::open(&self.root, runtime(sprites))
                .unwrap()
                .install(&sources)
                .unwrap();
        }

        fn open(&self, support: UiSupport) -> Result<PreparedProject, String> {
            PreparedProject::open_with_ui(&self.root, runtime(false), support)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.temp);
        }
    }

    fn copy_package(from: &std::path::Path, to: &std::path::Path) {
        fs::create_dir(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    fn runtime(sprites: bool) -> orr_package::Runtime {
        let mut runtime = orr_package::Runtime::content_only();
        if sprites {
            runtime.capabilities.insert("sprite".into());
        }
        runtime
    }

    fn ui_descriptor() -> (&'static str, &'static str) {
        ("korean-game-ui", "OrreryKoreanUI.otf")
    }

    fn remove_glyph_from_cmap(mut font: Vec<u8>, character: char) -> Vec<u8> {
        fn u16_at(bytes: &[u8], offset: usize) -> usize {
            u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize
        }
        fn u32_at(bytes: &[u8], offset: usize) -> usize {
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
        }
        let tables = u16_at(&font, 4);
        let cmap = (0..tables)
            .find_map(|index| {
                let record = 12 + index * 16;
                (font[record..record + 4] == *b"cmap").then(|| u32_at(&font, record + 8))
            })
            .expect("test font has a cmap table");
        let encodings = u16_at(&font, cmap + 2);
        for index in 0..encodings {
            let record = cmap + 4 + index * 8;
            let subtable = cmap + u32_at(&font, record + 4);
            if u16_at(&font, subtable) != 4 {
                continue;
            }
            let segments = u16_at(&font, subtable + 6) / 2;
            let end_codes = subtable + 14;
            let start_codes = end_codes + segments * 2 + 2;
            let target = character as usize;
            for segment in 0..segments {
                let start_at = start_codes + segment * 2;
                let end_at = end_codes + segment * 2;
                let start = u16_at(&font, start_at);
                let end = u16_at(&font, end_at);
                if start == target {
                    font[start_at..start_at + 2]
                        .copy_from_slice(&((target + 1) as u16).to_be_bytes());
                    return font;
                }
                if end == target {
                    font[end_at..end_at + 2].copy_from_slice(&((target - 1) as u16).to_be_bytes());
                    return font;
                }
            }
        }
        panic!(
            "test font has no cmap endpoint for U+{:04X}",
            character as u32
        )
    }

    fn failure(result: Result<PreparedProject, String>) -> String {
        match result {
            Err(error) => error,
            Ok(_) => panic!("expected project UI admission to fail"),
        }
    }

    #[test]
    fn default_host_rejects_authored_ui_before_reading_the_scene_even_when_feature_is_built() {
        let fixture = Fixture::new(Some(ui_descriptor()), false, false);
        let error = failure(PreparedProject::open(&fixture.root, runtime(false)));
        assert!(error.contains("does not support it"), "{error}");
        assert!(!error.contains("Arena scene"), "{error}");
    }

    #[test]
    fn ui_requires_a_package_in_the_verified_active_lock() {
        let fixture = Fixture::new(Some(ui_descriptor()), false, true);
        let error = failure(fixture.open(UiSupport::Supported));
        assert!(error.contains("not declared in the active lock"), "{error}");
    }

    #[test]
    fn ui_font_asset_must_be_declared_by_the_locked_package() {
        let fixture = Fixture::new(Some(("korean-game-ui", "not-declared.otf")), false, true);
        fixture.install_packages(None, false);
        let error = failure(fixture.open(UiSupport::Supported));
        assert!(
            error.contains("asset is not declared by package"),
            "{error}"
        );
    }

    #[test]
    fn whole_lock_rejects_font_license_and_unused_active_asset_tampering() {
        for (package_name, asset) in [
            ("korean-game-ui", "OrreryKoreanUI.otf"),
            ("korean-game-ui", "OFL.txt"),
            ("sample-sprites", "lantern_keeper.rgba"),
        ] {
            let fixture = Fixture::new(Some(ui_descriptor()), false, false);
            fixture.install_packages(None, true);
            let package = Project::open(&fixture.root, runtime(true))
                .unwrap()
                .list()
                .unwrap()
                .packages[package_name]
                .clone();
            let path = fixture
                .root
                .join(".orr/packages/objects")
                .join(package.digest)
                .join(asset);
            assert!(path.is_file());
            fs::write(path, b"tampered").unwrap();
            let error = failure(PreparedProject::open_with_ui(
                &fixture.root,
                runtime(true),
                UiSupport::Supported,
            ));
            assert!(error.contains("package verification"), "{error}");
            assert!(error.contains("installed content changed"), "{error}");
        }
    }

    #[test]
    fn malformed_and_oversized_fonts_are_rejected() {
        let malformed = Fixture::new(Some(ui_descriptor()), false, true);
        malformed.install_packages(Some(b"not an OpenType font"), false);
        assert!(failure(malformed.open(UiSupport::Supported)).contains("invalid game UI font"));

        let oversized = Fixture::new(Some(ui_descriptor()), false, true);
        let font = vec![0; 16 * 1024 * 1024 + 1];
        oversized.install_packages(Some(&font), false);
        assert!(failure(oversized.open(UiSupport::Supported)).contains("1 byte to 16 MiB"));
    }

    #[test]
    fn valid_font_missing_a_production_corpus_glyph_is_rejected_at_admission() {
        let fixture = Fixture::new(Some(ui_descriptor()), false, true);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font/OrreryKoreanUI.otf");
        let font = remove_glyph_from_cmap(fs::read(source).unwrap(), '가');
        fixture.install_packages(Some(&font), false);
        let error = failure(fixture.open(UiSupport::Supported));
        assert!(error.contains("lacks required glyph U+AC00"), "{error}");
    }

    #[test]
    fn ui_and_sprites_share_the_active_lock_and_prepared_font_is_owned() {
        let fixture = Fixture::new(Some(ui_descriptor()), true, true);
        fixture.install_packages(None, true);
        let manager = Project::open(&fixture.root, runtime(true)).unwrap();
        let lock = manager.verify().unwrap();
        assert!(lock.packages.contains_key("korean-game-ui"));
        assert!(lock.packages.contains_key("sample-sprites"));

        let prepared =
            PreparedProject::open_with_ui(&fixture.root, runtime(true), UiSupport::Supported)
                .unwrap();
        assert!(prepared.sprites().is_some());
        let prepared_ui = prepared.ui().unwrap();
        assert_eq!(prepared_ui.descriptor.font.package, "korean-game-ui");
        let expected_font = prepared_ui.font.clone();

        manager.remove("korean-game-ui").unwrap();
        assert_eq!(prepared.ui().unwrap().font, expected_font);
        let (_, _, sprites, ui) = prepared.into_parts();
        assert!(sprites.is_some());
        assert_eq!(ui.unwrap().font, expected_font);
    }
}

fn validate_target(scene: &Scene, guid: &str, role: &str) -> Result<(), String> {
    let parsed = Guid::parse(guid).map_err(|e| format!("{role} GUID {guid}: {e}"))?;
    let entity = scene
        .entities
        .get(&parsed)
        .ok_or_else(|| format!("{role} GUID {guid} is missing from the Arena scene"))?;
    let position = entity
        .components
        .iter()
        .find(|(name, _)| name == "Position");
    let has_position =
        position.is_some_and(|(_, value)| matches!(value.field("pos"), Some(Value::Vec2(_))));
    // The compiled Arena extractor emits bodies only for players and bullets.
    // Position alone would silently disappear from both sprite and follow views.
    let drawable = entity
        .components
        .iter()
        .any(|(name, _)| name == "PlayerTag" || name == "Bullet");
    if !has_position || !drawable {
        return Err(format!(
            "{role} GUID {guid} needs a usable Arena Position and PlayerTag or Bullet"
        ));
    }
    Ok(())
}

fn entry_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.split('/').any(|part| part == "." || part == "..") {
        return Err("project entry paths must be root-relative without traversal".into());
    }
    resolve_inside(root, root, relative, false)
}

/// Sidecar paths retain their relative-to-sidecar semantics, including `..`
/// within the project. Check each directory before parent removal, and reject
/// even temporary escapes, symlinks, special files, and different final roots.
fn resolve_inside(
    root: &Path,
    base: &Path,
    relative: &str,
    directory: bool,
) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.contains('\\')
        || relative.contains(':')
        || Path::new(relative).is_absolute()
        || !base.starts_with(root)
    {
        return Err("project path must be an explicit relative path inside the project".into());
    }
    let parts: Vec<_> = relative.split('/').collect();
    let mut path = base.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        match *part {
            "" => return Err("empty project path component".into()),
            "." => {}
            ".." => {
                if path == root || !path.pop() {
                    return Err("sidecar path escapes the project root".into());
                }
            }
            part => path.push(part),
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("project path {}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink rejected: {}", path.display()));
        }
        let needs_directory = directory || i + 1 < parts.len();
        if (needs_directory && !metadata.is_dir()) || (!needs_directory && !metadata.is_file()) {
            return Err(format!(
                "project path requires a regular {}: {}",
                if needs_directory { "directory" } else { "file" },
                path.display()
            ));
        }
    }
    let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("project path escapes the canonical project root".into());
    }
    Ok(canonical)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    // Do not open a FIFO/device merely to inspect its descriptor afterward.
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > limit {
        return Err(format!(
            "{} must be a regular file within {limit} bytes",
            path.display()
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "{} must be a regular file within {limit} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}
