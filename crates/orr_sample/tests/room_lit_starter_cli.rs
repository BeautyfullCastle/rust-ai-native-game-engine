#![cfg(all(
    target_os = "linux",
    feature = "project-create",
    feature = "room-lighting"
))]
use std::{fs, process::Command};
fn run(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orr_new_lit_room"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn actual_cli_creates_a_source_independent_admitted_lit_game_and_refuses_overwrite() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("light.json");
    fs::write(
        &source,
        orr_sample::room_lighting::Document::enabled_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let out = t.path().join("lit game");
    let args = [
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "cli-lit".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str(),
    ];
    let result = run(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = fs::read(out.join("orr.project.json")).unwrap();
    assert!(!run(&args).status.success());
    assert_eq!(fs::read(out.join("orr.project.json")).unwrap(), manifest);
    fs::remove_file(source).unwrap();
    let project =
        orr_sample::room_project::PreparedProject::open_with_lighting(&out, true).unwrap();
    assert!(project.lighting().unwrap().document.point_light.is_some());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("room-escape-lit-3d-v1"));
}
#[test]
fn malformed_cli_and_light_fail_before_destination_creation() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("never created");
    let source = t.path().join("invalid");
    fs::write(&source, b"invalid").unwrap();
    assert!(!run(&[
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "seed".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str()
    ])
    .status
    .success());
    assert!(!out.exists());
    for args in [
        vec!["--seed", "x"],
        vec!["--output", "x", "--output", "y", "--lighting", "z"],
        vec!["--bogus", "x", "--seed", "y", "--lighting", "z"],
    ] {
        let args: Vec<_> = args.iter().map(|s| std::ffi::OsStr::new(*s)).collect();
        assert!(!run(&args).status.success());
    }
    assert!(run(&["--help".as_ref()]).status.success());
    assert!(!out.exists());
}

#[cfg(not(feature = "room-character"))]
#[test]
fn character_request_without_feature_rejects_before_opening_the_lighting_input() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("never-open.fifo");
    assert!(Command::new("mkfifo")
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let out = t.path().join("never created");
    let result = run(&[
        "--character".as_ref(),
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "feature-gate".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("room-character"));
    assert!(!out.exists());
    assert_eq!(
        fs::read_dir(t.path()).unwrap().count(),
        1,
        "no staging directory or output created"
    );
}
#[cfg(feature = "room-character")]
#[test]
fn character_cli_creates_both_profiles_by_value_and_requires_both_consents() {
    use orr_sample::room_project::{PreparedProject, RoomProjectCapabilities};
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("light.json");
    fs::write(
        &source,
        orr_sample::room_lighting::Document::enabled_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let out = t.path().join("lit character game");
    let result = run(&[
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "both-profiles".as_ref(),
        "--lighting".as_ref(),
        source.as_os_str(),
        "--character".as_ref(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("room-escape-lit-character-3d-v1"));
    fs::remove_file(source).unwrap();
    let full = RoomProjectCapabilities {
        lighting: true,
        character: true,
    };
    let p = PreparedProject::open_with_capabilities(&out, full).unwrap();
    assert!(p.character().is_some());
    assert!(p.lighting().unwrap().document.point_light.is_some());
    for caps in [
        RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
        RoomProjectCapabilities {
            lighting: true,
            character: false,
        },
        RoomProjectCapabilities {
            lighting: false,
            character: false,
        },
    ] {
        assert!(PreparedProject::open_with_capabilities(&out, caps).is_err());
    }
}
#[test]
fn character_flag_rejects_duplicates_values_and_missing_required_options() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("never created");
    let absent = t.path().join("absent-light.json");
    for args in [
        vec![
            "--character".as_ref(),
            "--character".as_ref(),
            "--output".as_ref(),
            out.as_os_str(),
            "--seed".as_ref(),
            "seed".as_ref(),
            "--lighting".as_ref(),
            absent.as_os_str(),
        ],
        vec![
            "--character".as_ref(),
            "true".as_ref(),
            "--output".as_ref(),
            out.as_os_str(),
            "--seed".as_ref(),
            "seed".as_ref(),
            "--lighting".as_ref(),
            absent.as_os_str(),
        ],
        vec![
            "--character".as_ref(),
            "--output".as_ref(),
            out.as_os_str(),
            "--lighting".as_ref(),
            absent.as_os_str(),
        ],
    ] {
        assert!(!run(&args).status.success());
        assert!(!out.exists());
    }
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
}

#[test]
fn explicit_default_cli_creates_without_a_lighting_source_for_static_and_available_character() {
    use orr_sample::room_project::{PreparedProject, RoomProjectCapabilities};
    let t = tempfile::tempdir().unwrap();
    for character in [false, true] {
        let out = t.path().join(if character {
            "default character game"
        } else {
            "default static game"
        });
        let mut args = vec![
            "--lighting-default".as_ref(),
            "--output".as_ref(),
            out.as_os_str(),
            "--seed".as_ref(),
            "--foo".as_ref(),
        ];
        if character {
            args.push("--character".as_ref());
        }
        let result = run(&args);
        if character && !cfg!(feature = "room-character") {
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains("room-character"));
            assert!(!out.exists());
            continue;
        }
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let prepared = PreparedProject::open_with_capabilities(
            &out,
            RoomProjectCapabilities {
                lighting: true,
                character,
            },
        )
        .unwrap();
        assert_eq!(
            prepared.lighting().unwrap().document,
            orr_sample::room_lighting::Document::enabled_default()
        );
        #[cfg(feature = "room-character")]
        assert_eq!(prepared.character().is_some(), character);
        let base = t.path().join(if character {
            "base character"
        } else {
            "base static"
        });
        let baseline =
            orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
                output: base,
                template: if character {
                    orr_sample::project_create::ROOM_CHARACTER_TEMPLATE
                } else {
                    orr_sample::project_create::ROOM_TEMPLATE
                }
                .into(),
                seed: "--foo".into(),
            })
            .unwrap();
        assert_eq!(
            prepared.scene().frame().checksum(),
            baseline.initial_checksum
        );
        assert!(String::from_utf8(result.stdout).unwrap().contains(&format!(
            "initial checksum: 0x{:016x}",
            baseline.initial_checksum
        )));
        let bytes = fs::read(out.join("orr.project.json")).unwrap();
        assert!(!run(&args).status.success());
        assert_eq!(fs::read(out.join("orr.project.json")).unwrap(), bytes);
    }
}
#[test]
fn default_lighting_is_explicit_exclusive_and_rejects_duplicate_or_valued_flags_before_input() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("never created");
    let absent = t.path().join("must not read.json");
    let required = vec![
        "--output".as_ref(),
        out.as_os_str(),
        "--seed".as_ref(),
        "seed".as_ref(),
    ];
    let cases: Vec<Vec<&std::ffi::OsStr>> = vec![
        vec![],
        vec![
            "--lighting-default".as_ref(),
            "--lighting".as_ref(),
            absent.as_os_str(),
        ],
        vec![
            "--lighting".as_ref(),
            absent.as_os_str(),
            "--lighting-default".as_ref(),
        ],
        vec!["--lighting-default".as_ref(), "--lighting-default".as_ref()],
        vec!["--lighting-default".as_ref(), "true".as_ref()],
        vec!["--lighting".as_ref(), "--lighting-default".as_ref()],
        vec!["--lighting-default".as_ref(), "--bogus".as_ref()],
    ];
    for suffix in cases {
        let mut args = required.clone();
        args.extend(suffix);
        let result = run(&args);
        assert!(!result.status.success());
        assert!(!out.exists());
        assert!(
            !String::from_utf8_lossy(&result.stderr).contains("No such file"),
            "ambiguous/malformed options must reject before reading supplied input"
        );
    }
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
}
