//! Differential legacy Python/native versus in-process Rust package authoring.
//! Explicit actual binaries; Python is a test baseline, not a Rust product dependency.
#![cfg(target_os = "linux")]
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
    process::{Command, Output, Stdio},
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
            "orr-rust-assembled-package-cli-{}-{}",
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
#[derive(Clone, Copy, Debug)]
enum Frontend {
    Legacy,
    Rust,
}
fn required_binary(name: &str) -> std::ffi::OsString {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} required"));
    assert!(Path::new(&value).is_absolute() && Path::new(&value).is_file());
    value
}
fn command(
    frontend: Frontend,
    recipe: &Path,
    license: &Path,
    output: &Path,
    name: &str,
) -> Command {
    let mut command = match frontend {
        Frontend::Legacy => {
            let mut c = Command::new("python3");
            c.arg("-B")
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tools/create_assembled_model_package.py"),
                )
                .arg("--assembler")
                .arg(required_binary("ORR_ASSEMBLE_BIN"))
                .arg("--package-source")
                .arg(required_binary("ORR_PACKAGE_SOURCE_BIN"));
            c
        }
        Frontend::Rust => {
            let mut c = Command::new(required_binary("ORR_RUST_PACKAGE_BIN"));
            c.arg("create-assembled");
            c
        }
    };
    command
        .arg("--recipe")
        .arg(recipe)
        .args(["--asset-id", ID])
        .arg("--license")
        .arg(license)
        .args(["--name", name, "--version", "1.0.0", "--engine", "^0.0.1"])
        .arg("--output")
        .arg(output);
    command
}
#[expect(
    clippy::disallowed_types,
    reason = "bounded actual CLI regression timeout catches FIFO hangs"
)]
fn execute(mut command: Command) -> Output {
    use std::time::{Duration, Instant};
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "authoring child timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn invoke(recipe: &Path, license: &Path, output: &Path) -> Output {
    execute(command(Frontend::Rust, recipe, license, output, PACKAGE))
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
#[ignore = "requires ORR_RUST_PACKAGE_BIN, ORR_ASSEMBLE_BIN and ORR_PACKAGE_SOURCE_BIN actual binaries"]
fn rust_and_python_packages_are_exact_then_install_and_reopen_without_sources() {
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
    let legacy = scratch.0.join("legacy-package");
    let legacy_result = execute(command(
        Frontend::Legacy,
        &recipe_path,
        &license,
        &legacy,
        PACKAGE,
    ));
    assert_eq!(
        String::from_utf8_lossy(&legacy_result.stdout).trim(),
        legacy.to_str().unwrap()
    );
    successful(legacy_result);
    let result = invoke(&recipe_path, &license, &package);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        package.to_str().unwrap()
    );
    successful(result);
    assert_eq!(
        snapshot(&package),
        snapshot(&legacy),
        "all three authored file bytes must match"
    );
    assert_eq!(
        orr_package::inspect_for_install(&package, Runtime::content_only().engine_version).unwrap(),
        orr_package::inspect_for_install(&legacy, Runtime::content_only().engine_version).unwrap()
    );
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
    fs::rename(&legacy, scratch.0.join("hidden-legacy-package")).unwrap();
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

struct InputFixture {
    scratch: Scratch,
    inputs: PathBuf,
    recipe: PathBuf,
    license: PathBuf,
}
impl InputFixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let inputs = scratch.0.join("inputs");
        fs::create_dir(&inputs).unwrap();
        fs::write(inputs.join("original.glb"), GLB).unwrap();
        let model = orr_model::import::import_path(&inputs, "original.glb").unwrap();
        fs::write(inputs.join("part.model.json"), model.to_bytes().unwrap()).unwrap();
        let recipe = inputs.join("assembly.json");
        fs::write(
            &recipe,
            serde_json::to_vec(&json!({
                "format":"orr_static_model_assemble", "version":1,
                "parts":[{"key":"origin","source":"part.model.json","placement":IDENTITY}]
            }))
            .unwrap(),
        )
        .unwrap();
        let license = inputs.join("LICENSE.txt");
        fs::write(
            &license,
            b"User supplied fixture license; no rights inference.\n",
        )
        .unwrap();
        Self {
            scratch,
            inputs,
            recipe,
            license,
        }
    }
    fn names(&self) -> std::collections::BTreeSet<std::ffi::OsString> {
        fs::read_dir(&self.scratch.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect()
    }
    fn rejects(&self, recipe: &Path, license: &Path, name: &str) {
        let before = self.names();
        for (frontend, label) in [
            (Frontend::Legacy, "legacy-refused"),
            (Frontend::Rust, "rust-refused"),
        ] {
            let output = self.scratch.0.join(label);
            let result = execute(command(frontend, recipe, license, &output, name));
            assert!(
                !result.status.success(),
                "{frontend:?} unexpectedly accepted invalid input"
            );
            assert_eq!(
                result.status.code(),
                Some(1),
                "input error, not parser/crash: {frontend:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                fs::symlink_metadata(&output).is_err(),
                "failed authoring published output"
            );
            assert_eq!(
                self.names(),
                before,
                "owned stage must not leak after rejection"
            );
        }
    }
}

#[test]
#[ignore = "requires actual Rust and legacy native authoring binaries"]
fn differential_malformed_bounded_input_and_metadata_rejections() {
    let fixture = InputFixture::new();
    let original = fs::read(&fixture.recipe).unwrap();
    let mut duplicate = b"{\"version\":1,".to_vec();
    duplicate.extend_from_slice(&original[1..]);
    for bad in [b"{}".to_vec(), duplicate, vec![b' '; 16 * 1024 + 1]] {
        fs::write(&fixture.recipe, &bad).unwrap();
        fixture.rejects(&fixture.recipe, &fixture.license, PACKAGE);
        assert_eq!(fs::read(&fixture.recipe).unwrap(), bad);
    }
    fs::write(&fixture.recipe, &original).unwrap();
    let license = fs::read(&fixture.license).unwrap();
    for bad in [vec![0xff], b" \n\t".to_vec(), vec![b'x'; 1024 * 1024 + 1]] {
        fs::write(&fixture.license, &bad).unwrap();
        fixture.rejects(&fixture.recipe, &fixture.license, PACKAGE);
        assert_eq!(fs::read(&fixture.license).unwrap(), bad);
    }
    fs::write(&fixture.license, license).unwrap();
    fixture.rejects(&fixture.recipe, &fixture.license, ".");
    fixture.rejects(&fixture.recipe, &fixture.license, &"x".repeat(4097));
    let part = fixture.inputs.join("part.model.json");
    let source = fs::read(&part).unwrap();
    fs::write(&part, b"{\"format\":\"wrong\"}").unwrap();
    fixture.rejects(&fixture.recipe, &fixture.license, PACKAGE);
    fs::write(&part, &source).unwrap();
    // Sparse file tests byte admission before a model decoder allocation.
    let oversized = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&part)
        .unwrap();
    oversized.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(oversized);
    fixture.rejects(&fixture.recipe, &fixture.license, PACKAGE);
    assert_eq!(fs::metadata(part).unwrap().len(), 64 * 1024 * 1024 + 1);
}

#[test]
#[ignore = "requires actual Rust and legacy native authoring binaries; Linux path contracts"]
fn differential_unsafe_paths_and_existing_outputs_preserve_foreign_content() {
    use std::os::unix::fs::symlink;
    let fixture = InputFixture::new();
    let traversal = fixture.inputs.join("../inputs/assembly.json");
    fixture.rejects(&traversal, &fixture.license, PACKAGE);
    let internal = fixture.inputs.join(".orr");
    fs::create_dir(&internal).unwrap();
    fs::copy(&fixture.recipe, internal.join("assembly.json")).unwrap();
    fixture.rejects(&internal.join("assembly.json"), &fixture.license, PACKAGE);
    let linked_recipe = fixture.inputs.join("recipe-link.json");
    symlink(&fixture.recipe, &linked_recipe).unwrap();
    fixture.rejects(&linked_recipe, &fixture.license, PACKAGE);
    let linked_license = fixture.inputs.join("license-link.txt");
    symlink(&fixture.license, &linked_license).unwrap();
    fixture.rejects(&fixture.recipe, &linked_license, PACKAGE);
    let linked_parent = fixture.scratch.0.join("input-link");
    symlink(&fixture.inputs, &linked_parent).unwrap();
    fixture.rejects(
        &linked_parent.join("assembly.json"),
        &fixture.license,
        PACKAGE,
    );
    let fifo = fixture.inputs.join("recipe.fifo");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    fixture.rejects(&fifo, &fixture.license, PACKAGE);
    fixture.rejects(&fixture.recipe, &fifo, PACKAGE);
    let preserved = fixture.scratch.0.join("existing-dir");
    fs::create_dir(&preserved).unwrap();
    fs::write(preserved.join("sentinel"), b"preserve existing directory").unwrap();
    let regular = fixture.scratch.0.join("existing-file");
    fs::write(&regular, b"preserve existing file").unwrap();
    let dangling = fixture.scratch.0.join("dangling");
    symlink("absent-target", &dangling).unwrap();
    let linked = fixture.scratch.0.join("linked-output");
    symlink(&preserved, &linked).unwrap();
    let names = fixture.names();
    for target in [&preserved, &regular, &dangling, &linked] {
        for frontend in [Frontend::Legacy, Frontend::Rust] {
            let result = execute(command(
                frontend,
                &fixture.recipe,
                &fixture.license,
                target,
                PACKAGE,
            ));
            assert!(!result.status.success(), "must refuse existing {target:?}");
            assert_eq!(
                fs::read(preserved.join("sentinel")).unwrap(),
                b"preserve existing directory"
            );
            assert_eq!(fs::read(&regular).unwrap(), b"preserve existing file");
            assert_eq!(
                fs::read_link(&dangling).unwrap(),
                Path::new("absent-target")
            );
            assert_eq!(fs::read_link(&linked).unwrap(), preserved);
            assert_eq!(fixture.names(), names);
        }
    }
}

#[test]
#[ignore = "requires actual Rust package binary; deliberate frontend compatibility difference"]
fn rust_rejects_obsolete_external_executable_flags_without_execution() {
    let fixture = InputFixture::new();
    let script = fixture.scratch.0.join("would-execute.sh");
    let marker = fixture.scratch.0.join("executed-marker");
    fs::write(
        &script,
        format!("#!/bin/sh\nprintf touched > '{}'\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let before = fixture.names();
    for flag in ["--assembler", "--package-source", "--sequencer"] {
        let output = fixture.scratch.0.join("refused-obsolete");
        let mut cmd = command(
            Frontend::Rust,
            &fixture.recipe,
            &fixture.license,
            &output,
            PACKAGE,
        );
        cmd.arg(flag).arg(&script);
        let result = execute(cmd);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            !marker.exists(),
            "obsolete external tools must never execute"
        );
        assert!(!output.exists());
        assert_eq!(fixture.names(), before);
    }
}
