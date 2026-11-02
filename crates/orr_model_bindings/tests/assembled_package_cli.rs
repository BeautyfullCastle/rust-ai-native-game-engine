//! Actual authoring CLIs -> installed self-contained static prop -> saved binding.
//! No GPU, animation, or license-rights verification is claimed.
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Wrap, IDENTITY};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

const PACKAGE: &str = "assembled-prop";
const ASSET: &str = "model.model.json";
const ID: &str = "props/assembled.assemble.json";
const GLB: &[u8] = include_bytes!("../../orr_model/tests/fixtures/model.glb");

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-assembled-package-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("new exclusively owned test directory");
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, result);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn invoke(recipe: &Path, license: &Path, output: &Path) -> Output {
    let assembler = std::env::var_os("ORR_ASSEMBLE_BIN")
        .expect("set ORR_ASSEMBLE_BIN to the explicitly built native assembler");
    let package_source = std::env::var_os("ORR_PACKAGE_SOURCE_BIN")
        .expect("set ORR_PACKAGE_SOURCE_BIN to the explicitly built native package author");
    assert!(Path::new(&assembler).is_absolute());
    assert!(Path::new(&package_source).is_absolute());
    Command::new("python3")
        .arg("-B")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/create_assembled_model_package.py"),
        )
        .arg("--recipe")
        .arg(recipe)
        .args(["--asset-id", ID])
        .arg("--license")
        .arg(license)
        .args([
            "--name",
            PACKAGE,
            "--version",
            "1.0.0",
            "--engine",
            "^0.0.1",
        ])
        .arg("--assembler")
        .arg(assembler)
        .arg("--package-source")
        .arg(package_source)
        .arg("--output")
        .arg(output)
        .output()
        .expect("run real authoring frontend")
}
fn successful(output: Output) {
    assert!(
        output.status.success(),
        "authoring failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn assert_assembly(model: &StaticModel, input: &StaticModel, recipe: &[u8], cooked_input: &[u8]) {
    let source = model.source();
    let original = input.source();
    assert_eq!(source.asset_id, ID);
    assert_eq!(source.images, original.images);
    assert_eq!(source.materials, original.materials);
    assert_eq!(source.dependencies.len(), 2);
    assert_eq!(source.dependencies[0].uri, "$source");
    assert_eq!(source.dependencies[0].sha256, digest(recipe));
    assert_eq!(source.dependencies[1].uri, "part.model.json");
    assert_eq!(source.dependencies[1].sha256, digest(cooked_input));
    assert_eq!(source.primitives.len(), original.primitives.len() * 2);
    for (part, key) in ["origin", "shifted"].into_iter().enumerate() {
        for (index, primitive) in original.primitives.iter().enumerate() {
            let result = &source.primitives[part * original.primitives.len() + index];
            let suffix = primitive.id.strip_prefix(&original.asset_id).unwrap();
            assert_eq!(result.id, format!("{ID}#node=part:{key}{suffix}"));
            assert_eq!(result.vertices, primitive.vertices);
            assert_eq!(result.indices, primitive.indices);
            assert_eq!(result.material, primitive.material);
            let mut expected = primitive.transform;
            if part == 1 {
                // Independent affine-translation expectation, not assembler helper reuse.
                for column in &mut expected {
                    column[0] += 2.0 * column[3];
                }
            }
            assert_eq!(result.transform, expected);
        }
    }
}

#[test]
#[ignore = "requires explicitly built native authoring CLIs"]
fn actual_assembled_package_cli_installs_and_reopens_without_sources() {
    let scratch = Scratch::new();
    let inputs = scratch.0.join("inputs");
    fs::create_dir(&inputs).unwrap();
    fs::write(inputs.join("original.glb"), GLB).unwrap();
    let imported = orr_model::import::import_path(&inputs, "original.glb").unwrap();
    let mut source = imported.source().clone();
    assert!(!source.materials.is_empty());
    source.materials[0].base_color = [0.25, 0.5, 0.75, 1.0];
    source.materials[0].linear_filter = false;
    source.materials[0].wrap_s = Wrap::Repeat;
    let input = StaticModel::new(source).unwrap();
    let input_bytes = input.to_bytes().unwrap();
    fs::write(inputs.join("part.model.json"), &input_bytes).unwrap();
    let mut shifted = IDENTITY;
    shifted[3][0] = 2.0;
    let recipe = serde_json::to_vec_pretty(&json!({
        "format":"orr_static_model_assemble", "version":1,
        "parts":[
            {"key":"origin","source":"part.model.json","placement":IDENTITY},
            {"key":"shifted","source":"part.model.json","placement":shifted}
        ]
    }))
    .unwrap();
    let recipe_path = inputs.join("assembly.json");
    fs::write(&recipe_path, &recipe).unwrap();
    let license = inputs.join("LICENSE.txt");
    fs::write(
        &license,
        "Integration fixture; no rights assessment is performed.\n",
    )
    .unwrap();
    let original_inputs = snapshot(&inputs);
    let package = scratch.0.join("package");
    successful(invoke(&recipe_path, &license, &package));
    assert_eq!(snapshot(&inputs), original_inputs);
    let package_bytes = snapshot(&package);
    assert_eq!(
        package_bytes.keys().cloned().collect::<Vec<_>>(),
        ["LICENSE.txt", ASSET, "orr.package.json"].map(PathBuf::from)
    );
    let cooked = fs::read(package.join(ASSET)).unwrap();
    let model = StaticModel::from_bytes(&cooked).unwrap();
    assert_eq!(model.to_bytes().unwrap(), cooked);
    assert_assembly(&model, &input, &recipe, &input_bytes);
    assert!(!invoke(&recipe_path, &license, &package).status.success());
    assert_eq!(snapshot(&package), package_bytes);
    let malformed = inputs.join("invalid.json");
    fs::write(&malformed, b"{\"format\":\"wrong\"}").unwrap();
    let invalid_output = scratch.0.join("invalid-output");
    assert!(!invoke(&malformed, &license, &invalid_output)
        .status
        .success());
    assert!(!invalid_output.exists());

    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package))
        .unwrap();
    fs::rename(&inputs, scratch.0.join("hidden-inputs")).unwrap();
    fs::rename(&package, scratch.0.join("hidden-package")).unwrap();
    let project = model_bindings::open_project(&root).unwrap();
    let lock = project.verify().unwrap();
    let locked = &lock.packages[PACKAGE];
    assert_eq!(locked.files.len(), 2);
    assert_eq!(locked.files[ASSET], digest(&cooked));
    let loaded =
        model_bindings::load_asset_for_kind(&root, PACKAGE, ASSET, ModelKind::Static).unwrap();
    assert_eq!(loaded.source_hash(), digest(&cooked));
    assert_eq!(loaded.package_digest(), locked.digest);
    assert_eq!(loaded.static_model().unwrap().to_bytes().unwrap(), cooked);
    assert_assembly(
        loaded.static_model().unwrap(),
        &input,
        &recipe,
        &input_bytes,
    );
    let binding = Binding::from_asset(
        PACKAGE.into(),
        ASSET.into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    binding.validate(&loaded).unwrap();
    let document = Document {
        version: 2,
        scene: "scene.yaml".into(),
        project: ".".into(),
        bindings: BTreeMap::from([("e_00000001".into(), binding)]),
    };
    document.validate().unwrap();
    let sidecar = root.join("scene.models.json");
    fs::write(&sidecar, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    drop(loaded);
    drop(project);
    let reopened: Document = serde_json::from_slice(&fs::read(sidecar).unwrap()).unwrap();
    reopened.validate().unwrap();
    assert_eq!(reopened, document);
    let rebound = model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).unwrap();
    assert_eq!(rebound.source_hash(), digest(&cooked));
    assert_eq!(rebound.package_digest(), locked.digest);
    assert_eq!(rebound.static_model().unwrap().to_bytes().unwrap(), cooked);
}
