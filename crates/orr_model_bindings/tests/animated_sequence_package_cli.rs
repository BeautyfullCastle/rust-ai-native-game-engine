//! Real sequence authoring CLIs -> installed animated asset -> persisted clip binding.
//! CPU-only; no GPU, live crossfade, or license-rights verification is claimed.
#![cfg(feature = "animated-models")]
#![allow(clippy::float_arithmetic)]
#[allow(dead_code)]
#[path = "../../orr_model/tests/common/animation_clip_sequence_oracle.rs"]
mod oracle;

use orr_model::animation::AnimatedModel;
use orr_model_bindings::model_bindings::{
    self, AnimationDescriptor, Binding, Document, LocalTransform, ModelKind, PlaybackMode,
};
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

const PACKAGE: &str = "sequence-prop";
const ASSET: &str = "model.animated.json";
const ID: &str = "sequences/action.sequence.json";

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-sequence-package-cli-{}-{}",
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
    let assembler = std::env::var_os("ORR_SEQUENCE_BIN")
        .expect("set ORR_SEQUENCE_BIN to the explicitly built native sequencer");
    let package_source = std::env::var_os("ORR_PACKAGE_SOURCE_BIN")
        .expect("set ORR_PACKAGE_SOURCE_BIN to the explicitly built native package author");
    assert!(Path::new(&assembler).is_absolute());
    assert!(Path::new(&package_source).is_absolute());
    Command::new("python3")
        .arg("-B")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/create_animated_sequence_package.py"),
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
        .arg("--sequencer")
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
fn close(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 0.00005,
        "{actual} != {expected}"
    );
}
fn assert_sequence(
    model: &AnimatedModel,
    input: &AnimatedModel,
    recipe: &[u8],
    cooked_input: &[u8],
) {
    let source = model.source();
    let original = input.source();
    assert_eq!(source.asset_id, ID);
    assert_eq!(source.clips.len(), 3);
    assert_eq!(&source.clips[..2], original.clips);
    assert_eq!(source.clips[2].name, "joined");
    assert_eq!(source.clips[2].duration(), oracle::DURATION as f32);
    // Exact retained serialization covers rigs, node rest poses, vertex weights,
    // geometry, UVs, materials, images and the original clips/channels.
    let mut retained = source.clone();
    retained.asset_id = original.asset_id.clone();
    retained.dependencies = original.dependencies.clone();
    retained.clips.truncate(original.clips.len());
    assert_eq!(retained.primitives.len(), original.primitives.len());
    for (new, old) in retained.primitives.iter_mut().zip(&original.primitives) {
        assert_eq!(
            new.id,
            format!("{ID}#{}", old.id.split_once('#').unwrap().1)
        );
        new.id = old.id.clone();
    }
    assert_eq!(
        serde_json::to_vec(&retained).unwrap(),
        serde_json::to_vec(original).unwrap()
    );
    assert_eq!(source.dependencies.len(), 2);
    assert_eq!(source.dependencies[0].uri, "$source");
    assert_eq!(source.dependencies[0].sha256, digest(recipe));
    assert_eq!(source.dependencies[1].uri, "source.animated.json");
    assert_eq!(source.dependencies[1].sha256, digest(cooked_input));
    for time in [0., 0.625, 1.375, 2., 2.25, 2.5, 2.75, 3., 3.125, 4.375, 5.] {
        let pose = model.sample_clip(2, time as f32).unwrap();
        let locals = oracle::sequence_local(time);
        assert_eq!(pose.local().len(), locals.len());
        for (actual, expected) in pose.local().iter().zip(locals) {
            for i in 0..3 {
                close(actual.translation[i], expected.translation[i]);
                close(actual.scale[i], expected.scale[i]);
            }
            let dot: f64 = (0..4)
                .map(|i| f64::from(actual.rotation[i]) * expected.rotation[i])
                .sum();
            assert!((dot.abs() - 1.).abs() < 0.00001);
        }
        let mesh = model.deform(&pose).unwrap();
        assert_eq!(mesh.len(), 1);
        let expected = oracle::sequence_positions(time);
        assert_eq!(mesh[0].vertices.len(), expected.len());
        for (actual, expected) in mesh[0].vertices.iter().zip(expected) {
            for (actual, expected) in actual.position.into_iter().zip(expected) {
                close(actual, expected);
            }
        }
    }
}

#[test]
#[ignore = "requires explicitly built native authoring CLIs"]
fn actual_sequence_package_cli_installs_and_reopens_without_sources() {
    let scratch = Scratch::new();
    let inputs = scratch.0.join("inputs");
    fs::create_dir(&inputs).unwrap();
    let input = oracle::fixture();
    let input_bytes = input.to_bytes().unwrap();
    fs::write(inputs.join("source.animated.json"), &input_bytes).unwrap();
    let recipe = serde_json::to_vec_pretty(&json!({
        "format":"orr_animation_clip_sequence", "version":1,
        "source":"source.animated.json", "first_clip":0, "second_clip":1,
        "transition_seconds":1.0, "name":"joined"
    }))
    .unwrap();
    let recipe_path = inputs.join("join.sequence.json");
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
    let model = AnimatedModel::from_bytes(&cooked).unwrap();
    assert_eq!(model.to_bytes().unwrap(), cooked);
    assert_sequence(&model, &input, &recipe, &input_bytes);
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
    assert_eq!(
        locked
            .manifest
            .capabilities
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["animation", "models"]
    );
    assert_eq!(locked.files[ASSET], digest(&cooked));
    let loaded =
        model_bindings::load_asset_for_kind(&root, PACKAGE, ASSET, ModelKind::Animated).unwrap();
    assert_eq!(loaded.source_hash(), digest(&cooked));
    assert_eq!(loaded.package_digest(), locked.digest);
    assert_eq!(loaded.animated_model().unwrap().to_bytes().unwrap(), cooked);
    assert_sequence(
        loaded.animated_model().unwrap(),
        &input,
        &recipe,
        &input_bytes,
    );
    let binding = Binding::from_animated_asset(
        PACKAGE.into(),
        ASSET.into(),
        &loaded,
        AnimationDescriptor {
            clip_index: 2,
            playback: PlaybackMode::Once,
        },
        LocalTransform::default(),
    )
    .unwrap();
    binding.validate(&loaded).unwrap();
    assert_eq!(
        binding.animation,
        Some(AnimationDescriptor {
            clip_index: 2,
            playback: PlaybackMode::Once
        })
    );
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
    assert_eq!(
        rebound.animated_model().unwrap().to_bytes().unwrap(),
        cooked
    );
    assert_sequence(
        rebound.animated_model().unwrap(),
        &input,
        &recipe,
        &input_bytes,
    );
}
