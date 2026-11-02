use super::*;
use std::os::unix::fs::symlink;
fn metadata() -> Metadata {
    Metadata {
        name: "assembled-test".into(),
        version: "1.0.0".into(),
        engine: "*".into(),
    }
}
// Publisher fixtures are bytes only: package admission is not model decoding.
const MODEL_BYTES: &[u8] = b"publisher fixture, not a valid model";
const LICENSE_BYTES: &[u8] = b"User supplied fixture license\n";
#[test]
fn snapshot_refuses_links_fifo_bounds_and_internal_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("input"), b"abc").unwrap();
    assert_eq!(snapshot(&root.join("input"), 3).unwrap(), b"abc");
    assert!(snapshot(&root.join("input"), 2).is_err());
    symlink(root.join("input"), root.join("alias")).unwrap();
    assert!(snapshot(&root.join("alias"), 8).is_err());
    symlink(root, root.join("parent-alias")).unwrap();
    assert!(snapshot(&root.join("parent-alias/input"), 8).is_err());
    assert!(snapshot(&root.join("../input"), 8).is_err());
    fs::create_dir(root.join(".orr")).unwrap();
    fs::write(root.join(".orr/input"), b"a").unwrap();
    assert!(snapshot(&root.join(".orr/input"), 8).is_err());
    unix::mknodat(
        unix::CWD,
        root.join("pipe"),
        unix::FileType::Fifo,
        Mode::RUSR | Mode::WUSR,
        0,
    )
    .unwrap();
    assert!(snapshot(&root.join("pipe"), 8).is_err());
    assert!(snapshot(root, 8).is_err());
}
#[test]
fn publication_is_complete_exact_and_never_replaces_destination() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("package");
    preflight_output(&output).unwrap();
    let report = publish(&output, MODEL_BYTES, LICENSE_BYTES, &metadata()).unwrap();
    assert_eq!(report.output, output);
    assert!(report.durability_warning.is_none());
    assert_eq!(fs::read(output.join(MODEL)).unwrap(), MODEL_BYTES);
    assert_eq!(fs::read(output.join(LICENSE)).unwrap(), LICENSE_BYTES);
    let before = fs::read(output.join(MANIFEST)).unwrap();
    assert!(preflight_output(&output).is_err());
    assert!(publish(&output, b"different", LICENSE_BYTES, &metadata()).is_err());
    assert_eq!(fs::read(output.join(MANIFEST)).unwrap(), before);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    let dangling = temp.path().join("dangling");
    symlink("absent", &dangling).unwrap();
    assert!(preflight_output(&dangling).is_err());
    assert!(publish(&dangling, MODEL_BYTES, LICENSE_BYTES, &metadata()).is_err());
    let alias = temp.path().join("alias");
    symlink(temp.path(), &alias).unwrap();
    assert!(publish(&alias.join("out"), MODEL_BYTES, LICENSE_BYTES, &metadata()).is_err());
}
#[test]
fn before_publish_failure_cleans_only_owned_stage_and_after_publish_never_rolls_back() {
    for failpoint in [
        "stage-created",
        "manifest-created",
        "before-rename",
        "after-rename",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        fs::write(temp.path().join("keep"), b"original").unwrap();
        let result = publish_with(
            &output,
            MODEL_BYTES,
            LICENSE_BYTES,
            &metadata(),
            |point, _| {
                if point == failpoint {
                    Err("injected failure".into())
                } else {
                    Ok(())
                }
            },
        );
        if failpoint == "after-rename" {
            let report = result.unwrap();
            assert!(report.durability_warning.unwrap().contains("uncertain"));
            assert_eq!(fs::read(output.join(MODEL)).unwrap(), MODEL_BYTES);
            crate::inspect_for_install(&output, crate::Runtime::content_only().engine_version)
                .unwrap();
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
        } else {
            assert!(result.unwrap_err().contains("injected failure"));
            assert!(!output.exists());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        }
        assert_eq!(fs::read(temp.path().join("keep")).unwrap(), b"original");
    }
}
#[test]
fn collision_and_replaced_stage_identity_are_never_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("out");
    let mut collision = None;
    publish_with(
        &output,
        MODEL_BYTES,
        LICENSE_BYTES,
        &metadata(),
        |point, path| {
            if point == "stage-candidate" && collision.is_none() {
                fs::create_dir(path).unwrap();
                fs::write(path.join("foreign"), b"keep").unwrap();
                collision = Some(path.to_owned());
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(collision.unwrap().join("foreign")).unwrap(),
        b"keep"
    );
    let output = temp.path().join("replaced-out");
    let mut replaced = None;
    let error = publish_with(
        &output,
        MODEL_BYTES,
        LICENSE_BYTES,
        &metadata(),
        |point, path| {
            if point == "stage-created" {
                fs::rename(path, temp.path().join("owned-moved")).unwrap();
                fs::create_dir(path).unwrap();
                fs::write(path.join("foreign"), b"replacement").unwrap();
                replaced = Some(path.to_owned());
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("cleanup incomplete"));
    assert!(!output.exists());
    assert_eq!(
        fs::read(replaced.unwrap().join("foreign")).unwrap(),
        b"replacement"
    );
    assert!(temp.path().join("owned-moved").is_dir());
}
#[test]
fn tampered_stage_and_late_output_collision_fail_without_overwrite() {
    for mutation in ["model", "license", "output"] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let result = publish_with(
            &output,
            MODEL_BYTES,
            LICENSE_BYTES,
            &metadata(),
            |point, path| {
                if point == "before-rename" {
                    match mutation {
                        "model" => fs::write(path.join(MODEL), b"tamper").unwrap(),
                        "license" => fs::write(path.join(LICENSE), b"tamper").unwrap(),
                        _ => {
                            fs::create_dir(&output).unwrap();
                            fs::write(output.join("keep"), b"existing").unwrap();
                        }
                    }
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        if mutation == "output" {
            assert_eq!(fs::read(output.join("keep")).unwrap(), b"existing");
        } else {
            assert!(!output.exists());
        }
        assert_eq!(
            fs::read_dir(temp.path()).unwrap().count(),
            usize::from(mutation == "output")
        );
    }
}

#[test]
fn valid_manifest_mutations_cannot_change_requested_package_contract() {
    for mutation in [
        "name",
        "version",
        "engine",
        "capabilities",
        "dependencies",
        "files",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        fs::write(temp.path().join("keep"), b"existing unrelated data").unwrap();
        let mut ordinary_admission_succeeded = false;
        let result = publish_with(
            &output,
            MODEL_BYTES,
            LICENSE_BYTES,
            &metadata(),
            |point, path| {
                if point == "manifest-created" {
                    let target = path.join(MANIFEST);
                    let mut manifest: crate::Manifest =
                        serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
                    match mutation {
                        "name" => manifest.name = "different-package".into(),
                        "version" => manifest.version = "2.0.0".into(),
                        "engine" => manifest.engine = ">=0.0.0".into(),
                        "capabilities" => {
                            manifest.capabilities.insert("animation".into());
                        }
                        "dependencies" => {
                            manifest.dependencies.insert("other".into(), "1.0.0".into());
                        }
                        "files" => {
                            manifest.files.remove(MODEL);
                        }
                        _ => unreachable!(),
                    }
                    fs::write(&target, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
                    // Every mutation is independently valid package metadata. This
                    // regression must fail on request binding, not generic admission.
                    crate::inspect_for_install(path, crate::Runtime::content_only().engine_version)
                        .unwrap();
                    ordinary_admission_succeeded = true;
                }
                Ok(())
            },
        );
        assert!(ordinary_admission_succeeded, "{mutation}");
        assert!(
            result
                .unwrap_err()
                .contains("differs from requested metadata"),
            "{mutation}"
        );
        assert!(!output.exists(), "{mutation}");
        assert_eq!(
            fs::read(temp.path().join("keep")).unwrap(),
            b"existing unrelated data"
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1, "{mutation}");
    }
}

#[cfg(feature = "animation-authoring")]
#[test]
fn animated_publication_has_exact_closed_profile_and_no_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("animated");
    let kind = PackageKind::AnimatedSequence;
    let report = publish_kind(&output, MODEL_BYTES, LICENSE_BYTES, &metadata(), kind).unwrap();
    assert_eq!(report.output, output);
    assert!(report.durability_warning.is_none());
    assert_eq!(fs::read_dir(&output).unwrap().count(), 3);
    assert_eq!(
        fs::read(output.join("model.animated.json")).unwrap(),
        MODEL_BYTES
    );
    assert_eq!(fs::read(output.join(LICENSE)).unwrap(), LICENSE_BYTES);
    assert!(!output.join(MODEL).exists());
    let inspected =
        crate::inspect_for_install(&output, crate::Runtime::content_only().engine_version).unwrap();
    assert_eq!(
        inspected.manifest.capabilities,
        ["models".into(), "animation".into()].into_iter().collect()
    );
    assert_eq!(
        inspected.manifest.files,
        ["model.animated.json".into(), LICENSE.into()]
            .into_iter()
            .collect()
    );
    let before = fs::read(output.join(MANIFEST)).unwrap();
    assert!(publish_kind(&output, b"replacement", LICENSE_BYTES, &metadata(), kind).is_err());
    assert_eq!(fs::read(output.join(MANIFEST)).unwrap(), before);
    assert_eq!(
        fs::read(output.join("model.animated.json")).unwrap(),
        MODEL_BYTES
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[cfg(feature = "animation-authoring")]
#[test]
fn both_profiles_reject_cross_kind_capabilities_and_filenames() {
    for (kind, other) in [
        (PackageKind::Static, PackageKind::AnimatedSequence),
        (PackageKind::AnimatedSequence, PackageKind::Static),
    ] {
        for mutation in ["capabilities", "filename"] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("out");
            let result = publish_with_kind(
                &output,
                MODEL_BYTES,
                LICENSE_BYTES,
                &metadata(),
                kind,
                |point, path| {
                    if point == "manifest-created" {
                        let target = path.join(MANIFEST);
                        let mut manifest: crate::Manifest =
                            serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
                        if mutation == "capabilities" {
                            manifest.capabilities = other.capabilities();
                        } else {
                            fs::rename(path.join(kind.model_name()), path.join(other.model_name()))
                                .unwrap();
                            manifest.files.remove(kind.model_name());
                            manifest.files.insert(other.model_name().into());
                        }
                        fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
                        crate::inspect_for_install(
                            path,
                            crate::Runtime::content_only().engine_version,
                        )
                        .unwrap();
                    }
                    Ok(())
                },
            );
            let error = result.unwrap_err();
            assert!(!output.exists());
            if mutation == "capabilities" {
                assert!(error.contains("differs from requested metadata"), "{error}");
                assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
            } else {
                // A cross-profile filename is not owned by this publisher. Cleanup
                // must retain it, not broaden the list of files it may delete.
                assert!(
                    error.contains("unexpected staged package inventory"),
                    "{error}"
                );
                assert!(error.contains("cleanup incomplete"), "{error}");
                let stage = fs::read_dir(temp.path())
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                assert_eq!(
                    fs::read(stage.join(other.model_name())).unwrap(),
                    MODEL_BYTES
                );
                assert_eq!(fs::read_dir(stage).unwrap().count(), 1);
            }
        }
    }
}

#[cfg(feature = "animation-authoring")]
#[test]
fn animated_failures_clean_owned_files_before_commit_and_preserve_after_commit() {
    for failpoint in [
        "stage-created",
        "manifest-created",
        "before-rename",
        "after-rename",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        fs::write(temp.path().join("keep"), b"untouched").unwrap();
        let result = publish_with_kind(
            &output,
            MODEL_BYTES,
            LICENSE_BYTES,
            &metadata(),
            PackageKind::AnimatedSequence,
            |point, _| {
                if point == failpoint {
                    Err("animated injected failure".into())
                } else {
                    Ok(())
                }
            },
        );
        if failpoint == "after-rename" {
            assert!(result
                .unwrap()
                .durability_warning
                .unwrap()
                .contains("uncertain"));
            assert_eq!(
                fs::read(output.join("model.animated.json")).unwrap(),
                MODEL_BYTES
            );
            assert_eq!(fs::read_dir(&output).unwrap().count(), 3);
            crate::inspect_for_install(&output, crate::Runtime::content_only().engine_version)
                .unwrap();
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
        } else {
            assert!(result.unwrap_err().contains("animated injected failure"));
            assert!(!output.exists());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        }
        assert_eq!(fs::read(temp.path().join("keep")).unwrap(), b"untouched");
    }
}
