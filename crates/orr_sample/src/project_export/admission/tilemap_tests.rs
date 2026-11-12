//! CPU-only closure/admission checks; no exporter smoke, window or GPU claims.
use super::*;
use serde_json::{json, Value};
use crate::project_fixture::ProjectFixture;

const MAP: &str = "arena.tilemap.json";
fn map() -> Value {
    json!({"version":1,"sprite":{"package":"sample-sprites","document":"sprites.json"},
        "width":3,"height":2,"tile_size":[16,16],"origin":[-24,16],"units_per_pixel":1,
        "cells":[10,null,11,20,21,null]})
}
fn declare(root: &Path, actors: bool) -> Vec<u8> {
    let bytes = serde_json::to_vec(&map()).unwrap();
    fs::write(root.join(MAP), &bytes).unwrap();
    let path = root.join(PROJECT_MANIFEST);
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["tilemap"] = MAP.into();
    if !actors {
        manifest["entry"].as_object_mut().unwrap().remove("sprites");
    }
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    bytes
}
fn rejected(root: &Path) -> String {
    ProjectSnapshot::open(root)
        .err()
        .expect("invalid tilemap project must fail admission")
}

#[test]
fn nonempty_tiles_retain_exact_sidecar_and_verified_package_closure() {
    for actors in [false, true] {
        let fixture = ProjectFixture::new();
        let bytes = declare(&fixture.root, actors);
        let snapshot = ProjectSnapshot::open(&fixture.root).unwrap();
        let tile = snapshot.files.iter().find(|f| f.relative == MAP).unwrap();
        assert_eq!(tile.role, "tilemap_sidecar");
        assert_eq!(tile.bytes, bytes);
        assert_eq!(tile.sha256, hash(&bytes));
        assert_eq!(snapshot.entry.tilemap.as_deref(), Some(MAP));
        assert_eq!(snapshot.package_identities.len(), 1);
        let package_digest = &snapshot.package_identities["sample-sprites"];
        let prefix = format!(".orr/packages/objects/{package_digest}/");
        for name in ["sprites.json", "lantern_keeper.png", "orr.package.json"] {
            let relative = format!("{prefix}{name}");
            let file = snapshot
                .files
                .iter()
                .find(|f| f.relative == relative)
                .unwrap();
            assert_eq!(file.bytes, fs::read(fixture.root.join(relative)).unwrap());
            assert_eq!(file.sha256, hash(&file.bytes));
        }
        assert_eq!(
            snapshot
                .files
                .iter()
                .filter(|f| f.role == "tilemap_sidecar")
                .count(),
            1
        );
        snapshot.recheck().unwrap();
    }
}

#[test]
fn same_length_tile_edit_is_rejected_even_with_matching_metadata() {
    let fixture = ProjectFixture::new();
    declare(&fixture.root, true);
    let mut snapshot = ProjectSnapshot::open(&fixture.root).unwrap();
    let mut changed = map();
    changed["cells"][0] = 11.into();
    let changed = serde_json::to_vec(&changed).unwrap();
    let tile = snapshot
        .files
        .iter_mut()
        .find(|f| f.relative == MAP)
        .unwrap();
    assert_eq!(changed.len(), tile.bytes.len());
    fs::write(fixture.root.join(MAP), changed).unwrap();
    // Force matching observed metadata; the exact byte hash must still reject.
    tile.identity = regular_metadata(&tile.source, tile.limit).unwrap();
    assert!(snapshot.recheck().unwrap_err().contains("bytes changed"));
}

#[test]
fn malformed_missing_and_oversized_tilemaps_fail_admission() {
    for mode in 0..3 {
        let fixture = ProjectFixture::new();
        declare(&fixture.root, true);
        let path = fixture.root.join(MAP);
        match mode {
            0 => fs::write(path, b"{\"version\":1,\"version\":1}").unwrap(),
            1 => fs::remove_file(path).unwrap(),
            _ => fs::write(path, vec![b' '; orr_tilemap::MAX_BYTES + 1]).unwrap(),
        }
        assert!(!rejected(&fixture.root).is_empty());
    }
}

#[test]
fn invalid_region_and_combined_sprite_budget_are_not_export_permissions() {
    for invalid_region in [true, false] {
        let fixture = ProjectFixture::new();
        declare(&fixture.root, true);
        let mut document = map();
        if invalid_region {
            document["cells"][0] = 999999.into();
        } else {
            document["width"] = 64.into();
            document["height"] = 64.into();
            document["cells"] = json!(vec![10; 4096]);
        }
        fs::write(
            fixture.root.join(MAP),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        assert!(!rejected(&fixture.root).is_empty());
    }
}

#[test]
fn tampered_or_missing_atlas_and_descriptor_fail_admission() {
    for asset in ["sprites.json", "lantern_keeper.png"] {
        for remove in [false, true] {
            let fixture = ProjectFixture::new();
            declare(&fixture.root, false);
            let snapshot = ProjectSnapshot::open(&fixture.root).unwrap();
            let relative = format!(
                ".orr/packages/objects/{}/{asset}",
                snapshot.package_identities["sample-sprites"]
            );
            let path = fixture.root.join(relative);
            if remove {
                fs::remove_file(path).unwrap();
            } else {
                fs::write(path, b"tampered package asset").unwrap();
            }
            assert!(snapshot.recheck().is_err());
            assert!(!rejected(&fixture.root).is_empty());
        }
    }
}

#[test]
fn tile_sidecar_alias_of_other_admitted_path_is_rejected() {
    for alias in [
        "arena.sprites.json",
        "ARENA.SPRITES.JSON",
        "arena.scene.yaml",
        "orr.project.json",
    ] {
        let fixture = ProjectFixture::new();
        declare(&fixture.root, true);
        let path = fixture.root.join(PROJECT_MANIFEST);
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["entry"]["tilemap"] = alias.into();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(!rejected(&fixture.root).is_empty());
    }
}

#[cfg(unix)]
#[test]
fn tile_sidecar_symlink_is_rejected_before_and_after_snapshot() {
    let fixture = ProjectFixture::new();
    declare(&fixture.root, true);
    let snapshot = ProjectSnapshot::open(&fixture.root).unwrap();
    let original = fixture.root.join("saved-tilemap.json");
    fs::rename(fixture.root.join(MAP), &original).unwrap();
    std::os::unix::fs::symlink(&original, fixture.root.join(MAP)).unwrap();
    assert!(snapshot.recheck().is_err());
    assert!(!rejected(&fixture.root).is_empty());
}
