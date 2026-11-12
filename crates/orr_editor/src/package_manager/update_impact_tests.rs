//! Separate update inventory: no changes to legacy package test counts.
#![allow(clippy::disallowed_types)]
use super::{inspect_update_impact, AssetChange};
use crate::package_manager::plan_update;
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
}
fn bind_target(f: &Fixture) -> usize {
    let path = f.root.join("room.models.json");
    let mut doc: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let asset = model_bindings::load_asset(&f.root, TARGET, "model.glb").unwrap();
    let binding = Binding::from_asset(
        TARGET.into(),
        "model.glb".into(),
        &asset,
        LocalTransform::default(),
    )
    .unwrap();
    for saved in doc.bindings.values_mut() {
        *saved = binding.clone();
    }
    fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
    doc.bindings.len()
}
#[test]
fn every_saved_guid_reports_missing_changed_or_unchanged_without_writes() {
    for change in [
        AssetChange::BytesUnchanged,
        AssetChange::BytesChanged,
        AssetChange::Missing,
    ] {
        let f = Fixture::new();
        let count = bind_target(&f);
        match change {
            AssetChange::BytesChanged => {
                fs::write(f.v2.join("model.glb"), "not decoded by impact preview").unwrap()
            }
            AssetChange::Missing => {
                manifest(&f.v2, |m| {
                    m.files.remove("model.glb");
                });
                fs::remove_file(f.v2.join("model.glb")).unwrap();
            }
            _ => (),
        }
        let before = files(f.temp.path());
        let report = inspect_update_impact(&f.root, &f.v2).unwrap();
        assert_eq!(report.bindings().len(), count);
        assert!(count >= 2);
        assert!(report.bindings().iter().all(|b| b.change == change));
        assert_eq!(report.before_version(), "1.0.0");
        assert_eq!(report.after_version(), "2.0.0");
        report.validate_current().unwrap();
        assert!(plan_update(&f.root, &f.v2).is_err());
        assert_eq!(files(f.temp.path()), before);
    }
}
#[test]
fn zero_impacts_excludes_other_packages_and_never_writes() {
    let f = Fixture::new();
    let before = files(f.temp.path());
    let report = inspect_update_impact(&f.root, &f.v2).unwrap();
    assert!(report.bindings().is_empty());
    report.validate_current().unwrap();
    assert_eq!(files(f.temp.path()), before);
}
#[test]
fn stale_source_lock_and_saved_bindings_discard_result() {
    for mutation in 0..3 {
        let f = Fixture::new();
        bind_target(&f);
        let report = inspect_update_impact(&f.root, &f.v2).unwrap();
        match mutation {
            0 => fs::write(f.v2.join("LICENSE.txt"), "changed").unwrap(),
            1 => fs::write(f.root.join("orr.packages.lock.json"), "invalid").unwrap(),
            _ => {
                let p = f.root.join("room.models.json");
                let mut d: Document = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
                d.bindings.pop_first();
                fs::write(p, serde_json::to_vec(&d).unwrap()).unwrap();
            }
        }
        let before = files(f.temp.path());
        assert!(report.validate_current().is_err());
        assert_eq!(files(f.temp.path()), before);
    }
}
#[test]
fn same_version_and_dependency_changes_reject() {
    let f = Fixture::new();
    assert!(inspect_update_impact(&f.root, &f.v1).is_err());
    manifest(&f.v2, |m| {
        m.dependencies.insert(USED.into(), "1.0.0".into());
    });
    assert!(inspect_update_impact(&f.root, &f.v2).is_err());
}
#[test]
fn replaced_root_or_source_directory_rejects_identical_bytes() {
    for root in [true, false] {
        let f = Fixture::new();
        let report = inspect_update_impact(&f.root, &f.v2).unwrap();
        let path = if root { &f.root } else { &f.v2 };
        let old = f.temp.path().join("old");
        fs::rename(path, &old).unwrap();
        copy_tree(&old, path);
        assert!(report.validate_current().is_err());
    }
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn editor_feature_off_rejects_animation_capability() {
    let f = Fixture::new();
    manifest(&f.v2, |m| {
        m.capabilities.insert("animation".into());
    });
    assert!(!super::impact_runtime().capabilities.contains("animation"));
    assert!(inspect_update_impact(&f.root, &f.v2).is_err());
}

#[cfg(feature = "room-animated-models")]
#[test]
fn animated_saved_guids_report_unchanged_bytes() {
    use model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
    let f = Fixture::new();
    let source = f.temp.path().join("animated-v1");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo"),
        &source,
    );
    Project::open(&f.root, room_runtime())
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
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
        let before = files(f.temp.path());
        let report = inspect_update_impact(&f.root, &next).unwrap();
        assert_eq!(report.bindings().len(), 1);
        assert_eq!(report.bindings()[0].guid, *guid);
        assert_eq!(report.bindings()[0].change, AssetChange::BytesUnchanged);
        assert!(plan_update(&f.root, &next).is_err());
        assert_eq!(files(f.temp.path()), before);
    }
}
