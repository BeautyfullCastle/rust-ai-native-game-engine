use orr_package::{Project, Runtime};
use serde_json::json;
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
fn project(root: &Path) -> Project {
    let path = root.join("project");
    fs::create_dir(&path).unwrap();
    Project::open_for_install(path, Runtime::content_only().engine_version).unwrap()
}
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    bytes: Option<Vec<u8>>,
    modified: Option<String>,
    readonly: bool,
}
// Include empty directories and timestamps to catch cache/writer side effects.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let meta = fs::symlink_metadata(path).unwrap();
        assert!(!meta.file_type().is_symlink());
        out.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            Entry {
                bytes: meta.is_file().then(|| fs::read(path).unwrap()),
                modified: meta.modified().ok().map(|v| format!("{v:?}")),
                readonly: meta.permissions().readonly(),
            },
        );
        if meta.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                visit(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn error(root: &Path, project: &Project, direct: &[PathBuf], deps: &[PathBuf]) -> String {
    let before = snapshot(root);
    let error = project
        .preview_install_with_dependencies(direct, deps)
        .unwrap_err()
        .to_string();
    assert_eq!(snapshot(root), before);
    error
}
#[test]
fn cycle_closes_full_exact_version_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let a = source(&root, "a", "1.0.0-alpha.1+art", &[("b", "2.3.4")]);
    let b = source(&root, "b", "2.3.4", &[("c", "3.0.0")]);
    let c = source(&root, "c", "3.0.0", &[("a", "1.0.0-alpha.1+art")]);
    assert_eq!(
        error(&root, &project, &[a], &[c, b]),
        "dependency cycle: a@1.0.0-alpha.1+art -> b@2.3.4 -> c@3.0.0 -> a@1.0.0-alpha.1+art"
    );
}
#[test]
fn noncyclic_selecting_prefix_is_excluded() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let game = source(&root, "game", "1.0.0", &[("prefix", "1.0.0")]);
    let prefix = source(&root, "prefix", "1.0.0", &[("a", "2.0.0")]);
    let a = source(&root, "a", "2.0.0", &[("b", "3.0.0")]);
    let b = source(&root, "b", "3.0.0", &[("a", "2.0.0")]);
    assert_eq!(
        error(&root, &project, &[game], &[prefix, b, a]),
        "dependency cycle: a@2.0.0 -> b@3.0.0 -> a@2.0.0"
    );
}
#[test]
fn first_cycle_is_stable_across_source_order_permutations() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let game = source(&root, "game", "1.0.0", &[("z", "1.0.0"), ("a", "1.0.0")]);
    let other = source(&root, "other", "1.0.0", &[("z", "1.0.0")]);
    let a = source(&root, "a", "1.0.0", &[("b", "2.0.0")]);
    let b = source(&root, "b", "2.0.0", &[("a", "1.0.0")]);
    let z = source(&root, "z", "1.0.0", &[("zz", "1.0.0")]);
    let zz = source(&root, "zz", "1.0.0", &[("z", "1.0.0")]);
    let mut deps = vec![zz, z, b, a];
    for direct in [vec![game.clone(), other.clone()], vec![other, game]] {
        for _ in 0..deps.len() {
            deps.rotate_left(1);
            assert_eq!(
                error(&root, &project, &direct, &deps),
                "dependency cycle: a@1.0.0 -> b@2.0.0 -> a@1.0.0"
            );
        }
        deps.reverse();
    }
}
#[test]
fn completed_diamond_nodes_are_not_cycles_and_preview_matches_install() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let leaf = source(&root, "leaf", "3.0.0", &[]);
    let left = source(&root, "left", "1.0.0", &[("leaf", "3.0.0")]);
    let right = source(&root, "right", "2.0.0", &[("leaf", "3.0.0")]);
    let game = source(
        &root,
        "game",
        "1.0.0",
        &[("left", "1.0.0"), ("right", "2.0.0")],
    );
    let deps = [right, leaf, left];
    let before = snapshot(&root);
    let preview = project
        .preview_install_with_dependencies(std::slice::from_ref(&game), &deps)
        .unwrap();
    assert_eq!(snapshot(&root), before);
    assert_eq!(preview.candidate.packages.len(), 4);
    assert_eq!(
        project.install_with_dependencies(&[game], &deps).unwrap(),
        preview.candidate
    );
}
#[test]
fn missing_and_conflicting_back_edge_versions_keep_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let a = source(&root, "a", "1.0.0", &[("b", "2.0.0")]);
    assert_eq!(
        error(&root, &project, std::slice::from_ref(&a), &[]),
        "missing dependency/package: b@2.0.0"
    );
    let b = source(&root, "b", "2.0.0", &[("a", "9.0.0")]);
    assert_eq!(
        error(&root, &project, &[a], &[b]),
        "version conflict: a requires 9.0.0, available 1.0.0"
    );
}
#[test]
fn cli_cycle_dry_run_preserves_empty_and_installed_projects_with_stale_writer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = project(&root);
    let a = source(&root, "a", "1.0.0", &[("b", "2.0.0")]);
    let b = source(&root, "b", "2.0.0", &[("a", "1.0.0")]);
    for installed in [false, true] {
        if installed {
            let baseline = source(&root, "baseline", "1.0.0", &[]);
            project.install(&[baseline]).unwrap();
            fs::write(
                project.root().join(".orr/packages/writer.lock"),
                b"stale writer: preserve me\n",
            )
            .unwrap();
        }
        let before = snapshot(&root);
        let result = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .arg("install")
            .arg(project.root())
            .arg("--dry-run")
            .arg("--path")
            .arg(&a)
            .arg("--dependency-path")
            .arg(&b)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert_eq!(
            String::from_utf8(result.stderr).unwrap(),
            "orr_pkg: dependency cycle: a@1.0.0 -> b@2.0.0 -> a@1.0.0\n"
        );
        assert_eq!(snapshot(&root), before);
        if !installed {
            assert!(!project.root().join(".orr").exists());
            assert!(!project.root().join("orr.packages.lock.json").exists());
        }
    }
}
