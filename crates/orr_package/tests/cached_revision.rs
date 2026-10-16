use orr_package::{Lock, PackagePreview, Project, Runtime};
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
        let tmp = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = tempfile::tempdir_in(tmp).unwrap();
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

    fn source(&self, directory: &str, name: &str, version: &str, deps: &[(&str, &str)]) -> PathBuf {
        let path = self.root.path().join(directory);
        fs::create_dir(&path).unwrap();
        let dependencies: BTreeMap<_, _> = deps.iter().copied().collect();
        fs::write(
            path.join(MANIFEST),
            serde_json::to_vec(&json!({
                "schema":1, "name":name, "version":version, "engine":"^0.0.1",
                "capabilities":[], "dependencies":dependencies, "files":["asset.txt"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(path.join("asset.txt"), format!("{name}@{version}")).unwrap();
        path
    }

    fn object(&self, digest: &str) -> PathBuf {
        self.project_dir.join(".orr/packages/objects").join(digest)
    }

    fn revisions(&self) -> (PathBuf, PathBuf, String) {
        let first = self.source("v1", "art", "1.0.0", &[]);
        let old = self.project.install(std::slice::from_ref(&first)).unwrap();
        let second = self.source("v2", "art", "2.0.0", &[]);
        self.project.install(std::slice::from_ref(&second)).unwrap();
        (first, second, old.packages["art"].digest.clone())
    }

    fn unchanged<T>(&self, operation: impl FnOnce() -> T) -> T {
        let before = tree(self.root.path());
        let result = operation();
        assert_eq!(tree(self.root.path()), before);
        result
    }

    fn rejects(&self, name: &str, digest: &str) {
        self.unchanged(|| {
            assert!(self.project.preview_restore_cached(name, digest).is_err());
            assert!(self.project.restore_cached(name, digest).is_err());
        });
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory(u32),
    File(u64, String, u32),
    Link(PathBuf),
    Other(u32),
}

fn permissions(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Entry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let mode = permissions(&metadata);
        let entry = if metadata.file_type().is_symlink() {
            Entry::Link(fs::read_link(path).unwrap())
        } else if metadata.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                walk(root, &child.unwrap().path(), result);
            }
            Entry::Directory(mode)
        } else if metadata.is_file() {
            let mut input = fs::File::open(path).unwrap();
            let mut hash = Sha256::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let read = input.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
            }
            let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
            Entry::File(metadata.len(), digest, mode)
        } else {
            Entry::Other(mode)
        };
        result.insert(path.strip_prefix(root).unwrap().to_path_buf(), entry);
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

fn cli(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg(args[0])
        .arg(project)
        .args(&args[1..])
        .output()
        .unwrap()
}

fn cli_lock(output: Output) -> Lock {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn edit_manifest(path: &Path, edit: impl FnOnce(&mut Value)) {
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(path.join(MANIFEST)).unwrap()).unwrap();
    edit(&mut manifest);
    fs::write(path.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn cli_restores_exact_revision_after_original_sources_are_removed() {
    let f = Fixture::new();
    let first = f.source("v1", "art", "1.0.0", &[]);
    let old = cli_lock(cli(
        &f.project_dir,
        &["install", "--path", first.to_str().unwrap()],
    ));
    let digest = &old.packages["art"].digest;
    let second = f.source("v2", "art", "2.0.0", &[]);
    let current = cli_lock(cli(
        &f.project_dir,
        &["install", "--path", second.to_str().unwrap()],
    ));
    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
    let output = f.unchanged(|| cli(&f.project_dir, &["restore", "art", digest, "--dry-run"]));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("no activation or writer reservation"));
    let preview: PackagePreview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview.previous, current);
    assert_eq!(preview.candidate, old);
    let restored = cli_lock(cli(&f.project_dir, &["restore", "art", digest]));
    assert_eq!(restored, preview.candidate);
    assert_eq!(cli_lock(cli(&f.project_dir, &["verify"])), old);
    let host = Project::open(&f.project_dir, Runtime::content_only()).unwrap();
    assert_eq!(host.read_asset("art", "asset.txt").unwrap(), b"art@1.0.0");
    assert!(f.project.read_asset("art", "asset.txt").is_err());
}

#[test]
fn removed_objects_restore_repeat_and_do_not_require_cache_enumeration() {
    let f = Fixture::new();
    let source = f.source("source", "art", "1.0.0", &[]);
    let old = f.project.install(&[source]).unwrap();
    let digest = &old.packages["art"].digest;
    f.project.remove("art").unwrap();
    // An unrelated incomplete object is irrelevant: only the requested digest is read.
    fs::create_dir(f.object(&"f".repeat(64))).unwrap();
    fs::write(f.object(&"f".repeat(64)).join(MANIFEST), b"invalid").unwrap();
    assert_eq!(f.project.restore_cached("art", digest).unwrap(), old);
    let before = tree(f.root.path());
    assert_eq!(f.project.restore_cached("art", digest).unwrap(), old);
    assert_eq!(tree(f.root.path()), before);
    let preview = f.unchanged(|| f.project.preview_restore_cached("art", digest).unwrap());
    assert!(preview.changes.is_empty());
}

#[test]
fn a_transitive_cached_object_can_be_selected_directly() {
    let f = Fixture::new();
    let dependency = f.source("dependency", "art", "1.0.0", &[]);
    let source = f.source("root", "root", "1.0.0", &[("art", "1.0.0")]);
    let old = f
        .project
        .install_with_dependencies(&[source], &[dependency])
        .unwrap();
    let digest = &old.packages["art"].digest;
    let preview = f.unchanged(|| f.project.preview_restore_cached("art", digest).unwrap());
    assert!(!preview.previous.direct.contains_key("art"));
    assert_eq!(preview.candidate.direct["art"], "1.0.0");
    assert_eq!(preview.candidate.packages, old.packages);
    assert_eq!(
        f.project.restore_cached("art", digest).unwrap(),
        preview.candidate
    );
}

#[test]
fn dependencies_must_be_active_at_the_exact_required_versions() {
    let f = Fixture::new();
    let dep1 = f.source("dep1", "base", "1.0.0", &[]);
    let art1 = f.source("art1", "art", "1.0.0", &[("base", "1.0.0")]);
    let old = f
        .project
        .install_with_dependencies(&[art1], &[dep1])
        .unwrap();
    let digest = &old.packages["art"].digest;
    let dep2 = f.source("dep2", "base", "2.0.0", &[]);
    let art2 = f.source("art2", "art", "2.0.0", &[("base", "2.0.0")]);
    f.project
        .install_with_dependencies(&[art2], &[dep2])
        .unwrap();
    f.rejects("art", digest);
    f.project.remove("art").unwrap();
    f.rejects("art", digest);
    // Explicitly restore the dependency first; there is no automatic cache resolver.
    f.project
        .restore_cached("base", &old.packages["base"].digest)
        .unwrap();
    let restored = f.project.restore_cached("art", digest).unwrap();
    assert_eq!(restored.packages, old.packages);
    assert_eq!(restored.direct.len(), 2);
}

#[test]
fn malformed_requests_and_missing_objects_leave_a_fresh_project_untouched() {
    let f = Fixture::new();
    let good = "a".repeat(64);
    for name in ["", "../art", "ART", "a/b", "art name"] {
        f.rejects(name, &good);
    }
    for digest in [
        "".into(),
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
        "../object".into(),
        format!("{}\n", "a".repeat(63)),
    ] {
        f.rejects("art", &digest);
    }
    f.rejects("art", &good);
    assert!(!f.project_dir.join(".orr").exists());
    assert!(!f.project_dir.join(LOCK).exists());
}

#[test]
fn wrong_name_directory_identity_or_cached_bytes_reject_before_activation() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    f.rejects("other", &digest);
    let original = f.object(&digest);
    let wrong = "a".repeat(64);
    assert_ne!(digest, wrong);
    fs::rename(&original, f.object(&wrong)).unwrap();
    f.rejects("art", &wrong);
    fs::rename(f.object(&wrong), &original).unwrap();
    fs::write(original.join("asset.txt"), b"changed cached bytes").unwrap();
    f.rejects("art", &digest);
}

#[test]
fn malformed_cached_manifests_paths_cycles_and_engine_requirements_are_rejected() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let object = f.object(&digest);
    let original = fs::read(object.join(MANIFEST)).unwrap();
    for change in 0..5 {
        fs::write(object.join(MANIFEST), &original).unwrap();
        edit_manifest(&object, |m| match change {
            0 => m["unexpected"] = true.into(),
            1 => m["files"] = json!(["../outside"]),
            2 => m["dependencies"] = json!({"art":"1.0.0"}),
            3 => m["engine"] = "^9.0.0".into(),
            _ => m["name"] = "different".into(),
        });
        f.rejects("art", &digest);
    }
}

#[test]
fn active_content_and_lock_corruption_are_not_repaired_or_bypassed() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let current = f.project.list().unwrap();
    let active = f.object(&current.packages["art"].digest).join("asset.txt");
    let bytes = fs::read(&active).unwrap();
    fs::write(&active, b"corrupted active content").unwrap();
    f.rejects("art", &digest);
    fs::write(active, bytes).unwrap();
    fs::write(f.project_dir.join(LOCK), b"not a lock").unwrap();
    f.rejects("art", &digest);
}

#[test]
fn restore_preserves_host_capability_and_management_only_boundaries() {
    let f = Fixture::new();
    let source = f.source("source", "art", "1.0.0", &[]);
    edit_manifest(&source, |m| m["capabilities"] = json!(["sprite"]));
    let old = f.project.install(&[source]).unwrap();
    let digest = &old.packages["art"].digest;
    f.project.remove("art").unwrap();
    let unsupported = Project::open(&f.project_dir, Runtime::content_only()).unwrap();
    f.unchanged(|| {
        assert!(unsupported.preview_restore_cached("art", digest).is_err());
        assert!(unsupported.restore_cached("art", digest).is_err());
    });
    f.project.restore_cached("art", digest).unwrap();
    assert!(f.project.read_asset("art", "asset.txt").is_err());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    let supported = Project::open(&f.project_dir, runtime).unwrap();
    assert_eq!(supported.restore_cached("art", digest).unwrap(), old);
    assert_eq!(
        supported.read_asset("art", "asset.txt").unwrap(),
        b"art@1.0.0"
    );
}

#[test]
fn checked_cached_plans_reject_changed_content_and_previous_locks() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let object = f.object(&digest);
    let preview = f.project.preview_restore_cached("art", &digest).unwrap();
    let original = fs::read(object.join("asset.txt")).unwrap();
    fs::write(object.join("asset.txt"), b"changed after planning").unwrap();
    let error = f.unchanged(|| {
        f.project
            .install_checked(
                std::slice::from_ref(&object),
                &preview.previous,
                &preview.candidate,
                || Ok(()),
            )
            .unwrap_err()
    });
    assert!(
        error.to_string().contains("candidate package lock differs"),
        "{error}"
    );
    fs::write(object.join("asset.txt"), original).unwrap();
    let other = f.source("other", "other", "1.0.0", &[]);
    f.project.install(&[other]).unwrap();
    let error = f.unchanged(|| {
        f.project
            .install_checked(&[object], &preview.previous, &preview.candidate, || Ok(()))
            .unwrap_err()
    });
    assert!(
        error.to_string().contains("active package lock changed"),
        "{error}"
    );
}

#[test]
fn preview_preserves_writer_guards_and_restore_never_steals_them() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let guard = f.project_dir.join(".orr/packages/writer.lock");
    fs::write(&guard, b"another writer owns this\n").unwrap();
    f.unchanged(|| {
        assert!(f.project.preview_restore_cached("art", &digest).is_ok());
        let error = f.project.restore_cached("art", &digest).unwrap_err();
        assert!(error.to_string().contains("writer unavailable"), "{error}");
    });
    assert_eq!(fs::read(guard).unwrap(), b"another writer owns this\n");
}

#[test]
fn an_active_same_version_with_different_content_still_blocks_restore() {
    let f = Fixture::new();
    let first = f.source("first", "art", "1.0.0", &[]);
    let old = f.project.install(&[first]).unwrap();
    f.project.remove("art").unwrap();
    let second = f.source("second", "art", "1.0.0", &[]);
    fs::write(
        second.join("asset.txt"),
        b"another revision of the same version",
    )
    .unwrap();
    f.project.install(&[second]).unwrap();
    f.rejects("art", &old.packages["art"].digest);
}

#[test]
fn oversize_cached_manifest_and_asset_keep_existing_bounds() {
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let object = f.object(&digest);
    let original = fs::read(object.join(MANIFEST)).unwrap();
    fs::File::create(object.join(MANIFEST))
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    f.rejects("art", &digest);
    fs::write(object.join(MANIFEST), original).unwrap();
    fs::File::create(object.join("asset.txt"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    f.rejects("art", &digest);
}

#[test]
fn invalid_restore_cli_arguments_fail_without_json_or_filesystem_changes() {
    let f = Fixture::new();
    let digest = "a".repeat(64);
    for args in [
        vec!["restore"],
        vec!["restore", "art"],
        vec!["restore", "art", &digest, "--unknown"],
        vec!["restore", "art", &digest, "--dry-run", "extra"],
        vec!["restore", "art", "../bad"],
        vec!["restore", "other", &digest],
    ] {
        let output = f.unchanged(|| cli(&f.project_dir, &args));
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn cached_object_and_asset_symlinks_are_rejected_without_following_them() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let (_, _, digest) = f.revisions();
    let object = f.object(&digest);
    let outside = f.root.path().join("outside-object");
    fs::rename(&object, &outside).unwrap();
    symlink(&outside, &object).unwrap();
    f.rejects("art", &digest);
    fs::remove_file(&object).unwrap();
    fs::rename(outside, &object).unwrap();
    fs::remove_file(object.join("asset.txt")).unwrap();
    symlink(f.project_dir.join(LOCK), object.join("asset.txt")).unwrap();
    f.rejects("art", &digest);
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "bounded subprocess-only filesystem regression timeout"
)]
fn cached_fifo_inputs_fail_in_bounded_cli_processes() {
    use std::{
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    for filename in [MANIFEST, "asset.txt"] {
        let f = Fixture::new();
        let (_, _, digest) = f.revisions();
        let path = f.object(&digest).join(filename);
        fs::remove_file(&path).unwrap();
        assert!(Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        for preview in [false, true] {
            let before = tree(f.root.path());
            let mut command = Command::new(env!("CARGO_BIN_EXE_orr_pkg"));
            command
                .arg("restore")
                .arg(&f.project_dir)
                .arg("art")
                .arg(&digest)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if preview {
                command.arg("--dry-run");
            }
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("cached FIFO blocked package validation");
                }
                thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("regular file"));
            assert_eq!(tree(f.root.path()), before);
        }
    }
}
