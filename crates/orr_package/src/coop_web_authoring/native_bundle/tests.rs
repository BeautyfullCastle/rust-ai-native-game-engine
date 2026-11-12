use super::*;
use std::os::unix::fs::symlink;

const SCENE: &[u8] = b"schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario:\n    starting_gold: 250\n    reactor_hp: 75\n  CoopDefense::Campaign:\n    last_wave: 3\n";
struct Fixture {
    root: tempfile::TempDir,
    options: Options,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("source-player");
        let mut bytes = vec![0; 64];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16] = 3;
        bytes[18] = 62;
        bytes[52] = 64;
        fs::write(&runtime, &bytes).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        let scene = root.path().join("authored.yaml");
        fs::write(&scene, SCENE).unwrap();
        let options = Options {
            runtime,
            scene,
            runtime_sha256: digest(&bytes),
            output: root.path().join("new delivery"),
        };
        Self { root, options }
    }
    fn answer(&self) -> Vec<u8> {
        format!(
            "{}\n",
            expected_smoke(&fs::read(&self.options.scene).unwrap()).unwrap()
        )
        .into_bytes()
    }
    fn create(&self) -> Result<(Value, Option<String>), String> {
        let answer = self.answer();
        create_with(&self.options, |_| Ok(answer.clone()), |_, _| Ok(()))
    }
}
#[test]
fn explicit_trust_and_closed_flags_required_before_opening_paths() {
    let args: Vec<OsString> = [
        "--runtime",
        "player",
        "--runtime-sha256",
        &"a".repeat(64),
        "--scene",
        "game.yaml",
        "--output",
        "new",
        "--trusted-runtime",
    ]
    .map(Into::into)
    .to_vec();
    assert!(options(&args).is_ok());
    assert!(options(&args[..8]).is_err());
    for extra in [
        vec!["--trusted-runtime"],
        vec!["--scene", "other"],
        vec!["--source-sha", "a"],
        vec!["--"],
        vec!["--capture", "out.png"],
    ] {
        let mut bad = args.clone();
        bad.extend(extra.into_iter().map(OsString::from));
        assert!(options(&bad).is_err());
    }
    for sha in ["A".repeat(64), "g".repeat(64), "a".repeat(63)] {
        let mut bad = args.clone();
        bad[3] = sha.into();
        assert!(options(&bad).is_err());
    }
    let mut bad = args;
    bad[1] = "../player".into();
    assert!(options(&bad).is_err());
}
#[test]
fn flat_bundle_receipt_preserves_bytes_and_binds_launcher_hashes() {
    let f = Fixture::new();
    let (receipt, warning) = f.create().unwrap();
    assert!(warning.is_none());
    assert_eq!(receipt["kind"], "orr.coop-native-bundle/1");
    assert_eq!(receipt["smoke"]["idleTicks"], 60);
    assert_eq!(receipt["scene"]["sha256"], digest(SCENE));
    assert_eq!(
        fs::read(f.options.output.join("game.scene.yaml")).unwrap(),
        SCENE
    );
    assert_eq!(
        fs::read(f.options.output.join("coop_defense")).unwrap(),
        fs::read(&f.options.runtime).unwrap()
    );
    let launcher = fs::read_to_string(f.options.output.join("run-game")).unwrap();
    assert!(launcher.contains(&f.options.runtime_sha256));
    assert!(launcher.contains(&digest(SCENE)));
    assert_eq!(fs::read_dir(&f.options.output).unwrap().count(), 4);
    assert_eq!(
        fs::metadata(&f.options.output)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );
    assert_eq!(
        fs::metadata(f.options.output.join("run-game"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(f.options.output.join("bundle.json")).unwrap())
            .unwrap(),
        receipt
    );
    assert!(f.create().is_err());
    assert_eq!(fs::read_dir(&f.options.output).unwrap().count(), 4);
}
#[test]
fn runtime_digest_elf_and_permission_checks_are_fail_closed() {
    for defect in 0..7 {
        let mut f = Fixture::new();
        match defect {
            0 => f.options.runtime_sha256 = "0".repeat(64),
            1 => {
                fs::set_permissions(&f.options.runtime, fs::Permissions::from_mode(0o644)).unwrap()
            }
            2 => {
                fs::set_permissions(&f.options.runtime, fs::Permissions::from_mode(0o775)).unwrap()
            }
            3 => {
                fs::set_permissions(&f.options.runtime, fs::Permissions::from_mode(0o4755)).unwrap()
            }
            4 => fs::write(&f.options.runtime, b"#!/bin/sh\nexit 0\n").unwrap(),
            5 => {
                let mut b = fs::read(&f.options.runtime).unwrap();
                b[18] = 183;
                fs::write(&f.options.runtime, b).unwrap();
            }
            _ => {
                let mut b = fs::read(&f.options.runtime).unwrap();
                b[5] = 2;
                fs::write(&f.options.runtime, b).unwrap();
            }
        }
        if defect >= 4 {
            f.options.runtime_sha256 = digest(&fs::read(&f.options.runtime).unwrap());
        }
        assert!(f.create().is_err());
        assert!(!f.options.output.exists());
    }
}
#[test]
fn symlink_hardlink_fifo_directory_and_oversized_inputs_refused() {
    for defect in 0..6 {
        let f = Fixture::new();
        match defect {
            0 => {
                let p = f.root.path().join("real");
                fs::rename(&f.options.runtime, &p).unwrap();
                symlink(p, &f.options.runtime).unwrap();
            }
            1 => fs::hard_link(&f.options.runtime, f.root.path().join("alias")).unwrap(),
            2 => {
                fs::remove_file(&f.options.runtime).unwrap();
                unix::mknodat(
                    unix::CWD,
                    &f.options.runtime,
                    unix::FileType::Fifo,
                    Mode::RUSR | Mode::WUSR,
                    0,
                )
                .unwrap();
            }
            3 => {
                fs::remove_file(&f.options.runtime).unwrap();
                fs::create_dir(&f.options.runtime).unwrap();
            }
            4 => {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&f.options.runtime)
                    .unwrap()
                    .set_len(MAX_RUNTIME as u64 + 1)
                    .unwrap();
            }
            _ => fs::write(&f.options.scene, vec![b' '; MAX_SCENE + 1]).unwrap(),
        }
        assert!(create_with(
            &f.options,
            |_| panic!("must not execute invalid input"),
            |_, _| Ok(())
        )
        .is_err());
        assert!(!f.options.output.exists());
    }
    let mut f = Fixture::new();
    let link = f.root.path().join("ancestor");
    symlink(f.root.path(), &link).unwrap();
    f.options.runtime = link.join("source-player");
    assert!(f.create().is_err());
}
#[test]
fn scene_requires_existing_strict_native_admission_before_smoke() {
    for bad in [b"not a scene".to_vec(), vec![255], b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 150, reactor_hp: 0}}\n".to_vec(), b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::EnemyMotion: {progress_per_tick: [4,7,4,4,2]}}\n".to_vec()] {
        let f = Fixture::new(); fs::write(&f.options.scene, bad).unwrap();
        assert!(create_with(&f.options, |_| panic!("invalid scene executed"), |_, _| Ok(())).is_err());
        assert!(!f.options.output.exists());
    }
}
#[test]
fn exact_smoke_comparison_rejects_wrong_runtime_or_scene_and_extra_output() {
    let expected = expected_smoke(SCENE).unwrap();
    verify_smoke(format!("{expected}\n").as_bytes(), &expected).unwrap();
    verify_smoke(
        format!("Stormglass pixelfx active=0 started=1 dropped=2 canceled=3\n{expected}\n")
            .as_bytes(),
        &expected,
    )
    .unwrap();
    for bad in [
        "".to_owned(),
        expected.replace("reactor=75", "reactor=30"),
        format!("{expected}\nextra\n"),
        format!("{expected}\n{expected}"),
        format!("Stormglass pixelfx active=x started=1 dropped=2 canceled=3\n{expected}"),
    ] {
        assert!(verify_smoke(bad.as_bytes(), &expected).is_err());
    }
    let other = expected_smoke(
        &String::from_utf8_lossy(SCENE)
            .replace("250", "251")
            .into_bytes(),
    )
    .unwrap();
    assert_ne!(other, expected);
    assert!(verify_smoke(other.as_bytes(), &expected).is_err());
}
#[test]
fn failed_smoke_retains_stage_without_publishing_or_changing_sources() {
    let f = Fixture::new();
    let original = fs::read(&f.options.runtime).unwrap();
    let error =
        create_with(&f.options, |_| Err("fixture failure".into()), |_, _| Ok(())).unwrap_err();
    assert!(error.contains("unpublished stage retained"));
    assert!(!f.options.output.exists());
    assert_eq!(fs::read(&f.options.runtime).unwrap(), original);
    assert!(fs::read_dir(f.root.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-native-bundle-")));
}
#[test]
fn staged_addition_tamper_permissions_and_identity_change_rejected() {
    for defect in 0..6 {
        let f = Fixture::new();
        let answer = f.answer();
        let result = create_with(
            &f.options,
            |stage| {
                let file = stage.join("game.scene.yaml");
                match defect {
                    0 => fs::write(stage.join("foreign"), b"retain me").unwrap(),
                    1 => fs::write(&file, b"altered").unwrap(),
                    2 => fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap(),
                    3 => {
                        fs::remove_file(&file).unwrap();
                        fs::write(&file, SCENE).unwrap();
                    }
                    4 => {
                        fs::remove_file(&file).unwrap();
                        symlink(&f.options.scene, &file).unwrap();
                    }
                    _ => fs::set_permissions(stage, fs::Permissions::from_mode(0o777)).unwrap(),
                }
                Ok(answer.clone())
            },
            |_, _| Ok(()),
        );
        assert!(result.is_err(), "staged defect {defect} was accepted");
        assert!(
            !f.options.output.exists(),
            "staged defect {defect} published"
        );
    }
}
#[test]
fn staged_read_only_handle_pins_unlinked_inode_without_preventing_execution() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let stage = directory(root.path()).unwrap();
    let bytes = b"#!/bin/sh\nexit 0\n".to_vec();
    let pinned = write_file(&stage, "player", bytes.clone(), 0o755).unwrap();
    assert!(Command::new(root.path().join("player"))
        .status()
        .unwrap()
        .success());
    fs::remove_file(root.path().join("player")).unwrap();
    fs::write(root.path().join("player"), bytes).unwrap();
    fs::set_permissions(
        root.path().join("player"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(pinned.handle.metadata().unwrap().nlink(), 0);
    assert_ne!(
        fs::metadata(root.path().join("player")).unwrap().ino(),
        pinned.ino
    );
    let files = BTreeMap::from([("player", pinned)]);
    assert!(verify_stage(root.path(), &stage, &files).is_err());
}
#[test]
fn source_mutation_destination_race_and_symlink_output_do_not_replace_objects() {
    for defect in 0..3 {
        let f = Fixture::new();
        let answer = f.answer();
        let result = create_with(
            &f.options,
            |_| Ok(answer.clone()),
            |event, _| {
                if event == "before-rename" {
                    match defect {
                        0 => fs::write(&f.options.scene, b"changed source").unwrap(),
                        1 => {
                            fs::create_dir(&f.options.output).unwrap();
                            fs::write(f.options.output.join("foreign"), b"preserve").unwrap();
                        }
                        _ => symlink(&f.options.runtime, &f.options.output).unwrap(),
                    }
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        if defect == 1 {
            assert_eq!(
                fs::read(f.options.output.join("foreign")).unwrap(),
                b"preserve"
            );
        }
        if defect == 2 {
            assert!(fs::symlink_metadata(&f.options.output)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }
}
#[test]
fn after_rename_failure_reports_published_warning_not_retryable_failure() {
    let f = Fixture::new();
    let answer = f.answer();
    let (_, warning) = create_with(
        &f.options,
        |_| Ok(answer.clone()),
        |event, _| {
            if event == "after-rename" {
                Err("injected durability failure".into())
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert!(warning.unwrap().contains("do not retry"));
    assert_eq!(fs::read_dir(&f.options.output).unwrap().count(), 4);
}
#[test]
fn supervision_has_bounded_output_timeout_failure_and_inherited_pipes() {
    for (script, timeout, cap) in [
        ("printf ok", 1000, 1024),
        ("printf fail >&2; exit 1", 1000, 1024),
        ("while :; do printf 1234567890; done", 1000, 32),
        ("while :; do printf 1234567890 >&2; done", 1000, 32),
        ("sleep 10", 20, 1024),
        ("sleep 10 & printf ok", 1000, 1024),
        ("sleep 10 >/dev/null 2>&1 & printf ok", 1000, 1024),
    ] {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        let result = process::run_bounded(&mut command, Duration::from_millis(timeout), cap);
        if script.ends_with("printf ok") || script == "printf ok" {
            assert_eq!(result.unwrap(), b"ok");
        } else {
            assert!(result.is_err(), "{script}");
        }
    }
}
