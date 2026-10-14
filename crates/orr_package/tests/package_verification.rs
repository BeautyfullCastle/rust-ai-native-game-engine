use orr_package::{Lock, LockedPackage, Manifest, PackageVerification, Project, Runtime};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const LOCK: &str = "orr.packages.lock.json";
const MANIFEST: &str = "orr.package.json";

struct Fixture {
    root: tempfile::TempDir,
    project_dir: PathBuf,
    project: Project,
}

impl Fixture {
    fn new() -> Self {
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = tempfile::tempdir_in(temp).unwrap();
        let project_dir = root.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project =
            Project::open_for_install(&project_dir, Runtime::content_only().engine_version)
                .unwrap();
        Self {
            root,
            project_dir,
            project,
        }
    }

    fn source(&self, name: &str, deps: &[(&str, &str)]) -> PathBuf {
        let path = self.root.path().join(name);
        fs::create_dir(&path).unwrap();
        let dependencies: BTreeMap<_, _> = deps.iter().copied().collect();
        fs::write(
            path.join(MANIFEST),
            serde_json::to_vec(&json!({
                "schema":1,"name":name,"version":"1.0.0","engine":"^0.0.1",
                "capabilities":[],"dependencies":dependencies,"files":["asset.txt"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(path.join("asset.txt"), format!("{name} bytes")).unwrap();
        path
    }

    fn graph(&self) -> Lock {
        let art = self.source("art", &[("left", "1.0.0"), ("right", "1.0.0")]);
        let left = self.source("left", &[("shared", "1.0.0")]);
        let right = self.source("right", &[("shared", "1.0.0")]);
        let shared = self.source("shared", &[]);
        let other = self.source("other", &[]);
        self.project
            .install_with_dependencies(&[art, other], &[left, right, shared])
            .unwrap()
    }

    fn object(&self, lock: &Lock, name: &str) -> PathBuf {
        self.project_dir
            .join(".orr/packages/objects")
            .join(&lock.packages[name].digest)
    }

    fn cli(&self, tail: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .arg("verify")
            .arg(&self.project_dir)
            .args(tail)
            .output()
            .unwrap()
    }
}

fn report(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(warning.contains("Scoped verification"));
    assert!(warning.contains("unverified_packages"));
    assert!(warning.contains("not whole-project"));
    assert!(warning.contains("compiled capability validation is deferred"));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rejected(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "failure printed success JSON");
    String::from_utf8(output.stderr).unwrap()
}

// Read only regular files, so this preservation oracle cannot hang on a FIFO or
// follow a fixture symlink. Hash incrementally rather than retaining large files.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (String, u64, bool)> {
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, (String, u64, bool)>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let key = path.strip_prefix(root).unwrap().to_owned();
        let readonly = metadata.permissions().readonly();
        if metadata.file_type().is_symlink() {
            result.insert(
                key,
                (
                    format!("link:{}", fs::read_link(path).unwrap().display()),
                    0,
                    readonly,
                ),
            );
        } else if metadata.is_dir() {
            result.insert(key, ("directory".into(), 0, readonly));
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), result);
            }
        } else if metadata.is_file() {
            let mut file = fs::File::open(path).unwrap();
            let mut hash = Sha256::new();
            let mut buffer = [0; 8192];
            loop {
                let count = file.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            result.insert(
                key,
                (
                    hash.finalize()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                    metadata.len(),
                    readonly,
                ),
            );
        } else {
            result.insert(key, ("nonregular".into(), metadata.len(), readonly));
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

fn reseal(package: &mut LockedPackage) {
    let bytes = serde_json::to_vec(&("orr-package-v1", &package.manifest, &package.files)).unwrap();
    package.digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
}

#[test]
fn cli_and_api_report_exact_deterministic_dependency_closure() {
    let f = Fixture::new();
    let lock = f.graph();
    let before = snapshot(f.root.path());
    let actual = f.project.verify_package("art").unwrap();
    assert_eq!(actual.requested_package, "art");
    assert_eq!(
        actual
            .verified_packages
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["art", "left", "right", "shared"]
    );
    assert_eq!(
        actual
            .unverified_packages
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["other"]
    );
    for (name, selection) in actual
        .verified_packages
        .iter()
        .chain(&actual.unverified_packages)
    {
        assert_eq!(selection.version, lock.packages[name].manifest.version);
        assert_eq!(selection.digest, lock.packages[name].digest);
        assert_eq!(selection.direct, lock.direct.contains_key(name));
    }
    let first = f.cli(&["--package", "art"]);
    let second = f.cli(&["--package", "art"]);
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(report(first), serde_json::to_value(&actual).unwrap());
    assert_eq!(report(second), serde_json::to_value(&actual).unwrap());
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn transitive_selection_excludes_its_dependents_and_other_roots() {
    let f = Fixture::new();
    f.graph();
    let actual = f.project.verify_package("left").unwrap();
    assert_eq!(
        actual
            .verified_packages
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["left", "shared"]
    );
    assert_eq!(
        actual
            .unverified_packages
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["art", "other", "right"]
    );
    assert!(!actual.verified_packages["left"].direct);
    let leaf = f.project.verify_package("shared").unwrap();
    assert_eq!(leaf.verified_packages.len(), 1);
    assert_eq!(leaf.unverified_packages.len(), 4);
}

#[test]
fn unrelated_corrupt_payload_is_explicitly_unverified_and_full_verify_still_fails() {
    let f = Fixture::new();
    let lock = f.graph();
    fs::write(f.object(&lock, "other").join("asset.txt"), b"corrupt").unwrap();
    let before = snapshot(f.root.path());
    let scoped = report(f.cli(&["--package", "art"]));
    assert_eq!(
        scoped["unverified_packages"]["other"]["digest"],
        lock.packages["other"].digest
    );
    assert!(f.project.verify().is_err());
    assert!(rejected(f.cli(&[])).contains("installed content changed"));
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn unrelated_missing_object_and_manifest_are_not_read() {
    let f = Fixture::new();
    let lock = f.graph();
    let object = f.object(&lock, "other");
    fs::remove_file(object.join(MANIFEST)).unwrap();
    assert!(f.project.verify_package("art").is_ok());
    assert!(f.project.verify().is_err());
    fs::remove_dir_all(object).unwrap();
    let before = snapshot(f.root.path());
    assert!(f.project.verify_package("art").is_ok());
    assert!(f.project.verify().is_err());
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn selected_root_and_dependency_corruption_fail_without_success_json() {
    for name in ["art", "left", "shared"] {
        let f = Fixture::new();
        let lock = f.graph();
        fs::write(f.object(&lock, name).join("asset.txt"), b"changed").unwrap();
        let before = snapshot(f.root.path());
        let error = rejected(f.cli(&["--package", "art"]));
        assert!(
            error.contains(&format!("package {name}: installed content changed")),
            "{error}"
        );
        assert!(f.project.verify_package("art").is_err());
        assert_eq!(snapshot(f.root.path()), before);
    }
}

#[test]
fn selected_missing_content_and_changed_manifest_fail() {
    for target in [MANIFEST, "asset.txt"] {
        let f = Fixture::new();
        let lock = f.graph();
        fs::remove_file(f.object(&lock, "shared").join(target)).unwrap();
        let before = snapshot(f.root.path());
        assert!(rejected(f.cli(&["--package", "art"])).contains("package shared:"));
        assert_eq!(snapshot(f.root.path()), before);
    }
    let f = Fixture::new();
    let lock = f.graph();
    let mut changed = lock.packages["art"].manifest.clone();
    changed.version = "2.0.0".into();
    fs::write(
        f.object(&lock, "art").join(MANIFEST),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let before = snapshot(f.root.path());
    assert!(rejected(f.cli(&["--package", "art"])).contains("installed manifest changed"));
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn unrelated_invalid_lock_identity_graph_and_engine_still_reject() {
    let f = Fixture::new();
    let original = f.graph();
    for variant in [
        "identity",
        "unreachable",
        "cycle",
        "missing",
        "version",
        "engine",
        "path",
    ] {
        let mut lock = original.clone();
        let other = lock.packages.get_mut("other").unwrap();
        match variant {
            "identity" => other.digest = "0".repeat(64),
            "unreachable" => {
                lock.direct.remove("other");
            }
            "cycle" => {
                other
                    .manifest
                    .dependencies
                    .insert("other".into(), "1.0.0".into());
                reseal(other);
            }
            "missing" => {
                other
                    .manifest
                    .dependencies
                    .insert("absent".into(), "1.0.0".into());
                reseal(other);
            }
            "version" => {
                other
                    .manifest
                    .dependencies
                    .insert("shared".into(), "2.0.0".into());
                reseal(other);
            }
            "engine" => {
                other.manifest.engine = "^99.0.0".into();
                reseal(other);
            }
            "path" => {
                other.manifest.files = ["../escape".into()].into();
                other.files = [("../escape".into(), "0".repeat(64))].into();
                reseal(other);
            }
            _ => unreachable!(),
        }
        fs::write(f.project_dir.join(LOCK), serde_json::to_vec(&lock).unwrap()).unwrap();
        let before = snapshot(f.root.path());
        assert!(
            f.project.verify_package("art").is_err(),
            "variant {variant}"
        );
        rejected(f.cli(&["--package", "art"]));
        assert_eq!(snapshot(f.root.path()), before, "variant {variant}");
    }
}

#[test]
fn malformed_unknown_and_oversized_locks_reject_without_writes() {
    let f = Fixture::new();
    f.graph();
    for bytes in [
        b"{".to_vec(),
        br#"{"schema":1,"direct":{},"packages":{},"unknown":true}"#.to_vec(),
        vec![b' '; 1024 * 1024 + 1],
    ] {
        fs::write(f.project_dir.join(LOCK), bytes).unwrap();
        let before = snapshot(f.root.path());
        rejected(f.cli(&["--package", "art"]));
        assert_eq!(snapshot(f.root.path()), before);
    }
}

#[test]
fn host_capability_admission_covers_unrelated_lock_entries() {
    let f = Fixture::new();
    let art = f.source("art", &[]);
    let other = f.source("other", &[]);
    let path = other.join(MANIFEST);
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest.capabilities.insert("sprite".into());
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    f.project.install(&[art, other]).unwrap();
    let before = snapshot(f.root.path());
    let host = Project::open(&f.project_dir, Runtime::content_only()).unwrap();
    assert!(host.verify_package("art").is_err());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    let supported = Project::open(&f.project_dir, runtime).unwrap();
    assert!(supported.verify_package("art").is_ok());
    report(f.cli(&["--package", "art"]));
    assert!(f.project.read_asset("art", "asset.txt").is_err());
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn no_writer_is_created_or_stolen_and_sources_are_not_required() {
    let f = Fixture::new();
    f.graph();
    for name in ["art", "left", "right", "shared", "other"] {
        fs::remove_dir_all(f.root.path().join(name)).unwrap();
    }
    let guard = f.project_dir.join(".orr/packages/writer.lock");
    fs::write(&guard, b"owned by another operation\n").unwrap();
    let before = snapshot(f.root.path());
    report(f.cli(&["--package", "art"]));
    assert!(f.project.verify_package("shared").is_ok());
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn missing_package_and_invalid_requests_leave_fresh_project_untouched() {
    let f = Fixture::new();
    let before = snapshot(f.root.path());
    for name in ["missing", "", "../escape", "Bad", "a/b"] {
        assert!(f.project.verify_package(name).is_err());
        rejected(f.cli(&["--package", name]));
    }
    assert_eq!(snapshot(f.root.path()), before);
    assert!(!f.project_dir.join(".orr").exists());
}

#[test]
fn strict_cli_arguments_preserve_existing_full_verify_output() {
    let f = Fixture::new();
    let lock = f.graph();
    let before = snapshot(f.root.path());
    for args in [
        vec!["--package"],
        vec!["art"],
        vec!["--unknown", "art"],
        vec!["--package", "art", "extra"],
        vec!["--package", "art", "--package", "other"],
    ] {
        rejected(f.cli(&args));
    }
    let full = f.cli(&[]);
    assert!(full.status.success());
    assert_eq!(serde_json::from_slice::<Lock>(&full.stdout).unwrap(), lock);
    assert!(!String::from_utf8(full.stderr)
        .unwrap()
        .contains("Scoped verification"));
    assert_eq!(f.project.verify().unwrap(), lock);
    let help = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout)
        .unwrap()
        .contains("verify PROJECT [--package NAME]"));
    assert_eq!(snapshot(f.root.path()), before);
}

#[test]
fn oversize_selected_manifest_and_asset_keep_existing_bounds() {
    for (file, size) in [
        (MANIFEST, 1024 * 1024 + 1),
        ("asset.txt", 64 * 1024 * 1024 + 1),
    ] {
        let f = Fixture::new();
        let lock = f.graph();
        let target = f.object(&lock, "shared").join(file);
        fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap()
            .set_len(size)
            .unwrap();
        let before = snapshot(f.root.path());
        let error = rejected(f.cli(&["--package", "art"]));
        assert!(
            error.contains("regular file required within size bound"),
            "{error}"
        );
        assert_eq!(snapshot(f.root.path()), before);
    }
}

#[test]
fn complete_scope_is_reported_without_changing_whole_project_api() {
    let f = Fixture::new();
    let root = f.source("art", &[]);
    f.project.install(&[root]).unwrap();
    let report: PackageVerification = f.project.verify_package("art").unwrap();
    assert_eq!(report.verified_packages.len(), 1);
    assert!(report.unverified_packages.is_empty());
    assert!(f.project.verify().is_ok());
}

#[cfg(unix)]
#[test]
fn selected_object_asset_and_lock_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    for target in ["object", "asset", "lock"] {
        let f = Fixture::new();
        let lock = f.graph();
        let path = match target {
            "object" => f.object(&lock, "shared"),
            "asset" => f.object(&lock, "shared").join("asset.txt"),
            _ => f.project_dir.join(LOCK),
        };
        let saved = f.root.path().join("saved");
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let before = snapshot(f.root.path());
        assert!(rejected(f.cli(&["--package", "art"])).contains("symlink"));
        assert_eq!(snapshot(f.root.path()), before);
    }
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "bounded subprocess deadline for a filesystem regression"
)]
fn fifo_selected_inputs_reject_in_bounded_cli_processes() {
    use std::time::{Duration, Instant};
    for target in ["manifest", "asset", "lock"] {
        let f = Fixture::new();
        let lock = f.graph();
        let path = match target {
            "manifest" => f.object(&lock, "shared").join(MANIFEST),
            "asset" => f.object(&lock, "shared").join("asset.txt"),
            _ => f.project_dir.join(LOCK),
        };
        fs::remove_file(&path).unwrap();
        assert!(Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        let before = snapshot(f.root.path());
        let stdout = tempfile::tempfile().unwrap();
        let stderr = tempfile::tempfile().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .arg("verify")
            .arg(&f.project_dir)
            .args(["--package", "art"])
            .stdout(stdout.try_clone().unwrap())
            .stderr(stderr.try_clone().unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("scoped verification blocked on {target} FIFO");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        use std::io::{Seek, SeekFrom};
        let mut stdout = stdout;
        let mut stderr = stderr;
        stdout.seek(SeekFrom::Start(0)).unwrap();
        stderr.seek(SeekFrom::Start(0)).unwrap();
        let mut out = Vec::new();
        let mut err = String::new();
        stdout.read_to_end(&mut out).unwrap();
        stderr.read_to_string(&mut err).unwrap();
        assert!(!status.success());
        assert!(out.is_empty());
        assert!(
            err.contains("regular file required within size bound"),
            "{err}"
        );
        assert_eq!(snapshot(f.root.path()), before);
    }
}
