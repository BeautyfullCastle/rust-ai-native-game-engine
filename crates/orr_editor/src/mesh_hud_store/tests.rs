use super::{discard_invalid, has_sidecar, load, save, sidecar_path, sidecar_stamp};
use crate::mesh_hud::Document;
use std::{fs, path::Path};

fn scene(root: &Path) -> std::path::PathBuf {
    let path = root.join("level.scene.yaml");
    fs::write(&path, b"scene\n").unwrap();
    path
}

fn revision() -> [u8; 32] {
    [0x45; 32]
}

#[test]
fn sidecar_saves_loads_updates_and_removes_with_matching_digest() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let document = Document::default();
    assert!(!has_sidecar(&scene_path).unwrap());
    let first = save(&scene_path, Some(&document), None, revision())
        .unwrap()
        .unwrap();
    let expected_path = root.join("level.scene.yaml.mesh-hud.json");
    assert_eq!(sidecar_path(&scene_path).unwrap(), expected_path);
    assert_eq!(
        sidecar_stamp(&scene_path).unwrap().as_deref(),
        Some(first.as_str())
    );
    assert_eq!(
        load(&scene_path, revision()).unwrap().unwrap(),
        (document.clone(), first.clone())
    );

    let mut updated = document;
    updated.title = "Waypoint run".into();
    let second = save(&scene_path, Some(&updated), Some(&first), revision())
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(load(&scene_path, revision()).unwrap().unwrap().0, updated);
    assert_eq!(
        save(&scene_path, None, Some(&second), revision()).unwrap(),
        None
    );
    assert!(!has_sidecar(&scene_path).unwrap());
}

#[test]
fn conflicts_and_invalid_discard_preserve_unexpected_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let initial = save(&scene_path, Some(&Document::default()), None, revision())
        .unwrap()
        .unwrap();
    let path = sidecar_path(&scene_path).unwrap();
    let external = b"external writer bytes";
    fs::write(&path, external).unwrap();
    assert!(save(
        &scene_path,
        Some(&Document::default()),
        Some(&initial),
        revision()
    )
    .is_err());
    assert!(save(&scene_path, None, Some(&initial), revision()).is_err());
    assert!(discard_invalid(&scene_path, &initial, revision()).is_err());
    assert_eq!(fs::read(path).unwrap(), external);
}

#[test]
fn invalid_matching_sidecar_can_be_explicitly_discarded_but_valid_cannot() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let path = sidecar_path(&scene_path).unwrap();
    fs::write(&path, b"not a HUD document").unwrap();
    let observed = sidecar_stamp(&scene_path).unwrap().unwrap();
    assert!(discard_invalid(&scene_path, &observed, revision()).is_ok());
    assert!(!path.exists());

    let valid = save(&scene_path, Some(&Document::default()), None, revision())
        .unwrap()
        .unwrap();
    assert!(discard_invalid(&scene_path, &valid, revision()).is_err());
    assert!(path.exists());
}

#[test]
fn existing_sidecar_is_never_replaced_by_initial_save() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let path = sidecar_path(&scene_path).unwrap();
    fs::write(&path, b"preserve me").unwrap();
    assert!(save(&scene_path, Some(&Document::default()), None, revision()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"preserve me");
}
