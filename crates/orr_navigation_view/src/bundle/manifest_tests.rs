use super::*;
use crate::bundle::{verify_bundle, MANIFEST_PATH};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
};
const YAML: &str = include_str!("../../../../scenes/navigation_point.scene.yaml");
const TERRAIN: &[u8] = include_bytes!("../../../../scenes/navigation/point_demo.orrt");
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bundle");
        fs::create_dir_all(root.join("project/navigation")).unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        for (name, bytes, mode) in [
            (SCENE_PATH, YAML.as_bytes(), 0o644),
            ("project/navigation/point_demo.orrt", TERRAIN, 0o644),
            (RUNTIME_PATH, b"fixture runtime snapshot".as_slice(), 0o755),
            (LAUNCHER_PATH, LAUNCHER, 0o755),
        ] {
            fs::write(root.join(name), bytes).unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(mode)).unwrap();
        }
        let f = Self { _temp: temp, root };
        let manifest = f.manifest();
        f.write(&manifest);
        f
    }
    fn manifest(&self) -> Manifest {
        let player = AuthoredPlayer::open(&self.root.join(SCENE_PATH)).unwrap();
        Manifest::from_snapshots(
            &Snapshot::read(&self.root.join(SCENE_PATH), MAX_SCENE).unwrap(),
            "navigation/point_demo.orrt",
            &Snapshot::read(
                &self.root.join("project/navigation/point_demo.orrt"),
                orr_terrain::MAX_FILE_BYTES,
            )
            .unwrap(),
            &Snapshot::read(&self.root.join(RUNTIME_PATH), MAX_RUNTIME).unwrap(),
            Some(" user-revision-claim ".into()),
            &player,
        )
        .unwrap()
    }
    fn write(&self, manifest: &Manifest) {
        fs::write(self.root.join(MANIFEST_PATH), manifest.encode().unwrap()).unwrap();
        fs::set_permissions(
            self.root.join(MANIFEST_PATH),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
}
#[test]
fn closed_bundle_matches_initial_state_and_normalizes_published_modes() {
    let f = Fixture::new();
    let player = verify_bundle(&f.root, None).unwrap();
    let manifest = f.manifest();
    assert_eq!(manifest.payload.initial_checksum, player.checksum());
    assert_eq!(
        manifest.payload.source_revision.as_deref(),
        Some("user-revision-claim")
    );
    assert_eq!(
        Manifest::parse(&manifest.encode().unwrap()).unwrap(),
        manifest
    );
    assert!(verify_bundle(&f.root, Some(&f.root.join(RUNTIME_PATH))).is_ok());
    assert!(verify_bundle(&f.root, Some(&f.root.join(LAUNCHER_PATH))).is_err());
    fs::set_permissions(f.root.join(RUNTIME_PATH), fs::Permissions::from_mode(0o700)).unwrap();
    let constructed = f.manifest();
    assert_eq!(constructed.payload.files[2].mode, 0o755);
    assert!(verify_bundle(&f.root, None).is_err());
}
#[test]
fn duplicate_unknown_sequence_and_hash_tampering_are_refused() {
    let f = Fixture::new();
    let original = String::from_utf8(f.manifest().encode().unwrap()).unwrap();
    for text in [
        original.replacen("\"version\": 1", "\"version\": 1, \"version\": 1", 1),
        original.replacen("\"path\":", "\"unknown\": 4, \"path\":", 1),
        original.replacen("\"version\": 1", "\"version\": 2", 1),
    ] {
        assert!(Manifest::parse(text.as_bytes()).is_err());
    }
    let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
    value["payload"]["files"][0]["sha256"] = "0".repeat(64).into();
    assert!(Manifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    value["payload"] = serde_json::json!([]);
    assert!(Manifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(Manifest::parse(b"[]").is_err());
}
#[test]
fn exact_inventory_and_snapshot_recheck_reject_extra_missing_symlink_and_changes() {
    let f = Fixture::new();
    let snapshot = Snapshot::read(&f.root.join(SCENE_PATH), MAX_SCENE).unwrap();
    fs::write(f.root.join(SCENE_PATH), format!("{YAML}# mutation\n")).unwrap();
    assert!(snapshot.recheck().is_err());
    assert!(verify_bundle(&f.root, None).is_err());
    fs::write(f.root.join(SCENE_PATH), YAML).unwrap();
    fs::create_dir(f.root.join("extra-empty")).unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
    fs::remove_dir(f.root.join("extra-empty")).unwrap();
    fs::write(f.root.join("extra"), b"extra").unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
    fs::remove_file(f.root.join("extra")).unwrap();
    fs::remove_file(f.root.join(LAUNCHER_PATH)).unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
    symlink(RUNTIME_PATH, f.root.join(LAUNCHER_PATH)).unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
}
#[test]
fn descriptor_path_collision_traversal_and_initial_checksum_mismatch_fail() {
    let f = Fixture::new();
    for path in [
        "../escape.orrt",
        "project/../escape.orrt",
        "project/game.scene.yaml/child.orrt",
        "/absolute.orrt",
    ] {
        let mut manifest = f.manifest();
        manifest.payload.files[1].path = path.into();
        manifest.content_sha256 = digest(&manifest.payload).unwrap();
        assert!(manifest.validate().is_err(), "{path}");
    }
    let mut manifest = f.manifest();
    manifest.payload.initial_checksum ^= 1;
    manifest.content_sha256 = digest(&manifest.payload).unwrap();
    f.write(&manifest);
    assert!(verify_bundle(&f.root, None).is_err());
}
#[test]
fn launcher_and_manifest_mode_are_bound_and_no_source_claim_is_authority() {
    let f = Fixture::new();
    let mut manifest = f.manifest();
    manifest.payload.source_revision = Some("claim does not authorize other bytes".into());
    manifest.content_sha256 = digest(&manifest.payload).unwrap();
    f.write(&manifest);
    assert!(verify_bundle(&f.root, None).is_ok());
    fs::write(f.root.join(LAUNCHER_PATH), b"#!/bin/sh\nexit 0\n").unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
    fs::write(f.root.join(LAUNCHER_PATH), LAUNCHER).unwrap();
    fs::set_permissions(
        f.root.join(MANIFEST_PATH),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(verify_bundle(&f.root, None).is_err());
}
