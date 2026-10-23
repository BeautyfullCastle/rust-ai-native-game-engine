use super::{import_package, Request};
use serde_json::Value;
use std::{fs, path::Path};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Example static mesh license\n";

fn prepare_request(root: &Path, source_bytes: &[u8], license_bytes: &[u8]) -> Request {
    let inputs = root.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    let source = inputs.join("source.glb");
    let license = inputs.join("license.txt");
    fs::write(&source, source_bytes).unwrap();
    fs::write(&license, license_bytes).unwrap();
    Request {
        source,
        license,
        destination: root.join("mesh-art-package"),
        name: "mesh-art-package".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        graph_revision: [0x5a; 32],
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
    let binary = &STATIC_GLB[json_end..];
    let total_length = 12 + 8 + json.len() + binary.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(binary);
    glb
}

#[test]
fn imports_and_admits_static_package_without_mutating_source_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = prepare_request(&root, STATIC_GLB, LICENSE);
    let source = request.source.clone();
    let license = request.license.clone();
    let report = import_package(request).unwrap();

    assert_eq!(fs::read(source).unwrap(), STATIC_GLB);
    assert_eq!(fs::read(license).unwrap(), LICENSE);
    assert_eq!(fs::read(report.root.join("model.glb")).unwrap(), STATIC_GLB);
    assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), LICENSE);
    assert_eq!(report.asset, "model.glb");
    assert_eq!(report.license, "LICENSE.txt");
    assert_eq!(report.source_sha256, super::digest(STATIC_GLB));
    assert_eq!(report.license_sha256, super::digest(LICENSE));
    assert_eq!(report.snapshot.graph_revision(), [0x5a; 32]);
    assert!(report.snapshot.placement([orr_fp::FP::ZERO; 3]).is_ok());

    let manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(report.root.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(
        manifest.capabilities,
        std::collections::BTreeSet::from(["models".into()])
    );
    assert!(manifest.dependencies.is_empty());
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let inspected = orr_package::inspect(&report.root, &runtime).unwrap();
    assert_eq!(inspected.manifest, manifest);
    assert_eq!(
        fs::read_dir(report.root).unwrap().count(),
        3,
        "package contains only the declared model, license and manifest"
    );
}

#[test]
fn rejects_unacknowledged_or_invalid_license_without_publishing() {
    for license_bytes in [LICENSE, b" \n\t"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut request = prepare_request(&root, STATIC_GLB, license_bytes);
        let destination = request.destination.clone();
        request.license_acknowledged = false;
        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }

    for license_bytes in [b"\xff".as_slice(), b" \n\t", b""] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let request = prepare_request(&root, STATIC_GLB, license_bytes);
        let destination = request.destination.clone();
        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }
}

#[test]
fn rejects_invalid_or_external_glb_and_preserves_existing_destination() {
    let external = external_uri_glb();
    for source_bytes in [b"not a GLB".as_slice(), external.as_slice()] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let request = prepare_request(&root, source_bytes, LICENSE);
        let destination = request.destination.clone();
        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = prepare_request(&root, STATIC_GLB, LICENSE);
    let destination = request.destination.clone();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(import_package(request).is_err());
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
}

#[test]
fn rejects_bad_identity_and_store_destinations() {
    for (name, version) in [("../escape", "1.0.0"), ("mesh-art-package", "bad-version")] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut request = prepare_request(&root, STATIC_GLB, LICENSE);
        request.name = name.into();
        request.version = version.into();
        let destination = request.destination.clone();
        assert!(import_package(request).is_err());
        assert!(!destination.exists());
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = prepare_request(&root, STATIC_GLB, LICENSE);
    request.destination = root.join(".orr").join("blocked-package");
    assert!(import_package(request).is_err());
}

#[test]
fn rejects_source_path_traversal_and_missing_license() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut request = prepare_request(&root, STATIC_GLB, LICENSE);
    request.source = root.join("inputs").join("..").join("source.glb");
    assert!(import_package(request).is_err());

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let request = prepare_request(&root, STATIC_GLB, LICENSE);
    fs::remove_file(&request.license).unwrap();
    let destination = request.destination.clone();
    assert!(import_package(request).is_err());
    assert!(!destination.exists());
}

#[cfg(windows)]
#[test]
fn rejects_junctioned_input_ancestry() {
    use std::process::Command;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let target = root.join("input-target");
    let junction = root.join("input-junction");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("source.glb"), STATIC_GLB).unwrap();
    fs::write(target.join("license.txt"), LICENSE).unwrap();
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success());
    let request = Request {
        source: junction.join("source.glb"),
        license: target.join("license.txt"),
        destination: root.join("mesh-art-package"),
        name: "mesh-art-package".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        graph_revision: [0x5a; 32],
    };
    assert!(import_package(request).is_err());
    assert!(!root.join("mesh-art-package").exists());
    fs::remove_dir(&junction).unwrap();
}
