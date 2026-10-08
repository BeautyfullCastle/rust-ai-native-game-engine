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
pub struct PreparedProject {
    root: PathBuf,
    scene: PreparedArenaScene,
    sprites: Option<PreparedSprites>,
}
impl PreparedProject {
    /// Validate the entire active lock using the caller's real compiled inventory.
    pub fn open(root: impl AsRef<Path>, runtime: orr_package::Runtime) -> Result<Self, String> {
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
    pub fn into_parts(self) -> (PathBuf, PreparedArenaScene, Option<PreparedSprites>) {
        (self.root, self.scene, self.sprites)
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
