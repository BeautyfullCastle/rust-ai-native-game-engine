#![cfg(target_os = "linux")]
use orr_package::{Lock, Project, Runtime, SourceExportReport};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let project =
            Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
        Self {
            temp,
            root,
            project,
        }
    }
    fn source(&self, name: &str, deps: &[&str]) -> PathBuf {
        let source = self.temp.path().join(format!("source-{name}"));
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("assets")).unwrap();
        let dependencies: BTreeMap<_, _> = deps.iter().map(|name| (*name, "1.0.0")).collect();
        fs::write(
            source.join("orr.package.json"),
            serde_json::to_vec(&json!({
                "schema":1,"name":name,"version":"1.0.0","engine":"^0.0.1",
                "capabilities":["models"],"dependencies":dependencies,
                "files":["LICENSE.txt","assets/data.bin"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            source.join("LICENSE.txt"),
            b"Declared rights text; not an authorization\n",
        )
        .unwrap();
        fs::write(source.join("assets/data.bin"), format!("{name} payload")).unwrap();
        fs::write(source.join("undeclared-private.txt"), b"must not copy").unwrap();
        source
    }
    fn install(&self) -> Lock {
        let root = self.source("art", &["left", "right"]);
        let left = self.source("left", &["shared"]);
        let right = self.source("right", &["shared"]);
        let shared = self.source("shared", &[]);
        let other = self.source("other", &[]);
        self.project
            .install_with_dependencies(&[root, other], &[left, right, shared])
            .unwrap()
    }
    fn output(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    fn object(&self, lock: &Lock, name: &str) -> PathBuf {
        self.root
            .join(".orr/packages/objects")
            .join(&lock.packages[name].digest)
    }
}
fn inventory(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

#[test]
fn source_hidden_diamond_roundtrip_preserves_identity_license_and_host_boundary() {
    let fixture = Fixture::new();
    let lock = fixture.install();
    let retired = fixture.source("retired", &[]);
    fixture.project.install(&[retired]).unwrap();
    assert_eq!(fixture.project.remove("retired").unwrap(), lock);
    for name in ["art", "left", "right", "shared", "other", "retired"] {
        fs::rename(
            fixture.temp.path().join(format!("source-{name}")),
            fixture.temp.path().join(format!("hidden-{name}")),
        )
        .unwrap();
    }
    // Extra inactive objects and undeclared files are not exported.
    fs::write(
        fixture.object(&lock, "art").join("private-extra.txt"),
        b"private",
    )
    .unwrap();
    let before = inventory(&fixture.root);
    let output = fixture.output("portable sources");
    let report = fixture.project.export_sources("art", &output).unwrap();
    assert_eq!(
        report
            .sources
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["art", "left", "right", "shared"]
    );
    assert_eq!(fs::read_dir(&output).unwrap().count(), 4);
    for (name, source) in &report.sources {
        assert_eq!(source.digest, lock.packages[name].digest);
        let inspected = orr_package::inspect_for_install(
            output.join(&source.directory),
            Runtime::content_only().engine_version,
        )
        .unwrap();
        assert_eq!(inspected, lock.packages[name]);
        assert_eq!(inventory(&output.join(&source.directory)).len(), 3);
    }
    assert_eq!(inventory(&fixture.root), before);
    let second = fixture.output("second");
    assert_eq!(
        fixture.project.export_sources("art", &second).unwrap(),
        report
    );
    assert_eq!(inventory(&output), inventory(&second));

    let fresh = fixture.output("fresh");
    fs::create_dir(&fresh).unwrap();
    let installer =
        Project::open_for_install(&fresh, Runtime::content_only().engine_version).unwrap();
    let dependencies: Vec<_> = report
        .sources
        .iter()
        .filter(|(name, _)| name.as_str() != "art")
        .map(|(_, source)| output.join(&source.directory))
        .collect();
    let installed = installer
        .install_with_dependencies(
            &[output.join(&report.sources["art"].directory)],
            &dependencies,
        )
        .unwrap();
    assert_eq!(installed.direct.keys().collect::<Vec<_>>(), vec!["art"]);
    for name in report.sources.keys() {
        assert_eq!(installed.packages[name], lock.packages[name]);
    }
    assert!(installer.read_asset("art", "assets/data.bin").is_err());
    let unsupported = Project::open(&fresh, Runtime::content_only()).unwrap();
    assert!(unsupported.verify().is_err());
    let denied_output = fixture.output("unsupported-export");
    assert!(unsupported.export_sources("art", &denied_output).is_err());
    assert!(!denied_output.exists());
    let runtime = Runtime {
        engine_version: Runtime::content_only().engine_version,
        capabilities: ["models".into()].into(),
    };
    let consumer = Project::open(&fresh, runtime).unwrap();
    consumer.verify().unwrap();
    assert_eq!(
        consumer.read_asset("art", "assets/data.bin").unwrap(),
        b"art payload"
    );
    assert_eq!(
        consumer.read_asset("art", "LICENSE.txt").unwrap(),
        b"Declared rights text; not an authorization\n"
    );
}

#[test]
fn selected_or_dependency_tampering_fails_before_output_but_unrelated_bytes_are_out_of_scope() {
    let fixture = Fixture::new();
    let lock = fixture.install();
    fs::write(
        fixture.object(&lock, "other").join("assets/data.bin"),
        b"unrelated bad bytes",
    )
    .unwrap();
    fixture
        .project
        .export_sources("art", fixture.output("unrelated-allowed"))
        .unwrap();
    for name in ["art", "shared"] {
        let path = fixture.object(&lock, name).join("assets/data.bin");
        let before = fs::read(&path).unwrap();
        fs::write(&path, b"tampered").unwrap();
        let output = fixture.output(&format!("reject-{name}"));
        assert!(fixture.project.export_sources("art", &output).is_err());
        assert!(!output.exists());
        fs::write(path, before).unwrap();
    }
}

#[test]
fn manifest_tamper_invalid_lock_and_missing_name_never_publish() {
    let fixture = Fixture::new();
    let lock = fixture.install();
    let output = fixture.output("never");
    assert!(fixture.project.export_sources("missing", &output).is_err());
    assert!(fixture.project.export_sources("../art", &output).is_err());
    let path = fixture.object(&lock, "shared").join("orr.package.json");
    let manifest = fs::read(&path).unwrap();
    fs::write(&path, b"{}").unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    fs::write(path, manifest).unwrap();
    fs::write(fixture.root.join("orr.packages.lock.json"), b"{}").unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    assert!(!output.exists());
}

#[test]
fn output_paths_and_preexisting_destinations_are_preserved() {
    let fixture = Fixture::new();
    fixture.install();
    for name in ["directory", "file"] {
        let output = fixture.output(name);
        if name == "directory" {
            fs::create_dir(&output).unwrap();
            fs::write(output.join("marker"), b"preserve").unwrap();
        } else {
            fs::write(&output, b"preserve").unwrap();
        }
        assert!(fixture.project.export_sources("art", &output).is_err());
        let marker = if name == "directory" {
            output.join("marker")
        } else {
            output
        };
        assert_eq!(fs::read(marker).unwrap(), b"preserve");
    }
    let before = inventory(&fixture.root);
    assert!(fixture
        .project
        .export_sources("art", fixture.root.join("inside"))
        .is_err());
    assert_eq!(inventory(&fixture.root), before);
    for path in [
        "relative".into(),
        fixture.output("missing-parent/child"),
        fixture.output("../escape"),
        fixture.output("a//b"),
        fixture.output("trailing/"),
    ] {
        assert!(fixture.project.export_sources("art", path).is_err());
    }
    let output = fixture.output("dangling");
    std::os::unix::fs::symlink(fixture.output("absent"), &output).unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    assert!(fs::symlink_metadata(output)
        .unwrap()
        .file_type()
        .is_symlink());
    let parent = fixture.output("linked-parent");
    std::os::unix::fs::symlink(fixture.temp.path(), &parent).unwrap();
    assert!(fixture
        .project
        .export_sources("art", parent.join("new"))
        .is_err());
    assert!(!fixture.output("new").exists());
}

#[expect(
    clippy::disallowed_types,
    reason = "bounded subprocess-only filesystem regression timeout"
)]
fn assert_fifo_cli_rejected(project: &Path, output: &Path) {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .arg("export-source")
        .arg(project)
        .arg("art")
        .arg("--output")
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            let result = child.wait_with_output().unwrap();
            assert_eq!(result.status.code(), Some(1));
            assert!(result.stdout.is_empty());
            assert!(String::from_utf8_lossy(&result.stderr).contains("regular file required"));
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let result = child.wait_with_output().unwrap();
            panic!(
                "FIFO rejection timed out: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn links_nonregular_files_and_size_bound_fail_without_output() {
    let fixture = Fixture::new();
    let lock = fixture.install();
    let path = fixture.object(&lock, "shared").join("assets/data.bin");
    fs::remove_file(&path).unwrap();
    let output = fixture.output("never");
    std::os::unix::fs::symlink(
        fixture.temp.path().join("source-shared/assets/data.bin"),
        &path,
    )
    .unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    fs::remove_dir(&path).unwrap();
    assert!(Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert_fifo_cli_rejected(&fixture.root, &output);
    fs::remove_file(&path).unwrap();
    fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(fixture.project.export_sources("art", &output).is_err());
    assert!(!output.exists());
}

#[test]
fn actual_cli_outputs_installable_sources_and_failure_has_no_success_json() {
    let fixture = Fixture::new();
    fixture.install();
    let output = fixture.output("cli");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .arg("export-source")
            .arg(&fixture.root)
            .arg("art")
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["requested_package"], "art");
    assert_eq!(report["sources"].as_object().unwrap().len(), 4);
    let before = inventory(&output);
    let second = run();
    assert_eq!(second.status.code(), Some(1));
    assert!(second.stdout.is_empty());
    assert_eq!(inventory(&output), before);
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
}

#[test]
fn concurrent_exporters_never_clobber_the_winner() {
    let fixture = Fixture::new();
    fixture.install();
    let output = fixture.output("concurrent");
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let call = || {
            barrier.wait();
            fixture.project.export_sources("art", &output)
        };
        let first = scope.spawn(call);
        let second = scope.spawn(call);
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let report: &SourceExportReport = results
        .iter()
        .find_map(|result| result.as_ref().ok())
        .unwrap();
    for source in report.sources.values() {
        assert_eq!(
            orr_package::inspect_for_install(
                output.join(&source.directory),
                Runtime::content_only().engine_version
            )
            .unwrap()
            .digest,
            source.digest
        );
    }
}
