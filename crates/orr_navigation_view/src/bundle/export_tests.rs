use super::*;
fn fixture(root: &Path, runtime: &Path) -> Options {
    let input = root.join("author");
    fs::create_dir_all(input.join("navigation")).unwrap();
    let scene = input.join("point.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        input.join("navigation/point_demo.orrt"),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    Options {
        scene,
        runtime: runtime.into(),
        hash: hash(&fs::read(runtime).unwrap()),
        output: root.join("bundle"),
        revision: Some("user-claim".into()),
    }
}
#[test]
fn before_smoke_failure_retains_private_stage_without_publishing_or_deleting_foreign() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trusted-test-elf");
    let mut bytes = vec![0u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4..7].copy_from_slice(&[2, 1, 1]);
    bytes[16] = 2;
    bytes[18] = 62;
    bytes[52] = 64;
    fs::write(&runtime, bytes).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    let options = fixture(root.path(), &runtime);
    let foreign = root.path().join("foreign");
    fs::write(&foreign, b"untouched").unwrap();
    let failure = export(&options, |p| {
        if p == "before-smoke" {
            Err("injected-before-any-execution".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(failure.contains("private partial stage retained"));
    assert!(failure.contains("injected-before-any-execution"));
    assert!(!options.output.exists());
    assert_eq!(fs::read(foreign).unwrap(), b"untouched");
    let stages: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".orr-navigation-")
        })
        .collect();
    assert_eq!(stages.len(), 1);
    assert!(verify_bundle(&stages[0], None).is_ok());
    // The minimal ELF is not executable evidence: failure was injected before smoke.
}
#[test]
#[ignore = "requires explicit actual bundle-capable player; executes trusted smoke"]
fn actual_runtime_before_and_after_publish_failures_keep_exact_outcome() {
    let runtime = PathBuf::from(
        std::env::var_os("ORR_NAVIGATION_BUNDLE_PLAYER").expect("actual bundle runtime required"),
    );
    assert!(runtime.is_absolute());
    for point in ["before-publish", "after-publish"] {
        let root = tempfile::tempdir().unwrap();
        let options = fixture(root.path(), &runtime);
        let foreign = root.path().join("foreign");
        fs::write(&foreign, b"untouched").unwrap();
        let failure = export(&options, |p| {
            if p == point {
                Err(format!("injected-{point}"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(failure.contains(&format!("injected-{point}")));
        assert_eq!(fs::read(&foreign).unwrap(), b"untouched");
        if point == "before-publish" {
            assert!(!options.output.exists());
            assert!(failure.contains("not published"));
        } else {
            assert!(options.output.exists());
            assert!(failure.contains("durability/identity uncertain"));
            assert!(failure.contains("no rollback or retry"));
            assert!(verify_bundle(&options.output, None).is_ok());
        }
    }
}
