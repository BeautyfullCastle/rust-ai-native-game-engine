//! Exact pinned source validation. Requires trusted, stable ancestry; not a sandbox.
use super::Profile;
use crate::authoring_io::{directory, error, read_at, recheck_directory, same_snapshot, DIR_FLAGS};
use rustix::fs::{self as unix, AtFlags, Dir, Mode, OFlags};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

const STARTER: &str = "examples/static-game-starter/";
const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_RETAINED: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: usize = 128;
const MAX_PATH: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema: u32,
    commit: String,
    tree: String,
    files: Vec<Entry>,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    mode: String,
    blob: String,
}
#[derive(Default)]
struct Tree {
    children: BTreeMap<String, Node>,
}
enum Node {
    Tree(Tree),
    Blob(String, [u8; 20]),
}

fn git_hash(kind: &str, bytes: &[u8]) -> [u8; 20] {
    let mut hash = Sha1::new();
    hash.update(format!("{kind} {}\0", bytes.len()).as_bytes());
    hash.update(bytes);
    hash.finalize().into()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn blob_id(value: &str) -> Result<[u8; 20], String> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid inventory blob".into());
    }
    let mut result = [0; 20];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).map_err(error)?;
    }
    Ok(result)
}
fn path_parts(path: &str) -> Result<Vec<&str>, String> {
    let parts: Vec<_> = path.split('/').collect();
    if path.len() > MAX_PATH
        || parts.len() > MAX_DEPTH
        || path.contains(['\\', '\0'])
        || parts
            .iter()
            .any(|p| matches!(*p, "" | "." | ".." | ".git" | "target"))
    {
        return Err("invalid or unbounded inventory path".into());
    }
    Ok(parts)
}
fn insert(tree: &mut Tree, parts: &[&str], mode: &str, blob: [u8; 20]) -> Result<(), String> {
    if parts.len() == 1 {
        if tree.children.contains_key(parts[0]) {
            return Err("duplicate or colliding inventory path".into());
        }
        tree.children
            .insert(parts[0].into(), Node::Blob(mode.into(), blob));
    } else {
        let node = tree
            .children
            .entry(parts[0].into())
            .or_insert_with(|| Node::Tree(Tree::default()));
        match node {
            Node::Tree(child) => insert(child, &parts[1..], mode, blob)?,
            Node::Blob(..) => return Err("inventory path prefix collision".into()),
        }
    }
    Ok(())
}
fn tree_hash(tree: &Tree) -> [u8; 20] {
    let mut entries: Vec<_> = tree.children.iter().collect();
    entries.sort_by_cached_key(|(name, node)| {
        format!(
            "{name}{}",
            if matches!(node, Node::Tree(_)) {
                "/"
            } else {
                ""
            }
        )
        .into_bytes()
    });
    let mut bytes = Vec::new();
    for (name, node) in entries {
        let (mode, hash) = match node {
            Node::Tree(child) => ("40000", tree_hash(child)),
            Node::Blob(mode, hash) => (mode.as_str(), *hash),
        };
        bytes.extend_from_slice(mode.as_bytes());
        bytes.push(b' ');
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&hash);
    }
    git_hash("tree", &bytes)
}
fn inventory_tree(entries: &[Entry]) -> Result<String, String> {
    if entries.len() > MAX_ENTRIES {
        return Err("inventory entry limit exceeded".into());
    }
    let mut root = Tree::default();
    for entry in entries {
        if !matches!(entry.mode.as_str(), "100644" | "100755") {
            return Err("unsupported inventory mode".into());
        }
        insert(
            &mut root,
            &path_parts(&entry.path)?,
            &entry.mode,
            blob_id(&entry.blob)?,
        )?;
    }
    Ok(hex(&tree_hash(&root)))
}

pub(super) fn validate_engine(
    engine: &Path,
    profile: Profile,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let source = match profile {
        Profile::Legacy => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/user_game_bootstrap_source.json"
        )),
        Profile::MultiActionPreview => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/user_game_action_preview_source.json"
        )),
    };
    let inventory: Inventory = serde_json::from_str(source).map_err(error)?;
    if inventory.schema != 1
        || inventory.commit != profile.commit()
        || inventory.tree != profile.tree()
        || inventory_tree(&inventory.files)? != profile.tree()
    {
        return Err("inventory is not the pinned reviewed Git tree".into());
    }
    validate_files(engine, &inventory.files)
}

struct Scan<'a> {
    expected: BTreeMap<&'a str, &'a Entry>,
    seen: BTreeSet<String>,
    snapshots: BTreeMap<String, Vec<u8>>,
    retained: usize,
    count: usize,
    observed: Vec<(String, std::fs::Metadata)>,
}
fn validate_files(engine: &Path, entries: &[Entry]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    // Tests may supply a small inventory; production always authenticates embedded inventory first.
    inventory_tree(entries)?;
    if !engine.is_absolute()
        || engine
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err("engine must have an absolute, traversal-free path".into());
    }
    let fd = directory(engine)?;
    let mut scan = Scan {
        expected: entries.iter().map(|e| (e.path.as_str(), e)).collect(),
        seen: BTreeSet::new(),
        snapshots: BTreeMap::new(),
        retained: 0,
        count: 0,
        observed: Vec::new(),
    };
    scan.walk(&fd, "", 0)?;
    // Reopen ancestry without following links, then compare every observed identity
    // and metadata after the entire scan. This detects ordinary concurrent edits;
    // like the authoring primitives, it is not an atomic filesystem snapshot.
    for (relative, before) in &scan.observed {
        let path = engine.join(relative);
        let parent = directory(path.parent().ok_or("missing engine parent")?)?;
        let current = File::from(
            unix::openat(
                &parent,
                path.file_name().ok_or("missing engine filename")?,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(error)?,
        );
        if !same_snapshot(before, &current.metadata().map_err(error)?) {
            return Err("engine input changed after scan".into());
        }
    }
    recheck_directory(engine, &fd)?;
    if scan.seen.len() != entries.len() {
        return Err("pinned engine input is missing".into());
    }
    Ok(scan.snapshots)
}
impl Scan<'_> {
    fn walk(&mut self, parent: &File, prefix: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("engine directory depth limit exceeded".into());
        }
        let before = parent.metadata().map_err(error)?;
        let mut dir = Dir::read_from(parent).map_err(error)?;
        for entry in &mut dir {
            let entry = entry.map_err(error)?;
            let name = entry.file_name().to_str().map_err(error)?;
            if name == "." || name == ".." {
                continue;
            }
            self.count += 1;
            if self.count > MAX_ENTRIES {
                return Err("engine scan entry limit exceeded".into());
            }
            if prefix.is_empty() && matches!(name, ".git" | "target") {
                continue;
            }
            let path = format!("{prefix}{name}");
            if path.len() > MAX_PATH || depth + 1 > MAX_DEPTH {
                return Err("engine path bound exceeded".into());
            }
            let stat = unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
            if unix::FileType::from_raw_mode(stat.st_mode) == unix::FileType::Directory {
                let child = File::from(
                    unix::openat(parent, name, DIR_FLAGS, Mode::empty()).map_err(error)?,
                );
                let opened = child.metadata().map_err(error)?;
                if opened.dev() != stat.st_dev || opened.ino() != stat.st_ino {
                    return Err("engine directory changed".into());
                }
                self.walk(&child, &format!("{path}/"), depth + 1)?;
                self.observed.push((path.clone(), opened.clone()));
                let current =
                    unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
                if current.st_dev != opened.dev() || current.st_ino != opened.ino() {
                    return Err("engine directory replaced".into());
                }
            } else {
                if unix::FileType::from_raw_mode(stat.st_mode) != unix::FileType::RegularFile {
                    return Err(format!("nonregular engine input: {path}"));
                }
                let expected = self
                    .expected
                    .get(path.as_str())
                    .ok_or_else(|| format!("untracked engine input: {path}"))?;
                let file = File::from(
                    unix::openat(
                        parent,
                        name,
                        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(error)?,
                );
                let first = file.metadata().map_err(error)?;
                if !first.is_file() || first.dev() != stat.st_dev || first.ino() != stat.st_ino {
                    return Err("nonregular or replaced engine input".into());
                }
                let bytes = read_at(parent, OsStr::new(name), MAX_FILE)?;
                let after = file.metadata().map_err(error)?;
                let current =
                    unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
                if !same_snapshot(&first, &after)
                    || current.st_dev != first.dev()
                    || current.st_ino != first.ino()
                    || (first.mode() & 0o111 != 0) != (expected.mode == "100755")
                    || hex(&git_hash("blob", &bytes)) != expected.blob
                {
                    return Err(format!("engine input differs from pinned tree: {path}"));
                }
                self.observed.push((path.clone(), first));
                if !self.seen.insert(path.clone()) {
                    return Err("duplicate engine scan entry".into());
                }
                if let Some(relative) = path.strip_prefix(STARTER) {
                    self.retained = self
                        .retained
                        .checked_add(bytes.len())
                        .ok_or("starter size overflow")?;
                    if self.retained > MAX_RETAINED || self.snapshots.len() >= 128 {
                        return Err("starter snapshot limit exceeded".into());
                    }
                    self.snapshots.insert(relative.into(), bytes);
                }
            }
        }
        if !same_snapshot(&before, &parent.metadata().map_err(error)?) {
            return Err("engine directory changed during scan".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
