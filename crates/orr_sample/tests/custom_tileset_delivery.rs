//! Actual creator/export/runtime acceptance. Saved-file fixtures are not Editor UI
//! authoring evidence. Source hiding is ordinary relocation, not sandbox isolation.
#![cfg(all(
    feature = "project-create",
    feature = "project-export",
    feature = "tilemap",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::float_arithmetic)]
#[path = "custom_tileset_delivery/fixture.rs"]
mod fixture;
use fixture::*;
use orr_sample::project_runtime::PreparedRuntime;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn creator_installs_independent_tileset_preserves_actors_and_source_bytes() {
    let f = Fixture::new();
    let source = files(&f.source);
    f.create();
    assert_eq!(files(&f.source), source);
    let twin = f.temp.path().join("same seed twin");
    success(f.command(&twin).output().unwrap());
    assert_eq!(files(&f.project), files(&twin));
    let map = read_json(&f.project.join("arena.tilemap.json"));
    assert_eq!(map["version"], 1);
    assert_eq!(map["width"], 16);
    assert_eq!(map["height"], 12);
    assert_eq!(map["tile_size"], json!([16, 12]));
    assert_eq!(
        map["sprite"],
        json!({"package":PACKAGE,"document":"sprites.json"})
    );
    assert_eq!(map["cells"], json!(vec![None::<u32>; 192]));
    let lock = read_json(&f.project.join("orr.packages.lock.json"));
    assert_eq!(lock["packages"].as_object().unwrap().len(), 2);
    assert_eq!(lock["direct"].as_object().unwrap().len(), 2);
    let object = f
        .project
        .join(".orr/packages/objects")
        .join(lock["packages"][PACKAGE]["digest"].as_str().unwrap());
    for name in ["sprites.json", "atlas.png", "LICENSE.txt"] {
        assert_eq!(
            fs::read(object.join(name)).unwrap(),
            source[Path::new(name)]
        );
    }
    let actor = f.project.join(".orr/packages/objects").join(
        lock["packages"]["sample-sprites"]["digest"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        fs::read(actor.join("lantern_keeper.png")).unwrap(),
        include_bytes!("../../../assets/sprite_demo/lantern_keeper.png")
    );
    let sprites = read_json(&f.project.join("arena.sprites.json"));
    assert_eq!(sprites["bindings"].as_object().unwrap().len(), 2);
    for binding in sprites["bindings"].as_object().unwrap().values() {
        assert_eq!(binding["package"], "sample-sprites");
    }
    let initial = PreparedRuntime::open(&f.project)
        .unwrap()
        .initial_frame()
        .checksum();
    fs::rename(&f.source, f.temp.path().join("source hidden")).unwrap();
    assert_eq!(
        PreparedRuntime::open(&f.project)
            .unwrap()
            .initial_frame()
            .checksum(),
        initial
    );
    let before = files(&f.project);
    assert!(!f.command(&f.project).output().unwrap().status.success());
    assert_eq!(files(&f.project), before);
}

#[test]
fn invalid_external_packages_never_publish_or_modify_sources() {
    for case in [
        "malformed",
        "dependency",
        "capability",
        "extra-payload",
        "missing-atlas",
        "bad-png",
        "oversize",
        "wrong-size",
        "symlink",
    ] {
        let f = Fixture::new();
        match case {
            "malformed" => fs::write(f.source.join("orr.package.json"), b"not json").unwrap(),
            "dependency" | "capability" | "extra-payload" => {
                let path = f.source.join("orr.package.json");
                let mut v = read_json(&path);
                match case {
                    "dependency" => v["dependencies"] = json!({"sample-sprites":"1.0.0"}),
                    "capability" => v["capabilities"] = json!(["sprite", "script"]),
                    _ => {
                        v["files"].as_array_mut().unwrap().push(json!("extra.txt"));
                        fs::write(f.source.join("extra.txt"), b"extra").unwrap();
                    }
                }
                write_json(&path, &v);
            }
            "missing-atlas" => fs::remove_file(f.source.join("atlas.png")).unwrap(),
            "bad-png" => fs::write(f.source.join("atlas.png"), b"not png").unwrap(),
            "oversize" => {
                fs::write(f.source.join("LICENSE.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap()
            }
            "wrong-size" => {
                let path = f.source.join("sprites.json");
                let mut v = read_json(&path);
                for r in v["regions"].as_array_mut().unwrap() {
                    r["height"] = json!(11);
                }
                write_json(&path, &v);
            }
            "symlink" => {
                fs::rename(
                    f.source.join("atlas.png"),
                    f.temp.path().join("outside.png"),
                )
                .unwrap();
                std::os::unix::fs::symlink(
                    f.temp.path().join("outside.png"),
                    f.source.join("atlas.png"),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        f.reject();
    }
}

#[test]
fn malformed_dimensions_and_existing_destination_do_not_publish() {
    let f = Fixture::new();
    for (width, height, package) in [
        ("0", "12", f.source.as_path()),
        ("16", "-1", f.source.as_path()),
        ("4294967296", "12", f.source.as_path()),
        ("16", "12", Path::new("relative")),
    ] {
        let before = files(f.temp.path());
        assert!(
            !f.options(&f.project, package, width, height)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!f.project.exists());
        assert_eq!(files(f.temp.path()), before);
    }
    fs::create_dir(&f.project).unwrap();
    fs::write(f.project.join("keep"), b"existing project").unwrap();
    let before = files(f.temp.path());
    assert!(!f.command(&f.project).output().unwrap().status.success());
    assert_eq!(files(f.temp.path()), before);
}

#[path = "custom_tileset_delivery/export.rs"]
mod export;
