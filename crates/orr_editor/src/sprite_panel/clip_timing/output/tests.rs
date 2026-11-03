#![cfg(unix)]
use super::*;
fn document() -> SpriteDocument {
    SpriteDocument::from_json(r#"{
      "format":"orr_sprite","version":1,
      "atlas":{"image":"atlas.png","width":2,"height":1},
      "regions":[{"id":7,"x":0,"y":0,"width":1,"height":1},
                 {"id":9,"x":1,"y":0,"width":1,"height":1}],
      "clips":[{"id":"walk","mode":"loop","frames":[{"region":7,"duration_ms":81},{"region":9,"duration_ms":119}]},
               {"id":"idle","mode":"once","frames":[{"region":9,"duration_ms":302}]}]
    }"#).unwrap()
}
use std::os::unix::fs::symlink;

#[test]
fn publishes_complete_document_without_mutation_or_staging_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("height.json");
    let t = document();
    let original = t.to_json().unwrap();
    let mut calls = 0;
    let result = publish_new(&output, &t, || {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(t.to_json().unwrap(), original);
    assert_eq!(
        SpriteDocument::from_json(&fs::read_to_string(&output).unwrap())
            .unwrap()
            .to_json()
            .unwrap(),
        original
    );
    assert_eq!(result.bytes, t.to_json().unwrap().len());
    assert!(result.warning.is_none());
    assert_eq!(fs::read(&output).unwrap(), t.to_json().unwrap().as_bytes());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(publish_new(&output, &t, || Ok(())).is_err());
}

#[test]
fn refuses_invalid_paths_before_staging() {
    let dir = tempfile::tempdir().unwrap();
    let t = document();
    for output in [
        PathBuf::from("relative.json"),
        dir.path().join("height.bin"),
        dir.path().join("../height.json"),
        dir.path().join("./height.json"),
        dir.path().join(format!("{}.json", "x".repeat(4096))),
    ] {
        assert!(publish_new(&output, &t, || Ok(())).is_err());
    }
    for name in ["package.orr", ".orr", "PACKAGE.ORR", "store", "STORE"] {
        let parent = dir.path().join(name);
        fs::create_dir(&parent).unwrap();
        assert!(publish_new(&parent.join("height.json"), &t, || Ok(())).is_err());
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
}

#[test]
fn refuses_symlink_ancestry_and_existing_symlink_or_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let t = document();
    let parent = dir.path().join("link");
    symlink(dir.path(), &parent).unwrap();
    assert!(publish_new(&parent.join("out.json"), &t, || Ok(())).is_err());
    let output = dir.path().join("out.json");
    symlink(dir.path().join("missing"), &output).unwrap();
    assert!(publish_new(&output, &t, || Ok(())).is_err());
    assert!(fs::symlink_metadata(&output)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn callback_refusals_leave_no_output_or_staging() {
    for refuse_at in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.json");
        let t = document();
        let mut calls = 0;
        assert!(publish_new(&output, &t, || {
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
    let output = dir.path().join("out.json");
    let t = document();
    let mut calls = 0;
    assert!(publish_new(&output, &t, || {
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
        let output = dir.path().join("out.json");
        let t = document();
        let result = publish(
            &output,
            &t,
            || Ok(()),
            |point, stage| {
                if partial && point == Point::StageCreated {
                    fs::write(stage, b"part").unwrap();
                    return Err("injected partial-write failure".into());
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
    let output = dir.path().join("out.json");
    let t = document();
    let mut stage_path = PathBuf::new();
    let result = publish(
        &output,
        &t,
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
    assert!(result.unwrap_err().contains("cleanup refused/failed"));
    assert!(!output.exists());
    assert_eq!(fs::read(stage_path).unwrap(), b"foreign");
}

#[test]
fn parent_replacement_is_refused_without_foreign_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent");
    let moved = dir.path().join("moved");
    fs::create_dir(&parent).unwrap();
    let output = parent.join("out.json");
    let t = document();
    let mut calls = 0;
    let result = publish_new(&output, &t, || {
        calls += 1;
        if calls == 2 {
            fs::rename(&parent, &moved).unwrap();
            fs::create_dir(&parent).unwrap();
            fs::write(parent.join("foreign"), b"safe").unwrap();
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("cleanup refused/failed"));
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
        let output = dir.path().join("out.json");
        let t = document();
        let result = publish(
            &output,
            &t,
            || Ok(()),
            |point, _| {
                if point == fail_at {
                    Err("injected postpublication uncertainty".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), t.to_json().unwrap().as_bytes());
        if fail_at == Point::BeforeCleanup {
            assert!(result.warning.unwrap().contains("output was published"));
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        } else {
            assert!(result.warning.unwrap().contains("do not retry blindly"));
        }
    }
}

#[test]
fn postpublication_replacement_preserves_foreign_stage_and_reports_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.json");
    let t = document();
    let mut foreign_stage = PathBuf::new();
    let receipt = publish(
        &output,
        &t,
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
        .warning
        .as_ref()
        .unwrap()
        .contains("do not retry blindly"));
    assert!(receipt.warning.is_some());
    assert_eq!(fs::read(foreign_stage).unwrap(), b"foreign");
    assert_eq!(fs::read(output).unwrap(), t.to_json().unwrap().as_bytes());
}

#[test]
fn non_directory_ancestry_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("regular");
    fs::write(&parent, b"preserve").unwrap();
    let t = document();
    assert!(publish_new(&parent.join("out.json"), &t, || Ok(())).is_err());
    assert_eq!(fs::read(parent).unwrap(), b"preserve");
}

#[test]
fn final_callback_content_change_is_caught_by_second_exact_readback() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.json");
    let mut calls = 0;
    let result = publish_new(&output, &document(), || {
        calls += 1;
        if calls == 2 {
            let stage = fs::read_dir(dir.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            OpenOptions::new()
                .append(true)
                .open(stage)
                .unwrap()
                .write_all(b" ")
                .unwrap();
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("exact byte readback mismatch"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn existing_regular_destination_keeps_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.json");
    fs::write(&output, b"original").unwrap();
    assert!(publish_new(&output, &document(), || panic!(
        "existing destination refused first"
    ))
    .is_err());
    assert_eq!(fs::read(output).unwrap(), b"original");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
