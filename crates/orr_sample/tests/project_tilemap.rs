#![cfg(feature = "tilemap")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use orr_sample::{
    project::{PreparedProject, TilemapSupport, UiSupport},
    project_runtime::{compiled_runtime, PreparedRuntime},
    project_tilemap::{draws, usage},
};
use serde_json::{json, Value};
use std::{fs, path::Path};
#[path = "common/project.rs"]
mod fixture;
use fixture::{copy_tree, ProjectFixture};

fn map() -> Value {
    json!({"version":1,"sprite":{"package":"sample-sprites","document":"sprites.json"},
        "width":3,"height":2,"tile_size":[16,16],"origin":[-24,16],"units_per_pixel":1,
        "cells":[10,null,11,20,21,null]})
}
fn declare(root: &Path, map: Value, actors: bool) {
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let path = root.join("orr.project.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["tilemap"] = "arena.tilemap.json".into();
    if !actors {
        manifest["entry"].as_object_mut().unwrap().remove("sprites");
    }
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}
fn open(root: &Path) -> Result<PreparedProject, String> {
    PreparedProject::open_with_capabilities(
        root,
        compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
}
fn reject(root: &Path) -> String {
    match open(root) {
        Ok(_) => panic!("invalid tilemap admitted"),
        Err(e) => e,
    }
}
/// Reinstall an owned fixture source before declaring the tilemap; no lock JSON
/// or installed-object hashes are hand-forged to bypass package authority.
fn replace_source(fixture: &ProjectFixture, edit: impl FnOnce(&Path)) {
    let project = orr_package::Project::open(&fixture.root, compiled_runtime()).unwrap();
    let lock = project.verify().unwrap();
    let source = fixture.root.join("test-source");
    fs::create_dir(&source).unwrap();
    copy_tree(
        &fixture
            .root
            .join(".orr/packages/objects")
            .join(&lock.packages["sample-sprites"].digest),
        &source,
    );
    edit(&source);
    let manifest_path = source.join("orr.package.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = "0.2.0".into();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    project.install(&[source]).unwrap();
}

#[test]
fn tile_only_and_shared_actor_atlas_admit_once_with_row_major_draws() {
    for actors in [false, true] {
        let fixture = ProjectFixture::new();
        declare(&fixture.root, map(), actors);
        let prepared = open(&fixture.root).unwrap();
        let tiles = prepared.tilemap().unwrap();
        assert_eq!(
            tiles.original_bytes(),
            fs::read(fixture.root.join("arena.tilemap.json")).unwrap()
        );
        assert_eq!(tiles.usage().occupied_cells, 4);
        let draws = draws(tiles, 0).unwrap();
        assert_eq!(
            draws.iter().map(|d| d.instance.region).collect::<Vec<_>>(),
            [10, 11, 20, 21]
        );
        assert_eq!(
            draws
                .iter()
                .map(|d| d.instance.position)
                .collect::<Vec<_>>(),
            [[-16., 8.], [16., 8.], [-16., -8.], [0., -8.]]
        );
        for (index, draw) in draws.iter().enumerate() {
            assert_eq!(draw.instance.size, [16., 16.]);
            assert_eq!(draw.instance.tint, [1.; 4]);
            assert_eq!(draw.instance.rotation, 0.);
            assert!(!draw.instance.flip_x && !draw.instance.flip_y);
            assert_eq!(draw.instance.order, index as i32);
        }
        assert_eq!(
            usage(&fixture.root, compiled_runtime()).unwrap().unwrap(),
            tiles.usage()
        );
        let parts = prepared.into_presentation_parts();
        assert_eq!(parts.tilemap_asset.is_some(), !actors);
        assert_eq!(parts.sprites.is_some(), actors);
        let runtime = PreparedRuntime::open(&fixture.root).unwrap();
        let (_, presentation, _) = runtime.into_launch_parts();
        assert_eq!(presentation.assets().len(), 1);
    }
}

#[test]
fn empty_cells_and_region_zero_are_distinct_and_budget_is_combined() {
    let fixture = ProjectFixture::new();
    replace_source(&fixture, |source| {
        let path = source.join("sprites.json");
        let mut sprite: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut zero = sprite["regions"][0].clone();
        zero["id"] = 0.into();
        sprite["regions"].as_array_mut().unwrap().push(zero);
        fs::write(path, serde_json::to_vec(&sprite).unwrap()).unwrap();
    });
    let mut value = map();
    value["cells"] = json!([null, 0, null, null, 0, null]);
    declare(&fixture.root, value.clone(), false);
    let prepared = open(&fixture.root).unwrap();
    let tiles = prepared.tilemap().unwrap();
    assert_eq!(tiles.document.occupied_count(), 2);
    assert_eq!(draws(tiles, 4094).unwrap().len(), 2);
    assert!(draws(tiles, 4095).is_err());
    assert!(draws(tiles, usize::MAX).is_err());
    value["width"] = 64.into();
    value["height"] = 64.into();
    value["cells"] = json!(vec![Some(0_u32); 4096]);
    declare(&fixture.root, value.clone(), false);
    assert_eq!(
        draws(open(&fixture.root).unwrap().tilemap().unwrap(), 0)
            .unwrap()
            .len(),
        4096
    );
    declare(&fixture.root, value.clone(), true);
    // Re-add actor declaration because declare(false) removed it.
    let path = fixture.root.join("orr.project.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["sprites"] = "arena.sprites.json".into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(reject(&fixture.root).contains("4096"));
    value["cells"] = json!(vec![None::<u32>; 4096]);
    declare(&fixture.root, value, true);
    let prepared = open(&fixture.root).unwrap();
    assert!(draws(prepared.tilemap().unwrap(), 4096).unwrap().is_empty());
}

#[test]
fn malformed_refs_regions_and_bounded_documents_fail_full_admission() {
    for (field, value) in [
        ("package", json!("missing")),
        ("document", json!("../sprites.json")),
        ("document", json!("undeclared.json")),
        ("document", json!("a//b.json")),
    ] {
        let fixture = ProjectFixture::new();
        let mut document = map();
        document["sprite"][field] = value;
        declare(&fixture.root, document, false);
        assert!(!reject(&fixture.root).is_empty());
    }
    for mutation in [0, 1, 2] {
        let fixture = ProjectFixture::new();
        let mut document = map();
        match mutation {
            0 => document["cells"][0] = 999.into(),
            1 => document["tile_size"] = json!([8, 16]),
            _ => document["unsupported"] = true.into(),
        }
        declare(&fixture.root, document, false);
        assert!(!reject(&fixture.root).is_empty());
    }
    let fixture = ProjectFixture::new();
    declare(&fixture.root, map(), false);
    fs::write(
        fixture.root.join("arena.tilemap.json"),
        vec![b' '; orr_tilemap::MAX_BYTES + 1],
    )
    .unwrap();
    assert!(reject(&fixture.root).contains("regular file"));
}

#[test]
fn missing_capability_invalid_png_and_tamper_never_become_usage() {
    let fixture = ProjectFixture::new();
    replace_source(&fixture, |source| {
        fs::write(source.join("lantern_keeper.png"), b"not a PNG").unwrap()
    });
    declare(&fixture.root, map(), false);
    assert!(reject(&fixture.root).contains("PNG"));
    let fixture = ProjectFixture::new();
    // A lock-valid content-only package must not masquerade as a sprite package.
    replace_source(&fixture, |source| {
        let path = source.join("orr.package.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["capabilities"] = json!([]);
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    });
    declare(&fixture.root, map(), false);
    assert!(reject(&fixture.root).contains("sprite capability"));
    let fixture = ProjectFixture::new();
    declare(&fixture.root, map(), false);
    assert!(usage(&fixture.root, orr_package::Runtime::content_only()).is_err());
    let manager = orr_package::Project::open(&fixture.root, compiled_runtime()).unwrap();
    let package = &manager.verify().unwrap().packages["sample-sprites"];
    fs::write(
        fixture
            .root
            .join(".orr/packages/objects")
            .join(&package.digest)
            .join("lantern_keeper.png"),
        b"tampered",
    )
    .unwrap();
    assert!(usage(&fixture.root, compiled_runtime()).is_err());
}

#[test]
fn no_tilemap_usage_is_none_and_legacy_admission_cannot_drop_tiles() {
    let fixture = ProjectFixture::new();
    assert!(usage(&fixture.root, compiled_runtime()).unwrap().is_none());
    declare(&fixture.root, map(), false);
    assert!(PreparedProject::open(&fixture.root, compiled_runtime()).is_err());
    assert!(open(&fixture.root).unwrap().into_parts().is_err());
}

#[test]
fn explicit_null_and_symlink_sidecars_are_rejected() {
    let fixture = ProjectFixture::new();
    declare(&fixture.root, map(), false);
    let manifest_path = fixture.root.join("orr.project.json");
    let original = fs::read(&manifest_path).unwrap();
    let mut manifest: Value = serde_json::from_slice(&original).unwrap();
    manifest["entry"]["tilemap"] = Value::Null;
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(!reject(&fixture.root).is_empty());
    #[cfg(unix)]
    {
        fs::write(&manifest_path, original).unwrap();
        let sidecar = fixture.root.join("arena.tilemap.json");
        fs::rename(&sidecar, fixture.root.join("target.json")).unwrap();
        std::os::unix::fs::symlink("target.json", sidecar).unwrap();
        assert!(reject(&fixture.root).contains("symlink"));
    }
}
