#![cfg(all(windows, feature = "navigation"))]

use orr_package::{Manifest, Project, Runtime};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const PACKAGE: &str = "room-models";
const MODEL_ASSET: &str = "foreground.glb";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_orr_mesh_cook")
}

fn fixture_model() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo/foreground.glb")
        .canonicalize()
        .unwrap()
}

fn source_arguments(source: &Path, output: &Path) -> Vec<String> {
    vec![
        "--source".into(),
        source.display().to_string(),
        "--output".into(),
        output.display().to_string(),
    ]
}

fn project_arguments(project: &Path, asset: &str, output: &Path) -> Vec<String> {
    vec![
        "--project".into(),
        project.display().to_string(),
        "--package".into(),
        PACKAGE.into(),
        "--asset".into(),
        asset.into(),
        "--output".into(),
        output.display().to_string(),
    ]
}

fn run(arguments: &[String]) -> Output {
    Command::new(binary()).args(arguments).output().unwrap()
}

fn install_fixture(root: &Path, label: &str, capabilities: &[&str]) -> (PathBuf, PathBuf) {
    let source = root.join(format!("{label} source"));
    fs::create_dir_all(&source).unwrap();
    fs::copy(fixture_model(), source.join(MODEL_ASSET)).unwrap();
    fs::write(source.join("LICENSE.txt"), b"fixture license").unwrap();
    let manifest = Manifest {
        schema: 1,
        name: PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "^0.0.1".into(),
        capabilities: capabilities
            .iter()
            .map(|capability| (*capability).into())
            .collect(),
        dependencies: BTreeMap::new(),
        files: [MODEL_ASSET.to_owned(), "LICENSE.txt".into()]
            .into_iter()
            .collect::<BTreeSet<_>>(),
    };
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let project = root.join(format!("{label} project"));
    fs::create_dir(&project).unwrap();
    let installer =
        Project::open_for_install(&project, Runtime::content_only().engine_version).unwrap();
    installer.install(std::slice::from_ref(&source)).unwrap();
    (source, project)
}

fn assert_failure(arguments: &[String], code: i32, output: &Path) {
    let result = run(arguments);
    assert_eq!(result.status.code(), Some(code), "args: {arguments:?}");
    assert!(!output.exists(), "failed cook created {}", output.display());
}

fn model_runtime_project(project_root: &Path) -> Project {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    Project::open(project_root, runtime).unwrap()
}

fn installed_model(project_root: &Path) -> PathBuf {
    let package_digest =
        Project::open_for_install(project_root, Runtime::content_only().engine_version)
            .unwrap()
            .list()
            .unwrap()
            .packages[PACKAGE]
            .digest
            .clone();
    project_root
        .join(".orr/packages/objects")
        .join(package_digest)
        .join(MODEL_ASSET)
}

#[test]
fn cooks_verified_installed_model_without_source_extraction_or_project_changes() {
    let directory = tempfile::tempdir().unwrap();
    let (source, project_root) = install_fixture(directory.path(), "valid", &["models"]);
    let source_output = directory.path().join("source.orrmesh");
    let project_output = directory.path().join("installed.orrmesh");
    let source_result = run(&source_arguments(&fixture_model(), &source_output));
    assert!(
        source_result.status.success(),
        "{}",
        String::from_utf8_lossy(&source_result.stderr)
    );

    let project = model_runtime_project(&project_root);
    let package_lock = fs::read(project_root.join("orr.packages.lock.json")).unwrap();
    let installed_bytes = project.read_asset(PACKAGE, MODEL_ASSET).unwrap();
    assert_eq!(installed_bytes, fs::read(source.join(MODEL_ASSET)).unwrap());
    fs::rename(&source, directory.path().join("hidden source")).unwrap();
    assert!(!source.exists());

    let project_result = run(&project_arguments(
        &project_root,
        MODEL_ASSET,
        &project_output,
    ));
    assert!(
        project_result.status.success(),
        "{}",
        String::from_utf8_lossy(&project_result.stderr)
    );
    assert_eq!(
        fs::read(&project_output).unwrap(),
        fs::read(&source_output).unwrap()
    );
    assert_eq!(
        fs::read(project_root.join("orr.packages.lock.json")).unwrap(),
        package_lock
    );
    assert_eq!(
        project.read_asset(PACKAGE, MODEL_ASSET).unwrap(),
        installed_bytes
    );
    assert_eq!(project.verify().unwrap().packages.len(), 1);
}

#[test]
fn project_mode_rejects_invalid_arguments_before_creating_output() {
    let directory = tempfile::tempdir().unwrap();
    let (_, project) = install_fixture(directory.path(), "args", &["models"]);
    let output = directory.path().join("invalid.orrmesh");
    let source = fixture_model();

    let mut mixed = project_arguments(&project, MODEL_ASSET, &output);
    mixed.extend(["--source".into(), source.display().to_string()]);
    let incomplete = vec![
        "--project".into(),
        project.display().to_string(),
        "--package".into(),
        PACKAGE.into(),
        "--output".into(),
        output.display().to_string(),
    ];
    let relative = project_arguments(Path::new("relative-project"), MODEL_ASSET, &output);

    for arguments in [mixed, incomplete, relative] {
        assert_failure(&arguments, 2, &output);
    }
}

#[test]
fn project_mode_rejects_undeclared_missing_and_tampered_assets_without_output() {
    let directory = tempfile::tempdir().unwrap();
    let (_, project_root) = install_fixture(directory.path(), "asset", &["models"]);
    let output = directory.path().join("rejected.orrmesh");
    let undeclared = project_arguments(&project_root, "undeclared.glb", &output);
    assert_failure(&undeclared, 1, &output);

    let installed_model = installed_model(&project_root);
    let original = fs::read(&installed_model).unwrap();
    fs::remove_file(&installed_model).unwrap();
    let missing = project_arguments(&project_root, MODEL_ASSET, &output);
    assert_failure(&missing, 1, &output);
    fs::write(&installed_model, &original).unwrap();

    fs::write(&installed_model, b"tampered installed model").unwrap();
    let tampered = project_arguments(&project_root, MODEL_ASSET, &output);
    assert_failure(&tampered, 1, &output);
}

#[test]
fn project_mode_requires_a_known_models_capability() {
    let directory = tempfile::tempdir().unwrap();
    let (_, missing_models_project) = install_fixture(directory.path(), "no-models", &[]);
    let (_, unknown_capability_project) =
        install_fixture(directory.path(), "unknown-capability", &["mystery"]);
    let output = directory.path().join("rejected.orrmesh");

    let missing_models = project_arguments(&missing_models_project, MODEL_ASSET, &output);
    assert_failure(&missing_models, 1, &output);
    let unknown_capability = project_arguments(&unknown_capability_project, MODEL_ASSET, &output);
    assert_failure(&unknown_capability, 1, &output);
}
