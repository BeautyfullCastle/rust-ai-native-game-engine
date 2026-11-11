use super::super::tests::{fixture, SCENE};
use super::*;
use std::{process::Command, sync::OnceLock, time::Duration};
fn root() -> PathBuf {
    // The production API deliberately rejects ambiguous/non-ASCII directory names.
    // A host with a sandbox TEMP containing such names must select an existing
    // ordinary evidence directory explicitly; no reparse or permission fixtures.
    let parent = std::env::var_os("ORR_STORMGLASS_TEST_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = tempfile::Builder::new()
        .prefix("stormglass-distribution-")
        .tempdir_in(parent)
        .unwrap()
        .keep();
    eprintln!("retained evidence: {}", path.display());
    path
}
fn case() -> (PathBuf, Options) {
    let root = root();
    let input = root.join("input");
    let out = root.join("output");
    fs::create_dir(&input).unwrap();
    fs::create_dir(&out).unwrap();
    let bytes = fixture();
    let runtime = input.join("runtime.exe");
    let scene = input.join("scene.yaml");
    fs::write(&runtime, &bytes).unwrap();
    fs::write(&scene, SCENE).unwrap();
    let options = Options {
        runtime,
        runtime_sha256: hash(&bytes),
        scene,
        output: out.join("game.exe"),
        trusted_runtime: true,
        dry_run: false,
    };
    (root, options)
}
fn successful(probe: process::Probe, scene: &[u8]) -> process::Output {
    match probe {
        process::Probe::SceneHash => process::Output {exit_code:0,stdout:format!("{}\n",hash(scene)).into_bytes(),stderr:vec![]},
        process::Probe::Smoke => process::Output {exit_code:0,stdout:format!("{}\n",expected(scene).unwrap()).into_bytes(),stderr:vec![]},
        process::Probe::Override => process::Output {exit_code:1,stdout:vec![],stderr:b"Stormglass embedded prototype: embedded game does not accept --scene; rebuild its authored scene\n".to_vec()},
    }
}
fn protected(path: &Path, directory: bool) {
    for access in [0x4000_0000, win::DELETE] {
        let error = OpenOptions::new()
            .access_mode(access)
            .share_mode(7)
            .custom_flags(win::REPARSE | if directory { win::BACKUP } else { 0 })
            .open(path)
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32), "{error}");
    }
}
#[test]
fn dry_run_has_no_write_or_execute_and_reports_unverified_checks() {
    let (_root, mut options) = case();
    options.dry_run = true;
    options.trusted_runtime = false;
    let receipt = publish_with(&options, |_, _| panic!("dry-run executed")).unwrap();
    assert_eq!(receipt.status, "dry_run");
    assert!(
        !receipt.smoke_verified && !receipt.embedded_scene_verified && !receipt.override_rejected
    );
    assert_eq!(
        fs::read_dir(options.output.parent().unwrap())
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn fail_closed_admission_leaves_output_directory_empty() {
    let (_root, mut options) = case();
    let original = options.runtime_sha256.clone();
    options.runtime_sha256 = "0".repeat(64);
    assert!(publish_with(&options, |_, _| panic!("bad hash executed"))
        .unwrap_err()
        .contains("SHA256 mismatch"));
    options.runtime_sha256 = original;
    fs::write(&options.scene, b"invalid: source").unwrap();
    assert!(publish_with(&options, |_, _| panic!("invalid scene executed")).is_err());
    fs::write(&options.scene, vec![b'x'; 65537]).unwrap();
    assert!(publish_with(&options, |_, _| panic!("oversize scene executed")).is_err());
    fs::write(&options.scene, SCENE).unwrap();
    assert_eq!(
        fs::read_dir(options.output.parent().unwrap())
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn existing_file_or_directory_target_is_untouched_without_execution() {
    for directory in [false, true] {
        let (_root, options) = case();
        if directory {
            fs::create_dir(&options.output).unwrap();
        } else {
            fs::write(&options.output, b"preserve existing").unwrap();
        }
        assert!(
            publish_with(&options, |_, _| panic!("existing output executed"))
                .unwrap_err()
                .contains("already exists")
        );
        if !directory {
            assert_eq!(fs::read(&options.output).unwrap(), b"preserve existing");
        }
        assert_eq!(
            fs::read_dir(options.output.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }
}
#[test]
fn held_sources_artifact_ancestors_survive_mock_checks_and_no_replace_publish() {
    let (root, options) = case();
    let mut calls = 0;
    let receipt = publish_with(&options, |path, probe| {
        calls += 1;
        protected(path, false);
        protected(&options.runtime, false);
        protected(&options.scene, false);
        protected(options.output.parent().unwrap(), true);
        protected(&root, true);
        assert_eq!(fs::read(path).unwrap(), fixture());
        Ok(successful(probe, SCENE))
    })
    .unwrap();
    assert_eq!(calls, 3);
    assert_eq!(receipt.status, "published");
    assert!(receipt.smoke_verified && receipt.embedded_scene_verified && receipt.override_rejected);
    assert_eq!(fs::read(&options.output).unwrap(), fixture());
    assert_eq!(
        fs::read_dir(options.output.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn race_collision_does_not_replace_and_failed_stage_is_retained() {
    let (_root, options) = case();
    let error = publish_with(&options, |_, probe| {
        if matches!(probe, process::Probe::Override) {
            fs::write(&options.output, b"concurrent target").unwrap();
        }
        Ok(successful(probe, SCENE))
    })
    .unwrap_err();
    assert!(error.contains("c0000035"), "{error}");
    assert_eq!(fs::read(&options.output).unwrap(), b"concurrent target");
    assert_eq!(
        fs::read_dir(options.output.parent().unwrap())
            .unwrap()
            .count(),
        2
    );
}
#[test]
fn scene_hash_and_process_failures_never_publish_and_preserve_source() {
    for mismatch in [false, true] {
        let (_root, options) = case();
        let mut calls = 0;
        let error = publish_with(&options, |path, probe| {
            calls += 1;
            protected(path, false);
            protected(&options.runtime, false);
            if mismatch {
                Ok(successful(probe, &[SCENE, b"# comment\n"].concat()))
            } else {
                Err("injected cleanup failure".into())
            }
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.contains("retained artifact"));
        assert!(!options.output.exists());
        assert_eq!(fs::read(&options.runtime).unwrap(), fixture());
        assert_eq!(
            fs::read_dir(options.output.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }
}
#[test]
fn exact_probe_output_does_not_trim_or_ignore_extra_diagnostics() {
    let (_root, mut options) = case();
    options.dry_run = true;
    let receipt = publish_with(&options, |_, _| panic!()).unwrap();
    for probe in [
        process::Probe::SceneHash,
        process::Probe::Smoke,
        process::Probe::Override,
    ] {
        let good = successful(probe, SCENE);
        verify_output(&good, probe, &receipt).unwrap();
        let mut bad = successful(probe, SCENE);
        bad.stdout.push(b'\n');
        assert!(verify_output(&bad, probe, &receipt).is_err());
        let mut bad = successful(probe, SCENE);
        bad.stderr.push(b'\n');
        assert!(verify_output(&bad, probe, &receipt).is_err());
        let mut bad = successful(probe, SCENE);
        bad.exit_code = 23;
        assert!(verify_output(&bad, probe, &receipt).is_err());
    }
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE.get_or_init(|| {
        let root = root();
        let source = root.join("probe.rs");
        let output = root.join("probe.exe");
        fs::write(&source, include_str!("probe.rs")).unwrap();
        let build = Command::new("rustc")
            .args(["--edition", "2021", "-D", "warnings"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        fs::write(root.join("build.stdout"), &build.stdout).unwrap();
        fs::write(root.join("build.stderr"), &build.stderr).unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        output
    })
}
fn process_case(mode: &str) -> PathBuf {
    let root = root();
    let path = root.join("probe.exe");
    fs::copy(probe(), &path).unwrap();
    fs::write(root.join("mode"), mode).unwrap();
    path
}
#[test]
fn retained_supervisor_checks_environment_exit_stderr_timeout_cap_abort_and_descendants() {
    for (mode, expected_code) in [("shape", 0), ("nonzero", 23), ("stderr", 0)] {
        let path = process_case(mode);
        let output = process::test_run(&path, Duration::from_secs(15), 65536, false).unwrap();
        assert_eq!(output.exit_code, expected_code);
        if mode == "shape" {
            assert_eq!(output.stdout, b"shape checked\n");
        }
        if mode == "stderr" {
            assert_eq!(output.stderr, b"expected probe stderr\n");
        }
    }
    for (mode, timeout, cap, abort, expected) in [
        ("timeout", 100, 65536, false, "timed out"),
        ("overflow", 15000, 64, false, "exceeds limit"),
        ("marker", 15000, 65536, true, "before resume"),
    ] {
        let path = process_case(mode);
        let error = process::test_run(&path, Duration::from_millis(timeout), cap, abort)
            .err()
            .unwrap();
        assert!(
            error.contains(expected) && error.contains("cleanup confirmed"),
            "{error}"
        );
        assert!(!path.parent().unwrap().join("executed").exists());
    }
    let path = process_case("descendant");
    let output = process::test_run(&path, Duration::from_secs(15), 65536, false).unwrap();
    assert_eq!(output.exit_code, 0);
    std::thread::sleep(Duration::from_millis(700));
    assert!(!path.parent().unwrap().join("descendant-escaped").exists());
}

#[test]
fn existing_hardlink_target_is_refused_without_execution() {
    let (_root, options) = case();
    fs::hard_link(&options.runtime, &options.output).unwrap();
    assert!(
        publish_with(&options, |_, _| panic!("existing alias executed"))
            .unwrap_err()
            .contains("already exists")
    );
    assert_eq!(fs::read(&options.output).unwrap(), fixture());
}
#[test]
fn case_alias_is_refused_without_execution() {
    let (_root, mut options) = case();
    options.output = options
        .runtime
        .to_str()
        .unwrap()
        .to_ascii_uppercase()
        .into();
    assert!(publish_with(&options, |_, _| panic!("case alias executed"))
        .unwrap_err()
        .contains("distinct"));
}

#[test]
fn source_hardlink_fixture_is_refused() {
    let (_root, options) = case();
    let alias = options.runtime.parent().unwrap().join("alias.exe");
    fs::hard_link(&options.runtime, alias).unwrap();
    assert!(
        publish_with(&options, |_, _| panic!("hardlinked runtime executed"))
            .unwrap_err()
            .contains("single-link")
    );
}
