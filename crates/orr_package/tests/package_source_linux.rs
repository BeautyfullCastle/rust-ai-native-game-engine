#![cfg(target_os = "linux")]

use orr_package::{inspect_for_install, Project, Runtime};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink, net::UnixListener},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Barrier},
    thread,
};

#[path = "../src/source_authoring.rs"]
mod authoring;

const MANIFEST: &str = "orr.package.json";
const LOCK: &str = "orr.packages.lock.json";

fn fixture(root: &Path, label: &str) -> PathBuf {
    let source = root.join(label);
    fs::create_dir_all(source.join("assets")).unwrap();
    fs::write(source.join("assets/room.bin"), b"room content").unwrap();
    fs::write(source.join("LICENSE.txt"), b"license").unwrap();
    fs::write(
        source.join("unlisted.py"),
        b"raise RuntimeError('not a hook')",
    )
    .unwrap();
    fs::write(source.join(LOCK), b"keep the source project's lock").unwrap();
    source
}

fn options(source: &Path) -> authoring::Options {
    authoring::Options {
        source: source.into(),
        name: "room-art".into(),
        version: "1.0.0".into(),
        engine: "^0.0.1".into(),
        capabilities: Vec::new(),
        dependencies: Vec::new(),
        files: vec!["assets/room.bin".into(), "LICENSE.txt".into()],
    }
}

fn arguments(options: &authoring::Options) -> Vec<OsString> {
    let mut args = vec![
        "--source".into(),
        options.source.as_os_str().into(),
        "--name".into(),
        options.name.clone().into(),
        "--version".into(),
        options.version.clone().into(),
        "--engine".into(),
        options.engine.clone().into(),
    ];
    for capability in &options.capabilities {
        args.extend(["--capability".into(), capability.into()]);
    }
    for (name, version) in &options.dependencies {
        args.extend(["--dependency".into(), format!("{name}={version}").into()]);
    }
    for file in &options.files {
        args.extend(["--file".into(), file.into()]);
    }
    args
}

#[expect(
    clippy::disallowed_types,
    reason = "bounded subprocess-only filesystem regression timeout"
)]
fn run_source(arguments: &[OsString]) -> Output {
    use std::time::{Duration, Instant};
    // A FIFO regression must fail rather than hang the test runner indefinitely.
    let mut child = Command::new(env!("CARGO_BIN_EXE_orr_pkg_source"))
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "source CLI timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn package(arguments: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(arguments)
        .output()
        .unwrap();
    serde_json::from_str(&successful(output)).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Special,
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Entry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let entry = if metadata.file_type().is_symlink() {
            Entry::Symlink(fs::read_link(path).unwrap())
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
            Entry::Directory
        } else if metadata.is_file() {
            Entry::File(fs::read(path).unwrap())
        } else {
            Entry::Special
        };
        result.insert(path.strip_prefix(root).unwrap().into(), entry);
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn reject(options: &authoring::Options, observed: &Path) {
    let before = snapshot(observed);
    let output = run_source(&arguments(options));
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        successful(output)
    );
    assert_eq!(snapshot(observed), before);
}

#[test]
fn real_cli_creates_deterministically_previews_dependencies_and_survives_hidden_sources() {
    let help = successful(run_source(&["--help".into()]));
    assert!(help.starts_with("orr_pkg_source --source"));
    assert_eq!(run_source(&[]).status.code(), Some(2));
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "模型 source one");
    let mut selected = options(&source);
    selected.capabilities = vec!["sprite".into(), "models".into()];
    selected.dependencies = vec![
        ("palette-art".into(), "1.0.0".into()),
        ("base-art".into(), "1.0.0".into()),
    ];
    let parsed = authoring::Options::parse(arguments(&selected)).unwrap();
    assert_eq!(parsed.source, source);
    let before = snapshot(&source);
    let result = successful(run_source(&arguments(&selected)));
    assert!(result.contains("declared files: 2"));
    assert!(result.contains("declared bytes: 19"));
    let manifest = fs::read(source.join(MANIFEST)).unwrap();
    let mut after = snapshot(&source);
    after.remove(Path::new(MANIFEST));
    assert_eq!(after, before);
    let inspected = package(&["inspect", source.to_str().unwrap()]);
    assert_eq!(inspected["manifest"]["name"], "room-art");
    assert_eq!(inspected["files"].as_object().unwrap().len(), 2);

    let second_source = fixture(directory.path(), "다른 source two");
    let mut reordered = selected.clone();
    reordered.source = second_source.clone();
    reordered.files.reverse();
    reordered.capabilities.reverse();
    reordered.dependencies.reverse();
    successful(run_source(&arguments(&reordered)));
    assert_eq!(fs::read(second_source.join(MANIFEST)).unwrap(), manifest);
    assert_eq!(
        package(&["inspect", second_source.to_str().unwrap()]),
        inspected
    );
    reject(&selected, &source);

    let base = fixture(directory.path(), "base source");
    let palette = fixture(directory.path(), "palette source");
    for (path, name) in [(&base, "base-art"), (&palette, "palette-art")] {
        let mut dependency = options(path);
        dependency.name = name.into();
        successful(run_source(&arguments(&dependency)));
    }
    let project = directory.path().join("consumer project");
    fs::create_dir(&project).unwrap();
    let project_text = project.to_str().unwrap();
    let before = snapshot(&project);
    let missing = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args([
            "install",
            project_text,
            "--dry-run",
            "--path",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert_eq!(snapshot(&project), before);
    let preview = package(&[
        "install",
        project_text,
        "--dry-run",
        "--path",
        source.to_str().unwrap(),
        "--dependency-path",
        base.to_str().unwrap(),
        "--dependency-path",
        palette.to_str().unwrap(),
    ]);
    assert_eq!(
        preview["candidate"]["packages"].as_object().unwrap().len(),
        3
    );
    assert_eq!(snapshot(&project), before);
    let installed = package(&[
        "install",
        project_text,
        "--path",
        source.to_str().unwrap(),
        "--dependency-path",
        palette.to_str().unwrap(),
        "--dependency-path",
        base.to_str().unwrap(),
    ]);
    assert_eq!(installed, preview["candidate"]);
    assert_eq!(
        installed["packages"]["room-art"]["digest"],
        inspected["digest"]
    );
    let lock = fs::read(project.join(LOCK)).unwrap();

    for path in [&source, &second_source, &base, &palette] {
        let hidden = path.with_file_name(format!(
            "hidden-{}",
            path.file_name().unwrap().to_str().unwrap()
        ));
        fs::rename(path, hidden).unwrap();
        assert!(!path.exists());
    }
    assert_eq!(package(&["verify", project_text]), installed);
    // This test host explicitly supplies its inventory; never derive it from JSON.
    // Asset bytes are generic test content, not proof of model/sprite decoding.
    let mut supported = Runtime::content_only();
    supported.capabilities = ["models".into(), "sprite".into()].into_iter().collect();
    let consumer = Project::open(&project, supported).unwrap();
    assert_eq!(consumer.verify().unwrap().packages.len(), 3);
    assert_eq!(
        consumer.read_asset("room-art", "assets/room.bin").unwrap(),
        b"room content"
    );
    assert_eq!(
        consumer.read_asset("base-art", "LICENSE.txt").unwrap(),
        b"license"
    );
    assert!(consumer.read_asset("room-art", "unlisted.py").is_err());
    let unsupported = Project::open(&project, Runtime::content_only()).unwrap();
    assert!(unsupported
        .verify()
        .unwrap_err()
        .to_string()
        .contains("missing compiled capability"));
    assert!(unsupported
        .read_asset("room-art", "assets/room.bin")
        .is_err());
    assert_eq!(fs::read(project.join(LOCK)).unwrap(), lock);
}

#[test]
fn invalid_metadata_duplicates_collisions_and_cli_input_preserve_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "invalid");
    let valid = options(&source);
    let mut cases = Vec::new();
    for name in ["UpperCase", "", "a/b"] {
        let mut invalid = valid.clone();
        invalid.name = name.into();
        cases.push(invalid);
    }
    for version in ["1.0", "^1.0.0", "01.0.0"] {
        let mut invalid = valid.clone();
        invalid.version = version.into();
        cases.push(invalid);
    }
    for engine in ["not semver", "^99.0.0", ""] {
        let mut invalid = valid.clone();
        invalid.engine = engine.into();
        cases.push(invalid);
    }
    for capabilities in [
        vec!["Bad Capability".into()],
        vec!["models".into(), "models".into()],
        vec!["models".into(); 129],
    ] {
        let mut invalid = valid.clone();
        invalid.capabilities = capabilities;
        cases.push(invalid);
    }
    for dependencies in [
        vec![("base-art".into(), "1.0".into())],
        vec![("base-art".into(), "1.0.0".into()); 2],
        vec![("base-art".into(), "1.0.0".into()); 129],
    ] {
        let mut invalid = valid.clone();
        invalid.dependencies = dependencies;
        cases.push(invalid);
    }
    for files in [
        vec![],
        vec!["LICENSE.txt", "LICENSE.txt"],
        vec!["LICENSE.txt", "license.txt"],
        vec!["assets", "assets/room.bin"],
        vec!["../escape"],
        vec!["/absolute"],
        vec!["bad\\file"],
        vec!["CON"],
        vec!["assets//room.bin"],
        vec!["assets/./room.bin"],
        vec!["orr.package.json"],
        vec!["ORR.PACKAGE.JSON/child"],
        vec!["missing.bin"],
        vec!["assets"],
    ] {
        let mut invalid = valid.clone();
        invalid.files = files.into_iter().map(str::to_owned).collect();
        cases.push(invalid);
    }
    let mut too_many = valid.clone();
    too_many.files = (0..4097).map(|index| format!("file-{index}.bin")).collect();
    cases.push(too_many);
    for invalid in cases {
        reject(&invalid, &source);
    }
    let before = snapshot(&source);
    let args = arguments(&valid);
    for suffix in [
        vec!["--name".into(), "duplicate".into()],
        vec!["--unknown".into()],
        vec!["--file".into()],
        vec![OsString::from_vec(vec![0xff])],
    ] {
        let mut invalid = args.clone();
        invalid.extend(suffix);
        assert_eq!(run_source(&invalid).status.code(), Some(2));
    }
    let mut oversized = args;
    oversized[3] = "x".repeat(4097).into();
    assert_eq!(run_source(&oversized).status.code(), Some(2));
    assert_eq!(snapshot(&source), before);
}

#[test]
fn source_paths_must_be_normalized_bounded_unicode_non_root_and_absolute() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "source");
    let valid = options(&source);
    let text = source.to_str().unwrap();
    let mut paths: Vec<PathBuf> = [
        "relative".into(),
        "/".into(),
        format!("{text}/"),
        format!("{text}/."),
        format!("{text}/../source"),
        format!("/{text}"),
        format!("{text}\n"),
        format!("/{}", "x".repeat(4096)),
        format!("/{}", vec!["x"; 65].join("/")),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    paths.push(PathBuf::from(OsString::from_vec(
        b"/tmp/invalid-\xff".to_vec(),
    )));
    for path in paths {
        let mut invalid = valid.clone();
        invalid.source = path;
        reject(&invalid, &source);
    }
}

#[test]
fn symlinked_ancestors_roots_assets_and_nonregular_files_are_rejected_without_reading() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "source");
    let alias = directory.path().join("alias");
    symlink(&source, &alias).unwrap();
    reject(&options(&alias), directory.path());
    let parent_alias = directory.path().join("parent-alias");
    symlink(directory.path(), &parent_alias).unwrap();
    reject(&options(&parent_alias.join("source")), directory.path());
    symlink(source.join("LICENSE.txt"), source.join("linked.bin")).unwrap();
    symlink(source.join("assets"), source.join("linked-dir")).unwrap();
    symlink("missing", source.join("dangling.bin")).unwrap();
    let fifo = source.join("pipe.bin");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    for file in [
        "linked.bin",
        "linked-dir/room.bin",
        "dangling.bin",
        "pipe.bin",
        "assets",
    ] {
        let mut invalid = options(&source);
        invalid.files = vec![file.into()];
        reject(&invalid, directory.path());
        if file == "pipe.bin" {
            let output = run_source(&arguments(&invalid));
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("declared source must be a regular file within limits"));
        }
    }
}

#[test]
#[ignore = "supplemental AF_UNIX lane requires permission to bind a local socket; primary special-file acceptance uses FIFO"]
fn socket_source_is_rejected_without_changes() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "socket source");
    // Explicit opt-in only: even this supplemental lane must fail if its socket
    // cannot be created. Never turn a denied bind into a passing assertion.
    let _socket = UnixListener::bind(source.join("socket.bin")).unwrap();
    let mut invalid = options(&source);
    invalid.files = vec!["socket.bin".into()];
    reject(&invalid, directory.path());
}

#[test]
fn existing_manifest_files_directories_and_dangling_symlinks_are_never_replaced() {
    let directory = tempfile::tempdir().unwrap();
    for kind in ["file", "directory", "symlink"] {
        let source = fixture(directory.path(), kind);
        let manifest = source.join(MANIFEST);
        match kind {
            "file" => fs::write(&manifest, b"existing manifest sentinel").unwrap(),
            "directory" => {
                fs::create_dir(&manifest).unwrap();
                fs::write(manifest.join("sentinel"), b"keep").unwrap();
            }
            _ => symlink("nonexistent-target", &manifest).unwrap(),
        }
        reject(&options(&source), &source);
    }
}

#[test]
fn oversized_files_and_total_bytes_fail_without_source_or_lock_changes() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "oversized");
    let before = snapshot(&source);
    let huge = source.join("huge.bin");
    fs::File::create(&huge)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let mut invalid = options(&source);
    invalid.files = vec!["huge.bin".into()];
    assert!(!run_source(&arguments(&invalid)).status.success());
    assert_eq!(fs::metadata(&huge).unwrap().len(), 64 * 1024 * 1024 + 1);
    fs::remove_file(huge).unwrap();
    assert_eq!(snapshot(&source), before);

    // Sparse fixtures keep source disk use low; the bounded snapshot still
    // reaches the real 256 MiB total limit, without changing production limits.
    let mut invalid = options(&source);
    invalid.files.clear();
    for index in 0..5 {
        let relative = format!("part-{index}.bin");
        fs::File::create(source.join(&relative))
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        invalid.files.push(relative);
    }
    let error = authoring::create(&invalid)
        .err()
        .expect("total limit must reject");
    assert!(error.contains("package byte limit exceeded"), "{error}");
    for relative in &invalid.files {
        assert_eq!(
            fs::metadata(source.join(relative)).unwrap().len(),
            64 * 1024 * 1024
        );
        fs::remove_file(source.join(relative)).unwrap();
    }
    assert_eq!(snapshot(&source), before);
}

#[test]
fn concurrent_creators_preserve_exactly_one_winner_and_leave_no_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture(directory.path(), "concurrent");
    let before = snapshot(&source);
    let first = options(&source);
    let mut second = first.clone();
    second.name = "other-art".into();
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = [first, second]
        .into_iter()
        .map(|options| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                authoring::create(&options)
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let report = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(report.manifest, source.join(MANIFEST));
    assert_eq!(report.declared_files, 2);
    assert_eq!(report.declared_bytes, 19);
    let inspected = inspect_for_install(&source, Runtime::content_only().engine_version).unwrap();
    assert!(["room-art", "other-art"].contains(&inspected.manifest.name.as_str()));
    let mut after = snapshot(&source);
    after.remove(Path::new(MANIFEST));
    assert_eq!(after, before);
}
