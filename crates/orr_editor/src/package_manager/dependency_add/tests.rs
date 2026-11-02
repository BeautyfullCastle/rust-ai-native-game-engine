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
    open_room(&root).unwrap();
    let a = source(temp.path(), "a", &[("b", "1.0.0")]);
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
fn closure_preview_and_apply_preserve_existing_room() {
    let f = fixture();
    let before = open_room(&f.root).unwrap().1;
    let lock_bytes = fs::read(f.root.join("orr.packages.lock.json")).unwrap();
    let plan = plan_add_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
    assert_eq!(plan.unused(), &["d".to_string()]);
    assert_eq!(plan.root(), f.root);
    assert_eq!(
        plan.preview().candidate.direct.len(),
        before.direct.len() + 1
    );
    assert_eq!(
        plan.preview().candidate.packages.len(),
        before.packages.len() + 3
    );
    assert_eq!(
        fs::read(f.root.join("orr.packages.lock.json")).unwrap(),
        lock_bytes
    );
    let after = apply_add_with_dependencies(&plan).unwrap();
    assert_eq!(after, plan.preview().candidate);
    assert!(!after.packages.contains_key("d"));
    for (name, p) in before.packages {
        assert_eq!(after.packages.get(&name), Some(&p));
    }
}
#[test]
fn unused_candidate_and_valid_manifest_byte_changes_revoke_plan() {
    for change_manifest in [false, true] {
        let f = fixture();
        let plan = plan_add_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
        if change_manifest {
            let path = f.root.join("orr.project.json");
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
            open_room(&f.root).unwrap(); // semantically valid, but not previewed bytes
        } else {
            fs::write(f.deps[2].join("asset.txt"), b"unused changed").unwrap();
        }
        assert!(apply_add_with_dependencies(&plan).is_err());
        assert_eq!(open_room(&f.root).unwrap().1, plan.preview.previous);
    }
}
#[test]
fn source_missing_cycle_version_conflict_capability_and_duplicates_refuse() {
    for case in 0..7 {
        let f = fixture();
        let before = open_room(&f.root).unwrap().1;
        let mut deps = f.deps.clone();
        match case {
            0 => {
                deps.remove(1);
            }
            1 => manifest(&f.deps[1], |v| v["dependencies"] = json!({"a":"1.0.0"})),
            2 => manifest(&f.a, |v| v["dependencies"] = json!({"b":"2.0.0"})),
            3 => manifest(&f.deps[2], |v| v["capabilities"] = json!(["unknown-host"])),
            4 => deps.push(f.deps[0].clone()),
            5 => manifest(&f.deps[2], |v| v["name"] = json!("b")),
            _ => deps.clear(),
        }
        assert!(
            plan_add_with_dependencies(&f.root, &f.a, &deps).is_err(),
            "case {case}"
        );
        assert_eq!(open_room(&f.root).unwrap().1, before);
    }
}
#[test]
fn stale_lock_and_reachable_source_refuse_without_overwrite() {
    for change_lock in [false, true] {
        let f = fixture();
        let plan = plan_add_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
        if change_lock {
            let add = super::super::plan_add(&f.root, &f.deps[2]).unwrap();
            super::super::apply_add(&add).unwrap();
        } else {
            fs::write(f.deps[0].join("asset.txt"), b"changed").unwrap();
        }
        let before = open_room(&f.root).unwrap().1;
        assert!(apply_add_with_dependencies(&plan).is_err());
        assert_eq!(open_room(&f.root).unwrap().1, before);
    }
}

#[test]
fn writer_callback_rejects_late_unused_or_room_manifest_change() {
    for room_change in [false, true] {
        let f = fixture();
        let plan = plan_add_with_dependencies(&f.root, &f.a, &f.deps).unwrap();
        let project = Project::open(&f.root, room_runtime()).unwrap();
        let result = project.install_checked_with_dependencies(
            std::slice::from_ref(&f.a),
            &f.deps,
            &plan.preview.previous,
            &plan.preview.candidate,
            || {
                if room_change {
                    let path = f.root.join("orr.project.json");
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.push(b'\n');
                    fs::write(path, bytes).unwrap();
                } else {
                    fs::write(f.deps[2].join("asset.txt"), b"late unused mutation").unwrap();
                }
                unchanged_sources(&plan)?;
                unchanged_room(&plan)
            },
        );
        assert!(result.is_err());
        assert_eq!(open_room(&f.root).unwrap().1, plan.preview.previous);
    }
}
