#![cfg(unix)]
use super::*;
use crate::raw16::export_tests::{recipe, terrain};
use std::os::unix::fs::symlink;

#[test]
fn publishes_exact_bytes_and_recipe_without_staging_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("height.raw");
    let r = recipe();
    let t = terrain(&r, [-100, -99, 0, 65435]);
    let mut calls = 0;
    let result = export_new_checked(&output, &t, &r, || {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(result.bytes, 8);
    assert_eq!(result.recipe, r);
    assert!(result.durability_warning.is_none());
    assert!(result.cleanup_warning.is_none());
    assert_eq!(fs::read(&output).unwrap(), export_raw16(&t, &r).unwrap());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(export_new_checked(&output, &t, &r, || Ok(())).is_err());
}

#[test]
fn refuses_paths_and_invalid_conversion_before_staging() {
    let dir = tempfile::tempdir().unwrap();
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    for output in [
        PathBuf::from("relative.raw"),
        dir.path().join("height.bin"),
        dir.path().join("../height.raw"),
    ] {
        assert!(export_new_checked(&output, &t, &r, || Ok(())).is_err());
    }
    for name in ["package.orr", ".orr", "PACKAGE.ORR"] {
        let parent = dir.path().join(name);
        fs::create_dir(&parent).unwrap();
        assert!(export_new_checked(&parent.join("height.raw"), &t, &r, || Ok(())).is_err());
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    let output = dir.path().join("height.raw");
    let invalid = terrain(&r, [-101; 4]);
    assert!(export_new_checked(&output, &invalid, &r, || panic!(
        "conversion precedes callback"
    ))
    .is_err());
    assert!(!output.exists());
}

#[test]
fn refuses_symlink_ancestry_and_existing_symlink_or_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    let parent = dir.path().join("link");
    symlink(dir.path(), &parent).unwrap();
    assert!(export_new_checked(&parent.join("out.raw"), &t, &r, || Ok(())).is_err());
    let output = dir.path().join("out.raw");
    symlink(dir.path().join("missing"), &output).unwrap();
    assert!(export_new_checked(&output, &t, &r, || Ok(())).is_err());
    assert!(fs::symlink_metadata(&output)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn callback_refusals_leave_no_output_or_staging() {
    for refuse_at in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.raw");
        let r = recipe();
        let t = terrain(&r, [-100; 4]);
        let mut calls = 0;
        assert!(export_new_checked(&output, &t, &r, || {
            calls += 1;
            if calls == refuse_at {
                Err("snapshot changed".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn racing_destination_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.raw");
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    let mut calls = 0;
    assert!(export_new_checked(&output, &t, &r, || {
        calls += 1;
        if calls == 2 {
            fs::write(&output, b"foreign").unwrap();
        }
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(output).unwrap(), b"foreign");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn partial_write_failure_and_corrupt_readback_clean_only_owned_stage() {
    for partial in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.raw");
        let r = recipe();
        let t = terrain(&r, [-100; 4]);
        let result = publish(
            &output,
            &t,
            &r,
            || Ok(()),
            |point, stage| {
                if partial && point == Point::StageCreated {
                    fs::write(stage, b"part").unwrap();
                    return Err(Error("injected partial-write failure".into()));
                }
                if !partial && point == Point::Written {
                    fs::write(stage, b"bad").unwrap();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn stage_replacement_is_refused_and_foreign_file_is_not_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.raw");
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    let mut stage_path = PathBuf::new();
    let result = publish(
        &output,
        &t,
        &r,
        || Ok(()),
        |point, stage| {
            if point == Point::Written {
                stage_path = stage.to_owned();
                fs::remove_file(stage).unwrap();
                fs::write(stage, b"foreign").unwrap();
            }
            Ok(())
        },
    );
    assert!(result.unwrap_err().0.contains("cleanup refused/failed"));
    assert!(!output.exists());
    assert_eq!(fs::read(stage_path).unwrap(), b"foreign");
}

#[test]
fn parent_replacement_is_refused_without_foreign_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent");
    let moved = dir.path().join("moved");
    fs::create_dir(&parent).unwrap();
    let output = parent.join("out.raw");
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    let mut calls = 0;
    let result = export_new_checked(&output, &t, &r, || {
        calls += 1;
        if calls == 2 {
            fs::rename(&parent, &moved).unwrap();
            fs::create_dir(&parent).unwrap();
            fs::write(parent.join("foreign"), b"safe").unwrap();
        }
        Ok(())
    });
    assert!(result.unwrap_err().0.contains("cleanup refused/failed"));
    assert!(!output.exists());
    assert_eq!(fs::read(parent.join("foreign")).unwrap(), b"safe");
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
}

#[test]
fn postpublication_failures_are_receipt_warnings_not_safe_retry_errors() {
    for fail_at in [
        Point::Linked,
        Point::BeforeDirectorySync,
        Point::BeforeCleanup,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.raw");
        let r = recipe();
        let t = terrain(&r, [-100; 4]);
        let result = publish(
            &output,
            &t,
            &r,
            || Ok(()),
            |point, _| {
                if point == fail_at {
                    Err(Error("injected postpublication uncertainty".into()))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), export_raw16(&t, &r).unwrap());
        if fail_at == Point::BeforeCleanup {
            assert!(result
                .cleanup_warning
                .unwrap()
                .contains("output was published"));
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        } else {
            assert!(result
                .durability_warning
                .unwrap()
                .contains("do not retry blindly"));
        }
    }
}

#[test]
fn postpublication_replacement_preserves_foreign_stage_and_reports_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.raw");
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    let mut foreign_stage = PathBuf::new();
    let receipt = publish(
        &output,
        &t,
        &r,
        || Ok(()),
        |point, stage| {
            if point == Point::Linked {
                foreign_stage = stage.to_owned();
                fs::remove_file(stage).unwrap();
                fs::write(stage, b"foreign").unwrap();
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(receipt
        .durability_warning
        .unwrap()
        .contains("do not retry blindly"));
    assert!(receipt.cleanup_warning.is_some());
    assert_eq!(fs::read(foreign_stage).unwrap(), b"foreign");
    assert_eq!(fs::read(output).unwrap(), export_raw16(&t, &r).unwrap());
}

#[test]
fn non_directory_ancestry_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("regular");
    fs::write(&parent, b"preserve").unwrap();
    let r = recipe();
    let t = terrain(&r, [-100; 4]);
    assert!(export_new_checked(&parent.join("out.raw"), &t, &r, || Ok(())).is_err());
    assert_eq!(fs::read(parent).unwrap(), b"preserve");
}
