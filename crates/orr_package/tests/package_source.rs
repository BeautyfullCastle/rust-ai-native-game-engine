#![cfg(windows)]

use orr_package::{inspect_for_install, Manifest, Project, Runtime};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Barrier},
    thread,
};

#[path = "../examples/package_source/authoring.rs"]
mod authoring;

use authoring::{Options, Report};

const MANIFEST: &str = "orr.package.json";
const LOCK: &str = "orr.packages.lock.json";

struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join(label);
        fs::create_dir_all(&source).unwrap();
        Self {
            _root: root,
            source,
        }
    }

    fn payload(&self) {
        fs::write(self.source.join("asset.bin"), b"sample asset bytes").unwrap();
        fs::create_dir(self.source.join("licenses")).unwrap();
        fs::write(self.source.join("licenses/NOTICE.txt"), b"license text").unwrap();
    }
}

fn options(source: &Path, files: &[&str]) -> Options {
    Options {
        source: source.to_path_buf(),
        name: "sample-art".into(),
        version: "1.0.0".into(),
        engine: "^0.0.1".into(),
        capabilities: Vec::new(),
        dependencies: Vec::new(),
        files: files.iter().map(|file| (*file).to_owned()).collect(),
    }
}

fn parse(arguments: &[&str]) -> Result<Options, String> {
    Options::parse(arguments.iter().map(OsString::from).collect())
}

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    Directory,
    File(Vec<u8>),
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, TreeEntry>) {
        let metadata = fs::symlink_metadata(current).unwrap();
        let relative = current.strip_prefix(root).unwrap().to_path_buf();
        assert!(!metadata.file_type().is_symlink());
        if metadata.is_dir() {
            result.insert(relative, TreeEntry::Directory);
            for entry in fs::read_dir(current).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
        } else if metadata.is_file() {
            result.insert(relative, TreeEntry::File(fs::read(current).unwrap()));
        } else {
            panic!("unexpected non-regular fixture path: {}", current.display());
        }
    }

    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn assert_rejected_without_changes(options: &Options) {
    let before = tree_snapshot(&options.source);
    assert!(authoring::create(options).is_err());
    assert_eq!(tree_snapshot(&options.source), before);
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn required_absolute_file(name: &str) -> PathBuf {
    let value = env::var_os(name).unwrap_or_else(|| panic!("{name} is required"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{name} must be absolute");
    assert!(path.is_file(), "{name} must name an existing file");
    fs::canonicalize(path).unwrap()
}

#[test]
fn usage_and_parser_cover_required_options_order_and_errors() {
    for option in ["--source", "--name", "--version", "--engine", "--file"] {
        assert!(authoring::USAGE.contains(option));
    }
    assert!(authoring::USAGE.contains("--capability"));
    assert!(authoring::USAGE.contains("--dependency"));

    let parsed = parse(&[
        "--file",
        "asset.bin",
        "--dependency",
        "base-art=2.0.0",
        "--source",
        "C:\\source with spaces",
        "--capability",
        "models",
        "--engine",
        "^0.0.1",
        "--version",
        "1.0.0",
        "--name",
        "sample-art",
        "--file",
        "licenses/NOTICE.txt",
    ])
    .unwrap();
    assert_eq!(parsed.source, PathBuf::from("C:\\source with spaces"));
    assert_eq!(parsed.name, "sample-art");
    assert_eq!(parsed.version, "1.0.0");
    assert_eq!(parsed.engine, "^0.0.1");
    assert_eq!(parsed.capabilities, vec!["models".to_owned()]);
    assert_eq!(
        parsed.dependencies,
        vec![("base-art".to_owned(), "2.0.0".to_owned())]
    );
    assert_eq!(
        parsed.files,
        vec!["asset.bin".to_owned(), "licenses/NOTICE.txt".to_owned()]
    );

    for invalid_arguments in [
        vec!["--source", "C:\\source", "--name", "sample-art"],
        vec![
            "--source",
            "C:\\source",
            "--name",
            "sample-art",
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
            "--file",
            "asset.bin",
            "--mystery",
        ],
        vec![
            "--source",
            "C:\\source",
            "--name",
            "sample-art",
            "--name",
            "other-art",
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
            "--file",
            "asset.bin",
        ],
        vec![
            "--source",
            "C:\\source",
            "--name",
            "sample-art",
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
            "--file",
        ],
        vec![
            "--source",
            "C:\\source",
            "--name",
            "sample-art",
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
            "--file",
            "asset.bin",
            "--file",
            "asset.bin",
        ],
    ] {
        assert!(parse(&invalid_arguments).is_err());
    }

    let duplicate_capability = parse(&[
        "--source",
        "C:\\source",
        "--name",
        "sample-art",
        "--version",
        "1.0.0",
        "--engine",
        "^0.0.1",
        "--capability",
        "models",
        "--capability",
        "models",
        "--file",
        "asset.bin",
    ]);
    assert!(duplicate_capability.is_err());
}

#[test]
fn create_emits_stable_manifest_validated_by_authoritative_inspector() {
    let first = Fixture::new("first source");
    first.payload();
    fs::write(first.source.join("unlisted.py"), b"inert payload").unwrap();
    let mut first_options = options(&first.source, &["asset.bin", "licenses/NOTICE.txt"]);
    first_options.capabilities = vec!["models".into(), "sprite".into()];
    first_options.dependencies = vec![("base-art".into(), "2.0.0".into())];

    let first_report: Report = authoring::create(&first_options).unwrap();
    assert_eq!(
        fs::canonicalize(&first_report.manifest).unwrap(),
        fs::canonicalize(first.source.join(MANIFEST)).unwrap()
    );
    assert_eq!(first_report.declared_files, 2);
    assert_eq!(first_report.declared_bytes, 30);
    let first_bytes = fs::read(&first_report.manifest).unwrap();
    let first_manifest: Manifest = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(first_manifest.schema, 1);
    assert_eq!(first_manifest.name, "sample-art");
    assert_eq!(first_manifest.version, "1.0.0");
    assert_eq!(first_manifest.engine, "^0.0.1");
    assert_eq!(
        first_manifest.capabilities.into_iter().collect::<Vec<_>>(),
        vec!["models".to_owned(), "sprite".to_owned()]
    );
    assert_eq!(first_manifest.dependencies["base-art"], "2.0.0");
    assert_eq!(
        first_manifest.files.into_iter().collect::<Vec<_>>(),
        vec!["asset.bin".to_owned(), "licenses/NOTICE.txt".to_owned()]
    );

    let first_inspection =
        inspect_for_install(&first.source, Runtime::content_only().engine_version).unwrap();
    assert_eq!(first_inspection.manifest.name, "sample-art");
    assert_eq!(
        first_inspection
            .files
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["asset.bin", "licenses/NOTICE.txt"]
    );
    assert!(!first.source.join(LOCK).exists());
    assert!(!first.source.join(".orr").exists());
    assert_eq!(
        fs::read(first.source.join("unlisted.py")).unwrap(),
        b"inert payload"
    );

    let second = Fixture::new("second source");
    second.payload();
    fs::write(second.source.join("unlisted.py"), b"inert payload").unwrap();
    let mut second_options = options(&second.source, &["asset.bin", "licenses/NOTICE.txt"]);
    second_options.capabilities = vec!["sprite".into(), "models".into()];
    second_options.dependencies = vec![("base-art".into(), "2.0.0".into())];
    let second_report = authoring::create(&second_options).unwrap();
    assert_eq!(fs::read(second_report.manifest).unwrap(), first_bytes);
    let second_inspection =
        inspect_for_install(&second.source, Runtime::content_only().engine_version).unwrap();
    assert_eq!(second_inspection, first_inspection);
}

#[test]
fn create_rejects_invalid_metadata_paths_and_collisions_without_source_changes() {
    let fixture = Fixture::new("invalid package");
    fixture.payload();
    let base = options(&fixture.source, &["asset.bin"]);
    let mut cases = Vec::new();

    let mut invalid = base.clone();
    invalid.name = "UpperCase".into();
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.version = "1.0".into();
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.engine = "not an engine requirement".into();
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.capabilities = vec!["Invalid Capability".into()];
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.dependencies = vec![("base-art".into(), "1.0".into())];
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.dependencies = vec![
        ("base-art".into(), "1.0.0".into()),
        ("base-art".into(), "2.0.0".into()),
    ];
    cases.push(invalid);
    let mut invalid = base.clone();
    invalid.files = vec!["asset.bin".into(), "asset.bin".into()];
    cases.push(invalid);
    for invalid_path in [
        "../escape.bin",
        "C:/escape.bin",
        "nested\\asset.bin",
        "CON",
        "bad//path.bin",
        "/absolute.bin",
    ] {
        let mut invalid = base.clone();
        invalid.files = vec![invalid_path.into()];
        cases.push(invalid);
    }
    let mut case_collision = base.clone();
    case_collision.files = vec!["asset.bin".into(), "ASSET.BIN".into()];
    cases.push(case_collision);
    let mut ancestor_collision = base.clone();
    ancestor_collision.files = vec!["asset.bin".into(), "asset.bin/child.bin".into()];
    cases.push(ancestor_collision);
    let mut missing = base.clone();
    missing.files = vec!["missing.bin".into()];
    cases.push(missing);

    for invalid in cases {
        assert_rejected_without_changes(&invalid);
    }
}

#[test]
fn create_rejects_empty_and_oversized_file_lists_without_residue() {
    let empty_fixture = Fixture::new("empty package");
    empty_fixture.payload();
    let empty = options(&empty_fixture.source, &[]);
    assert_rejected_without_changes(&empty);

    let fixture = Fixture::new("oversized package");
    let huge_path = fixture.source.join("oversized.bin");
    let huge_file = fs::File::create(&huge_path).unwrap();
    huge_file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(huge_file);
    let before_names = fs::read_dir(&fixture.source)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    let oversized = options(&fixture.source, &["oversized.bin"]);
    assert!(authoring::create(&oversized).is_err());
    assert_eq!(
        fs::metadata(&huge_path).unwrap().len(),
        64 * 1024 * 1024 + 1
    );
    assert_eq!(
        fs::read_dir(&fixture.source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>(),
        before_names
    );
    assert!(!fixture.source.join(MANIFEST).exists());

    let mut excessive_files = options(&fixture.source, &["oversized.bin"]);
    excessive_files.files = (0..4097)
        .map(|index| format!("missing-{index}.bin"))
        .collect();
    assert!(authoring::create(&excessive_files).is_err());
    assert!(!fixture.source.join(MANIFEST).exists());
    assert_eq!(
        fs::read_dir(&fixture.source)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>(),
        before_names
    );
}

#[test]
fn existing_manifest_file_or_directory_is_never_replaced() {
    let file_fixture = Fixture::new("existing manifest file");
    file_fixture.payload();
    let sentinel = b"keep existing package metadata byte-for-byte";
    fs::write(file_fixture.source.join(MANIFEST), sentinel).unwrap();
    let file_options = options(&file_fixture.source, &["asset.bin"]);
    let file_before = tree_snapshot(&file_fixture.source);
    assert!(authoring::create(&file_options).is_err());
    assert_eq!(
        fs::read(file_fixture.source.join(MANIFEST)).unwrap(),
        sentinel
    );
    assert_eq!(tree_snapshot(&file_fixture.source), file_before);

    let directory_fixture = Fixture::new("manifest directory");
    directory_fixture.payload();
    let manifest_directory = directory_fixture.source.join(MANIFEST);
    fs::create_dir(&manifest_directory).unwrap();
    fs::write(manifest_directory.join("sentinel"), b"preserve directory").unwrap();
    let directory_options = options(&directory_fixture.source, &["asset.bin"]);
    let directory_before = tree_snapshot(&directory_fixture.source);
    assert!(authoring::create(&directory_options).is_err());
    assert_eq!(
        fs::read(manifest_directory.join("sentinel")).unwrap(),
        b"preserve directory"
    );
    assert_eq!(tree_snapshot(&directory_fixture.source), directory_before);
}

#[test]
fn concurrent_creators_have_exactly_one_no_clobber_winner() {
    let fixture = Fixture::new("concurrent package");
    fixture.payload();
    let first_options = options(&fixture.source, &["asset.bin", "licenses/NOTICE.txt"]);
    let second_options = first_options.clone();
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let first = thread::spawn(move || {
        first_barrier.wait();
        authoring::create(&first_options)
    });
    let second = thread::spawn(move || {
        second_barrier.wait();
        authoring::create(&second_options)
    });
    barrier.wait();
    let results = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let inspected =
        inspect_for_install(&fixture.source, Runtime::content_only().engine_version).unwrap();
    assert_eq!(inspected.manifest.name, "sample-art");
    let names = fs::read_dir(&fixture.source)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 3);
    assert!(fixture.source.join("asset.bin").exists());
    assert!(fixture.source.join("licenses").is_dir());
    assert!(fixture.source.join(MANIFEST).is_file());
}

#[test]
fn metadata_capabilities_never_substitute_for_consuming_runtime_support() {
    let fixture = Fixture::new("capability package");
    fixture.payload();
    let mut package_options = options(&fixture.source, &["asset.bin"]);
    package_options.capabilities = vec!["sprite".into()];
    authoring::create(&package_options).unwrap();
    let inspected =
        inspect_for_install(&fixture.source, Runtime::content_only().engine_version).unwrap();
    assert_eq!(
        inspected
            .manifest
            .capabilities
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["sprite".to_owned()]
    );

    let project_root = fixture._root.path().join("project");
    fs::create_dir(&project_root).unwrap();
    let project =
        Project::open_for_install(&project_root, Runtime::content_only().engine_version).unwrap();
    project
        .install(std::slice::from_ref(&fixture.source))
        .unwrap();
    let content_only = Project::open(&project_root, Runtime::content_only()).unwrap();
    assert!(content_only.verify().is_err());
    assert!(content_only.read_asset("sample-art", "asset.bin").is_err());
    assert!(project_root.join(LOCK).is_file());
}

#[test]
fn content_only_consumer_installs_reads_and_verifies_unqualified_content() {
    let fixture = Fixture::new("content-only package");
    fixture.payload();
    let package_options = options(&fixture.source, &["asset.bin", "licenses/NOTICE.txt"]);
    authoring::create(&package_options).unwrap();

    let project_root = fixture._root.path().join("consumer project");
    fs::create_dir(&project_root).unwrap();
    let project =
        Project::open_for_install(&project_root, Runtime::content_only().engine_version).unwrap();
    project
        .install(std::slice::from_ref(&fixture.source))
        .unwrap();
    let consumer = Project::open(&project_root, Runtime::content_only()).unwrap();
    assert_eq!(
        consumer.read_asset("sample-art", "asset.bin").unwrap(),
        b"sample asset bytes"
    );
    assert_eq!(
        consumer
            .read_asset("sample-art", "licenses/NOTICE.txt")
            .unwrap(),
        b"license text"
    );
    let lock = consumer.verify().unwrap();
    assert_eq!(lock.packages["sample-art"].files.len(), 2);
}

#[test]
#[ignore = "requires explicit built CLI binaries and a fresh evidence directory"]
fn actual_cli_creates_inspects_installs_and_reads_package() {
    let source_binary = required_absolute_file("ORR_PACKAGE_SOURCE_BIN");
    let package_binary = required_absolute_file("ORR_PKG_BIN");
    let evidence_value = env::var_os("ORR_PACKAGE_SOURCE_EVIDENCE")
        .expect("ORR_PACKAGE_SOURCE_EVIDENCE is required");
    let evidence = PathBuf::from(evidence_value);
    assert!(
        evidence.is_absolute(),
        "ORR_PACKAGE_SOURCE_EVIDENCE must be absolute"
    );
    assert!(evidence.is_dir(), "ORR_PACKAGE_SOURCE_EVIDENCE must exist");
    assert_eq!(
        fs::read_dir(&evidence).unwrap().count(),
        0,
        "evidence directory must be empty"
    );

    let source = evidence.join("unicode package 源");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("assets")).unwrap();
    fs::write(source.join("assets/room.bin"), b"room asset bytes").unwrap();
    fs::write(source.join("LICENSE.txt"), b"license bytes").unwrap();
    let unlisted = source.join("do-not-execute.py");
    let unlisted_bytes = b"raise RuntimeError('content is never a hook')";
    fs::write(&unlisted, unlisted_bytes).unwrap();

    let created = Command::new(&source_binary)
        .args([
            "--source",
            source.to_str().unwrap(),
            "--name",
            "room-art",
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
            "--file",
            "assets/room.bin",
            "--file",
            "LICENSE.txt",
        ])
        .output()
        .unwrap();
    let created_stdout = successful(created);
    assert!(created_stdout.contains("created manifest:"));
    assert!(created_stdout.contains("declared files: 2"));
    assert!(created_stdout.contains("declared bytes: 29"));

    let inspected_output = Command::new(&package_binary)
        .args(["inspect", source.to_str().unwrap()])
        .output()
        .unwrap();
    let inspected_stdout = successful(inspected_output);
    let inspected_json: Value = serde_json::from_str(&inspected_stdout).unwrap();
    assert_eq!(inspected_json["manifest"]["name"], "room-art");
    assert_eq!(inspected_json["manifest"]["capabilities"], json!([]));
    assert_eq!(inspected_json["files"].as_object().unwrap().len(), 2);

    let project_root = evidence.join("game project");
    fs::create_dir(&project_root).unwrap();
    let installed = Command::new(&package_binary)
        .args([
            "install",
            project_root.to_str().unwrap(),
            "--path",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    successful(installed);
    let verified = Command::new(&package_binary)
        .args(["verify", project_root.to_str().unwrap()])
        .output()
        .unwrap();
    successful(verified);

    let consumer = Project::open(&project_root, Runtime::content_only()).unwrap();
    assert_eq!(
        consumer.read_asset("room-art", "assets/room.bin").unwrap(),
        b"room asset bytes"
    );
    assert_eq!(
        consumer.read_asset("room-art", "LICENSE.txt").unwrap(),
        b"license bytes"
    );
    let lock = consumer.verify().unwrap();
    assert_eq!(lock.packages["room-art"].files.len(), 2);
    assert!(!lock.packages["room-art"]
        .files
        .contains_key("do-not-execute.py"));
    assert_eq!(fs::read(&unlisted).unwrap(), unlisted_bytes);
    assert!(!source.join(LOCK).exists());
    assert!(!source.join(".orr").exists());

    let receipt = json!({
        "source_binary": source_binary,
        "package_binary": package_binary,
        "source": source,
        "project": project_root,
        "declared_files": ["assets/room.bin", "LICENSE.txt"],
        "unlisted_file_unchanged": true,
        "verified_package": "room-art",
        "consumer_read_asset": "assets/room.bin",
        "capability_admission": "content_only",
    });
    fs::write(
        evidence.join("package-source-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
