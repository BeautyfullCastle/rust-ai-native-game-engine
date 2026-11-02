//! Offline fixed-source starter materialization; never invokes Cargo, Git or a runtime.
mod inventory;
mod materialize;
mod publish;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};
const USAGE: &str = "orr_pkg bootstrap-user-game --engine PATH --destination NEW_PATH --name NAME [--profile legacy|multi-action-preview]";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Profile {
    Legacy,
    MultiActionPreview,
}
impl Profile {
    fn name(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::MultiActionPreview => "multi-action-preview",
        }
    }
    fn commit(self) -> &'static str {
        match self {
            Self::Legacy => "27e53b356b25aa505995f3e8b322093948ef9db0",
            Self::MultiActionPreview => "6f97ac85a80aabda26679b482d182c9365bff92e",
        }
    }
    fn tree(self) -> &'static str {
        match self {
            Self::Legacy => "59158bede83603d7cb272ce7a101b4178956c598",
            Self::MultiActionPreview => "cd7cc2a03990a8778a99e60a82c3cfbb425c4d87",
        }
    }
    fn omitted(self) -> &'static [&'static str] {
        match self {
            Self::Legacy => &["tests/standalone_x11.rs", "tests/support/standalone_x11.rs"],
            Self::MultiActionPreview => &[
                "tests/standalone_x11.rs",
                "tests/support/standalone_x11.rs",
                "tests/multi_action_standalone_x11.rs",
                "tests/multi_action_editor_x11.rs",
                "tests/support/multi_action_fixture_player.rs",
                "tests/support/multi_action_fixture.rs",
            ],
        }
    }
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty()
        || bytes.len() > 4096
        || bytes.contains(&0)
        || bytes.contains(&b'\\')
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("empty, oversized, traversal or ambiguous path".into());
    }
    let full = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let normalized: PathBuf = full
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if normalized.as_os_str().as_encoded_bytes().len() > 4096
        || normalized.components().count() > 128
        || normalized.to_str().is_none()
    {
        return Err("path exceeds bounds or is not Unicode".into());
    }
    Ok(normalized)
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !matches!(name, "test" | "build" | "deps" | "examples")
}
fn run(args: &[OsString]) -> Result<(), String> {
    let mut options = BTreeMap::new();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        let key = pair[0].to_str().ok_or("flag must be Unicode")?;
        if !matches!(key, "--engine" | "--destination" | "--name" | "--profile")
            || options.insert(key, &pair[1]).is_some()
        {
            return Err("unknown or duplicate bootstrap flag".into());
        }
    }
    if !pairs.remainder().is_empty() {
        return Err("missing flag value".into());
    }
    let get = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let engine = absolute(Path::new(get("--engine")?))?;
    let destination = absolute(Path::new(get("--destination")?))?;
    let name = get("--name")?.to_str().ok_or("name must be Unicode")?;
    if !valid_name(name) {
        return Err(
            "project name must be a lowercase Cargo-safe nonreserved name <=64 characters".into(),
        );
    }
    if destination.starts_with(&engine) {
        return Err("destination must be outside the engine checkout".into());
    }
    let profile = match options
        .get("--profile")
        .map(|v| v.to_str())
        .unwrap_or(Some("legacy"))
    {
        Some("legacy") => Profile::Legacy,
        Some("multi-action-preview") => Profile::MultiActionPreview,
        _ => return Err("unknown bootstrap profile".into()),
    };
    publish::preflight(&destination)?;
    let snapshots = inventory::validate_engine(&engine, profile)?;
    let files = materialize::generated_files(snapshots, &engine, name, profile)?;
    if let Some(warning) = publish::publish(&destination, &files)? {
        eprintln!("warning: published; durability uncertain: {warning}");
    }
    #[derive(serde::Serialize)]
    struct Report<'a> {
        destination: &'a Path,
        engine_tree: &'static str,
        files: usize,
        cargo_ran: bool,
    }
    println!(
        "{}",
        serde_json::to_string(&Report {
            destination: &destination,
            engine_tree: profile.tree(),
            files: files.len(),
            cargo_ran: false
        })
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let mut values = Vec::new();
    let mut bytes = 0usize;
    for arg in args {
        bytes = bytes.saturating_add(arg.as_encoded_bytes().len());
        if bytes > 65536 || values.len() >= 16 {
            eprintln!("bootstrap argv exceeds limits");
            return 2;
        }
        values.push(arg);
    }
    if values.len() == 1 && values[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    if values.len() % 2 != 0
        || values.chunks_exact(2).any(|p| {
            !matches!(
                p[0].to_str(),
                Some("--engine" | "--destination" | "--name" | "--profile")
            )
        })
    {
        eprintln!("invalid bootstrap arguments; {USAGE}");
        return 2;
    }
    match run(&values) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("bootstrap refused: {e}");
            1
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_paths_and_closed_profiles() {
        for name in [
            "", "Test", "test", "build", "deps", "examples", "a/b", "a.b",
        ] {
            assert!(!valid_name(name));
        }
        assert!(valid_name("my-game_2"));
        assert!(absolute(Path::new("a/../b")).is_err());
        assert!(absolute(Path::new("a\\b")).is_err());
        assert_eq!(Profile::Legacy.name(), "legacy");
        assert_ne!(Profile::Legacy.tree(), Profile::MultiActionPreview.tree());
    }
}
