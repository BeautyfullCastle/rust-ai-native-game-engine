use super::*;
use orr_fp::{FPVec3, FP};
use orr_reflect::Value;
use orr_remote::json::value_to_json;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn wall_template() -> Template {
    let body = json!({
        "pos": value_to_json(&Value::Vec3(FPVec3::new(FP::ZERO, FP::HALF, FP::ZERO))),
        "rot": {"kind":"unit","x":0,"y":0,"z":0,"w":1},
        "vel": value_to_json(&Value::Vec3(FPVec3::new(FP::ZERO, FP::ZERO, FP::ZERO))),
        "omega": value_to_json(&Value::Vec3(FPVec3::new(FP::ZERO, FP::ZERO, FP::ZERO))),
        "inv_mass": value_to_json(&Value::Fixed(FP::ZERO)),
        "inv_inertia": value_to_json(&Value::Vec3(FPVec3::new(FP::ZERO, FP::ZERO, FP::ZERO))),
        "linear_damping": value_to_json(&Value::Fixed(FP::ZERO)),
        "angular_damping": value_to_json(&Value::Fixed(FP::ZERO)),
        "kind":"static"
    });
    let collider = json!({
        "shape": {
            "kind":"box",
            "half_extents": value_to_json(&Value::Vec3(FPVec3::new(FP::ONE, FP::ONE, FP::ONE)))
        },
        "restitution": value_to_json(&Value::Fixed(FP::ZERO)),
        "friction": value_to_json(&Value::Fixed(FP::HALF)),
        "layer":1,
        "mask":0
    });
    Template::capture(vec![(
        "Wall template".into(),
        json!({
            "RoomEscapeV1::Actor":{"kind":4,"ordinal":0},
            "orr_physics3d::Body":body,
            "orr_physics3d::Collider":collider
        }),
    )])
    .expect("valid room wall template")
}

fn path_in(directory: &Path) -> PathBuf {
    directory.join("unicode wall stamp 東京.room-stamp.json")
}

#[test]
fn stamp_roundtrip_is_bounded_and_requires_explicit_hash_for_replace() {
    let directory = tempfile::tempdir().unwrap();
    let path = path_in(directory.path());
    let template = wall_template();
    let original = template.encode().unwrap();
    let original_hash = save(&path, &template, None).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(digest(&fs::read(&path).unwrap()), original_hash);
    assert!(save(&path, &template, None).is_err());

    let (loaded, loaded_hash) = load(&path).unwrap().unwrap();
    assert_eq!(loaded_hash, original_hash);
    assert_eq!(loaded.encode().unwrap(), original);

    let replacement = Template::capture(vec![
        (
            "Wall template".into(),
            template.walls()[0].components.clone(),
        ),
        ("Second wall".into(), template.walls()[0].components.clone()),
    ])
    .unwrap();
    let replacement_hash = save(&path, &replacement, Some(&loaded_hash)).unwrap();
    assert_ne!(replacement_hash, loaded_hash);
    assert_eq!(load(&path).unwrap().unwrap().0.walls().len(), 2);
    assert!(save(&path, &template, Some(&loaded_hash)).is_err());
}

#[test]
fn stamp_rejects_bad_names_traversal_nonregular_and_malformed_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let template = wall_template();
    assert!(checked_path(Path::new("relative.room-stamp.json")).is_err());
    assert!(checked_path(&directory.path().join("..").join("bad.room-stamp.json")).is_err());
    assert!(checked_path(&directory.path().join("NUL.room-stamp.json")).is_err());
    assert!(checked_path(&directory.path().join("COM¹.room-stamp.json")).is_err());
    assert!(checked_path(&directory.path().join("bad.json")).is_err());
    assert!(checked_path(&directory.path().join("bad?.room-stamp.json")).is_err());
    assert!(validate_expected(Some(&"A".repeat(64))).is_err());
    assert!(validate_expected(Some(&"a".repeat(63))).is_err());

    let nonregular = directory.path().join("folder.room-stamp.json");
    fs::create_dir(&nonregular).unwrap();
    assert!(save(&nonregular, &template, None).is_err());

    let malformed = directory.path().join("malformed.room-stamp.json");
    fs::write(&malformed, b"{\"unexpected\":true}").unwrap();
    assert!(load(&malformed).is_err());

    let oversized = directory.path().join("oversized.room-stamp.json");
    fs::write(&oversized, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(load(&oversized).is_err());
}

#[cfg(windows)]
#[test]
fn stamp_rejects_reparse_file_and_directory_paths() {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.room-stamp.json");
    fs::write(&target, b"{}").unwrap();
    let file_link = directory.path().join("linked.room-stamp.json");
    if symlink_file(&target, &file_link).is_ok() {
        assert!(load(&file_link).is_err());
    }
    let outside = tempfile::tempdir().unwrap();
    let directory_link = directory.path().join("linked-directory");
    if symlink_dir(outside.path(), &directory_link).is_ok() {
        assert!(checked_path(&directory_link.join("wall.room-stamp.json")).is_err());
    }
}
