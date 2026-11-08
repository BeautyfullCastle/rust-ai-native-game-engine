#![cfg(target_os = "linux")]

use super::{apply, prepare};
use crate::sprite_bindings;
use orr_package::{Lock, Project};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const PIXELS: [u8; 8] = [255, 32, 64, 255, 21, 83, 177, 113];
const LICENSE: &[u8] = b"Synthetic test artwork. CC0-1.0.\n";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("orr.project.json"),
            br#"{"schema":1,"engine":"*"}"#,
        )
        .unwrap();
        let source = root.join("new-sprites");
        write_sprite(&source, "new-sprites", "1.0.0");
        Self {
            _temp: temp,
            root,
            project,
            source,
        }
    }

    fn opened(&self) -> Project {
        sprite_bindings::open_project(&self.project).unwrap()
    }

    fn old_graph(&self) -> Lock {
        let dependency = self.root.join("old-dependency");
        let direct = self.root.join("old-direct");
        write_content(&dependency, "old-dependency", json!({}));
        write_content(&direct, "old-direct", json!({"old-dependency":"1.0.0"}));
        self.opened()
            .install_with_dependencies(&[direct], &[dependency])
            .unwrap()
    }
}

fn png(pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels)
            .unwrap();
    }
    bytes
}

fn document() -> Value {
    json!({
        "format":"orr_sprite", "version":1,
        "atlas":{"image":"atlas.png","width":2,"height":1},
        "regions":[
            {"id":10,"x":0,"y":0,"width":1,"height":1},
            {"id":20,"x":1,"y":0,"width":1,"height":1}
        ],
        "clips":[
            {"id":"walk","mode":"loop","frames":[
                {"region":10,"duration_ms":80},{"region":20,"duration_ms":120}]},
            {"id":"stop","mode":"once","frames":[
                {"region":20,"duration_ms":30},{"region":10,"duration_ms":70}]}
        ]
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn write_sprite(path: &Path, name: &str, version: &str) {
    fs::create_dir(path).unwrap();
    write_json(
        &path.join("orr.package.json"),
        &json!({
            "schema":1,"name":name,"version":version,"engine":"*",
            "capabilities":["sprite"],"dependencies":{},
            "files":["sprites.json","atlas.png","LICENSE.txt"]
        }),
    );
    write_json(&path.join("sprites.json"), &document());
    fs::write(path.join("atlas.png"), png(&PIXELS)).unwrap();
    fs::write(path.join("LICENSE.txt"), LICENSE).unwrap();
}

fn write_content(path: &Path, name: &str, dependencies: Value) {
    fs::create_dir(path).unwrap();
    write_json(
        &path.join("orr.package.json"),
        &json!({
            "schema":1,"name":name,"version":"1.0.0","engine":"*",
            "capabilities":[],"dependencies":dependencies,"files":["payload.txt"]
        }),
    );
    fs::write(path.join("payload.txt"), format!("immutable {name}\n")).unwrap();
}

fn manifest(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path.join("orr.package.json")).unwrap()).unwrap()
}

// Include directory entries as well as bytes: a read-only preparation must not
// quietly create an empty cache or writer directory.
fn tree(path: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                out.insert(relative, None);
                visit(root, &path, out);
            } else {
                out.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(path, path, &mut result);
    result
}

fn lock_bytes(project: &Path) -> Vec<u8> {
    fs::read(project.join("orr.packages.lock.json")).unwrap()
}

fn assert_old_objects(project: &Path, previous: &Lock) {
    let opened = sprite_bindings::open_project(project).unwrap();
    let current = opened.verify().unwrap();
    for (name, old) in &previous.packages {
        assert_eq!(current.packages.get(name), Some(old));
        assert_eq!(
            opened.read_asset(name, "payload.txt").unwrap(),
            format!("immutable {name}\n").as_bytes()
        );
    }
    for (name, version) in &previous.direct {
        assert_eq!(current.direct.get(name), Some(version));
    }
}

#[test]
fn preparation_is_read_only_and_candidate_adds_exactly_one_direct_package() {
    let fixture = Fixture::new();
    let before_project = tree(&fixture.project);
    let before_source = tree(&fixture.source);
    let plan = prepare(&fixture.project, &fixture.source).unwrap();
    assert_eq!(tree(&fixture.project), before_project);
    assert_eq!(tree(&fixture.source), before_source);
    assert!(!fixture.project.join(".orr").exists());
    assert_eq!(plan.project(), fixture.project);
    assert_eq!(plan.source(), fixture.source);
    assert_eq!(plan.previous(), &Lock::default());
    let summary = plan.summary();
    assert_eq!(
        (&*summary.name, &*summary.version),
        ("new-sprites", "1.0.0")
    );
    assert_eq!(
        (
            summary.width,
            summary.height,
            summary.regions,
            summary.clips
        ),
        (2, 1, 2, 2)
    );
    assert_eq!(summary.files, ["LICENSE.txt", "atlas.png", "sprites.json"]);
    assert_eq!(summary.digest.len(), 64);
    assert_eq!(
        plan.candidate().direct,
        BTreeMap::from([("new-sprites".into(), "1.0.0".into())])
    );
    assert_eq!(plan.candidate().packages.len(), 1);
    let package = &plan.candidate().packages["new-sprites"];
    assert_eq!(package.digest, summary.digest);
    assert_eq!(package.files.len(), 3);
    assert_eq!(package.manifest.name, summary.name);
}

#[test]
fn checked_install_preserves_old_graph_and_loads_typed_pixels_uvs_and_clips_without_source() {
    let fixture = Fixture::new();
    let previous = fixture.old_graph();
    let old_objects = tree(&fixture.project.join(".orr/packages/objects"));
    let source_bytes = tree(&fixture.source);
    let plan = prepare(&fixture.project, &fixture.source).unwrap();
    assert_eq!(plan.previous(), &previous);
    let mut expected = previous.clone();
    expected.direct.insert("new-sprites".into(), "1.0.0".into());
    expected.packages.insert(
        "new-sprites".into(),
        plan.candidate().packages["new-sprites"].clone(),
    );
    assert_eq!(plan.candidate(), &expected);
    let called = Cell::new(false);
    let installed = apply(&plan, || {
        called.set(true);
        assert!(fixture.project.join(".orr/packages/writer.lock").is_file());
        assert_eq!(fixture.opened().list().unwrap(), previous);
        Ok(())
    })
    .unwrap();
    assert!(called.get());
    assert_eq!(installed, expected);
    assert_eq!(fixture.opened().verify().unwrap(), expected);
    assert_old_objects(&fixture.project, &previous);
    let after_objects = tree(&fixture.project.join(".orr/packages/objects"));
    for (path, bytes) in old_objects {
        assert_eq!(after_objects.get(&path), Some(&bytes));
    }
    assert_eq!(tree(&fixture.source), source_bytes);
    for name in ["sprites.json", "atlas.png", "LICENSE.txt"] {
        assert_eq!(
            fixture.opened().read_asset("new-sprites", name).unwrap(),
            fs::read(fixture.source.join(name)).unwrap()
        );
    }
    fs::rename(&fixture.source, fixture.root.join("hidden-original-source")).unwrap();
    let asset =
        sprite_bindings::load_asset(&fixture.project, "new-sprites", "sprites.json").unwrap();
    assert_eq!(asset.rgba, PIXELS);
    assert_eq!(asset.document.uv_rect(10), Some([0.0, 0.0, 0.5, 1.0]));
    assert_eq!(asset.document.uv_rect(20), Some([0.5, 0.0, 1.0, 1.0]));
    let walk = asset.document.clip("walk").unwrap();
    assert_eq!(walk.duration_ms(), 200);
    assert_eq!(
        [
            walk.sample(0).region,
            walk.sample(79).region,
            walk.sample(80).region,
            walk.sample(199).region,
            walk.sample(200).region
        ],
        [10, 10, 20, 20, 10]
    );
    let stop = asset.document.clip("stop").unwrap();
    assert_eq!(stop.sample(29).region, 20);
    assert_eq!(stop.sample(30).region, 10);
    assert!(!stop.sample(99).finished);
    assert!(stop.sample(100).finished);
    assert_eq!(stop.sample(u64::MAX).region, 10);
}

#[test]
fn existing_direct_and_dependency_only_names_cannot_be_upgraded() {
    let fixture = Fixture::new();
    let previous = fixture.old_graph();
    assert!(!previous.direct.contains_key("old-dependency"));
    let before = tree(&fixture.project);
    for name in ["old-direct", "old-dependency"] {
        let source = fixture.root.join(format!("replacement-{name}"));
        write_sprite(&source, name, "2.0.0");
        // A dependency-only name may also be rejected by the existing exact
        // dependency-version resolver before the wrapper's name admission.
        assert!(prepare(&fixture.project, &source).is_err());
        assert_eq!(tree(&fixture.project), before);
    }
}

#[test]
fn package_schema_capabilities_dependencies_and_exact_file_set_are_admitted_before_activation() {
    let mutations = [
        ("schema", json!(2)),
        ("capabilities", json!([])),
        ("capabilities", json!(["sprite", "models"])),
        ("dependencies", json!({"unavailable":"1.0.0"})),
        ("files", json!(["sprites.json", "atlas.png"])),
        (
            "files",
            json!(["sprites.json", "atlas.png", "LICENSE.txt", "extra.txt"]),
        ),
        (
            "files",
            json!(["sprites.json", "LICENSE.txt", "../atlas.png"]),
        ),
        ("files", json!(["sprites.json", "LICENSE.txt", "other.png"])),
    ];
    for (key, value) in mutations {
        let fixture = Fixture::new();
        fs::write(fixture.source.join("extra.txt"), b"extra").unwrap();
        fs::write(fixture.source.join("other.png"), png(&PIXELS)).unwrap();
        let mut spec = manifest(&fixture.source);
        spec[key] = value;
        write_json(&fixture.source.join("orr.package.json"), &spec);
        let before = tree(&fixture.project);
        assert!(
            prepare(&fixture.project, &fixture.source).is_err(),
            "accepted {key}: {spec}"
        );
        assert_eq!(tree(&fixture.project), before);
    }
    let fixture = Fixture::new();
    fs::remove_file(fixture.source.join("LICENSE.txt")).unwrap();
    assert!(prepare(&fixture.project, &fixture.source).is_err());
    assert!(!fixture.project.join(".orr").exists());
}

#[test]
fn typed_document_png_and_license_validation_rejects_invalid_or_oversized_payloads() {
    for case in 0..11 {
        let fixture = Fixture::new();
        match case {
            0 => {
                let mut doc = document();
                doc["version"] = json!(2);
                write_json(&fixture.source.join("sprites.json"), &doc);
            }
            1 => {
                let mut doc = document();
                doc["regions"][0]["width"] = json!(3);
                write_json(&fixture.source.join("sprites.json"), &doc);
            }
            2 => {
                let mut doc = document();
                doc["clips"][0]["frames"][0]["region"] = json!(999);
                write_json(&fixture.source.join("sprites.json"), &doc);
            }
            3 => {
                let mut doc = document();
                doc["atlas"]["width"] = json!(3);
                write_json(&fixture.source.join("sprites.json"), &doc);
            }
            4 => {
                fs::write(fixture.source.join("atlas.png"), b"not a PNG").unwrap();
            }
            5 => {
                let bytes = png(&PIXELS);
                fs::write(fixture.source.join("atlas.png"), &bytes[..bytes.len() / 2]).unwrap();
            }
            6 => {
                fs::write(fixture.source.join("LICENSE.txt"), b" \n\t").unwrap();
            }
            7 => {
                fs::write(fixture.source.join("LICENSE.txt"), [255, 254]).unwrap();
            }
            8 => {
                fs::write(
                    fixture.source.join("LICENSE.txt"),
                    vec![b'x'; 64 * 1024 + 1],
                )
                .unwrap();
            }
            9 => {
                fs::write(
                    fixture.source.join("sprites.json"),
                    vec![b' '; 4 * 1024 * 1024 + 1],
                )
                .unwrap();
            }
            10 => {
                fs::write(
                    fixture.source.join("atlas.png"),
                    vec![0; 16 * 1024 * 1024 + 1],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = tree(&fixture.project);
        assert!(
            prepare(&fixture.project, &fixture.source).is_err(),
            "accepted invalid case {case}"
        );
        assert_eq!(tree(&fixture.project), before);
    }
}

#[test]
fn paths_and_symlinked_sources_or_assets_are_rejected_without_cache_writes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let before = tree(&fixture.project);
    assert!(prepare(&fixture.project, Path::new("relative-source")).is_err());
    assert!(prepare(
        &fixture.project,
        &fixture.root.join("new-sprites/../new-sprites")
    )
    .is_err());
    let alias = fixture.root.join("source-alias");
    symlink(&fixture.source, &alias).unwrap();
    assert!(prepare(&fixture.project, &alias).is_err());
    let project_alias = fixture.root.join("project-alias");
    symlink(&fixture.project, &project_alias).unwrap();
    assert!(prepare(&project_alias, &fixture.source).is_err());
    let real_png = fixture.root.join("real-atlas.png");
    fs::rename(fixture.source.join("atlas.png"), &real_png).unwrap();
    symlink(&real_png, fixture.source.join("atlas.png")).unwrap();
    assert!(prepare(&fixture.project, &fixture.source).is_err());
    assert_eq!(tree(&fixture.project), before);
}

#[test]
fn changed_source_bytes_after_preparation_reject_before_callback_and_keep_old_active_lock() {
    for file in [
        "atlas.png",
        "sprites.json",
        "LICENSE.txt",
        "orr.package.json",
    ] {
        let fixture = Fixture::new();
        let previous = fixture.old_graph();
        let plan = prepare(&fixture.project, &fixture.source).unwrap();
        let before_lock = lock_bytes(&fixture.project);
        let path = fixture.source.join(file);
        if file == "orr.package.json" {
            // Package identity canonicalizes manifest JSON layout. Change a
            // semantic field rather than asserting a raw-whitespace fence.
            let mut spec = manifest(&fixture.source);
            spec["version"] = json!("1.0.1");
            write_json(&path, &spec);
        } else if file == "atlas.png" {
            fs::write(path, png(&[0, 0, 0, 255, 255, 255, 255, 255])).unwrap();
        } else {
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
        }
        let called = Cell::new(false);
        let error = apply(&plan, || {
            called.set(true);
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.contains("candidate package lock differs"),
            "{file}: {error}"
        );
        assert!(!called.get());
        assert_eq!(lock_bytes(&fixture.project), before_lock);
        assert_old_objects(&fixture.project, &previous);
        assert!(!fixture
            .opened()
            .list()
            .unwrap()
            .packages
            .contains_key("new-sprites"));
    }
}

#[test]
fn changed_active_lock_rejects_stale_plan_and_preserves_the_new_current_lock() {
    let fixture = Fixture::new();
    fixture.old_graph();
    let stale = prepare(&fixture.project, &fixture.source).unwrap();
    let other = fixture.root.join("other-sprites");
    write_sprite(&other, "other-sprites", "1.0.0");
    let current = apply(&prepare(&fixture.project, &other).unwrap(), || Ok(())).unwrap();
    let before = lock_bytes(&fixture.project);
    let called = Cell::new(false);
    let error = apply(&stale, || {
        called.set(true);
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("active package lock changed"), "{error}");
    assert!(!called.get());
    assert_eq!(lock_bytes(&fixture.project), before);
    assert_eq!(fixture.opened().verify().unwrap(), current);
}

#[test]
fn callback_refusal_runs_under_writer_and_does_not_activate_staged_content() {
    let fixture = Fixture::new();
    let previous = fixture.old_graph();
    let plan = prepare(&fixture.project, &fixture.source).unwrap();
    let before = lock_bytes(&fixture.project);
    let called = Cell::new(0);
    let error = apply(&plan, || {
        called.set(called.get() + 1);
        assert!(fixture.project.join(".orr/packages/writer.lock").is_file());
        assert_eq!(fixture.opened().list().unwrap(), previous);
        let nested = fixture
            .opened()
            .install(std::slice::from_ref(&fixture.source))
            .unwrap_err();
        assert!(
            nested.to_string().contains("writer unavailable"),
            "{nested}"
        );
        Err("live editor context changed".into())
    })
    .unwrap_err();
    assert!(error.contains("live editor context changed"), "{error}");
    assert_eq!(called.get(), 1);
    assert_eq!(lock_bytes(&fixture.project), before);
    assert_old_objects(&fixture.project, &previous);
    assert!(fixture
        .opened()
        .read_asset("new-sprites", "sprites.json")
        .is_err());
    assert!(!fixture.project.join(".orr/packages/writer.lock").exists());
    // Deliberately no assertion that staging was rolled back: immutable orphan
    // objects are permitted, and this API must not promise cleanup or retries.
}

#[test]
fn corrupted_installed_object_blocks_preparation_without_rewriting_the_active_lock() {
    let fixture = Fixture::new();
    let previous = fixture.old_graph();
    let before = lock_bytes(&fixture.project);
    let digest = &previous.packages["old-dependency"].digest;
    let object = fixture
        .project
        .join(".orr/packages/objects")
        .join(digest)
        .join("payload.txt");
    fs::write(&object, b"corrupted prior object").unwrap();
    assert!(prepare(&fixture.project, &fixture.source).is_err());
    assert_eq!(lock_bytes(&fixture.project), before);
    assert_eq!(fs::read(&object).unwrap(), b"corrupted prior object");
    assert!(!fixture.project.join(".orr/packages/writer.lock").exists());
}
