use orr_package::{Lock, PackagePreview, Project, Runtime};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn source(root: &Path, name: &str, version: &str, deps: &[(&str, &str)]) -> PathBuf {
    let dir = root.join(format!("{name}-{version}"));
    fs::create_dir(&dir).unwrap();
    let dependencies: BTreeMap<_, _> = deps.iter().copied().collect();
    fs::write(
        dir.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":name,"version":version,"engine":"^0.0.1",
            "capabilities":[],"dependencies":dependencies,"files":["asset.txt"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(dir.join("asset.txt"), format!("{name}@{version}")).unwrap();
    dir
}

fn edit_manifest(source: &Path, edit: impl FnOnce(&mut Value)) {
    let path = source.join("orr.package.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn project(root: &Path) -> Project {
    let path = root.join("project");
    fs::create_dir(&path).unwrap();
    Project::open_for_install(path, Runtime::content_only().engine_version).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File(Vec<u8>),
    Link(PathBuf),
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    kind: EntryKind,
    modified: Option<String>,
    readonly: bool,
}

// Include empty directories, file contents, links, and modification timestamps.
// Checking only the lock would miss object staging and writer-guard side effects.
fn tree(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let directory = metadata.is_dir();
        let kind = if metadata.file_type().is_symlink() {
            EntryKind::Link(fs::read_link(path).unwrap())
        } else if directory {
            EntryKind::Directory
        } else {
            EntryKind::File(fs::read(path).unwrap())
        };
        out.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            Entry {
                kind,
                modified: metadata.modified().ok().map(|value| format!("{value:?}")),
                readonly: metadata.permissions().readonly(),
            },
        );
        if directory {
            for child in fs::read_dir(path).unwrap() {
                walk(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

fn unchanged<T>(root: &Path, operation: impl FnOnce() -> T) -> T {
    let before = tree(root);
    let result = operation();
    assert_eq!(tree(root), before, "preview modified a project/source entry");
    result
}

#[test]
fn install_preview_is_deterministic_read_only_and_matches_actual_install() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let leaf = source(&root, "leaf", "1.0.0", &[]);
    let left = source(&root, "left", "1.0.0", &[("leaf", "1.0.0")]);
    let right = source(&root, "right", "1.0.0", &[("leaf", "1.0.0")]);
    let game = source(
        &root,
        "game",
        "1.0.0",
        &[("left", "1.0.0"), ("right", "1.0.0")],
    );
    let other = source(&root, "other", "1.0.0", &[]);
    let unused = source(&root, "unused", "1.0.0", &[]);
    let first = unchanged(&root, || {
        project.preview_install_with_dependencies(
            &[game.clone(), other.clone()],
            &[right.clone(), unused.clone(), leaf.clone(), left.clone()],
        )
    })
    .unwrap();
    let second = unchanged(&root, || {
        project.preview_install_with_dependencies(
            &[other.clone(), game.clone()],
            &[left.clone(), leaf.clone(), unused, right.clone()],
        )
    })
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    let encoded = serde_json::to_vec(&first).unwrap();
    assert_eq!(
        serde_json::from_slice::<PackagePreview>(&encoded).unwrap(),
        first
    );
    assert_eq!(first.previous, Lock::default());
    assert_eq!(first.changes.keys().map(String::as_str).collect::<Vec<_>>(),
        ["game", "leaf", "left", "other", "right"]);
    assert!(!first.candidate.packages.contains_key("unused"));
    for (name, change) in &first.changes {
        assert!(change.before.is_none());
        let after = change.after.as_ref().unwrap();
        assert_eq!(after.version, "1.0.0");
        assert_eq!(after.digest, first.candidate.packages[name].digest);
        assert_eq!(after.direct, name == "game" || name == "other");
    }
    assert!(!project.root().join(".orr").exists());
    assert!(!project.root().join("orr.packages.lock.json").exists());
    assert_eq!(
        project.install_with_dependencies(&[game, other], &[right, leaf, left]).unwrap(),
        first.candidate
    );
}

#[test]
fn noop_and_version_update_previews_match_checked_install() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let old = source(&root, "art", "1.0.0", &[]);
    let next = source(&root, "art", "2.0.0", &[]);
    let installed = project.install(std::slice::from_ref(&old)).unwrap();
    let noop = unchanged(&root, || project.preview_install(&[old])).unwrap();
    assert!(noop.changes.is_empty());
    assert_eq!(noop.previous, installed);
    assert_eq!(noop.candidate, installed);
    let preview = unchanged(&root, || project.preview_install(std::slice::from_ref(&next))).unwrap();
    assert_eq!(preview.previous, installed);
    assert_eq!(preview.changes.len(), 1);
    let change = &preview.changes["art"];
    let before = change.before.as_ref().unwrap();
    let after = change.after.as_ref().unwrap();
    assert_eq!(before.version, "1.0.0");
    assert_eq!(after.version, "2.0.0");
    assert!(before.direct && after.direct);
    assert_eq!(before.digest, installed.packages["art"].digest);
    assert_eq!(after.digest, preview.candidate.packages["art"].digest);
    assert_ne!(before.digest, after.digest);
    assert_eq!(
        project.install_checked(&[next], &preview.previous, &preview.candidate, || Ok(())).unwrap(),
        preview.candidate
    );
}

#[test]
fn directness_changes_and_transitive_removal_are_reported_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let leaf = source(&root, "leaf", "1.0.0", &[]);
    let game = source(&root, "game", "1.0.0", &[("leaf", "1.0.0")]);
    project.install_with_dependencies(&[game], std::slice::from_ref(&leaf)).unwrap();
    assert!(unchanged(&root, || project.preview_remove("leaf"))
        .unwrap_err().to_string().contains("not a direct package"));
    let promotion = unchanged(&root, || project.preview_install(std::slice::from_ref(&leaf))).unwrap();
    assert_eq!(promotion.changes.len(), 1);
    let change = &promotion.changes["leaf"];
    assert!(!change.before.as_ref().unwrap().direct);
    assert!(change.after.as_ref().unwrap().direct);
    assert_eq!(change.before.as_ref().unwrap().digest, change.after.as_ref().unwrap().digest);
    assert_eq!(project.install(&[leaf]).unwrap(), promotion.candidate);

    let demotion = unchanged(&root, || project.preview_remove("leaf")).unwrap();
    assert_eq!(demotion.changes.len(), 1);
    assert!(demotion.changes["leaf"].before.as_ref().unwrap().direct);
    assert!(!demotion.changes["leaf"].after.as_ref().unwrap().direct);
    assert_eq!(demotion.previous.packages, demotion.candidate.packages);
    assert_eq!(project.remove("leaf").unwrap(), demotion.candidate);

    let removal = unchanged(&root, || project.preview_remove("game")).unwrap();
    assert_eq!(removal.changes.keys().map(String::as_str).collect::<Vec<_>>(), ["game", "leaf"]);
    for change in removal.changes.values() {
        assert!(change.before.is_some());
        assert!(change.after.is_none());
    }
    assert_eq!(removal.candidate, Lock::default());
    assert_eq!(project.remove("game").unwrap(), removal.candidate);
}

#[test]
fn previews_do_not_acquire_replace_or_remove_an_existing_writer_guard() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    let extra = source(&root, "extra", "1.0.0", &[]);
    project.install(&[art]).unwrap();
    let guard = project.root().join(".orr/packages/writer.lock");
    fs::write(&guard, b"owned by another writer\n").unwrap();
    unchanged(&root, || project.preview_install(&[extra])).unwrap();
    unchanged(&root, || project.preview_remove("art")).unwrap();
    assert!(unchanged(&root, || project.preview_remove("absent")).is_err());
    assert!(unchanged(&root, || project.preview_install(&[])).is_err());
    assert_eq!(fs::read(guard).unwrap(), b"owned by another writer\n");
}

#[test]
fn previews_preserve_host_capability_checks_and_metadata_only_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let metadata_tool = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    edit_manifest(&art, |manifest| manifest["capabilities"] = json!(["sprite"]));
    let host = Project::open(metadata_tool.root(), Runtime::content_only()).unwrap();
    assert!(unchanged(&root, || host.preview_install(std::slice::from_ref(&art)))
        .unwrap_err().to_string().contains("missing compiled capability"));
    let preview = unchanged(&root, || {
        metadata_tool.preview_install(std::slice::from_ref(&art))
    }).unwrap();
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    let enabled_host = Project::open(metadata_tool.root(), runtime).unwrap();
    assert_eq!(
        unchanged(&root, || enabled_host.preview_install(std::slice::from_ref(&art))).unwrap(),
        preview
    );
    metadata_tool.install(&[art]).unwrap();
    assert!(unchanged(&root, || host.preview_remove("art")).is_err());
    assert_eq!(
        unchanged(&root, || enabled_host.preview_remove("art")).unwrap().candidate,
        Lock::default()
    );
    assert!(metadata_tool.read_asset("art", "asset.txt").is_err());
}

#[test]
fn conflicting_missing_duplicate_and_cyclic_sources_fail_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let leaf = source(&root, "leaf", "1.0.0", &[]);
    let leaf_next = source(&root, "leaf", "2.0.0", &[]);
    let game = source(&root, "game", "1.0.0", &[("leaf", "1.0.0")]);
    assert!(unchanged(&root, || project.preview_install(std::slice::from_ref(&game)))
        .unwrap_err().to_string().contains("missing dependency/package"));
    assert!(unchanged(&root, || {
        project.preview_install_with_dependencies(std::slice::from_ref(&game), &[leaf_next])
    })
        .unwrap_err().to_string().contains("version conflict"));
    assert!(unchanged(&root, || {
        project.preview_install_with_dependencies(
            std::slice::from_ref(&leaf),
            std::slice::from_ref(&leaf),
        )
    })
        .unwrap_err().to_string().contains("duplicate package name"));
    let a = source(&root, "cycle-a", "1.0.0", &[("cycle-b", "1.0.0")]);
    let b = source(&root, "cycle-b", "1.0.0", &[("cycle-a", "1.0.0")]);
    assert!(unchanged(&root, || project.preview_install_with_dependencies(&[a], &[b]))
        .unwrap_err().to_string().contains("dependency cycle"));
    assert!(!project.root().join(".orr").exists());

    project.install_with_dependencies(&[game], std::slice::from_ref(&leaf)).unwrap();
    fs::write(leaf.join("asset.txt"), b"changed without a version bump").unwrap();
    assert!(unchanged(&root, || project.preview_install(&[leaf]))
        .unwrap_err().to_string().contains("same name/version has different content"));
}

#[test]
fn installed_byte_tampering_blocks_install_and_remove_previews() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    let extra = source(&root, "extra", "1.0.0", &[]);
    let lock = project.install(&[art]).unwrap();
    let object = project.root().join(".orr/packages/objects").join(&lock.packages["art"].digest);
    fs::write(object.join("asset.txt"), b"tampered").unwrap();
    assert!(unchanged(&root, || project.preview_install(std::slice::from_ref(&extra)))
        .unwrap_err().to_string().contains("installed content changed"));
    assert!(unchanged(&root, || project.preview_remove("art"))
        .unwrap_err().to_string().contains("installed content changed"));
    assert_eq!(project.list().unwrap(), lock);

    fs::write(object.join("asset.txt"), b"art@1.0.0").unwrap();
    edit_manifest(&object, |manifest| manifest["version"] = json!("2.0.0"));
    assert!(unchanged(&root, || project.preview_install(&[extra]))
        .unwrap_err().to_string().contains("installed manifest changed"));
    assert!(unchanged(&root, || project.preview_remove("art")).is_err());
}

#[test]
fn malformed_lock_and_bounded_input_errors_leave_every_entry_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    for name in ["", "../secret", "line\nbreak", &"x".repeat(4096)] {
        assert_eq!(unchanged(&root, || project.preview_remove(name)).unwrap_err().to_string(),
            "invalid package/capability name");
    }
    assert!(unchanged(&root, || project.preview_install(&[])).is_err());
    assert!(unchanged(&root, || project.preview_install(&vec![art.clone(); 129])).is_err());
    assert!(unchanged(&root, || {
        project.preview_install_with_dependencies(
            std::slice::from_ref(&art),
            &vec![art.clone(); 128],
        )
    }).is_err());
    edit_manifest(&art, |manifest| {
        manifest["files"] = json!((0..4097).map(|i| format!("f{i}")).collect::<Vec<_>>());
    });
    assert!(unchanged(&root, || project.preview_install(std::slice::from_ref(&art)))
        .unwrap_err().to_string().contains("bounds"));
    fs::write(art.join("orr.package.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(unchanged(&root, || project.preview_install(std::slice::from_ref(&art))).is_err());
    assert!(!project.root().join(".orr").exists());

    fs::write(project.root().join("orr.packages.lock.json"), b"{broken").unwrap();
    assert!(unchanged(&root, || project.preview_install(&[art])).is_err());
    assert!(unchanged(&root, || project.preview_remove("art")).is_err());
    fs::write(project.root().join("orr.packages.lock.json"),
        br#"{"schema":1,"direct":{"absent":"1.0.0"},"packages":{}}"#).unwrap();
    assert!(unchanged(&root, || project.preview_remove("absent")).is_err());
    assert!(!project.root().join(".orr").exists());
}

#[cfg(unix)]
#[test]
fn preview_rejects_symlinked_source_assets_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    let target = root.join("outside.txt");
    fs::write(&target, b"outside source").unwrap();
    fs::remove_file(art.join("asset.txt")).unwrap();
    symlink(&target, art.join("asset.txt")).unwrap();
    assert!(unchanged(&root, || project.preview_install(&[art]))
        .unwrap_err().to_string().contains("symlink rejected"));
    assert!(!project.root().join(".orr").exists());
}

#[test]
fn checked_apply_rejects_sources_changed_after_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    let preview = unchanged(&root, || project.preview_install(std::slice::from_ref(&art))).unwrap();
    fs::write(art.join("asset.txt"), b"changed after preview").unwrap();
    let mut callback_ran = false;
    let error = project.install_checked(&[art], &preview.previous, &preview.candidate, || {
        callback_ran = true;
        Ok(())
    }).unwrap_err();
    assert!(error.to_string().contains("candidate package lock differs"));
    assert!(!callback_ran);
    assert_eq!(project.list().unwrap(), preview.previous);
    assert!(!project.root().join("orr.packages.lock.json").exists());
    assert!(!project.root().join(".orr/packages/objects").exists());
    assert!(!project.root().join(".orr/packages/writer.lock").exists());
}

#[test]
fn checked_apply_rejects_active_lock_changed_after_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let art = source(&root, "art", "1.0.0", &[]);
    let extra = source(&root, "extra", "1.0.0", &[]);
    let preview = unchanged(&root, || project.preview_install(std::slice::from_ref(&art))).unwrap();
    let intervening = project.install(&[extra]).unwrap();
    let before = fs::read(project.root().join("orr.packages.lock.json")).unwrap();
    let mut callback_ran = false;
    let error = project.install_checked(&[art], &preview.previous, &preview.candidate, || {
        callback_ran = true;
        Ok(())
    }).unwrap_err();
    assert!(error.to_string().contains("active package lock changed"));
    assert!(!callback_ran);
    assert_eq!(project.verify().unwrap(), intervening);
    assert_eq!(fs::read(project.root().join("orr.packages.lock.json")).unwrap(), before);
    assert!(!project.root().join(".orr/packages/writer.lock").exists());
}

#[test]
fn cli_dry_run_emits_preview_json_and_leaves_project_and_sources_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let leaf = source(&root, "leaf", "1.0.0", &[]);
    let game = source(&root, "game", "1.0.0", &[("leaf", "1.0.0")]);
    let expected = unchanged(&root, || {
        project.preview_install_with_dependencies(
            std::slice::from_ref(&game),
            std::slice::from_ref(&leaf),
        )
    }).unwrap();
    let output = unchanged(&root, || Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg("install").arg(project.root()).arg("--dry-run")
        .arg("--path").arg(&game).arg("--dependency-path").arg(&leaf)
        .output().unwrap());
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<PackagePreview>(&output.stdout).unwrap(), expected);
    assert!(String::from_utf8_lossy(&output.stderr)
        .to_ascii_lowercase().contains("compiled capability"));
    assert!(!project.root().join(".orr").exists());
    project.install_with_dependencies(&[game], &[leaf]).unwrap();
    let expected = unchanged(&root, || project.preview_remove("game")).unwrap();
    fs::write(project.root().join(".orr/packages/writer.lock"), b"preserve guard").unwrap();
    let output = unchanged(&root, || Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg("remove").arg(project.root()).args(["game", "--dry-run"])
        .output().unwrap());
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<PackagePreview>(&output.stdout).unwrap(), expected);
}

#[test]
fn cli_dry_run_errors_have_no_json_or_filesystem_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let game = source(&root, "game", "1.0.0", &[("absent", "1.0.0")]);
    let missing = root.join("missing-source");
    for source in [&game, &missing] {
        let output = unchanged(&root, || Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .arg("install").arg(project.root()).arg("--dry-run").arg("--path").arg(source)
            .output().unwrap());
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let output = unchanged(&root, || Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg("remove").arg(project.root()).args(["absent", "--dry-run"])
        .output().unwrap());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert!(!project.root().join(".orr").exists());
    assert!(!project.root().join("orr.packages.lock.json").exists());
}
