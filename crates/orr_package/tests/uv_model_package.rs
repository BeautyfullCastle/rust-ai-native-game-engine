#![cfg(all(feature = "model-authoring", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]
use orr_model::StaticModel;
use orr_package::{
    uv_model_package::{self, Metadata, Options},
    Project, Runtime,
};
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    input: PathBuf,
    options: Options,
    source: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let input = root.join("inputs");
        fs::create_dir(&input).unwrap();
        let model = orr_model::import::import_with_resolver(
            "props/original.glb",
            include_bytes!("../../orr_model/tests/fixtures/model.glb"),
            |_| panic!("embedded fixture"),
        )
        .unwrap();
        let source = model.to_bytes().unwrap();
        fs::write(input.join("prop.model.json"), &source).unwrap();
        let recipe = input.join("tile.uv.json");
        fs::write(&recipe,serde_json::to_vec(&json!({"format":"orr_static_material_uv","version":1,"source":"prop.model.json","material":0,"offset":[0.25,-0.5],"scale":[2.0,0.5],"quarter_turns":1})).unwrap()).unwrap();
        let license = input.join("LICENSE.txt");
        fs::write(&license, b"Original fixture license\n").unwrap();
        let options = Options {
            recipe,
            asset_id: "variants/tile.uv.json".into(),
            license,
            metadata: Metadata {
                name: "uv-tile".into(),
                version: "1.0.0".into(),
                engine: "*".into(),
            },
            output: root.join("package"),
        };
        Self {
            _temp: temp,
            root,
            input,
            options,
            source,
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_orr_pkg"));
        cmd.arg("create-uv-model")
            .arg("--recipe")
            .arg(&self.options.recipe)
            .arg("--asset-id")
            .arg(&self.options.asset_id)
            .arg("--license")
            .arg(&self.options.license)
            .args(["--name", "uv-tile", "--version", "1.0.0", "--engine", "*"])
            .arg("--output")
            .arg(&self.options.output);
        cmd
    }
}
#[test]
fn cli_creates_installs_and_loads_source_hidden_uv_model() {
    let f = Fixture::new();
    let output = f.command().output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        f.options.output.to_str().unwrap()
    );
    let mut files: Vec<_> = fs::read_dir(&f.options.output)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["LICENSE.txt", "model.model.json", "orr.package.json"].map(std::ffi::OsString::from)
    );
    assert_eq!(
        fs::read(f.options.output.join("LICENSE.txt")).unwrap(),
        b"Original fixture license\n"
    );
    let expected =
        orr_model::static_uv::transform_recipe(&f.options.recipe, &f.options.asset_id).unwrap();
    assert_eq!(
        fs::read(f.options.output.join("model.model.json")).unwrap(),
        expected
    );
    let project_root = f.root.join("game");
    fs::create_dir(&project_root).unwrap();
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(&project_root, runtime.clone()).unwrap();
    project
        .install(std::slice::from_ref(&f.options.output))
        .unwrap();
    fs::rename(&f.input, f.root.join("hidden-inputs")).unwrap();
    fs::rename(&f.options.output, f.root.join("hidden-package")).unwrap();
    let project = Project::open(&project_root, runtime).unwrap();
    project.verify().unwrap();
    let loaded =
        StaticModel::from_bytes(&project.read_asset("uv-tile", "model.model.json").unwrap())
            .unwrap();
    assert_eq!(loaded.to_bytes().unwrap(), expected);
    let original = StaticModel::from_bytes(&f.source).unwrap();
    assert_eq!(loaded.source().images, original.source().images);
    assert_eq!(loaded.source().materials, original.source().materials);
    for (actual, old) in loaded
        .source()
        .primitives
        .iter()
        .zip(&original.source().primitives)
    {
        assert_eq!(actual.indices, old.indices);
        assert_eq!(actual.transform, old.transform);
        let selected = old.material == 0;
        for (actual, old) in actual.vertices.iter().zip(&old.vertices) {
            assert_eq!(actual.position, old.position);
            assert_eq!(actual.normal, old.normal);
            if selected {
                assert_eq!(actual.uv, [-old.uv[1] * 0.5 + 0.25, old.uv[0] * 2.0 - 0.5]);
            } else {
                assert_eq!(actual.uv, old.uv);
            }
        }
    }
    assert_ne!(
        loaded.source().primitives[0].vertices,
        original.source().primitives[0].vertices
    );
}
#[test]
fn rejects_bad_recipe_metadata_license_without_partial_output() {
    let f = Fixture::new();
    let original = fs::read(&f.options.recipe).unwrap();
    for bytes in [b"{}".to_vec(), b"{\"format\":null}".to_vec()] {
        fs::write(&f.options.recipe, bytes).unwrap();
        assert!(uv_model_package::create(&f.options).is_err());
        assert!(!f.options.output.exists());
    }
    fs::write(&f.options.recipe, original).unwrap();
    let mut options = f.options.clone();
    options.metadata.version = "invalid".into();
    assert!(uv_model_package::create(&options).is_err());
    assert!(!options.output.exists());
    fs::write(&options.license, b" \n").unwrap();
    assert!(uv_model_package::create(&options).is_err());
    assert!(!options.output.exists());
    assert!(!fs::read_dir(&f.root).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr")));
}
#[test]
fn existing_file_directory_and_dangling_link_are_never_replaced() {
    let f = Fixture::new();
    fs::write(&f.options.output, b"retain").unwrap();
    assert!(uv_model_package::create(&f.options).is_err());
    assert_eq!(fs::read(&f.options.output).unwrap(), b"retain");
    let mut options = f.options.clone();
    options.output = f.root.join("existing-directory");
    fs::create_dir(&options.output).unwrap();
    fs::write(options.output.join("keep"), b"retain").unwrap();
    assert!(uv_model_package::create(&options).is_err());
    assert_eq!(fs::read(options.output.join("keep")).unwrap(), b"retain");
    options.output = f.root.join("alias");
    std::os::unix::fs::symlink(f.root.join("absent"), &options.output).unwrap();
    assert!(uv_model_package::create(&options).is_err());
    assert!(fs::symlink_metadata(&options.output)
        .unwrap()
        .file_type()
        .is_symlink());
}
#[test]
fn linked_and_corrupt_source_reject_without_output() {
    let f = Fixture::new();
    fs::rename(
        f.input.join("prop.model.json"),
        f.input.join("actual.model.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink("actual.model.json", f.input.join("prop.model.json")).unwrap();
    assert!(uv_model_package::create(&f.options).is_err());
    assert!(!f.options.output.exists());
    let mut options = f.options.clone();
    options.recipe = f.input.join("corrupt.uv.json");
    fs::write(&options.recipe, fs::read(&f.options.recipe).unwrap()).unwrap();
    let mut recipe: serde_json::Value =
        serde_json::from_slice(&fs::read(&options.recipe).unwrap()).unwrap();
    recipe["source"] = json!("broken.model.json");
    fs::write(&options.recipe, serde_json::to_vec(&recipe).unwrap()).unwrap();
    fs::write(f.input.join("broken.model.json"), b"invalid").unwrap();
    assert!(uv_model_package::create(&options).is_err());
    assert!(!options.output.exists());
}
#[test]
fn cli_refuses_external_tools_duplicate_options_and_describes_help() {
    let f = Fixture::new();
    for extra in [["--transformer", "never-run"], ["--name", "duplicate"]] {
        let out = f.command().args(extra).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(!f.options.output.exists());
    }
    let out = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["create-uv-model", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .contains("ID.uv.json"));
}
