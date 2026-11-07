#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use crate::package_manager::{open_room_project, room_runtime};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Manifest, Project};
use orr_sample::{
    room_game::{KEY, PLAYER, RoomActor, RoomConfig, RoomEscapeV1},
    room_project,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
const TARGET: &str = "upgrade-art";
const MODEL: &str = "model.model.json";
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for item in fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            let m = fs::symlink_metadata(&path).unwrap();
            assert!(!m.file_type().is_symlink());
            if m.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    v1: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let root = temp.path().join("room");
        fs::create_dir(&root).unwrap();
        let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
        let scene = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &scene).unwrap();
        fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
        fs::write(
            root.join("room.camera.json"),
            orr_sample::room_camera::Document::readable_default()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        let base = orr_model::import::import_with_resolver(
            "fixture.glb",
            include_bytes!("../../../../orr_model/tests/fixtures/model.glb"),
            |_| panic!("embedded fixture"),
        )
        .unwrap();
        let v1 = temp.path().join("v1");
        let source = temp.path().join("v2");
        for (path, version, rgb) in [
            (&v1, "1.0.0", [1., 1., 1.]),
            (&source, "2.0.0", [0.2, 0.7, 0.4]),
        ] {
            fs::create_dir(path).unwrap();
            let mut model = base.source().clone();
            model.materials[0].base_color[..3].copy_from_slice(&rgb);
            fs::write(
                path.join(MODEL),
                orr_model::StaticModel::new(model)
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
            )
            .unwrap();
            fs::write(
                path.join("LICENSE.txt"),
                b"Fixture rights require explicit review",
            )
            .unwrap();
            let manifest = Manifest {
                schema: 1,
                name: TARGET.into(),
                version: version.into(),
                engine: "*".into(),
                capabilities: BTreeSet::from(["models".into()]),
                dependencies: BTreeMap::new(),
                files: BTreeSet::from([MODEL.into(), "LICENSE.txt".into()]),
            };
            fs::write(
                path.join("orr.package.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        Project::open(&root, room_runtime())
            .unwrap()
            .install(std::slice::from_ref(&v1))
            .unwrap();
        let project = Project::open(&root, room_runtime()).unwrap();
        let asset = model_bindings::load_asset_from_project(
            &project,
            TARGET,
            MODEL,
            model_bindings::ModelKind::Static,
        )
        .unwrap();
        let prepared = room_project::PreparedScene::parse(&scene).unwrap();
        let mut bindings = BTreeMap::new();
        for entity in prepared.frame().entities().filter(|e| {
            prepared
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| [PLAYER, KEY].contains(&a.kind))
        }) {
            let mut transform = LocalTransform::default();
            transform.translation = [0.5, 0.25, -0.5];
            transform.scale = [0.75, 1.25, 1.];
            bindings.insert(
                prepared.index().guid(entity).unwrap().to_string(),
                Binding::from_asset(TARGET.into(), MODEL.into(), &asset, transform).unwrap(),
            );
        }
        assert_eq!(bindings.len(), 2);
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings,
            })
            .unwrap(),
        )
        .unwrap();
        open_room_project(&root).unwrap();
        Self {
            temp,
            root,
            source,
            v1,
        }
    }
    fn output(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
}
#[test]
fn new_copy_rebinds_all_selected_guids_preserves_gameplay_and_survives_originals_hidden() {
    let f = Fixture::new();
    let original = files(&f.root);
    let source = files(&f.source);
    let old = open_room_project(&f.root).unwrap();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    assert_eq!(files(&f.root), original);
    let output = f.output("updated");
    let report = apply_copy(&plan, &output, true).unwrap();
    assert_eq!(report.bindings.len(), 2);
    assert_eq!(report.before_version, "1.0.0");
    assert_eq!(report.after_version, "2.0.0");
    assert_eq!(files(&f.root), original);
    assert_eq!(files(&f.source), source);
    let copied = open_room_project(&output).unwrap();
    assert_eq!(
        old.scene().frame().to_bytes(),
        copied.scene().frame().to_bytes()
    );
    assert_eq!(
        fs::read(output.join("room.camera.json")).unwrap(),
        original[Path::new("room.camera.json")]
    );
    for (guid, binding) in &copied.models().document.bindings {
        let prior = &old.models().document.bindings[guid];
        assert_eq!(binding.transform, prior.transform);
        assert_eq!(binding.kind, prior.kind);
        assert_eq!(binding.asset, prior.asset);
        assert_ne!(binding.package_digest, prior.package_digest);
        assert_ne!(binding.source_hash, prior.source_hash);
        let asset = &copied.models().assets[&(TARGET.into(), MODEL.into())];
        binding.validate(asset).unwrap();
        assert_eq!(
            asset.static_model().unwrap().source().materials[0].base_color[..3],
            [0.2, 0.7, 0.4]
        );
    }
    let mut expected = old.scene().simulation().unwrap();
    let mut actual = copied.scene().simulation().unwrap();
    for tick in 1..=8 {
        let inputs = orr_sim::TickInputs::new(tick, 1);
        expected.step(&inputs);
        actual.step(&inputs);
        assert_eq!(expected.frame().checksum(), actual.frame().checksum());
    }
    drop(copied);
    drop(old);
    fs::rename(&f.root, f.output("hidden-original")).unwrap();
    fs::rename(&f.source, f.output("hidden-source")).unwrap();
    fs::rename(&f.v1, f.output("hidden-v1")).unwrap();
    let reopened = open_room_project(&output).unwrap();
    assert_eq!(reopened.models().document.bindings.len(), 2);
}
#[test]
fn consent_and_strict_arguments_reject_before_input_or_output_access() {
    assert!(
        run([
            "--project",
            "/missing-room",
            "--source",
            "/missing-source",
            "--output",
            "/missing-output"
        ]
        .into_iter()
        .map(Into::into))
        .is_err()
    );
    assert!(
        run([
            "--project",
            "/missing-room",
            "--project",
            "/missing-source",
            "--output",
            "/missing-output",
            "--acknowledge-model-changes"
        ]
        .into_iter()
        .map(Into::into))
        .unwrap_err()
        .contains("duplicate")
    );
    let f = Fixture::new();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let output = f.output("no-consent");
    assert!(apply_copy(&plan, &output, false).is_err());
    assert!(!output.exists());
}
#[test]
#[ignore = "requires explicitly built ORR_ROOM_PACKAGE_UPGRADE_TOOL"]
fn actual_cli_new_copy_success_and_collision_preserve_originals() {
    let executable = std::env::var_os("ORR_ROOM_PACKAGE_UPGRADE_TOOL").expect("built CLI required");
    let f = Fixture::new();
    let output = f.output("cli-output");
    let before = files(&f.root);
    let call = || {
        std::process::Command::new(&executable)
            .arg("--project")
            .arg(&f.root)
            .arg("--source")
            .arg(&f.source)
            .arg("--output")
            .arg(&output)
            .arg("--acknowledge-model-changes")
            .output()
            .unwrap()
    };
    let first = call();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["bindings"].as_array().unwrap().len(), 2);
    assert_eq!(files(&f.root), before);
    let copied = files(&output);
    let second = call();
    assert!(!second.status.success());
    assert!(second.stdout.is_empty());
    assert_eq!(files(&output), copied);
    assert_eq!(files(&f.root), before);
    open_room_project(&output).unwrap();
}
#[path = "adversarial_tests.rs"]
mod adversarial_tests;
