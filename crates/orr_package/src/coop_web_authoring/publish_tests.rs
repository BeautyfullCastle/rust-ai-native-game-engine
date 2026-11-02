use super::*;
#[test]
fn atomic_output_preserves_existing_and_mutated_stage() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("manifest");
    assert!(write(&out, b"valid", |_, _| Ok(())).unwrap().is_none());
    assert!(write(&out, b"new", |_, _| Ok(())).is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"valid");
    let new = t.path().join("new");
    let error = write(&new, b"original", |phase, stage| {
        if phase == "before-link" {
            std::fs::write(stage, b"changed").unwrap();
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("retained"));
    assert!(!new.exists());
}
#[test]
fn collision_and_postpublish_uncertainty_do_not_rollback() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("collision");
    assert!(write(&out, b"ours", |phase, _| {
        if phase == "before-link" {
            std::fs::write(&out, b"foreign").unwrap();
        }
        Ok(())
    })
    .is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"foreign");
    let done = t.path().join("done");
    assert!(
        write(&done, b"complete", |phase, _| if phase == "after-link" {
            Err("injected sync".into())
        } else {
            Ok(())
        })
        .unwrap()
        .is_some()
    );
    assert_eq!(std::fs::read(&done).unwrap(), b"complete");
}
