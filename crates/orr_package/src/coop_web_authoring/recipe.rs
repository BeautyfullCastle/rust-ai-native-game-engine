//! Portable recipe admission only: no build execution, fetching, or script overrides.
use super::{absolute, manifest, prepared::snapshot};
use crate::authoring_io::{directory, recheck_directory};
use rustix::fs::{openat, Mode, OFlags};
use serde_json::Value;
use std::{
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
};
const MAX_RECIPE: usize = 8192;
const USAGE: &str = "orr_pkg build-coop-recipe --recipe FILE --output NEW_HTML\nPackages an existing build using its trusted local WASM and trusted Node gates. No fetch or build. No overwrite. Unlike the legacy Python positional command, --recipe is required; duplicate options and oversized paths are refused.";
pub(super) struct Recipe {
    source: PathBuf,
    original: Vec<u8>,
    build: PathBuf,
    output: PathBuf,
    scene: Option<PathBuf>,
    animations: Option<PathBuf>,
    audio: Option<PathBuf>,
}
impl Recipe {
    pub(super) fn build_dir(&self) -> &Path {
        &self.build
    }
    pub(super) fn output(&self) -> &Path {
        &self.output
    }
    pub(super) fn scene(&self) -> Option<&Path> {
        self.scene.as_deref()
    }
    pub(super) fn animations(&self) -> Option<&Path> {
        self.animations.as_deref()
    }
    pub(super) fn audio(&self) -> Option<&Path> {
        self.audio.as_deref()
    }
    pub(super) fn recheck(&self) -> Result<(), String> {
        if snapshot(&self.source, MAX_RECIPE)? != self.original {
            return Err("recipe changed during verification".into());
        }
        Ok(())
    }
}
fn input_path(root: &Path, value: &Value, is_directory: bool) -> Result<PathBuf, String> {
    let text = value
        .as_str()
        .ok_or("recipe input must be a relative path string")?;
    if text.is_empty()
        || text.chars().count() > 1024
        || text.contains(['\\', ':', '\0'])
        || text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("recipe paths must be bounded canonical relative paths".into());
    }
    let path = absolute(&root.join(text))?;
    if is_directory {
        let fd = directory(&path)?;
        recheck_directory(&path, &fd)?;
    } else {
        let parent = path.parent().ok_or("missing recipe input parent")?;
        let fd = directory(parent)?;
        let leaf = File::from(
            openat(
                &fd,
                path.file_name().ok_or("missing recipe input name")?,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| e.to_string())?,
        );
        if !leaf.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("recipe input must be a regular non-symlink file".into());
        }
        recheck_directory(parent, &fd)?;
    }
    Ok(path)
}
fn prepare(source: &Path, output: &Path) -> Result<Recipe, String> {
    let source = absolute(source)?;
    let output = absolute(output)?;
    let original = snapshot(&source, MAX_RECIPE)?;
    let data = manifest::parse_bounded(&original, MAX_RECIPE)?;
    let object = data
        .as_object()
        .ok_or("expected closed orr_coop_project version 1 recipe")?;
    if object.keys().any(|k| {
        !matches!(
            k.as_str(),
            "format" | "version" | "build" | "scene" | "animations" | "audio"
        )
    }) || data.get("format").and_then(Value::as_str) != Some("orr_coop_project")
        || data.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Err("expected closed orr_coop_project version 1 recipe".into());
    }
    let root = source.parent().ok_or("missing recipe parent")?;
    let build = input_path(
        root,
        data.get("build").ok_or("recipe requires build")?,
        true,
    )?;
    let optional = |name: &str| {
        data.get(name)
            .map(|value| input_path(root, value, false))
            .transpose()
    };
    let scene = optional("scene")?;
    let animations = optional("animations")?;
    let audio = optional("audio")?;
    // Output collision and parent admission remain the shared exclusive publisher's policy.
    super::publish::preflight(&output)?;
    Ok(Recipe {
        source,
        original,
        build,
        output,
        scene,
        animations,
        audio,
    })
}
fn options(args: &[OsString]) -> Result<(PathBuf, PathBuf), String> {
    if args.len() != 4 {
        return Err("expected --recipe FILE --output NEW_HTML".into());
    }
    let mut recipe = None;
    let mut output = None;
    for pair in args.chunks_exact(2) {
        let raw = pair[1].as_encoded_bytes();
        if raw.is_empty() || raw.len() > 4096 || raw.contains(&0) {
            return Err("empty or oversized option path".into());
        }
        match pair[0].to_str() {
            Some("--recipe") if recipe.is_none() => recipe = Some(PathBuf::from(&pair[1])),
            Some("--output") if output.is_none() => output = Some(PathBuf::from(&pair[1])),
            _ => return Err("unknown or duplicate recipe option".into()),
        }
    }
    Ok((
        recipe.ok_or("missing --recipe")?,
        output.ok_or("missing --output")?,
    ))
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    // Bound collection even for callers supplying an unbounded iterator.
    let args: Vec<_> = args.into_iter().take(5).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let (source, output) = match options(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return 2;
        }
    };
    let result = prepare(&source, &output).and_then(|recipe| {
        let warning = super::offline::build_recipe(&recipe)?;
        if let Some(warning) = warning {
            eprintln!("{warning}");
        }
        println!("{}", recipe.output().display());
        Ok(())
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Recipe build failed: {e}");
            1
        }
    }
}
#[cfg(test)]
#[path = "recipe_tests.rs"]
mod tests;
