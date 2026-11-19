//! Cheap content check for cooked JSON model candidates in the package browser.
//!
//! A file name ending in `.model.json` or `.animated.json` is only a hint. This
//! module reads a bounded prefix of the installed file and checks its `format`
//! and `version` members with the same classifier the catalog uses. It does not
//! validate the model body. Preview and Use still run the full loader.
//! Animated candidates are only offered by editors built with the
//! `room-animated-models` feature (part of `full`).

use crate::model_bindings::{cooked_json_kind, ModelKind, COOKED_HEADER_PROBE_BYTES};
use std::{
    collections::HashMap,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// Uncached files probed in one frame. More wait for the next frame.
pub(crate) const PROBES_PER_FRAME: usize = 32;
const MAX_ENTRIES: usize = 8192;

pub(crate) type Verdict = Result<ModelKind, String>;

/// True for names whose cooked JSON content must be checked before offering.
pub(crate) fn is_cooked_json(asset: &str) -> bool {
    let lower = asset.to_ascii_lowercase();
    lower.ends_with(".model.json") || lower.ends_with(".animated.json")
}

/// Verdicts per installed package digest, project folder, and asset path. A
/// package digest names immutable content, so a verdict never goes stale.
#[derive(Default)]
pub(crate) struct Catalog {
    entries: HashMap<(PathBuf, String, String), Verdict>,
}

impl Catalog {
    pub(crate) fn verdict(&self, root: &Path, digest: &str, asset: &str) -> Option<&Verdict> {
        self.entries
            .get(&(root.to_owned(), digest.to_owned(), asset.to_owned()))
    }

    /// Reads and records one verdict.
    pub(crate) fn probe(&mut self, root: &Path, digest: &str, asset: &str) -> &Verdict {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.clear();
        }
        self.entries
            .entry((root.to_owned(), digest.to_owned(), asset.to_owned()))
            .or_insert_with(|| read_verdict(root, digest, asset))
    }
}

fn read_verdict(root: &Path, digest: &str, asset: &str) -> Verdict {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("package digest is not valid".into());
    }
    let relative = Path::new(asset);
    if asset.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("asset path is not a plain relative path".into());
    }
    let path = root
        .join(".orr/packages/objects")
        .join(digest)
        .join(relative);
    // `open_file` refuses links and non-files and reads through one handle.
    let file = orr_fsid::open_file(&path).map_err(|error| format!("cannot read file: {error}"))?;
    let mut prefix = Vec::new();
    file.take(COOKED_HEADER_PROBE_BYTES as u64)
        .read_to_end(&mut prefix)
        .map_err(|error| format!("cannot read file: {error}"))?;
    let kind = cooked_json_kind(&prefix)?;
    let lower = asset.to_ascii_lowercase();
    let expected = if lower.ends_with(".animated.json") {
        ModelKind::Animated
    } else {
        ModelKind::Static
    };
    if kind != expected {
        return Err(format!(
            "the file name is for {} model but the content is {} model",
            label(expected),
            label(kind)
        ));
    }
    Ok(kind)
}

fn label(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Static => "a static",
        ModelKind::Animated => "an animated",
    }
}
