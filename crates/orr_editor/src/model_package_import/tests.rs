use super::{import_package, Request, SourceFormat};
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
        source_format: SourceFormat::RawGlb,
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

fn cooked_bytes() -> Vec<u8> {
    let imported =
        orr_model::import::import_with_resolver("variants/blue.recolor.json", STATIC_GLB, |_| {
            panic!("fixture is self-contained")
        })
        .unwrap();
    let mut source = imported.source().clone();
    source.dependencies.push(orr_model::Dependency {
        uri: "historical/unavailable.model.json".into(),
        sha256: "ab".repeat(32),
    });
    let model = orr_model::StaticModel::new(source).unwrap();
    // Deliberately not the canonical serialization: publication must not recook.
    let mut bytes = b" \n\t".to_vec();
    bytes.extend(serde_json::to_vec_pretty(model.source()).unwrap());
    bytes.extend_from_slice(b"\n\t ");
    bytes
}

fn cooked_request(root: &Path, source: &[u8]) -> Request {
    let mut request = make_request(root, source);
    let cooked_source = request.source.with_file_name("variant.model.json");
    fs::rename(&request.source, &cooked_source).unwrap();
    request.source = cooked_source;
    request.source_format = SourceFormat::CookedStatic;
    request.destination = root.join("cooked-model-package");
    request.name = "cooked-model-package".into();
    request
}

#[test]
fn cooked_static_preserves_noncanonical_bytes_identity_history_license_and_hashes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let bytes = cooked_bytes();
    let before = orr_model::StaticModel::from_bytes(&bytes).unwrap();
    assert_ne!(before.to_bytes().unwrap(), bytes);
    let request = cooked_request(&root, &bytes);
    let source_path = request.source.clone();
    let license = b"  Authored variant rights\r\n\tLicense terms.\n";
    fs::write(&request.license, license).unwrap();
    assert!(!root
        .join("input/historical/unavailable.model.json")
        .exists());

    let report = import_package(request).unwrap();
    let output = fs::read(report.root.join("model.orrmodel")).unwrap();
    assert_eq!(output, bytes);
    assert_eq!(fs::read(source_path).unwrap(), bytes);
    assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), license);
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(report.source_sha256, super::digest(&bytes));
    assert_eq!(report.license_sha256, super::digest(license));
    let after = orr_model::StaticModel::from_bytes(&output).unwrap();
    assert_eq!(after.source(), before.source());
    assert_eq!(after.source().asset_id, "variants/blue.recolor.json");
    assert_eq!(
        after.source().dependencies[1].uri,
        "historical/unavailable.model.json"
    );
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.capabilities,
        std::collections::BTreeSet::from(["models".into()])
    );
    assert!(package.manifest.dependencies.is_empty());
    assert_eq!(
        package.manifest.files,
        std::collections::BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
    );
    assert_eq!(fs::read_dir(&report.root).unwrap().count(), 3);
    assert!(!report.root.join("model.glb").exists());
}

#[test]
fn cooked_static_rejects_malformed_json_and_raw_glb_without_fallback() {
    for bytes in [b"not JSON".as_slice(), b"null", STATIC_GLB] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let request = cooked_request(&root, bytes);
        let destination = request.destination.clone();
        let error = import_package(request).err().unwrap();
        assert!(error.contains("cooked static model import"), "{error}");
        assert!(!destination.exists());
    }
}

#[test]
fn cooked_static_validates_version_dependencies_and_complete_runtime_geometry() {
    let valid: Value = serde_json::from_slice(&cooked_bytes()).unwrap();
    for (pointer, replacement) in [
        ("/version", serde_json::json!(2)),
        ("/format", serde_json::json!("orr_animated_model")),
        ("/dependencies/0/sha256", serde_json::json!("invalid")),
        ("/dependencies/1/uri", serde_json::json!("../escape.bin")),
        ("/dependencies/1/uri", serde_json::json!("$source")),
        ("/primitives/0/indices/0", serde_json::json!(u32::MAX)),
        ("/primitives/0/material", serde_json::json!(u32::MAX)),
        (
            "/primitives/0/vertices/0/normal",
            serde_json::json!([0, 0, 0]),
        ),
        (
            "/primitives/0/transform",
            serde_json::to_value([[0; 4]; 4]).unwrap(),
        ),
        ("/materials/0/image", serde_json::json!(u32::MAX)),
        ("/images/0/rgba8", serde_json::json!([])),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut document = valid.clone();
        *document.pointer_mut(pointer).unwrap() = replacement;
        let bytes = serde_json::to_vec(&document).unwrap();
        let request = cooked_request(&root, &bytes);
        let destination = request.destination.clone();
        assert!(import_package(request).is_err(), "accepted {pointer}");
        assert!(!destination.exists(), "published {pointer}");
    }
}

#[test]
fn cooked_static_requires_static_mode_and_explicit_model_json_suffix() {
    for suffix in [
        "variant.glb",
        "variant.json",
        "variant.orrmodel",
        "variant.model.json.bak",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut request = cooked_request(&root, &cooked_bytes());
        let source = request.source.with_file_name(suffix);
        fs::rename(&request.source, &source).unwrap();
        request.source = source;
        let destination = request.destination.clone();
        let error = import_package(request).err().unwrap();
        assert!(error.contains(".model.json"), "{error}");
        assert!(!destination.exists());
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = cooked_request(&root, &cooked_bytes());
    request.kind = ModelKind::Animated;
    let destination = request.destination.clone();
    let error = import_package(request).err().unwrap();
    assert!(error.contains("requires Static model mode"), "{error}");
    assert!(!destination.exists());
}

#[test]
fn cooked_static_rejects_oversize_source_before_parsing_or_publishing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = cooked_request(&root, b"{}");
    fs::OpenOptions::new()
        .write(true)
        .open(&request.source)
        .unwrap()
        .set_len(super::MAX_SOURCE_BYTES + 1)
        .unwrap();
    let destination = request.destination.clone();
    let error = import_package(request).err().unwrap();
    assert!(error.contains("byte limit"), "{error}");
    assert!(!destination.exists());
}

#[test]
fn cooked_static_requires_rights_and_bounded_nonempty_utf8_license() {
    for (acknowledged, license) in [
        (false, b"license".to_vec()),
        (true, Vec::new()),
        (true, b" \n\t".to_vec()),
        (true, vec![0xff]),
        (true, vec![b'x'; super::MAX_LICENSE_BYTES as usize + 1]),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut request = cooked_request(&root, &cooked_bytes());
        request.license_acknowledged = acknowledged;
        fs::write(&request.license, license).unwrap();
        let destination = request.destination.clone();
        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }
}

#[test]
fn cooked_static_preserves_existing_destination_file_or_directory() {
    for directory in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let request = cooked_request(&root, &cooked_bytes());
        let destination = request.destination.clone();
        let sentinel = if directory {
            fs::create_dir(&destination).unwrap();
            destination.join("model.orrmodel")
        } else {
            destination.clone()
        };
        fs::write(&sentinel, b"preserve existing output").unwrap();
        assert!(import_package(request).is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"preserve existing output");
    }
}

#[test]
fn publication_failure_cleans_own_output_for_both_asset_names() {
    for asset_name in ["model.glb", "model.orrmodel"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let stage = root.join("stage");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join(asset_name), b"validated source").unwrap();
        // Missing staged license fails after the model was written. This tests
        // the existing trusted, quiescent-directory cleanup policy unchanged.
        let destination = root.join("published");
        let error = super::publish_directory(
            &stage,
            &destination,
            asset_name,
            b"validated source",
            b"license",
            b"manifest",
        )
        .unwrap_err();
        assert!(error.contains("read staged LICENSE.txt"), "{error}");
        assert!(!destination.exists());
        assert_eq!(
            fs::read(stage.join(asset_name)).unwrap(),
            b"validated source"
        );
    }
}

#[test]
fn raw_glb_default_remains_explicit_and_does_not_accept_cooked_json() {
    assert_eq!(SourceFormat::default(), SourceFormat::RawGlb);
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = make_request(&root, &cooked_bytes());
    let destination = request.destination.clone();
    let error = import_package(request).err().unwrap();
    assert!(error.contains("static GLB import"), "{error}");
    assert!(!destination.exists());
}
