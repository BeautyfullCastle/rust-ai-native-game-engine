use super::{has_sidecar, load, save, sidecar_path, sidecar_stamp};
use crate::mesh_lighting::{Document, MAX_BYTES};
use orr_render::Lighting;
use std::{fs, path::Path};

fn scene(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, b"scene\n").unwrap();
    path
}

fn document() -> Document {
    Document::from_lighting(Lighting::default()).unwrap()
}

fn revision() -> [u8; 32] {
    [0x5a; 32]
}

#[test]
fn lighting_sidecar_roundtrips_and_saves_with_expected_digest() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let lighting = document();
    assert!(!has_sidecar(&scene_path).unwrap());
    let first = save(&scene_path, Some(&lighting), None, revision())
        .unwrap()
        .unwrap();
    assert!(has_sidecar(&scene_path).unwrap());
    let sidecar = sidecar_path(&scene_path).unwrap();
    assert_eq!(sidecar, root.join("level.scene.yaml.mesh-lighting.json"));
    assert_eq!(
        sidecar_stamp(&scene_path).unwrap().as_deref(),
        Some(first.as_str())
    );
    let (loaded, loaded_digest) = load(&scene_path, revision()).unwrap().unwrap();
    assert_eq!(loaded, lighting);
    assert_eq!(loaded_digest, first);

    let changed = Document::from_lighting(Lighting {
        exposure: 2.25,
        tonemap: false,
        ..Lighting::default()
    })
    .unwrap();
    let second = save(&scene_path, Some(&changed), Some(&first), revision())
        .unwrap()
        .unwrap();
    assert_ne!(second, first);
    assert_eq!(load(&scene_path, revision()).unwrap().unwrap().0, changed);
    assert_eq!(
        save(&scene_path, None, Some(&second), revision()).unwrap(),
        None
    );
    assert!(!has_sidecar(&scene_path).unwrap());
}

#[test]
fn conflicting_save_and_remove_preserve_external_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let initial = save(&scene_path, Some(&document()), None, revision())
        .unwrap()
        .unwrap();
    let sidecar = sidecar_path(&scene_path).unwrap();
    let external = b"external writer bytes";
    fs::write(&sidecar, external).unwrap();

    assert!(save(&scene_path, Some(&document()), Some(&initial), revision()).is_err());
    assert!(save(&scene_path, None, Some(&initial), revision()).is_err());
    assert_eq!(fs::read(sidecar).unwrap(), external);
}

#[test]
fn initial_save_does_not_replace_existing_sidecar() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    fs::write(&sidecar, b"preexisting bytes").unwrap();

    assert!(save(&scene_path, Some(&document()), None, revision()).is_err());
    assert_eq!(fs::read(sidecar).unwrap(), b"preexisting bytes");
}

#[test]
fn load_rejects_wrong_graph_corrupt_and_oversized_sidecars() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    save(&scene_path, Some(&document()), None, revision()).unwrap();

    assert!(load(&scene_path, [0x5b; 32]).is_err());
    fs::write(&sidecar, b"not json").unwrap();
    assert!(load(&scene_path, revision()).is_err());
    fs::write(&sidecar, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(load(&scene_path, revision()).is_err());
}

#[test]
fn removal_can_explicitly_discard_invalid_but_matching_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    let invalid = b"invalid lighting state";
    fs::write(&sidecar, invalid).unwrap();
    let observed = sidecar_stamp(&scene_path).unwrap().unwrap();
    assert!(save(&scene_path, Some(&document()), Some(&observed), revision()).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), invalid);

    assert_eq!(
        save(&scene_path, None, Some(&observed), revision()).unwrap(),
        None
    );
    assert!(!sidecar.exists());
}

#[test]
fn nonregular_sidecar_is_rejected_and_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "level.scene.yaml");
    let sidecar = sidecar_path(&scene_path).unwrap();
    fs::create_dir(&sidecar).unwrap();

    assert!(load(&scene_path, revision()).is_err());
    assert!(sidecar_stamp(&scene_path).is_err());
    assert!(save(&scene_path, None, None, revision()).is_err());
    assert!(sidecar.is_dir());
}
