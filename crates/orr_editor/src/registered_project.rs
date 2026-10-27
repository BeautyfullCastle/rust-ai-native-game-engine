//! Bounded selection of a compiled, registered game and an owned scene.
//!
//! This manifest selects code already admitted by the application's registry;
//! it never loads code, enables features, installs packages or owns their lock.
//! Schema authority stays with the immutable compiled handle and the existing
//! exact host/typed-stream handshake, not a duplicate manifest schema digest.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;

use crate::static_adapter::{GameplayContracts, RegisteredGameplay};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SCENE_BYTES: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    game: String,
    build_id: String,
    scene: String,
    input: String,
}

/// Read-only startup candidate. First host startup consumes these owned bytes,
/// never an unchecked reread of `scene_path`. Normal Save/Restart remains under
/// the editor's existing document/host lifecycle.
#[derive(Clone)]
pub struct PreparedRegisteredProject {
    root: PathBuf,
    manifest_path: PathBuf,
    scene_path: PathBuf,
    scene_text: Arc<str>,
    game: RegisteredGameplay,
}
impl std::fmt::Debug for PreparedRegisteredProject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedRegisteredProject")
            .field("game", &self.game.name())
            .field("manifest_path", &self.manifest_path)
            .field("scene_path", &self.scene_path)
            .field("scene_bytes", &self.scene_text.len())
            .finish()
    }
}
impl PreparedRegisteredProject {
    /// Open one explicit manifest. No directory search or fallback is performed.
    /// This bounds ordinary trusted/quiescent file opening; it does not claim an
    /// atomic multi-file snapshot against hostile concurrent replacement.
    pub fn open(
        manifest_path: impl AsRef<Path>,
        games: &'static GameplayContracts,
    ) -> Result<Self, String> {
        let supplied = manifest_path.as_ref();
        let manifest_bytes = read_regular(supplied, MAX_MANIFEST_BYTES)?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("registered project manifest: {error}"))?;
        if manifest.schema != "orr.registered-project/1" {
            return Err("unsupported registered project schema".into());
        }
        if !canonical_build(&manifest.build_id) {
            return Err("registered project build_id must be a nonzero canonical hex u64".into());
        }
        let game = games
            .handle(&manifest.game)
            .ok_or("registered project game is not compiled into this application")?;
        if game.name() != manifest.game {
            return Err("registered project game must use its canonical registered ID".into());
        }
        if game.expected_identity().build_id != manifest.build_id {
            return Err("registered project build identity mismatch".into());
        }
        if manifest.input != "commands" || !game.has_commands() {
            return Err("registered project requires its compiled commands capability".into());
        }
        let manifest_path = fs::canonicalize(supplied).map_err(|error| error.to_string())?;
        let root = manifest_path
            .parent()
            .ok_or("registered project manifest has no parent directory")?
            .to_path_buf();
        let scene_path = resolve_scene(&root, &manifest.scene)?;
        if scene_path == manifest_path {
            return Err("registered project scene must differ from its manifest".into());
        }
        let scene_text = String::from_utf8(read_regular(&scene_path, MAX_SCENE_BYTES)?)
            .map_err(|error| format!("registered scene must be UTF-8: {error}"))?;
        // A fresh typed host additionally performs its game-owned bake admission.
        // Parsing uses only the already admitted compiled reflection registry.
        orr_reflect::Scene::parse(&scene_text, &game.types())
            .map_err(|error| format!("registered scene: {error}"))?;
        Ok(Self {
            root,
            manifest_path,
            scene_path,
            scene_text: scene_text.into(),
            game,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
    pub fn scene_path(&self) -> &Path {
        &self.scene_path
    }
    pub fn scene_text(&self) -> &str {
        &self.scene_text
    }
    pub fn game(&self) -> RegisteredGameplay {
        self.game
    }
    pub fn host_spec(&self) -> crate::HostSpec {
        crate::HostSpec::PreparedRegistered {
            project: self.clone(),
            listen: None,
            debug_hooks: false,
        }
    }
}

fn canonical_build(value: &str) -> bool {
    value.len() == 18
        && value.starts_with("0x")
        && value.as_bytes()[2..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        && value != "0x0000000000000000"
}
fn portable(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value.is_ascii()
        && value.split('/').count() <= 16
        && value.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 100
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        })
}
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reject directory junctions/reparse points as well as leaf symlinks.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn resolve_scene(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !portable(relative) {
        return Err("registered scene requires a portable relative path inside the project".into());
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if is_link(&metadata)
            || (index + 1 < parts.len() && !metadata.is_dir())
            || (index + 1 == parts.len() && !metadata.is_file())
        {
            return Err("registered scene path must contain regular directories and a regular file, without links".into());
        }
    }
    let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !path.starts_with(root) {
        return Err("registered scene escapes its canonical project root".into());
    }
    Ok(path)
}
fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if is_link(&before) || !before.is_file() || before.len() > limit {
        return Err(format!(
            "project input must be a regular file within {limit} bytes"
        ));
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.len() > limit {
        return Err("project input changed before opening".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let after = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit
        || is_link(&after)
        || !after.is_file()
        || before.len() != opened.len()
        || opened.len() != after.len()
        || bytes.len() as u64 != after.len()
        || before.modified().ok() != opened.modified().ok()
        || opened.modified().ok() != after.modified().ok()
    {
        return Err("project input changed while reading".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_a_closed_five_field_object_with_duplicate_rejection() {
        let valid = r#"{"schema":"orr.registered-project/1","game":"Fixture","build_id":"0x0000000000000001","scene":"game.scene.yaml","input":"commands"}"#;
        assert!(serde_json::from_str::<Manifest>(valid).is_ok());
        for invalid in [
            valid.replace(
                "\"input\":\"commands\"",
                "\"input\":\"commands\",\"code\":\"remote.dll\"",
            ),
            valid.replace(
                "\"game\":\"Fixture\"",
                "\"game\":\"Fixture\",\"game\":\"Other\"",
            ),
            valid.replace("\"scene\":\"game.scene.yaml\"", "\"scene\":[]"),
            valid.replace("\"input\":\"commands\"", "\"input\":null"),
            "[]".into(),
        ] {
            assert!(serde_json::from_str::<Manifest>(&invalid).is_err());
        }
    }
    #[test]
    fn portable_paths_and_canonical_build_ids_have_closed_domains() {
        for path in ["game.scene.yaml", "scenes/game-1.scene.yaml"] {
            assert!(portable(path), "{path}");
        }
        for path in [
            "",
            "../game.yaml",
            "scenes/../game.yaml",
            "./game.yaml",
            "/game.yaml",
            "C:/game.yaml",
            "C:\\game.yaml",
            "game.yaml:stream",
            "scenes//game.yaml",
            "CON.yaml",
            "a/LPT1.yaml",
            "game.yaml.",
            "game yaml",
            "game\0yaml",
            "https://example.com/game.yaml",
            "file:///game.yaml",
            "게임.yaml",
        ] {
            assert!(!portable(path), "{path}");
        }
        assert!(!portable(&"a".repeat(241)));
        assert!(!portable(&format!("{}.yaml", "a".repeat(100))));
        for id in [
            "0x0000000000000001",
            "0x5354415254455201",
            "0xffffffffffffffff",
        ] {
            assert!(canonical_build(id));
        }
        for id in [
            "0x0000000000000000",
            "1",
            "0x1",
            "0X5354415254455201",
            "0x535441525445520A",
            "0xgggggggggggggggg",
        ] {
            assert!(!canonical_build(id));
        }
    }
}
