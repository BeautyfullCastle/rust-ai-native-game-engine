use super::*;
fn files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("Cargo.toml".into(), b"package".to_vec()),
        ("src/lib.rs".into(), b"source".to_vec()),
    ])
}
#[test]
fn publishes_exact_tree_and_refuses_existing_objects() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("game");
    assert!(publish(&out, &files()).unwrap().is_none());
    assert_eq!(fs::read(out.join("src/lib.rs")).unwrap(), b"source");
    assert!(publish(&out, &files()).is_err());
    assert_eq!(fs::read(out.join("Cargo.toml")).unwrap(), b"package");
    for (n, link) in [("file", false), ("link", true)] {
        let p = t.path().join(n);
        if link {
            std::os::unix::fs::symlink("missing", &p).unwrap();
        } else {
            fs::write(&p, b"keep").unwrap();
        }
        assert!(publish(&p, &files()).is_err());
    }
}
#[test]
fn failure_and_foreign_contents_are_never_recursively_deleted() {
    for phase in ["stage-created", "file-written", "before-rename"] {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("game");
        let mut retained = None;
        let error = publish_with(&out, &files(), |point, stage| {
            if point == phase {
                fs::write(stage.join("foreign"), b"keep").unwrap();
                retained = Some(stage.to_owned());
                Err("injected".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.contains("stage retained"));
        assert!(!out.exists());
        assert_eq!(
            fs::read(retained.unwrap().join("foreign")).unwrap(),
            b"keep"
        );
    }
}
#[test]
fn rename_failure_collision_and_mutation_leave_no_false_success() {
    for fault in [
        "unsupported",
        "collision",
        "bytes",
        "inventory",
        "replacement",
    ] {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("game");
        let error = publish_with(&out, &files(), |point, stage| {
            if point == "before-rename" {
                match fault {
                    "unsupported" => return Err("renameat2 ENOSYS injected; no fallback".into()),
                    "collision" => {
                        fs::create_dir(&out).unwrap();
                        fs::write(out.join("foreign"), b"keep").unwrap();
                    }
                    "bytes" => fs::write(stage.join("Cargo.toml"), b"changed").unwrap(),
                    "inventory" => fs::write(stage.join("foreign"), b"keep").unwrap(),
                    "replacement" => {
                        fs::rename(stage, stage.with_extension("owned")).unwrap();
                        fs::create_dir(stage).unwrap();
                        fs::write(stage.join("foreign"), b"keep").unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("stage retained"));
        if fault == "collision" {
            assert_eq!(fs::read(out.join("foreign")).unwrap(), b"keep");
        } else {
            assert!(!out.exists());
        }
    }
}
#[test]
fn postrename_failure_is_published_uncertain_not_rolled_back() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("game");
    let warning = publish_with(&out, &files(), |phase, _| {
        if phase == "after-rename" {
            Err("parent sync unavailable".into())
        } else {
            Ok(())
        }
    })
    .unwrap()
    .unwrap();
    assert!(warning.contains("sync"));
    assert_eq!(fs::read(out.join("src/lib.rs")).unwrap(), b"source");
    assert!(publish(&out, &files()).is_err());
}
#[test]
fn paths_and_budgets_fail_before_claiming_output() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("game");
    for path in ["../escape", "/absolute", "a//b", "a/./b", "a\\b"] {
        assert!(publish(&out, &BTreeMap::from([(path.into(), vec![1])])).is_err());
        assert!(!out.exists());
    }
    let collision = BTreeMap::from([("a".into(), vec![1]), ("a/b".into(), vec![2])]);
    assert!(publish(&out, &collision).is_err());
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
}
