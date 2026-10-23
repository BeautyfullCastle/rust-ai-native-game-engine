use super::{import_package, Request};
use crate::model_bindings::ModelKind;
use serde_json::Value;
use std::{fs, path::Path};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");

fn make_request(root: &Path, source: &[u8]) -> Request {
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    let source_path = input.join("source.glb");
    fs::write(&source_path, source).unwrap();
    let license_path = input.join("LICENSE.txt");
    fs::write(&license_path, b"Example license text\n").unwrap();
    Request {
        source: source_path,
        license: license_path,
        destination: root.join("raw-model-package"),
        name: "raw-model-package".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        license_acknowledged: true,
    }
}

fn external_uri_glb() -> Vec<u8> {
    let json_length = u32::from_le_bytes(STATIC_GLB[12..16].try_into().unwrap()) as usize;
    let json_end = 20 + json_length;
    let mut document: Value = serde_json::from_slice(&STATIC_GLB[20..json_end]).unwrap();
    document["buffers"][0]["uri"] = Value::String("external.bin".into());
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let binary_chunk = &STATIC_GLB[json_end..];
    let total_length = 12 + 8 + json.len() + binary_chunk.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(binary_chunk);
    glb
}

#[test]
fn imports_valid_static_glb_unchanged_and_copies_explicit_license() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, STATIC_GLB);
    let source = request.source.clone();
    let license = request.license.clone();
    let license_bytes = fs::read(&license).unwrap();
    let report = import_package(request).unwrap();

    assert_eq!(fs::read(source).unwrap(), STATIC_GLB);
    assert_eq!(fs::read(report.root.join("model.glb")).unwrap(), STATIC_GLB);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        license_bytes
    );
    assert_eq!(report.asset, "model.glb");
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(report.source_sha256, super::digest(STATIC_GLB));
    assert_eq!(
        report.license_sha256,
        super::digest(b"Example license text\n")
    );
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.capabilities,
        std::collections::BTreeSet::from(["models".into()])
    );
}

#[test]
fn invalid_glb_does_not_publish_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, b"not a GLB");
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[test]
fn existing_destination_is_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, STATIC_GLB);
    let destination = request.destination.clone();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep").unwrap();

    assert!(import_package(request).is_err());
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
}

#[test]
fn license_acknowledgement_is_required_before_destination_creation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = make_request(&root, STATIC_GLB);
    request.license_acknowledged = false;
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[test]
fn invalid_package_name_and_version_do_not_publish() {
    for (name, version) in [("../escape", "1.0.0"), ("valid-name", "not-semver")] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut request = make_request(&root, STATIC_GLB);
        request.name = name.into();
        request.version = version.into();
        let destination = request.destination.clone();

        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }
}

#[test]
fn oversized_license_is_rejected_without_publishing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, STATIC_GLB);
    fs::write(&request.license, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[test]
fn windows_filename_and_store_paths_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = make_request(&root, STATIC_GLB);
    request.source = root.join("input").join("COM¹.glb");
    assert!(import_package(request).is_err());

    let mut request = make_request(&root, STATIC_GLB);
    request.source = root.join("input").join("source.glb:stream");
    assert!(import_package(request).is_err());

    let mut request = make_request(&root, STATIC_GLB);
    request.destination = root.join(".orr").join("packages").join("raw-model-package");
    assert!(import_package(request).is_err());
}

#[test]
fn canonical_store_destination_is_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let store = root.join(".orr");
    fs::create_dir(&store).unwrap();
    let destination = store.join("packages").join("raw-model-package");

    assert!(super::validate_destination_policy(&destination).is_err());
}

#[test]
fn external_glb_resources_are_rejected_without_publishing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, &external_uri_glb());
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[test]
fn invalid_license_and_traversal_path_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, STATIC_GLB);
    fs::write(&request.license, []).unwrap();
    let destination = request.destination.clone();
    assert!(import_package(request).is_err());
    assert!(!destination.exists());

    let mut request = make_request(&root, STATIC_GLB);
    request.source = root.join("..").join("outside.glb");
    assert!(import_package(request).is_err());
}

#[cfg(feature = "room-animated-models")]
#[test]
fn imports_animated_glb_only_with_clip_and_explicit_capability() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let animated = include_bytes!("../../../orr_model/tests/fixtures/animated_strip.glb");
    let mut request = make_request(&root, animated);
    request.kind = ModelKind::Animated;

    let report = import_package(request).unwrap();
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert!(package.manifest.capabilities.contains("models"));
    assert!(package.manifest.capabilities.contains("animation"));
}

#[cfg(feature = "room-animated-models")]
#[test]
fn animated_import_rejects_a_model_without_clips() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = make_request(&root, STATIC_GLB);
    request.kind = ModelKind::Animated;
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[cfg(not(feature = "room-animated-models"))]
#[test]
fn animated_request_is_rejected_without_room_animation_support() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let animated = include_bytes!("../../../orr_model/tests/fixtures/animated_strip.glb");
    let mut request = make_request(&root, animated);
    request.kind = ModelKind::Animated;
    let destination = request.destination.clone();

    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}
