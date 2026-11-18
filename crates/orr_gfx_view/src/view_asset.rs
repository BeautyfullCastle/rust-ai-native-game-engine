//! View assets: the small project files that `orr_gfx::ViewRef` points at.
//!
//! A view asset is a `.orrview` file in the project's asset folder. It bundles what the view layer
//! needs to draw one kind of entity: a model, and optionally a tint, a scale, a sound name and the
//! name of an entity view. The simulation never reads it. It stores only the id of the asset
//! ([`orr_gfx::view_asset_id`] of the file's path).
//!
//! The file is JSON, version 1:
//!
//! ```json
//! { "version": 1, "name": "Knight", "model": "models/knight.glb",
//!   "scale": 1.0, "tint": [1.0, 0.8, 0.6], "sound": "knight_step", "entity_view": "knight" }
//! ```
//!
//! Only `version` and `model` are required. `model` is a `.glb` or `.gltf` path relative to the asset
//! folder, with `/` between folders. `name` defaults to the file name without its extension. `scale`
//! multiplies the scale of the entity (default 1). Unknown keys are refused, so a typo shows up.
//!
//! [`ViewAssets::load_dir`] reads every `.orrview` file under a folder. Two files whose paths hash to
//! the same id are an error, so an id never means two assets.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The file extension of a view asset.
pub const VIEW_ASSET_EXTENSION: &str = "orrview";

/// The format version this reader understands.
pub const VIEW_ASSET_VERSION: u64 = 1;

/// The deepest folder `load_dir` visits, counted from the asset folder.
const MAX_DEPTH: u32 = 4;
/// The most view assets `load_dir` reads.
const MAX_ASSETS: usize = 4096;
/// The biggest `.orrview` file, in bytes.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// One view asset, as read from its file.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewAsset {
    /// The id that `ViewRef::asset` holds: [`orr_gfx::view_asset_id`] of `path`.
    pub id: u64,
    /// Path of the file relative to the asset folder, like `avatars/knight.orrview`.
    pub path: String,
    /// Display name.
    pub name: String,
    /// Model path relative to the asset folder.
    pub model: String,
    /// Multiplies the scale of the entity.
    pub scale: f32,
    /// Linear color the model is tinted with. Kept for the view; the default renderer does not use it yet.
    pub tint: Option<[f32; 3]>,
    /// Name of a sound for the entity. Kept for the view; the default renderer does not play it.
    pub sound: Option<String>,
    /// Name of an entity view for the entity. Kept for the view.
    pub entity_view: Option<String>,
}

/// Why a view asset could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewAssetError {
    /// The file or the folder could not be read.
    Io {
        /// What was read.
        path: String,
        /// Why not.
        message: String,
    },
    /// The text is not a valid view asset.
    Invalid {
        /// The path of the asset.
        path: String,
        /// What is wrong.
        message: String,
    },
    /// Two files have the same id.
    SameId {
        /// The id both paths hash to.
        id: u64,
        /// The first file.
        first: String,
        /// The second file.
        second: String,
    },
}

impl fmt::Display for ViewAssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read {path}: {message}"),
            Self::Invalid { path, message } => write!(f, "{path}: {message}"),
            Self::SameId { id, first, second } => write!(f, "{first} and {second} have the same id {id}; rename one of them"),
        }
    }
}

impl std::error::Error for ViewAssetError {}

impl ViewAsset {
    /// Reads the text of the view asset at `path` (relative to the asset folder, with `/`).
    pub fn parse(path: &str, text: &str) -> Result<Self, ViewAssetError> {
        let bad = |message: String| ViewAssetError::Invalid { path: path.to_string(), message };
        if !path.ends_with(&format!(".{VIEW_ASSET_EXTENSION}")) {
            return Err(bad(format!("a view asset file ends in .{VIEW_ASSET_EXTENSION}")));
        }
        let value: Value = serde_json::from_str(text).map_err(|e| bad(format!("not valid JSON: {e}")))?;
        let Value::Object(map) = &value else { return Err(bad("the file must hold one JSON object".into())) };
        for key in map.keys() {
            if !["version", "name", "model", "scale", "tint", "sound", "entity_view"].contains(&key.as_str()) {
                return Err(bad(format!("unknown key '{key}' (version, name, model, scale, tint, sound, entity_view)")));
            }
        }
        match map.get("version").and_then(Value::as_u64) {
            Some(VIEW_ASSET_VERSION) => {}
            Some(other) => return Err(bad(format!("version {other} is not supported (expected {VIEW_ASSET_VERSION})"))),
            None => return Err(bad("\"version\" is missing: write \"version\": 1".into())),
        }
        let text_of = |key: &str| -> Result<Option<String>, ViewAssetError> {
            match map.get(key) {
                None => Ok(None),
                Some(Value::String(s)) if !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control) => Ok(Some(s.clone())),
                Some(_) => Err(bad(format!("\"{key}\" must be a text of 1 to 128 bytes without control characters"))),
            }
        };
        let model = text_of("model")?.ok_or_else(|| bad("\"model\" is missing: the path of a .glb or .gltf file".into()))?;
        check_model_path(&model).map_err(|why| bad(format!("\"model\" {why}")))?;
        let scale = match map.get("scale") {
            None => 1.0,
            Some(v) => v.as_f64().filter(|s| s.is_finite() && *s > 0.0 && *s <= 1000.0).ok_or_else(|| bad("\"scale\" must be a number above 0 and at most 1000".into()))? as f32,
        };
        let tint = match map.get("tint") {
            None => None,
            Some(Value::Array(items)) if items.len() == 3 => {
                let mut rgb = [0.0_f32; 3];
                for (slot, item) in rgb.iter_mut().zip(items) {
                    *slot = item.as_f64().filter(|c| c.is_finite() && (0.0..=16.0).contains(c)).ok_or_else(|| bad("\"tint\" must be three numbers from 0 to 16".into()))? as f32;
                }
                Some(rgb)
            }
            Some(_) => return Err(bad("\"tint\" must be three numbers from 0 to 16".into())),
        };
        let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(&format!(".{VIEW_ASSET_EXTENSION}")).to_string();
        Ok(Self {
            id: orr_gfx::view_asset_id(path),
            path: path.to_string(),
            name: text_of("name")?.unwrap_or(stem),
            model,
            scale,
            tint,
            sound: text_of("sound")?,
            entity_view: text_of("entity_view")?,
        })
    }
}

fn check_model_path(path: &str) -> Result<(), String> {
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err("must be relative to the asset folder, with / between folders".into());
    }
    if path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err("must not have empty parts, . or ..".into());
    }
    let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("glb" | "gltf")) {
        return Err("must be a .glb or .gltf file".into());
    }
    Ok(())
}

/// The view assets of a project, by id.
#[derive(Clone, Debug, Default)]
pub struct ViewAssets {
    root: PathBuf,
    by_id: BTreeMap<u64, ViewAsset>,
}

impl ViewAssets {
    /// A set of already read assets; `root` is the asset folder their model paths start from.
    /// Two assets with the same id are an error.
    pub fn from_assets(root: impl Into<PathBuf>, assets: impl IntoIterator<Item = ViewAsset>) -> Result<Self, ViewAssetError> {
        let mut by_id: BTreeMap<u64, ViewAsset> = BTreeMap::new();
        for asset in assets {
            if let Some(first) = by_id.get(&asset.id) {
                return Err(ViewAssetError::SameId { id: asset.id, first: first.path.clone(), second: asset.path });
            }
            by_id.insert(asset.id, asset);
        }
        Ok(Self { root: root.into(), by_id })
    }

    /// Reads every `.orrview` file under `root`, at most four folders deep, skipping hidden folders.
    /// A model that does not exist is an error naming the asset.
    pub fn load_dir(root: impl AsRef<Path>) -> Result<Self, ViewAssetError> {
        let root = root.as_ref();
        let mut files = Vec::new();
        visit(root, "", 0, &mut files)?;
        files.sort();
        let mut assets = Vec::with_capacity(files.len());
        for relative in files {
            let full = root.join(&relative);
            let io = |message: String| ViewAssetError::Io { path: full.display().to_string(), message };
            let length = std::fs::metadata(&full).map_err(|e| io(e.to_string()))?.len();
            if length > MAX_FILE_BYTES {
                return Err(ViewAssetError::Invalid { path: relative, message: format!("the file has {length} bytes, the most is {MAX_FILE_BYTES}") });
            }
            let text = std::fs::read_to_string(&full).map_err(|e| io(e.to_string()))?;
            let asset = ViewAsset::parse(&relative, &text)?;
            if !root.join(&asset.model).is_file() {
                return Err(ViewAssetError::Invalid { path: relative, message: format!("the model {} does not exist", asset.model) });
            }
            assets.push(asset);
        }
        Self::from_assets(root, assets)
    }

    /// The asset folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The asset with this id, if the project has one.
    pub fn get(&self, id: u64) -> Option<&ViewAsset> {
        self.by_id.get(&id)
    }

    /// All assets, ordered by id.
    pub fn iter(&self) -> impl Iterator<Item = &ViewAsset> {
        self.by_id.values()
    }

    /// How many assets there are.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True when there are none.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

fn visit(dir: &Path, prefix: &str, depth: u32, out: &mut Vec<String>) -> Result<(), ViewAssetError> {
    let read = std::fs::read_dir(dir).map_err(|e| ViewAssetError::Io { path: dir.display().to_string(), message: e.to_string() })?;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && depth < MAX_DEPTH {
            visit(&entry.path(), &path, depth + 1, out)?;
        } else if kind.is_file() && name.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case(VIEW_ASSET_EXTENSION)) {
            if out.len() >= MAX_ASSETS {
                return Err(ViewAssetError::Invalid { path, message: format!("more than {MAX_ASSETS} view assets") });
            }
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNIGHT: &str = r#"{ "version": 1, "name": "Knight", "model": "models/knight.glb", "scale": 1.5, "tint": [1, 0.5, 0.25], "sound": "step", "entity_view": "knight" }"#;

    #[test]
    fn a_full_asset_is_read_and_its_id_is_the_hash_of_its_path() {
        let asset = ViewAsset::parse("avatars/knight.orrview", KNIGHT).unwrap();
        assert_eq!(asset.id, orr_gfx::view_asset_id("avatars/knight.orrview"));
        assert_eq!(asset.id, orr_gfx::ViewRef::at("avatars/knight.orrview").asset);
        assert_eq!((asset.name.as_str(), asset.model.as_str(), asset.scale), ("Knight", "models/knight.glb", 1.5));
        assert_eq!((asset.tint, asset.sound.as_deref(), asset.entity_view.as_deref()), (Some([1.0, 0.5, 0.25]), Some("step"), Some("knight")));
    }

    #[test]
    fn a_minimal_asset_gets_the_file_name_and_scale_one() {
        let asset = ViewAsset::parse("a/b/pawn.orrview", r#"{"version":1,"model":"pawn.glb"}"#).unwrap();
        assert_eq!((asset.name.as_str(), asset.scale, asset.tint, asset.sound), ("pawn", 1.0, None, None));
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        let cases = [
            (r#"{"model":"a.glb"}"#, "version"),
            (r#"{"version":2,"model":"a.glb"}"#, "not supported"),
            (r#"{"version":1}"#, "model"),
            (r#"{"version":1,"model":"../a.glb"}"#, "empty parts"),
            (r#"{"version":1,"model":"a.png"}"#, ".glb"),
            (r#"{"version":1,"model":"a.glb","scal":2}"#, "unknown key 'scal'"),
            (r#"{"version":1,"model":"a.glb","scale":0}"#, "scale"),
            (r#"{"version":1,"model":"a.glb","tint":[1,2]}"#, "tint"),
            (r#"[1]"#, "one JSON object"),
            ("not json", "not valid JSON"),
        ];
        for (text, why) in cases {
            let error = ViewAsset::parse("x.orrview", text).unwrap_err().to_string();
            assert!(error.contains(why), "{text}: {error}");
        }
        assert!(ViewAsset::parse("x.json", KNIGHT).is_err());
    }

    #[test]
    fn two_assets_with_one_id_are_refused() {
        let one = ViewAsset::parse("a.orrview", r#"{"version":1,"model":"a.glb"}"#).unwrap();
        let mut two = ViewAsset::parse("b.orrview", r#"{"version":1,"model":"b.glb"}"#).unwrap();
        two.id = one.id;
        assert!(matches!(ViewAssets::from_assets(".", [one, two]), Err(ViewAssetError::SameId { .. })));
    }

    #[test]
    fn a_folder_is_loaded_and_ids_resolve() {
        let dir = std::env::temp_dir().join(format!("orr_view_assets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("avatars")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("avatars/red.glb"), b"glTF").unwrap();
        std::fs::write(dir.join("avatars/red.orrview"), r#"{"version":1,"model":"avatars/red.glb"}"#).unwrap();
        std::fs::write(dir.join(".hidden/blue.orrview"), "ignored").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let assets = ViewAssets::load_dir(&dir).unwrap();

        assert_eq!(assets.len(), 1);
        let red = assets.get(orr_gfx::ViewRef::at("avatars/red.orrview").asset).expect("the id of the path resolves");
        assert_eq!((red.name.as_str(), red.model.as_str()), ("red", "avatars/red.glb"));
        assert!(assets.get(12345).is_none());
        assert_eq!(assets.root(), dir.as_path());

        std::fs::remove_file(dir.join("avatars/red.glb")).unwrap();
        let error = ViewAssets::load_dir(&dir).unwrap_err().to_string();
        assert!(error.contains("does not exist"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
