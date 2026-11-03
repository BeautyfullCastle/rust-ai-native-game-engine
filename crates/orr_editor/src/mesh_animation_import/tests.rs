use super::*;
use serde_json::Value;

const ANIMATED: &[u8] = include_bytes!("../../../../assets/animation_demo/animated.glb");
const STATIC: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE_BYTES: &[u8] = b"Example animation license\r\nAll original bytes retained.\n";

fn request(root: &Path, source: &[u8], license: &[u8]) -> Request {
    let inputs = root.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    fs::write(inputs.join("source.glb"), source).unwrap();
    fs::write(inputs.join("license.txt"), license).unwrap();
    Request {
        source: inputs.join("source.glb"),
        license: inputs.join("license.txt"),
        destination: root.join("animated-package"),
        name: "animated-package".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        graph_revision: [0x5a; 32],
    }
}

fn changed_glb(change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let json_len = u32::from_le_bytes(ANIMATED[12..16].try_into().unwrap()) as usize;
    let mut json: Value = serde_json::from_slice(&ANIMATED[20..20 + json_len]).unwrap();
    change(&mut json);
    let mut json = serde_json::to_vec(&json).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let binary = &ANIMATED[20 + json_len..];
    let total = 20 + json.len() + binary.len();
    let mut bytes = b"glTF".to_vec();
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&(total as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(binary);
    bytes
}

fn assert_no_stage(root: &Path) {
    assert!(!fs::read_dir(root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-mesh-animation-import-")));
}

#[test]
fn preserves_raw_bytes_and_admits_new_animation_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    let source = req.source.clone();
    let license = req.license.clone();
    let report = import_package(req).unwrap();
    assert!(report.publication_warning.is_none());
    assert_eq!(fs::read(source).unwrap(), ANIMATED);
    assert_eq!(fs::read(license).unwrap(), LICENSE_BYTES);
    assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), ANIMATED);
    assert_eq!(
        fs::read(report.root.join(&report.license)).unwrap(),
        LICENSE_BYTES
    );
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(report.root.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(manifest.capabilities, BTreeSet::from(["animation".into()]));
    assert!(manifest.dependencies.is_empty());
    assert_eq!(
        manifest.files,
        BTreeSet::from([ASSET.into(), LICENSE.into()])
    );
    assert_eq!(fs::read_dir(&report.root).unwrap().count(), 3);
    let again = Snapshot::from_package(&report.root, ASSET, LICENSE, [0x5a; 32]).unwrap();
    assert_eq!(
        again.encode_embedded("mesh.scene.yaml").unwrap(),
        report.snapshot.encode_embedded("mesh.scene.yaml").unwrap()
    );
    assert!(!again.clips().is_empty());
    let sidecar: Value =
        serde_json::from_slice(&again.encode_embedded("mesh.scene.yaml").unwrap()).unwrap();
    assert_eq!(sidecar["asset_sha256"], report.source_sha256);
    assert_eq!(sidecar["license_sha256"], report.license_sha256);
    assert_no_stage(&root);
}

#[test]
fn rejects_static_no_animation_external_and_malformed_sources_atomically() {
    let no_animation = changed_glb(|json| {
        json.as_object_mut().unwrap().remove("animations");
    });
    let external = changed_glb(|json| {
        json["buffers"][0]["uri"] = "external.bin".into();
    });
    let external_image = changed_glb(|json| {
        json["images"] = serde_json::json!([{"uri":"external.png"}]);
    });
    for bytes in [
        STATIC,
        no_animation.as_slice(),
        external.as_slice(),
        external_image.as_slice(),
        b"invalid",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, bytes, LICENSE_BYTES);
        let dest = req.destination.clone();
        assert!(import_package(req).is_err());
        assert!(!dest.exists());
        assert_no_stage(&root);
    }
}

#[test]
fn rejects_license_bounds_source_bounds_and_missing_acknowledgement() {
    for license in [
        b"".as_slice(),
        b" \n\t",
        b"\xff",
        &vec![b'a'; MAX_LICENSE_BYTES as usize + 1],
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        assert!(import_package(request(&root, ANIMATED, license)).is_err());
        assert!(!root.join("animated-package").exists());
        assert_no_stage(&root);
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    fs::OpenOptions::new()
        .write(true)
        .open(&req.source)
        .unwrap()
        .set_len(MAX_SOURCE_BYTES + 1)
        .unwrap();
    assert!(import_package(req).is_err());
    let mut req = request(&root, ANIMATED, LICENSE_BYTES);
    req.license_acknowledged = false;
    assert!(import_package(req).is_err());
    assert_no_stage(&root);
}

#[test]
fn rejects_bad_identity_paths_aliases_and_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for (name, version) in [
        ("../bad", "1.0.0"),
        ("Valid", "1.0.0"),
        ("good", "not-semver"),
    ] {
        let mut req = request(&root, ANIMATED, LICENSE_BYTES);
        req.name = name.into();
        req.version = version.into();
        assert!(import_package(req).is_err());
        assert_no_stage(&root);
    }
    for destination in [
        PathBuf::from("relative"),
        root.join("inputs/../bad"),
        root.join("CON"),
        root.join(".orr/out"),
    ] {
        let mut req = request(&root, ANIMATED, LICENSE_BYTES);
        req.destination = destination;
        assert!(import_package(req).is_err());
    }
    let mut req = request(&root, ANIMATED, LICENSE_BYTES);
    req.license = req.source.clone();
    assert!(import_package(req).is_err());
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    fs::remove_file(&req.license).unwrap();
    fs::hard_link(&req.source, &req.license).unwrap();
    assert!(import_package(req).is_err());
    // Remove the hard link before preparing the next fixture (writes would alias).
    fs::remove_file(root.join("inputs/license.txt")).unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    fs::create_dir(&req.destination).unwrap();
    fs::write(req.destination.join("keep"), b"foreign").unwrap();
    assert!(import_package(req).is_err());
    assert_eq!(
        fs::read(root.join("animated-package/keep")).unwrap(),
        b"foreign"
    );
}

#[test]
fn injected_source_and_stage_changes_do_not_publish() {
    for fault in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, ANIMATED, LICENSE_BYTES);
        let source = req.source.clone();
        let dest = req.destination.clone();
        let result = import_with_hook(req, |phase, stage, _| {
            if phase == Phase::BeforePublish {
                match fault {
                    0 => fs::write(&source, b"changed").unwrap(),
                    1 => fs::write(stage.join(ASSET), b"changed").unwrap(),
                    2 => {
                        fs::remove_file(stage.join(ASSET)).unwrap();
                        fs::write(stage.join(ASSET), ANIMATED).unwrap();
                    }
                    _ => fs::write(stage.join("foreign"), b"keep").unwrap(),
                }
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(!dest.exists());
    }
}

#[test]
fn replacement_stage_is_not_recursively_removed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    let mut replaced = PathBuf::new();
    assert!(import_with_hook(req, |phase, stage, _| {
        if phase == Phase::BeforePublish {
            fs::rename(stage, root.join("retained-original")).unwrap();
            fs::create_dir(stage).unwrap();
            fs::write(stage.join("foreign"), b"keep").unwrap();
            replaced = stage.to_path_buf();
        }
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(replaced.join("foreign")).unwrap(), b"keep");
    assert_eq!(
        fs::read(root.join("retained-original").join(ASSET)).unwrap(),
        ANIMATED
    );
}

#[test]
fn replacement_parent_is_not_published_or_cleaned() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    let parent = root.join("parent");
    fs::create_dir(&parent).unwrap();
    let mut req = req;
    req.destination = parent.join("out");
    let mut replaced = PathBuf::new();
    assert!(import_with_hook(req, |phase, stage, _| {
        if phase == Phase::BeforePublish {
            fs::rename(&parent, root.join("old-parent")).unwrap();
            fs::create_dir(&parent).unwrap();
            fs::create_dir(stage).unwrap();
            fs::write(stage.join("foreign"), b"keep").unwrap();
            replaced = stage.to_path_buf();
        }
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(replaced.join("foreign")).unwrap(), b"keep");
    assert!(!parent.join("out").exists());
}

#[test]
fn publication_race_preserves_existing_destination_and_cleans_own_stage() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    assert!(
        import_with_hook(request(&root, ANIMATED, LICENSE_BYTES), |phase, _, dest| {
            if phase == Phase::BeforePublish {
                fs::create_dir(dest).unwrap();
                fs::write(dest.join("foreign"), b"keep").unwrap();
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(
        fs::read(root.join("animated-package/foreign")).unwrap(),
        b"keep"
    );
    assert_no_stage(&root);
}

#[test]
fn failures_before_publication_cleanup_but_after_publication_report_existing_output() {
    for phase_to_fail in [Phase::Staged, Phase::BeforePublish, Phase::Published] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let result = import_with_hook(request(&root, ANIMATED, LICENSE_BYTES), |phase, _, _| {
            if phase == phase_to_fail {
                Err("injected failure".into())
            } else {
                Ok(())
            }
        });
        if phase_to_fail == Phase::Published {
            let report = result.unwrap();
            assert!(report.root.exists());
            assert!(report
                .publication_warning
                .unwrap()
                .contains("was published"));
        } else {
            assert!(result.is_err());
            assert!(!root.join("animated-package").exists());
        }
        assert_no_stage(&root);
    }
}

#[cfg(unix)]
#[test]
fn symlink_sources_parents_and_destinations_are_rejected() {
    use std::os::unix::fs::symlink;
    for fault in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root, ANIMATED, LICENSE_BYTES);
        match fault {
            0 => {
                let link = root.join("source.glb");
                symlink(&req.source, &link).unwrap();
                req.source = link;
            }
            1 => {
                let link = root.join("linked");
                symlink(root.join("inputs"), &link).unwrap();
                req.source = link.join("source.glb");
            }
            _ => symlink(root.join("missing"), &req.destination).unwrap(),
        }
        assert!(import_package(req).is_err());
        assert_no_stage(&root);
    }
}

#[test]
fn same_byte_source_replacement_is_rejected_by_retained_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, ANIMATED, LICENSE_BYTES);
    let source = req.source.clone();
    assert!(import_with_hook(req, |phase, _, _| {
        if phase == Phase::BeforePublish {
            fs::remove_file(&source).unwrap();
            fs::write(&source, ANIMATED).unwrap();
        }
        Ok(())
    })
    .is_err());
    assert!(!root.join("animated-package").exists());
    assert_no_stage(&root);
}

#[test]
fn native_publisher_never_replaces_even_an_empty_destination() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let stage = root.join("stage");
    let destination = root.join("existing");
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("source"), b"retained").unwrap();
    fs::create_dir(&destination).unwrap();
    assert!(publish_new_directory(&stage, &destination).is_err());
    assert_eq!(fs::read(stage.join("source")).unwrap(), b"retained");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn postpublication_readback_fault_reports_warning_without_claiming_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let report = import_with_hook(request(&root, ANIMATED, LICENSE_BYTES), |phase, _, dest| {
        if phase == Phase::Published {
            fs::write(dest.join(ASSET), b"foreign changed output").unwrap();
        }
        Ok(())
    })
    .unwrap();
    assert!(report
        .publication_warning
        .unwrap()
        .contains("was published"));
    assert_eq!(
        fs::read(report.root.join(ASSET)).unwrap(),
        b"foreign changed output"
    );
    assert_no_stage(&root);
}

#[test]
fn non_glb_extensions_are_rejected_without_staging() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root, ANIMATED, LICENSE_BYTES);
    let renamed = root.join("inputs/animated.gltf");
    fs::rename(&req.source, &renamed).unwrap();
    req.source = renamed;
    assert!(import_package(req).is_err());
    assert_no_stage(&root);
}
