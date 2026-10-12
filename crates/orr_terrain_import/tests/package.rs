mod common;
use common::*;
use orr_package::{Project, Runtime};
use orr_terrain::Terrain;
use orr_terrain_view::package::{self, install_and_reload};
use std::fs;
#[test]
fn imported_terrain_installs_reopens_and_survives_original_raw_and_source_removal() {
    let root = temp();
    let raw_path = root.path().join("map.raw");
    let bytes = raw(&SAMPLES, orr_terrain_import::Endian::Little);
    fs::write(&raw_path, &bytes).unwrap();
    let cooked = root.path().join("map.orrt");
    orr_terrain_import::publish::import_file(&raw_path, &cooked, &recipe()).unwrap();
    let t = Terrain::load(&fs::read(&cooked).unwrap()).unwrap();
    let project_root = root.path().join("project");
    let source = root.path().join("source");
    let installed = install_and_reload(&t, &project_root, &source, "1.0.0").unwrap();
    fs::write(&raw_path, b"changed raw").unwrap();
    fs::remove_file(&raw_path).unwrap();
    fs::remove_file(&cooked).unwrap();
    fs::remove_dir_all(&source).unwrap();
    let project = Project::open(&project_root, package::runtime()).unwrap();
    project.verify().unwrap();
    let loaded = project
        .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
        .unwrap();
    assert_eq!(loaded, t.cook());
    assert_eq!(
        Terrain::load(&loaded).unwrap().revision(),
        installed.terrain.revision()
    );
}
#[test]
fn package_capability_and_undeclared_asset_rejections_are_real_api_gates() {
    let root = temp();
    let project_root = root.path().join("project");
    install_and_reload(
        &terrain(),
        &project_root,
        &root.path().join("source"),
        "1.0.0",
    )
    .unwrap();
    let wrong = Project::open(&project_root, Runtime::content_only()).unwrap();
    assert!(wrong
        .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
        .is_err());
    let correct = Project::open(&project_root, package::runtime()).unwrap();
    assert!(correct
        .read_asset(package::PACKAGE_NAME, "undeclared.orrt")
        .is_err());
}
#[test]
fn installed_tamper_fails_verification_and_read_asset() {
    let root = temp();
    let project_root = root.path().join("project");
    let installed = install_and_reload(
        &terrain(),
        &project_root,
        &root.path().join("source"),
        "1.0.0",
    )
    .unwrap();
    let object = project_root
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join(package::ASSET_PATH);
    // Deliberate fixture-local corruption is not an importer production write.
    let mut permissions = fs::metadata(&object).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(windows)]
    permissions.set_readonly(false);
    fs::set_permissions(&object, permissions).unwrap();
    fs::write(object, b"tampered").unwrap();
    let fresh = Project::open(&project_root, package::runtime()).unwrap();
    assert!(fresh.verify().is_err());
    assert!(fresh
        .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
        .is_err());
}
#[test]
fn incompatible_engine_and_missing_declared_file_fail_installation() {
    let root = temp();
    let source_root = root.path().join("source");
    source(
        &source_root,
        "terrain-source",
        &["terrain_v1"],
        &["lab.orrt"],
    );
    let project_root = root.path().join("project");
    fs::create_dir(&project_root).unwrap();
    let project = Project::open(&project_root, package::runtime()).unwrap();
    let manifest_path = source_root.join("orr.package.json");
    let original = fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["engine"] = "=99.0.0".into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(project.install(std::slice::from_ref(&source_root)).is_err());
    fs::write(manifest_path, original).unwrap();
    fs::remove_file(source_root.join("lab.orrt")).unwrap();
    assert!(project.install(&[source_root]).is_err());
}
