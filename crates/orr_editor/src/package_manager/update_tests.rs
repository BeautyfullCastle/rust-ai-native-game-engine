//! Separate update inventory: no changes to legacy package test counts.
#![allow(clippy::disallowed_types)]
use super::{apply_update, plan_update, UpdatePlan};
use crate::package_manager::room_runtime;
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Manifest, Project, Runtime};
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject},
};
use orr_sim::Simulation;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const TARGET: &str = "update-target";
const USED: &str = "sample-imported-scene";

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for item in fs::read_dir(from).unwrap() {
        let item = item.unwrap();
        let ty = item.file_type().unwrap();
        assert!(!ty.is_symlink());
        if ty.is_dir() {
            copy_tree(&item.path(), &to.join(item.file_name()));
        } else {
            fs::copy(item.path(), to.join(item.file_name())).unwrap();
        }
    }
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for item in fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
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
fn manifest(path: &Path, edit: impl FnOnce(&mut Manifest)) {
    let p = path.join("orr.package.json");
    let mut value: Manifest = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    edit(&mut value);
    fs::write(p, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    v1: PathBuf,
    v2: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let root = temp.path().join("room");
        fs::create_dir(&root).unwrap();
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
        let scene = Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &scene).unwrap();
        fs::write(
            root.join("room.camera.json"),
            orr_sample::room_camera::Document::readable_default()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets")
            .canonicalize()
            .unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[assets.join("imported_scene_demo")])
            .unwrap();
        let loaded = model_bindings::load_asset(&root, USED, "foreground.glb").unwrap();
        let binding = Binding::from_asset(
            USED.into(),
            "foreground.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let prepared = room_project::PreparedScene::parse(&scene).unwrap();
        let bindings = prepared
            .frame()
            .entities()
            .filter(|e| {
                prepared
                    .frame()
                    .get::<RoomActor>(*e)
                    .is_some_and(|a| [PLAYER, KEY].contains(&a.kind))
            })
            .map(|e| {
                (
                    prepared.index().guid(e).unwrap().to_string(),
                    binding.clone(),
                )
            })
            .collect();
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
        let v1 = temp.path().join("source-v1");
        let v2 = temp.path().join("source-v2");
        for (path, version) in [(&v1, "1.0.0"), (&v2, "2.0.0")] {
            fs::create_dir(path).unwrap();
            fs::copy(
                assets.join("imported_scene_demo/foreground.glb"),
                path.join("model.glb"),
            )
            .unwrap();
            fs::write(
                path.join("LICENSE.txt"),
                format!("Owned fixture version {version}\n"),
            )
            .unwrap();
            let value = Manifest {
                schema: 1,
                name: TARGET.into(),
                version: version.into(),
                engine: "*".into(),
                capabilities: BTreeSet::from(["models".into()]),
                dependencies: BTreeMap::new(),
                files: BTreeSet::from(["model.glb".into(), "LICENSE.txt".into()]),
            };
            fs::write(
                path.join("orr.package.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
        Project::open(&root, room_runtime())
            .unwrap()
            .install(std::slice::from_ref(&v1))
            .unwrap();
        PreparedProject::open(&root).unwrap();
        Self { temp, root, v1, v2 }
    }
    fn project(&self) -> Project {
        Project::open(&self.root, room_runtime()).unwrap()
    }
    fn plan(&self) -> UpdatePlan {
        plan_update(&self.root, &self.v2).unwrap()
    }
    fn reject_plan(&self, source: &Path) -> String {
        let before = files(self.temp.path());
        let error = plan_update(&self.root, source)
            .err()
            .expect("update must be rejected");
        assert_eq!(files(self.temp.path()), before);
        error
    }
    fn reject_apply(&self, plan: &UpdatePlan) -> String {
        let before = files(self.temp.path());
        let error = apply_update(plan).unwrap_err();
        assert_eq!(files(self.temp.path()), before);
        error
    }
    fn extra(&self, name: &str, deps: &[(&str, &str)]) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("LICENSE.txt"), "Owned fixture").unwrap();
        let value = Manifest {
            schema: 1,
            name: name.into(),
            version: "1.0.0".into(),
            engine: "*".into(),
            capabilities: BTreeSet::new(),
            dependencies: deps
                .iter()
                .map(|(n, v)| ((*n).into(), (*v).into()))
                .collect(),
            files: BTreeSet::from(["LICENSE.txt".into()]),
        };
        fs::write(
            path.join("orr.package.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        path
    }
}

#[test]
fn update_preview_is_read_only_and_explicit_apply_preserves_room_and_other_packages() {
    let f = Fixture::new();
    let before = files(f.temp.path());
    let prepared = PreparedProject::open(&f.root).unwrap();
    let frame = prepared.scene().frame().to_bytes();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: prepared.path().into(),
        text: prepared.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let plan = f.plan();
    assert_eq!(files(f.temp.path()), before);
    assert_eq!(plan.preview.previous.direct[TARGET], "1.0.0");
    assert_eq!(plan.preview.candidate.direct[TARGET], "2.0.0");
    assert_ne!(
        plan.preview.previous.packages[TARGET].digest,
        plan.preview.candidate.packages[TARGET].digest
    );
    let installed = apply_update(&plan).unwrap();
    assert_eq!(installed, plan.preview.candidate);
    for (name, old) in &plan.preview.previous.packages {
        if name != TARGET {
            assert_eq!(installed.packages[name], *old);
        }
    }
    for (path, bytes) in &before {
        if path != Path::new("room/orr.packages.lock.json") {
            assert_eq!(
                files(f.temp.path()).get(path),
                Some(bytes),
                "existing bytes changed: {}",
                path.display()
            );
        }
    }
    assert_eq!(
        PreparedProject::open(&f.root)
            .unwrap()
            .scene()
            .frame()
            .to_bytes(),
        frame
    );
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history(), &history);
    f.reject_apply(&plan);
}
#[test]
fn update_rejects_same_version_changed_bytes_and_noop() {
    let f = Fixture::new();
    f.reject_plan(&f.v1);
    manifest(&f.v2, |m| m.version = "1.0.0".into());
    f.reject_plan(&f.v2);
}
#[test]
fn update_rejects_new_name_and_transitive_only_target() {
    let f = Fixture::new();
    manifest(&f.v2, |m| m.name = "new-name".into());
    f.reject_plan(&f.v2);
    manifest(&f.v2, |m| m.name = TARGET.into());
    let wrapper = f.extra("wrapper", &[(TARGET, "1.0.0")]);
    f.project().install(&[wrapper]).unwrap();
    f.project().remove(TARGET).unwrap();
    assert!(!f.project().verify().unwrap().direct.contains_key(TARGET));
    f.reject_plan(&f.v2);
}
#[test]
fn update_rejects_every_saved_static_reference() {
    let f = Fixture::new();
    let path = f.root.join("room.models.json");
    let original: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let loaded = model_bindings::load_asset(&f.root, TARGET, "model.glb").unwrap();
    let binding = Binding::from_asset(
        TARGET.into(),
        "model.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    for guid in original.bindings.keys() {
        let mut document = original.clone();
        document.bindings.insert(guid.clone(), binding.clone());
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        PreparedProject::open(&f.root).unwrap();
        f.reject_plan(&f.v2);
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn update_rejects_every_saved_animated_reference() {
    use model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
    let f = Fixture::new();
    let source = f.temp.path().join("animated-v1");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo"),
        &source,
    );
    f.project().install(std::slice::from_ref(&source)).unwrap();
    let next = f.temp.path().join("animated-v2");
    copy_tree(&source, &next);
    manifest(&next, |m| m.version = "2.0.0".into());
    let asset = model_bindings::load_asset_for_kind(
        &f.root,
        "sample-animation",
        "animated.glb",
        ModelKind::Animated,
    )
    .unwrap();
    let binding = Binding::from_animated_asset(
        "sample-animation".into(),
        "animated.glb".into(),
        &asset,
        AnimationDescriptor {
            clip_index: 1,
            playback: PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )
    .unwrap();
    let path = f.root.join("room.models.json");
    let original: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for guid in original.bindings.keys() {
        let mut doc = original.clone();
        doc.bindings.insert(guid.clone(), binding.clone());
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        PreparedProject::open(&f.root).unwrap();
        f.reject_plan(&next);
    }
}
#[test]
fn update_rejects_dependency_conflict_and_changed_closure() {
    let f = Fixture::new();
    let wrapper = f.extra("wrapper", &[(TARGET, "1.0.0")]);
    f.project().install(&[wrapper]).unwrap();
    f.reject_plan(&f.v2);
    let f = Fixture::new();
    let leaf = f.extra("leaf", &[]);
    f.project().install(&[leaf]).unwrap();
    manifest(&f.v2, |m| {
        m.dependencies.insert("leaf".into(), "1.0.0".into());
    });
    f.reject_plan(&f.v2);
    let f = Fixture::new();
    manifest(&f.v2, |m| {
        m.dependencies.insert("missing".into(), "1.0.0".into());
    });
    f.reject_plan(&f.v2);
    // A dropped edge would also deactivate a transitive leaf; unchanged closure
    // succeeds only when the exact existing requirement and identity survive.
    let f = Fixture::new();
    let leaf = f.extra("leaf", &[]);
    manifest(&f.v1, |m| {
        m.version = "1.1.0".into();
        m.dependencies.insert("leaf".into(), "1.0.0".into());
    });
    f.project().install(&[f.v1.clone(), leaf]).unwrap();
    f.project().remove("leaf").unwrap();
    let previous = f.project().verify().unwrap();
    assert!(!previous.direct.contains_key("leaf"));
    f.reject_plan(&f.v2);
    manifest(&f.v2, |m| {
        m.dependencies.insert("leaf".into(), "1.0.0".into());
    });
    let plan = f.plan();
    let installed = apply_update(&plan).unwrap();
    assert_eq!(installed.packages["leaf"], previous.packages["leaf"]);
    assert_eq!(installed.packages[USED], previous.packages[USED]);
    assert_eq!(
        installed.direct.keys().collect::<Vec<_>>(),
        previous.direct.keys().collect::<Vec<_>>()
    );
    PreparedProject::open(&f.root).unwrap();
}
#[test]
fn update_rejects_source_bytes_or_manifest_changed_after_preview() {
    for which in ["model.glb", "LICENSE.txt", "orr.package.json"] {
        let f = Fixture::new();
        let plan = f.plan();
        let path = f.v2.join(which);
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(path, bytes).unwrap();
        f.reject_apply(&plan);
    }
}
#[test]
fn update_rejects_active_lock_changed_after_preview() {
    let f = Fixture::new();
    let plan = f.plan();
    let other = f.extra("other", &[]);
    f.project().install(&[other]).unwrap();
    f.reject_apply(&plan);
}
#[test]
fn update_rejects_scene_camera_models_and_manifest_changed_after_preview() {
    for relative in [
        "room.scene.yaml",
        "room.camera.json",
        "room.models.json",
        "orr.project.json",
    ] {
        let f = Fixture::new();
        let plan = f.plan();
        let path = f.root.join(relative);
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b'\n');
        fs::write(path, bytes).unwrap();
        PreparedProject::open(&f.root).unwrap();
        f.reject_apply(&plan);
    }
}
#[test]
fn update_rejects_replaced_root_and_retargeted_plan() {
    let f = Fixture::new();
    let plan = f.plan();
    let moved = f.temp.path().join("original-root");
    fs::rename(&f.root, &moved).unwrap();
    copy_tree(&moved, &f.root);
    f.reject_apply(&plan);
    let f = Fixture::new();
    let mut plan = f.plan();
    let other = f.temp.path().join("other-root");
    copy_tree(&f.root, &other);
    plan.root = other;
    f.reject_apply(&plan);
}
#[test]
fn update_rejects_replaced_source_directory_even_with_identical_bytes() {
    let f = Fixture::new();
    let plan = f.plan();
    let moved = f.temp.path().join("original-source");
    fs::rename(&f.v2, &moved).unwrap();
    copy_tree(&moved, &f.v2);
    f.reject_apply(&plan);
}
#[test]
fn update_rejects_tampered_candidate_or_retained_package_identity() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.preview.candidate = plan.preview.previous.clone();
    f.reject_apply(&plan);
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.preview
        .candidate
        .packages
        .get_mut(USED)
        .unwrap()
        .digest = "0".repeat(64);
    f.reject_apply(&plan);
}
#[test]
fn update_rejects_writer_contention_without_stealing_guard() {
    let f = Fixture::new();
    let plan = f.plan();
    fs::write(f.root.join(".orr/packages/writer.lock"), b"another writer").unwrap();
    let error = f.reject_apply(&plan);
    assert!(error.contains("writer"), "{error}");
}
#[test]
fn update_rejects_invalid_or_newly_referenced_saved_bindings() {
    let f = Fixture::new();
    let plan = f.plan();
    let asset = model_bindings::load_asset(&f.root, TARGET, "model.glb").unwrap();
    let binding = Binding::from_asset(
        TARGET.into(),
        "model.glb".into(),
        &asset,
        LocalTransform::default(),
    )
    .unwrap();
    let path = f.root.join("room.models.json");
    let mut doc: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    *doc.bindings.values_mut().next().unwrap() = binding;
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    f.reject_apply(&plan);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["unknown_package_references"] = serde_json::json!([TARGET]);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    f.reject_plan(&f.v2);
}
