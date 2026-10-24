use orr_package::{Manifest, Project, Runtime};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    source: PathBuf,
}
fn manifest(name: &str) -> Manifest {
    Manifest {
        schema: 1,
        name: name.into(),
        version: "1.0.0".into(),
        engine: "^0.0.1".into(),
        capabilities: Default::default(),
        dependencies: Default::default(),
        files: ["asset.txt".into()].into(),
    }
}
fn write_manifest(path: &Path, m: &Manifest) {
    fs::write(
        path.join("orr.package.json"),
        serde_json::to_vec(m).unwrap(),
    )
    .unwrap();
}
fn installer(path: &Path) -> Project {
    Project::open_for_install(path, Runtime::content_only().engine_version).unwrap()
}
fn fixture() -> Fixture {
    // Canonicalize only the trusted temp root, never caller-controlled inputs.
    let root = std::env::temp_dir();
    #[cfg(unix)]
    let root = fs::canonicalize(root).unwrap();
    let tmp = tempfile::tempdir_in(root).unwrap();
    let project = tmp.path().join("game");
    let source = tmp.path().join("art");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&source).unwrap();
    write_manifest(&source, &manifest("art"));
    fs::write(source.join("asset.txt"), b"original").unwrap();
    installer(&project)
        .install(std::slice::from_ref(&source))
        .unwrap();
    Fixture {
        _tmp: tmp,
        project,
        source,
    }
}
fn cli(f: &Fixture, json: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orr_pkg_diff"));
    c.arg(&f.project).arg("art").arg("--source").arg(&f.source);
    if json {
        c.arg("--json");
    }
    c.output().unwrap()
}
fn error(f: &Fixture) {
    let o = cli(f, true);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert!(o.stdout.is_empty());
    assert!(!o.stderr.is_empty());
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.strip_prefix(root).unwrap().to_owned(), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
#[test]
fn identical_cli_is_deterministic_and_equal() {
    let f = fixture();
    let a = cli(&f, true);
    let b = cli(&f, true);
    assert_eq!(a.status.code(), Some(0));
    assert!(a.stderr.is_empty());
    assert_eq!(a.stdout, b.stdout);
    let j: serde_json::Value = serde_json::from_slice(&a.stdout).unwrap();
    assert_eq!(j["status"], "same");
    assert_eq!(j["counts"]["unchanged"], 1);
    assert_eq!(j["files"], serde_json::json!([]));
    assert_eq!(cli(&f, false).stdout, cli(&f, false).stdout);
}
#[test]
fn exact_sorted_asset_changes_and_hashes() {
    let f = fixture();
    let mut m = manifest("art");
    m.version = "1.0.1".into();
    m.files = ["asset.txt".into(), "remove.txt".into(), "same.txt".into()].into();
    fs::write(f.source.join("remove.txt"), b"gone").unwrap();
    fs::write(f.source.join("same.txt"), b"same").unwrap();
    write_manifest(&f.source, &m);
    installer(&f.project)
        .install(std::slice::from_ref(&f.source))
        .unwrap();
    m.files.remove("remove.txt");
    m.files.insert("add.txt".into());
    fs::write(f.source.join("add.txt"), b"abc").unwrap();
    fs::write(f.source.join("asset.txt"), b"abc").unwrap();
    write_manifest(&f.source, &m);
    let o = cli(&f, true);
    assert_eq!(o.status.code(), Some(1));
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        j["counts"],
        serde_json::json!({"added":1,"removed":1,"changed":1,"unchanged":1})
    );
    let rows = j["files"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["add.txt", "asset.txt", "remove.txt"]
    );
    assert_eq!(rows[0]["before_sha256"], serde_json::Value::Null);
    assert_eq!(
        rows[0]["after_sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(rows[1]["change"], "changed");
    assert_eq!(rows[2]["after_sha256"], serde_json::Value::Null);
}
#[test]
fn metadata_differences_preserve_candidate_resolution_boundary() {
    let f = fixture();
    let mut m = manifest("art");
    m.version = "1.0.1".into();
    m.engine = "*".into();
    m.capabilities.insert("sprites".into());
    m.dependencies.insert("unavailable".into(), "1.0.0".into());
    write_manifest(&f.source, &m);
    let o = cli(&f, true);
    assert_eq!(o.status.code(), Some(1));
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["counts"]["unchanged"], 1);
    assert_eq!(j["after_manifest"], serde_json::to_value(&m).unwrap());
    assert!(installer(&f.project)
        .preview_install(std::slice::from_ref(&f.source))
        .is_err());
}
#[test]
fn strict_usage_names_and_help() {
    let f = fixture();
    for args in [
        vec![],
        vec!["--json"],
        vec!["a", "art", "--source"],
        vec!["a", "art", "--source", "b", "--json", "--json"],
        vec!["a", "art", "--source", "b", "--other"],
    ] {
        let o = Command::new(env!("CARGO_BIN_EXE_orr_pkg_diff"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert!(o.stdout.is_empty());
    }
    let o = Command::new(env!("CARGO_BIN_EXE_orr_pkg_diff"))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8(o.stdout).unwrap().contains("Usage:"));
    let mut m = manifest("other");
    write_manifest(&f.source, &m);
    error(&f);
    m = manifest("art");
    m.version = "bad".into();
    write_manifest(&f.source, &m);
    error(&f);
    let o = Command::new(env!("CARGO_BIN_EXE_orr_pkg_diff"))
        .arg(&f.project)
        .args(["missing", "--source"])
        .arg(&f.source)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
}
#[test]
fn installed_tamper_and_missing_rejected() {
    for mode in 0..3 {
        let f = fixture();
        let lock = installer(&f.project).list().unwrap();
        // Test-only mutation of owned installed fixture; production never constructs object paths.
        let root = f
            .project
            .join(".orr/packages/objects")
            .join(&lock.packages["art"].digest);
        match mode {
            0 => fs::write(root.join("asset.txt"), b"tamper").unwrap(),
            1 => fs::remove_file(root.join("asset.txt")).unwrap(),
            _ => fs::write(root.join("orr.package.json"), b"{}").unwrap(),
        }
        let before = snapshot(&f.project);
        error(&f);
        assert_eq!(before, snapshot(&f.project));
    }
}
#[test]
fn source_paths_missing_links_and_nonregular_rejected() {
    let f = fixture();
    fs::remove_file(f.source.join("asset.txt")).unwrap();
    error(&f);
    fs::create_dir(f.source.join("asset.txt")).unwrap();
    error(&f);
    fs::remove_dir(f.source.join("asset.txt")).unwrap();
    let mut m = manifest("art");
    m.files = ["../outside".into()].into();
    write_manifest(&f.source, &m);
    error(&f);
    #[cfg(unix)]
    {
        write_manifest(&f.source, &manifest("art"));
        std::os::unix::fs::symlink("../outside", f.source.join("asset.txt")).unwrap();
        error(&f);
    }
}
#[test]
fn inherited_small_metadata_and_sparse_size_limits() {
    let f = fixture();
    let mut m = manifest("art");
    m.files = (0..4097).map(|i| format!("a{i}")).collect();
    write_manifest(&f.source, &m);
    error(&f);
    write_manifest(&f.source, &manifest("art"));
    fs::OpenOptions::new()
        .write(true)
        .open(f.source.join("asset.txt"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    error(&f);
    fs::write(
        f.source.join("orr.package.json"),
        vec![b' '; 1024 * 1024 + 1],
    )
    .unwrap();
    error(&f);
}
#[test]
fn lock_identity_graph_and_engine_errors_preserve_inputs() {
    let f = fixture();
    let path = f.project.join("orr.packages.lock.json");
    let good = fs::read(&path).unwrap();
    for mutation in 0..3 {
        let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
        match mutation {
            0 => v["schema"] = serde_json::json!(99),
            1 => v["packages"]["art"]["digest"] = serde_json::json!("invalid"),
            _ => v["direct"]["missing"] = serde_json::json!("1.0.0"),
        }
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let before = snapshot(&f.project);
        error(&f);
        assert_eq!(before, snapshot(&f.project));
    }
    fs::write(&path, good).unwrap();
    let mut m = manifest("art");
    m.engine = ">=999.0.0".into();
    write_manifest(&f.source, &m);
    error(&f);
}
#[test]
fn canonical_file_order_and_case_collision() {
    let f = fixture();
    let mut m = manifest("art");
    m.files.insert("z.txt".into());
    fs::write(f.source.join("z.txt"), b"z").unwrap();
    write_manifest(&f.source, &m);
    let first = cli(&f, true);
    let mut v = serde_json::to_value(&m).unwrap();
    v["files"] = serde_json::json!(["z.txt", "asset.txt"]);
    fs::write(
        f.source.join("orr.package.json"),
        serde_json::to_vec_pretty(&v).unwrap(),
    )
    .unwrap();
    assert_eq!(first.stdout, cli(&f, true).stdout);
    m.files.insert("ASSET.txt".into());
    write_manifest(&f.source, &m);
    error(&f);
}
#[test]
fn cli_json_human_and_pre_emission_failure_contract() {
    let f = fixture();
    fs::write(f.source.join("asset.txt"), b"edited").unwrap();
    let j = cli(&f, true);
    assert_eq!(j.status.code(), Some(1));
    assert!(j.stderr.is_empty());
    serde_json::from_slice::<serde_json::Value>(&j.stdout).unwrap();
    let human = String::from_utf8(cli(&f, false).stdout).unwrap();
    assert!(human.contains("\"asset.txt\""));
    assert!(human.contains("Before manifest:"));
    fs::remove_file(f.source.join("orr.package.json")).unwrap();
    error(&f);
}
#[test]
fn reads_never_create_or_modify_project_or_source() {
    let f = fixture();
    let project = snapshot(&f.project);
    let source = snapshot(&f.source);
    assert_eq!(cli(&f, true).status.code(), Some(0));
    assert_eq!(project, snapshot(&f.project));
    assert_eq!(source, snapshot(&f.source));
    let empty = f._tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let before = snapshot(&empty);
    assert!(orr_package_diff::compare(&empty, "art", &f.source).is_err());
    assert_eq!(before, snapshot(&empty));
}
#[test]
fn author_edit_compare_then_separate_explicit_install() {
    let f = fixture();
    let installed = snapshot(&f.project);
    fs::write(f.source.join("asset.txt"), b"new authored content").unwrap();
    assert_eq!(cli(&f, true).status.code(), Some(1));
    assert_eq!(installed, snapshot(&f.project));
    assert!(installer(&f.project)
        .install(std::slice::from_ref(&f.source))
        .is_err());
    assert_eq!(installed, snapshot(&f.project));
    let mut updated = manifest("art");
    updated.version = "1.0.1".into();
    write_manifest(&f.source, &updated);
    installer(&f.project)
        .install(std::slice::from_ref(&f.source))
        .unwrap();
    assert_eq!(cli(&f, true).status.code(), Some(0));
}

#[test]
fn encode_overflow_and_flush_failures_are_not_success() {
    let f = fixture();
    let mut report = orr_package_diff::compare(&f.project, "art", &f.source).unwrap();
    report.package = "x".repeat(orr_package_diff::MAX_OUTPUT_BYTES);
    assert!(orr_package_diff::encode(&report, true).is_err());
    assert!(orr_package_diff::encode(&report, false).is_err());
    struct FlushFailure;
    impl std::io::Write for FlushFailure {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("flush failed"))
        }
    }
    assert!(orr_package_diff::emit(FlushFailure, b"complete report").is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn cli_failed_output_sink_returns_error() {
    let f = fixture();
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_orr_pkg_diff"))
        .arg(&f.project)
        .args(["art", "--source"])
        .arg(&f.source)
        .stdout(full)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!o.stderr.is_empty());
}

#[test]
fn corrupt_unselected_package_still_blocks_whole_project_comparison() {
    let f = fixture();
    let other = f._tmp.path().join("other");
    fs::create_dir(&other).unwrap();
    write_manifest(&other, &manifest("other"));
    fs::write(other.join("asset.txt"), b"other").unwrap();
    let lock = installer(&f.project).install(&[other]).unwrap();
    let installed = f
        .project
        .join(".orr/packages/objects")
        .join(&lock.packages["other"].digest)
        .join("asset.txt");
    fs::write(installed, b"tampered").unwrap();
    let before = snapshot(&f.project);
    error(&f);
    assert_eq!(before, snapshot(&f.project));
}
