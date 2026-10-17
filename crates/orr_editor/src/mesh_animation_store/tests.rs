use super::*;
#[cfg(feature = "mesh-animated-models")]
use crate::mesh_animation::Snapshot;
use std::fs;
#[cfg(feature = "mesh-animated-models")]
use std::path::Path;

#[cfg(feature = "mesh-animated-models")]
const GRAPH_REVISION: [u8; 32] = [0x4a; 32];

#[cfg(feature = "mesh-animated-models")]
fn fixture() -> Snapshot {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", GRAPH_REVISION).unwrap()
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn sidecar_round_trip_is_scene_bound_and_digest_guarded() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("test.mesh.yaml");
    fs::write(&scene, b"scene").unwrap();
    let snapshot = fixture();
    let initial_digest = save(&scene, Some(&snapshot), None, GRAPH_REVISION)
        .unwrap()
        .unwrap();
    assert_eq!(
        sidecar_path(&scene).unwrap().file_name().unwrap(),
        "test.mesh.yaml.mesh-animation.json"
    );
    let (reopened, reopened_digest) = load(&scene, GRAPH_REVISION).unwrap().unwrap();
    assert_eq!(initial_digest, reopened_digest);
    assert_eq!(reopened.clips(), snapshot.clips());
    assert!(load(&scene, [0x11; 32]).is_err());
    assert!(save(&scene, Some(&snapshot), None, GRAPH_REVISION).is_err());
    let modified = snapshot
        .clone()
        .with_settings(crate::mesh_animation::Settings {
            rate: 1.25,
            ..snapshot.settings()
        })
        .unwrap();
    let modified_digest = save(
        &scene,
        Some(&modified),
        Some(&initial_digest),
        GRAPH_REVISION,
    )
    .unwrap()
    .unwrap();
    assert_ne!(initial_digest, modified_digest);
    assert!(save(&scene, None, Some(&initial_digest), GRAPH_REVISION).is_err());
    assert_eq!(
        save(&scene, None, Some(&modified_digest), GRAPH_REVISION).unwrap(),
        None
    );
    assert!(load(&scene, GRAPH_REVISION).unwrap().is_none());
}

#[test]
fn sidecar_presence_detects_orphaned_files_without_decoding_them() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("test.mesh.yaml");
    fs::write(&scene, b"scene").unwrap();
    let sidecar = super::sidecar_path(&scene).unwrap();
    fs::write(&sidecar, b"not valid animation JSON").unwrap();
    assert!(super::has_sidecar(&scene).unwrap());
    fs::remove_file(&sidecar).unwrap();
    assert!(!super::has_sidecar(&scene).unwrap());
}

#[cfg(unix)]
#[test]
fn sidecar_presence_detects_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("test.mesh.yaml");
    fs::write(&scene, b"scene").unwrap();
    let sidecar = super::sidecar_path(&scene).unwrap();
    symlink(directory.path().join("missing-target"), &sidecar).unwrap();
    assert!(super::has_sidecar(&scene).unwrap());
}
