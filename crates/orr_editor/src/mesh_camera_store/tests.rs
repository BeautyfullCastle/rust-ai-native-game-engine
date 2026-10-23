use super::{load, save, sidecar_path, sidecar_stamp};
use crate::mesh_camera::{Document, MAX_BYTES};
use orr_render::camera3d::OrbitCamera;
use std::{fs, path::Path};

fn scene(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, b"scene\\n").unwrap();
    path
}

fn document() -> Document {
    Document::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.7, 0.4, 12.0)).unwrap()
}

fn revision() -> [u8; 32] {
    [0x5a; 32]
}

#[test]
fn camera_sidecar_roundtrips_and_uses_optimistic_save_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.mesh.scene.yaml");
    let camera = document();
    let first_hash = save(&scene_path, Some(&camera), None, revision())
        .unwrap()
        .unwrap();
    let sidecar = sidecar_path(&scene_path).unwrap();
    assert_eq!(sidecar, root.join("room.mesh.scene.yaml.mesh-camera.json"));
    assert_eq!(
        sidecar_stamp(&scene_path).unwrap().as_deref(),
        Some(first_hash.as_str())
    );
    let (loaded, loaded_hash) = load(&scene_path, revision()).unwrap().unwrap();
    assert_eq!(loaded, camera);
    assert_eq!(loaded_hash, first_hash);

    let updated = Document::from_orbit(OrbitCamera::new([3.0, 2.0, 1.0], 0.2, -0.1, 8.0)).unwrap();
    let second_hash = save(&scene_path, Some(&updated), Some(&first_hash), revision())
        .unwrap()
        .unwrap();
    assert_ne!(second_hash, first_hash);
    assert_eq!(load(&scene_path, revision()).unwrap().unwrap().0, updated);
    assert_eq!(
        save(&scene_path, None, Some(&second_hash), revision()).unwrap(),
        None
    );
    assert_eq!(sidecar_stamp(&scene_path).unwrap(), None);
}

#[test]
fn stale_save_and_remove_leave_conflicting_sidecar_untouched() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.scene.yaml");
    let camera = document();
    let old_hash = save(&scene_path, Some(&camera), None, revision())
        .unwrap()
        .unwrap();
    let sidecar = sidecar_path(&scene_path).unwrap();
    let winner = b"newer external writer";
    fs::write(&sidecar, winner).unwrap();

    assert!(save(&scene_path, Some(&camera), Some(&old_hash), revision()).is_err());
    assert!(save(&scene_path, None, Some(&old_hash), revision()).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), winner);
}

#[test]
fn initial_save_never_overwrites_a_preexisting_sidecar() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    let winner = b"existing sidecar";
    fs::write(&sidecar, winner).unwrap();

    assert!(save(&scene_path, Some(&document()), None, revision()).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), winner);
}

#[test]
fn load_rejects_wrong_revision_corrupt_and_oversized_sidecars() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    let camera = document();
    save(&scene_path, Some(&camera), None, revision()).unwrap();

    assert!(load(&scene_path, [0x5b; 32]).is_err());
    fs::write(&sidecar, b"not json").unwrap();
    assert!(load(&scene_path, revision()).is_err());
    fs::write(&sidecar, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(load(&scene_path, revision()).is_err());
}

#[test]
fn nonregular_sidecar_is_rejected_and_not_removed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    fs::create_dir(&sidecar).unwrap();

    assert!(load(&scene_path, revision()).is_err());
    assert!(sidecar_stamp(&scene_path).is_err());
    assert!(save(&scene_path, None, None, revision()).is_err());
    assert!(sidecar.is_dir());
}
