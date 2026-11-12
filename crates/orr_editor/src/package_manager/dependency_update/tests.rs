use super::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};
fn source(root: &Path, name: &str, deps: &[(&str, &str)]) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    let deps: BTreeMap<_, _> = deps.iter().copied().collect();
    fs::write(path.join("asset.txt"), name).unwrap();
    fs::write(path.join("LICENSE.txt"), b"Fixture only").unwrap();
    fs::write(path.join("orr.package.json"),serde_json::to_vec(&json!({"schema":1,"name":name,"version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":deps,"files":["asset.txt","LICENSE.txt"]})).unwrap()).unwrap();
    path
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    a: PathBuf,
    deps: Vec<PathBuf>,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let root = temp.path().join("room");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("room.scene.yaml"),
        include_str!("../../../tests/fixtures/room_initial.scene.yaml"),
    )
    .unwrap();
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&json!({
            "schema":2, "engine":"*", "entry": {
                "game":"room-escape-v1", "scene":"room.scene.yaml", "models":"room.models.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    // Bootstrap a real model package before Room admission; an empty binding
    // document is deliberately not an admitted Room project.
    use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
    let baseline = source(temp.path(), "baseline", &[]);
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo/foreground.glb"),
        baseline.join("foreground.glb"),
    )
    .unwrap();
    manifest(&baseline, |value| {
        value["files"] = json!(["asset.txt", "LICENSE.txt", "foreground.glb"])
    });
    Project::open_for_install(&root, room_runtime().engine_version)
        .unwrap()
        .install(&[baseline])
        .unwrap();
    let loaded = model_bindings::load_asset(&root, "baseline", "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        "baseline".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = orr_sample::room_project::PreparedScene::parse(include_str!(
        "../../../tests/fixtures/room_initial.scene.yaml"
    ))
    .unwrap();
    let bindings = scene
        .frame()
        .entities()
        .take(1)
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
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
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let project_path = root.join("orr.project.json");
    let mut project: Value = serde_json::from_slice(&fs::read(&project_path).unwrap()).unwrap();
    project["entry"]["camera"] = json!("room.camera.json");
    fs::write(project_path, serde_json::to_vec(&project).unwrap()).unwrap();
    open_room(&root).unwrap();
    let a = source(temp.path(), "a", &[("baseline", "1.0.0")]);
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo/foreground.glb"),
        a.join("foreground.glb"),
    )
    .unwrap();
    manifest(&a, |v| {
        v["files"] = json!(["asset.txt", "LICENSE.txt", "foreground.glb"])
    });
    let old = super::super::plan_add(&root, &a).unwrap();
    super::super::apply_add(&old).unwrap();
    manifest(&a, |v| {
        v["version"] = json!("2.0.0");
        v["dependencies"] = json!({"baseline":"1.0.0", "b":"1.0.0"});
    });
    let b = source(temp.path(), "b", &[("c", "1.0.0")]);
    let c = source(temp.path(), "c", &[]);
    let d = source(temp.path(), "d", &[]);
    Fixture {
        _temp: temp,
        root,
        a,
        deps: vec![b, c, d],
    }
}
fn manifest(path: &Path, edit: impl FnOnce(&mut Value)) {
    let p = path.join("orr.package.json");
    let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    edit(&mut v);
    fs::write(p, serde_json::to_vec(&v).unwrap()).unwrap();
}
#[test]
fn additive_update_preserves_old_entries_and_admitted_room() {
    let f = fixture();
    let before = open_room(&f.root).unwrap().1;
    let bytes = fs::read(f.root.join("orr.packages.lock.json")).unwrap();
    let plan = plan_update_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
    assert_eq!(plan.name(), "a");
    assert_eq!(plan.root(), f.root);
    assert_eq!(plan.unused(), &["d".to_owned()]);
    assert_eq!(
        fs::read(f.root.join("orr.packages.lock.json")).unwrap(),
        bytes
    );
    let installed = apply_update_with_dependencies(&plan).unwrap();
    assert_eq!(installed, plan.preview().candidate);
    assert_eq!(installed.direct.len(), before.direct.len());
    assert_eq!(installed.packages.len(), before.packages.len() + 2);
    assert_eq!(installed.direct["a"], "2.0.0");
    for (name, p) in before.packages {
        if name != "a" {
            assert_eq!(installed.packages.get(&name), Some(&p));
        }
    }
}
#[test]
fn rejects_nonadditive_or_conflicting_dependencies_and_invalid_candidates() {
    for case in 0..9 {
        let f = fixture();
        let before = open_room(&f.root).unwrap().1;
        let mut deps = f.deps.clone();
        match case {
            0 => manifest(&f.a, |v| v["dependencies"] = json!({"b":"1.0.0"})),
            1 => manifest(&f.a, |v| v["dependencies"]["baseline"] = json!("2.0.0")),
            2 => manifest(&f.a, |v| v["version"] = json!("1.0.0")),
            3 => manifest(&f.deps[1], |v| v["dependencies"] = json!({"a":"2.0.0"})),
            4 => {
                deps.remove(1);
            }
            5 => manifest(&f.deps[2], |v| v["capabilities"] = json!(["unknown-host"])),
            6 => deps.push(f.deps[0].clone()),
            7 => manifest(&f.deps[2], |v| v["name"] = json!("baseline")),
            _ => deps.clear(),
        }
        assert!(
            plan_update_with_dependencies(&f.root, &f.a, &deps).is_err(),
            "case {case}"
        );
        assert_eq!(open_room(&f.root).unwrap().1, before);
    }
}
#[test]
fn static_binding_to_updated_target_is_refused() {
    use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
    let f = fixture();
    let path = f.root.join("room.models.json");
    let mut doc: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let loaded = model_bindings::load_asset(&f.root, "a", "foreground.glb").unwrap();
    *doc.bindings.values_mut().next().unwrap() = Binding::from_asset(
        "a".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
    open_room(&f.root).unwrap();
    assert!(plan_update_with_dependencies(&f.root, &f.a, &f.deps)
        .err()
        .unwrap()
        .contains("identity of model binding"));
}
fn mutate(f: &Fixture, case: u32) {
    match case {
        0 => fs::write(f.deps[2].join("asset.txt"), b"unused mutation").unwrap(),
        1..=4 => {
            let file = match case {
                1 => "orr.project.json",
                2 => "room.models.json",
                3 => "room.camera.json",
                _ => "room.scene.yaml",
            };
            let path = f.root.join(file);
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
        }
        _ => {
            let plan = super::super::plan_add(&f.root, &f.deps[2]).unwrap();
            super::super::apply_add(&plan).unwrap();
        }
    }
}
#[test]
fn stale_unused_manifest_scene_sidecars_and_lock_refuse_apply() {
    for case in 0..6 {
        let f = fixture();
        let plan = plan_update_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
        mutate(&f, case);
        let before = open_room(&f.root).unwrap().1;
        assert!(
            apply_update_with_dependencies(&plan).is_err(),
            "case {case}"
        );
        assert_eq!(open_room(&f.root).unwrap().1, before);
    }
}
#[test]
fn writer_callback_rechecks_unused_and_complete_saved_room() {
    for case in 0..5 {
        let f = fixture();
        let plan = plan_update_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
        let result = Project::open(&f.root, room_runtime())
            .unwrap()
            .install_checked_with_dependencies(
                std::slice::from_ref(&f.a),
                &f.deps,
                &plan.preview.previous,
                &plan.preview.candidate,
                || {
                    mutate(&f, case);
                    unchanged_sources(&plan)?;
                    unchanged_room(&plan)
                },
            );
        assert!(result.is_err(), "case {case}");
        assert_eq!(open_room(&f.root).unwrap().1, plan.preview.previous);
    }
}

#[cfg(feature = "room-animated-models")]
#[test]
fn animated_binding_to_updated_target_is_refused() {
    use orr_model_bindings::model_bindings::{
        self, AnimationDescriptor, Binding, Document, LocalTransform, ModelKind, PlaybackMode,
    };
    let f = fixture();
    let source = f._temp.path().join("animated-source");
    fs::create_dir(&source).unwrap();
    for file in ["animated.glb", "LICENSE.txt", "orr.package.json"] {
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/animation_demo")
                .join(file),
            source.join(file),
        )
        .unwrap();
    }
    Project::open(&f.root, room_runtime())
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
    let loaded = model_bindings::load_asset_for_kind(
        &f.root,
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
            clip_index: 1,
            playback: PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )
    .unwrap();
    let path = f.root.join("room.models.json");
    let mut doc: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    *doc.bindings.values_mut().next().unwrap() = binding;
    fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
    open_room(&f.root).unwrap();
    manifest(&source, |v| {
        v["version"] = json!("2.0.0");
        v["dependencies"] = json!({"b":"1.0.0"});
    });
    assert!(plan_update_with_dependencies(&f.root, &source, &f.deps)
        .err()
        .unwrap()
        .contains("identity of model binding"));
}
