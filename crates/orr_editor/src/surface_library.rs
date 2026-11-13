//! Surface shaders of the open scene. A material names a shader; the file is
//! `shaders/<name>.wgsl` next to the scene file. Each file is compiled on the
//! CPU when it is first needed and again when it changes on disk, so a person
//! can keep the shader open in a text editor and see each save in the viewport.
#![allow(clippy::disallowed_types)]

use orr_render::SurfaceShader;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Folder of the surface shaders, next to the scene file.
pub const SHADER_DIR: &str = "shaders";
/// How often each file, and the folder listing, is looked at again.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Versions are unique in the process, so a viewport never confuses two
/// different files that had the same name in two projects.
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// One shader file and the result of its last compile.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Changes every time the file is read again.
    pub version: u64,
    /// The compiled shader, or why it cannot be used, with the file and line.
    pub result: Result<SurfaceShader, String>,
    stamp: Option<(SystemTime, u64)>,
    checked: Instant,
}

#[derive(Default)]
pub struct SurfaceLibrary {
    root: Option<PathBuf>,
    entries: BTreeMap<String, Entry>,
    listing: Vec<String>,
    listed: Option<Instant>,
}

/// Names Windows keeps for devices; a file with such a name opens the device.
const DEVICE_NAMES: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// Whether `name` can name a shader file: 1 to 48 letters, digits, `_` and `-`,
/// and not a Windows device name.
pub fn valid_name(name: &str) -> bool {
    (1..=orr_gfx::SHADER_NAME_BYTES).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !DEVICE_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl SurfaceLibrary {
    /// Points the library at the shaders of the scene file `scene`. Another
    /// scene folder forgets every loaded shader.
    pub fn set_scene(&mut self, scene: Option<&Path>) {
        let root = scene.and_then(Path::parent).map(|dir| dir.join(SHADER_DIR));
        if root != self.root {
            self.root = root;
            self.entries.clear();
            self.listing.clear();
            self.listed = None;
        }
    }

    /// The folder shaders are read from, if a scene file is open.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Reads the shaders in `names` that are not loaded yet, and those whose
    /// file changed, looking at each file at most every half second. An empty
    /// name means "no shader" and is skipped. Returns true when an entry changed.
    pub fn refresh<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> bool {
        let now = Instant::now();
        let mut changed = false;
        for name in names {
            if name.is_empty() {
                continue;
            }
            if self.entries.get(name).is_some_and(|entry| now.duration_since(entry.checked) < CHECK_INTERVAL) {
                continue;
            }
            let path = self.root.as_ref().map(|root| root.join(format!("{name}.wgsl")));
            let current = path.as_deref().and_then(stamp);
            if let Some(entry) = self.entries.get_mut(name).filter(|entry| entry.stamp == current) {
                entry.checked = now;
                continue;
            }
            let result = self.load(name, path.as_deref());
            let version = NEXT_VERSION.fetch_add(1, Ordering::Relaxed);
            self.entries.insert(name.to_string(), Entry { version, result, stamp: current, checked: now });
            changed = true;
        }
        changed
    }

    fn load(&self, name: &str, path: Option<&Path>) -> Result<SurfaceShader, String> {
        if !valid_name(name) {
            return Err(format!("'{name}' is not a shader name: use 1 to {} letters, digits, _ and -", orr_gfx::SHADER_NAME_BYTES));
        }
        let path = path.ok_or("Save the scene to a file first: shaders are read from its folder")?;
        let shown = format!("{SHADER_DIR}/{name}.wgsl");
        let source = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
        SurfaceShader::compile(name, &source).map_err(|e| format!("{shown} {e}"))
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// Names of the `.wgsl` files in the shader folder, sorted. The folder is
    /// read again at most every half second.
    pub fn available(&mut self) -> &[String] {
        let now = Instant::now();
        if self.listed.is_none_or(|at| now.duration_since(at) >= CHECK_INTERVAL) {
            self.listed = Some(now);
            self.listing = self.root.as_deref().and_then(|root| std::fs::read_dir(root).ok()).map_or_else(Vec::new, |read| {
                let mut names: Vec<String> = read
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        (path.extension().is_some_and(|e| e == "wgsl")).then(|| path.file_stem()?.to_str().map(str::to_string)).flatten()
                    })
                    .filter(|name| valid_name(name))
                    .collect();
                names.sort();
                names
            });
        }
        &self.listing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "// @param tint color = 1, 0, 0\nfn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = params.tint; return s; }\n";

    /// Lets every entry be looked at again, as half a second passing does.
    fn age(library: &mut SurfaceLibrary) {
        for entry in library.entries.values_mut() {
            entry.checked -= CHECK_INTERVAL;
        }
    }

    #[test]
    fn shaders_load_from_the_scene_folder_and_reload_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let scene = dir.path().join("level.scene.yaml");
        std::fs::create_dir(dir.path().join(SHADER_DIR)).unwrap();
        let file = dir.path().join(SHADER_DIR).join("tint.wgsl");
        std::fs::write(&file, GOOD).unwrap();
        std::fs::write(dir.path().join(SHADER_DIR).join("other.wgsl"), GOOD).unwrap();
        let mut library = SurfaceLibrary::default();
        library.set_scene(Some(&scene));
        assert_eq!(library.available(), ["other", "tint"]);
        assert!(library.refresh(["tint", "missing", "bad name", "", "con"]));
        let first = library.get("tint").unwrap().clone();
        assert_eq!(first.result.as_ref().unwrap().params()[0].name, "tint");
        assert!(library.get("missing").unwrap().result.as_ref().unwrap_err().contains("shaders/missing.wgsl"));
        assert!(library.get("bad name").unwrap().result.as_ref().unwrap_err().contains("is not a shader name"));
        assert!(library.get("con").unwrap().result.as_ref().unwrap_err().contains("is not a shader name"));
        assert!(library.get("").is_none(), "an empty name means no shader");

        // Nothing changed: no new version.
        age(&mut library);
        assert!(!library.refresh(["tint"]));
        // A broken save is reported with its line; the next good save is used again.
        std::fs::write(&file, "fn surface(in: SurfaceInput) -> Surface {\n  return missing_value;\n}\n").unwrap();
        age(&mut library);
        assert!(library.refresh(["tint"]));
        let broken = library.get("tint").unwrap();
        assert!(broken.version > first.version);
        assert!(broken.result.as_ref().unwrap_err().starts_with("shaders/tint.wgsl line 2"), "{:?}", broken.result);
        std::fs::write(&file, GOOD.replace("1, 0, 0", "0, 1, 0")).unwrap();
        age(&mut library);
        assert!(library.refresh(["tint"]));
        assert_eq!(library.get("tint").unwrap().result.as_ref().unwrap().default_values()[1], 1.0);

        // Another scene folder starts empty.
        library.set_scene(Some(&std::env::temp_dir().join("elsewhere").join("x.scene.yaml")));
        assert!(library.get("tint").is_none());
    }

    #[test]
    fn each_shader_is_checked_on_its_own_schedule() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(SHADER_DIR)).unwrap();
        for name in ["a", "b"] {
            std::fs::write(dir.path().join(SHADER_DIR).join(format!("{name}.wgsl")), GOOD).unwrap();
        }
        let mut library = SurfaceLibrary::default();
        library.set_scene(Some(&dir.path().join("s.scene.yaml")));
        library.refresh(["a", "b"]);
        let before = library.get("b").unwrap().version;
        std::fs::write(dir.path().join(SHADER_DIR).join("b.wgsl"), GOOD.replace("1, 0, 0", "0, 0, 1")).unwrap();
        age(&mut library);
        // One caller looks only at "a"; another caller in the same frame still gets "b" checked.
        library.refresh(["a"]);
        assert!(library.refresh(["b"]));
        assert!(library.get("b").unwrap().version > before);
    }

    #[test]
    fn a_scene_without_a_file_cannot_load_shaders() {
        let mut library = SurfaceLibrary::default();
        library.set_scene(None);
        library.refresh(["tint"]);
        assert!(library.get("tint").unwrap().result.as_ref().unwrap_err().contains("Save the scene"));
        assert!(library.available().is_empty());
    }
}
