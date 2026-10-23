//! Saved-state API contracts; no GPU or live-runtime acceptance claim.
#![cfg(feature = "room-project")]
#[allow(dead_code)]
#[path = "../examples/room_package_usage/authoring.rs"]
mod authoring;

use authoring::{inspect_usage, Request};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, here: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(here).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), result);
            } else if entry.file_type().unwrap().is_file() {
                result.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("room");
        let source = temp.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&source).unwrap();
        let model = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo/foreground.glb");
        for name in ["one.glb", "two.glb"] {
            fs::copy(&model, source.join(name)).unwrap();
        }
        fs::write(source.join("orr.package.json"), serde_json::to_vec(&serde_json::json!({"schema":1,"name":"usage-models","version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":["one.glb","two.glb"]})).unwrap()).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let mut scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
        // Names are optional, and never used as identity.
        for entity in scene.entities.values_mut() {
            entity.name = None;
        }
        let text = scene.to_yaml();
        fs::write(root.join("room.scene.yaml"), &text).unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
        let loaded = model_bindings::load_asset(&root, "usage-models", "one.glb").unwrap();
        let binding = Binding::from_asset(
            "usage-models".into(),
            "one.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let prepared = PreparedScene::parse(&text).unwrap();
        let bindings = prepared
            .frame()
            .entities()
            .filter(|e| {
                [PLAYER, KEY].contains(&prepared.frame().get::<RoomActor>(*e).unwrap().kind)
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
        Self { temp, root, source }
    }
    fn request(&self) -> Request {
        let package = "usage-models".to_owned();
        let digest = Project::open(&self.root, room_project::compiled_runtime())
            .unwrap()
            .verify()
            .unwrap()
            .packages[&package]
            .digest
            .clone();
        Request {
            project: self.root.clone(),
            package,
            digest,
        }
    }
    fn json(&self, file: &str, change: impl FnOnce(&mut serde_json::Value)) {
        let path = self.root.join(file);
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn rejects_unchanged(&self, request: &Request) {
        let before = files(self.temp.path());
        assert!(inspect_usage(request).is_err());
        assert_eq!(before, files(self.temp.path()));
    }
}

#[test]
fn every_guid_sharing_one_asset_is_reported_deterministically_without_writes() {
    let fixture = Fixture::new();
    let before = files(fixture.temp.path());
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let frame = prepared.scene().frame().to_bytes();
    let report = inspect_usage(&fixture.request()).unwrap();
    assert_eq!(report.bindings.len(), 2);
    assert!(report
        .bindings
        .windows(2)
        .all(|pair| pair[0].guid < pair[1].guid));
    for binding in &report.bindings {
        assert_eq!(binding.asset, "one.glb");
        assert!(binding.name.is_none());
        assert_eq!(binding.package_digest, report.package.digest);
        assert_eq!(binding.source_hash.len(), 64);
    }
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&inspect_usage(&fixture.request()).unwrap()).unwrap()
    );
    assert_eq!(frame, prepared.scene().frame().to_bytes());
    assert_eq!(
        frame,
        PreparedProject::open(&fixture.root)
            .unwrap()
            .scene()
            .frame()
            .to_bytes()
    );
    assert_eq!(before, files(fixture.temp.path()));
    // Optional owned acceptance artifact for exercising the actual CLI binary.
    // Never part of the query implementation or a prerequisite for ordinary tests.
    if let Some(destination) = std::env::var_os("ORR_ROOM_USAGE_FIXTURE_OUT") {
        let destination = PathBuf::from(destination);
        fs::create_dir(&destination).unwrap();
        for (relative, bytes) in files(&fixture.root) {
            let path = destination.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }
}

#[test]
fn multiple_assets_and_duplicate_display_names_keep_distinct_guids() {
    let fixture = Fixture::new();
    let mut scene = Scene::parse(
        &fs::read_to_string(fixture.root.join("room.scene.yaml")).unwrap(),
        &room_project::types(),
    )
    .unwrap();
    for entity in scene.entities.values_mut() {
        entity.name = Some("same display name".into());
    }
    fs::write(fixture.root.join("room.scene.yaml"), scene.to_yaml()).unwrap();
    let loaded = model_bindings::load_asset(&fixture.root, "usage-models", "two.glb").unwrap();
    fixture.json("room.models.json", |v| {
        let (_, b) = v["bindings"]
            .as_object_mut()
            .unwrap()
            .iter_mut()
            .next()
            .unwrap();
        *b = serde_json::to_value(
            Binding::from_asset(
                "usage-models".into(),
                "two.glb".into(),
                &loaded,
                LocalTransform::default(),
            )
            .unwrap(),
        )
        .unwrap();
    });
    let report = inspect_usage(&fixture.request()).unwrap();
    assert_eq!(report.bindings.len(), 2);
    assert!(report
        .bindings
        .iter()
        .all(|b| b.name.as_deref() == Some("same display name")));
    assert_ne!(report.bindings[0].asset, report.bindings[1].asset);
}

#[test]
fn installed_snapshot_survives_missing_source_directory() {
    let fixture = Fixture::new();
    let request = fixture.request();
    fs::rename(&fixture.source, fixture.temp.path().join("hidden source")).unwrap();
    let before = files(fixture.temp.path());
    assert_eq!(inspect_usage(&request).unwrap().bindings.len(), 2);
    assert_eq!(before, files(fixture.temp.path()));
}

#[test]
fn empty_usage_is_explicitly_not_removal_permission() {
    let fixture = Fixture::new();
    let unused = fixture.temp.path().join("unused");
    fs::create_dir(&unused).unwrap();
    fs::write(unused.join("orr.package.json"), serde_json::to_vec(&serde_json::json!({"schema":1,"name":"unused","version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":[]})).unwrap()).unwrap();
    let project = Project::open(&fixture.root, room_project::compiled_runtime()).unwrap();
    let lock = project.install(&[unused]).unwrap();
    let report = inspect_usage(&Request {
        project: fixture.root.clone(),
        package: "unused".into(),
        digest: lock.packages["unused"].digest.clone(),
    })
    .unwrap();
    assert!(report.bindings.is_empty());
    assert_eq!(report.scope, "saved-room-model-bindings");
    assert!(report.warning.contains("not removal permission"));
}

#[test]
fn invalid_missing_and_wrong_digest_requests_fail_without_writes() {
    let fixture = Fixture::new();
    for name in ["", "../usage-models", "missing", &"a".repeat(100)] {
        let mut request = fixture.request();
        request.package = name.into();
        fixture.rejects_unchanged(&request);
    }
    for digest in ["".into(), "A".repeat(64), "0".repeat(63), "0".repeat(64)] {
        let mut request = fixture.request();
        request.digest = digest;
        fixture.rejects_unchanged(&request);
    }
}

#[test]
fn non_model_installed_package_is_not_a_model_usage_query() {
    let fixture = Fixture::new();
    let source = fixture.temp.path().join("content");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("orr.package.json"), br#"{"schema":1,"name":"content","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{},"files":[]}"#).unwrap();
    let lock = Project::open(&fixture.root, room_project::compiled_runtime())
        .unwrap()
        .install(&[source])
        .unwrap();
    fixture.rejects_unchanged(&Request {
        project: fixture.root.clone(),
        package: "content".into(),
        digest: lock.packages["content"].digest.clone(),
    });
}

#[test]
fn stale_binding_identity_hash_and_unknown_guid_fail_closed() {
    for field in ["package_digest", "source_hash", "guid"] {
        let fixture = Fixture::new();
        let request = fixture.request();
        fixture.json("room.models.json", |v| {
            let bindings = v["bindings"].as_object_mut().unwrap();
            let guid = bindings.keys().next().unwrap().clone();
            if field == "guid" {
                let binding = bindings.remove(&guid).unwrap();
                bindings.insert("e_deadbeef".into(), binding);
            } else {
                bindings.get_mut(&guid).unwrap()[field] = "0".repeat(64).into();
            }
        });
        fixture.rejects_unchanged(&request);
    }
}

#[test]
fn malformed_sidecar_lock_and_wrong_entry_fail_closed() {
    for (file, field, value) in [
        ("room.models.json", "opaque", serde_json::json!(true)),
        (
            "room.models.json",
            "scene",
            serde_json::json!("elsewhere.scene.yaml"),
        ),
        ("orr.packages.lock.json", "schema", serde_json::json!(999)),
        (
            "orr.project.json",
            "entry",
            serde_json::json!({"game":"room-escape-v1","scene":"../escape.yaml","models":"room.models.json"}),
        ),
    ] {
        let fixture = Fixture::new();
        let request = fixture.request();
        fixture.json(file, |v| v[field] = value);
        fixture.rejects_unchanged(&request);
    }
}

#[test]
fn tampered_installed_asset_fails_without_repair() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let candidate = files(&fixture.root)
        .keys()
        .find(|p| p.ends_with("one.glb"))
        .unwrap()
        .clone();
    fs::write(fixture.root.join(candidate), b"tampered").unwrap();
    fixture.rejects_unchanged(&request);
}

#[cfg(unix)]
#[test]
fn symlinked_sidecar_is_rejected_without_modification() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let path = fixture.root.join("room.models.json");
    let target = fixture.temp.path().join("outside.json");
    fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    fixture.rejects_unchanged(&request);
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn parser_requires_each_bounded_option_once() {
    let parse = |args: &[&str]| authoring::parse_args(args.iter().map(OsString::from));
    assert!(parse(&["--help"]).unwrap().is_none());
    assert!(
        parse(&["--project", "p", "--package", "x", "--digest", "d"])
            .unwrap()
            .is_some()
    );
    for args in [
        vec![],
        vec!["--help", "extra"],
        vec!["--project", "p", "--project", "p", "--digest", "d"],
        vec!["--unknown", "p", "--package", "x", "--digest", "d"],
        vec!["--project", "", "--package", "x", "--digest", "d"],
        vec!["--project", "p", "--package", "x", "--digest", "d", "extra"],
    ] {
        assert!(parse(&args).is_err());
    }
}

#[test]
fn animated_capability_uses_the_actual_compiled_room_route() {
    let fixture = Fixture::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let lock = Project::open_for_install(&fixture.root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
    let request = Request {
        project: fixture.root.clone(),
        package: "sample-animation".into(),
        digest: lock.packages["sample-animation"].digest.clone(),
    };
    #[cfg(not(feature = "room-animated-models"))]
    fixture.rejects_unchanged(&request);
    #[cfg(feature = "room-animated-models")]
    {
        use model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
        let loaded = model_bindings::load_asset_for_kind(
            &fixture.root,
            "sample-animation",
            "animated.glb",
            ModelKind::Animated,
        )
        .unwrap();
        let binding = Binding::from_animated_asset(
            "sample-animation".into(),
            "animated.glb".into(),
            &loaded,
            AnimationDescriptor {
                clip_index: 0,
                playback: PlaybackMode::Loop,
            },
            LocalTransform::default(),
        )
        .unwrap();
        fixture.json("room.models.json", |v| {
            let (_, item) = v["bindings"]
                .as_object_mut()
                .unwrap()
                .iter_mut()
                .next()
                .unwrap();
            *item = serde_json::to_value(binding).unwrap();
        });
        let before = files(fixture.temp.path());
        let report = inspect_usage(&request).unwrap();
        assert_eq!(report.bindings.len(), 1);
        assert_eq!(report.bindings[0].kind, ModelKind::Animated);
        assert_eq!(before, files(fixture.temp.path()));
    }
}

#[test]
fn existing_explanation_distinguishes_transitive_activation_from_scene_usage() {
    let fixture = Fixture::new();
    let parent = fixture.temp.path().join("parent");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("orr.package.json"), br#"{"schema":1,"name":"parent","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{"usage-models":"1.0.0"},"files":[]}"#).unwrap();
    let project = Project::open(&fixture.root, room_project::compiled_runtime()).unwrap();
    project.install(&[parent]).unwrap();
    // Core removal drops direct selection but retains the dependency content.
    project.remove("usage-models").unwrap();
    let request = fixture.request();
    let before = files(fixture.temp.path());
    let report = inspect_usage(&request).unwrap();
    assert!(!report.package.direct);
    assert_eq!(
        report.package.required_by.into_iter().collect::<Vec<_>>(),
        ["parent"]
    );
    assert_eq!(
        report.package.selected_by["parent"],
        ["parent", "usage-models"]
    );
    assert_eq!(report.bindings.len(), 2);
    assert_eq!(before, files(fixture.temp.path()));
}
