#![cfg(all(windows, feature = "project-create", feature = "room-project"))]

#[path = "../src/windows_room/authoring.rs"]
mod authoring;

use orr_ecs::Frame;
use orr_fp::FP;
use orr_games::room_escape_game::{
    RoomActor, RoomEscapeV1, RoomInput, RoomRun, EXIT, INTERACT, KEY, PLAYER,
};
use orr_physics3d::Body;
use orr_sample::room_project::{self, PreparedProject};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn options(output: &Path, seed: &str) -> authoring::Options {
    authoring::Options {
        output: output.to_path_buf(),
        seed: seed.to_owned(),
    }
}

fn ordinary_windows_path(path: &Path) -> PathBuf {
    let text = path.to_str().expect("temporary path must be Unicode");
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text))
}

fn argument_list(values: &[&str]) -> Vec<std::ffi::OsString> {
    values.iter().map(std::ffi::OsString::from).collect()
}

#[test]
fn creator_options_parse_both_flag_orders_and_unicode_paths() {
    let output = r"C:\Games\Room Ω";
    for arguments in [
        argument_list(&["--output", output, "--seed", "valid-seed"]),
        argument_list(&["--seed", "valid-seed", "--output", output]),
    ] {
        let parsed = authoring::Options::parse(arguments).unwrap();
        assert_eq!(parsed.output, PathBuf::from(output));
        assert_eq!(parsed.seed, "valid-seed");
    }
}

#[test]
fn creator_options_reject_unknown_duplicate_missing_and_invalid_seed_values() {
    let missing = authoring::Options::parse(argument_list(&["--output", "C:\\Room"]));
    assert_eq!(missing.unwrap_err(), authoring::USAGE);

    for arguments in [
        argument_list(&["--output", "C:\\Room", "--output", "C:\\Other"]),
        argument_list(&["--seed", "seed", "--seed", "other"]),
        argument_list(&["--output", "C:\\Room", "--mystery", "seed"]),
    ] {
        let error = authoring::Options::parse(arguments).unwrap_err();
        assert!(error.contains("unknown or duplicate option"), "{error}");
    }

    for invalid_seed in [String::new(), "contains space".into(), "bad/seed".into()] {
        let error = authoring::Options::parse(vec![
            "--output".into(),
            "C:\\Room".into(),
            "--seed".into(),
            invalid_seed.into(),
        ])
        .unwrap_err();
        assert!(error.contains("seed must contain"), "{error}");
    }
    let long_seed = "s".repeat(129);
    assert!(authoring::Options::parse(vec![
        "--output".into(),
        "C:\\Room".into(),
        "--seed".into(),
        long_seed.into(),
    ])
    .unwrap_err()
    .contains("seed exceeds 128 bytes"));
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            assert!(
                !file_type.is_symlink(),
                "unexpected symlink: {}",
                entry.path().display()
            );
            if file_type.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(file_type.is_file());
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                files.insert(relative, fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn assert_no_extra_entries(parent: &Path, expected: &[&str]) {
    let mut actual: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    actual.sort();
    let mut expected: Vec<_> = expected.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "creator left an unexpected stage or artifact"
    );
}

#[test]
fn room_creation_is_reproducible_and_admits_the_closed_model_project() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let first_path = root.join("Room project Ω");
    let second_path = root.join("Second Room project Ω");
    let first = authoring::create(&options(&first_path, "same-seed")).unwrap();
    let second = authoring::create(&options(&second_path, "same-seed")).unwrap();

    assert_eq!(tree(&first.output), tree(&second.output));
    assert_eq!(first.entity_guids, second.entity_guids);
    assert_eq!(first.entity_guids.len(), 7);
    assert_eq!(first.initial_checksum, second.initial_checksum);
    let prepared = PreparedProject::open(&first.output).unwrap();
    assert_eq!(prepared.scene().frame().checksum(), first.initial_checksum);
    assert_eq!(prepared.models().document.bindings.len(), 2);
    assert_eq!(prepared.models().assets.len(), 1);
    assert!(prepared.camera().is_some());
    let package =
        orr_package::Project::open(&first.output, room_project::compiled_runtime()).unwrap();
    let lock = package.verify().unwrap();
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.direct.len(), 1);
    for source_asset in [
        "background.glb",
        "foreground.glb",
        "LICENSE.txt",
        "generate.py",
    ] {
        assert!(package
            .read_asset("sample-imported-scene", source_asset)
            .is_ok());
    }
    assert_no_extra_entries(&root, &["Room project Ω", "Second Room project Ω"]);
}

#[test]
fn different_seed_changes_authored_guids_without_changing_gameplay_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let first = authoring::create(&options(&root.join("one"), "first-seed")).unwrap();
    let second = authoring::create(&options(&root.join("two"), "second-seed")).unwrap();

    assert!(first
        .entity_guids
        .iter()
        .all(|guid| !second.entity_guids.contains(guid)));
    assert_eq!(first.initial_checksum, second.initial_checksum);
    for report in [&first, &second] {
        let prepared = PreparedProject::open(&report.output).unwrap();
        assert_eq!(prepared.scene().frame().checksum(), report.initial_checksum);
        assert_eq!(prepared.models().document.bindings.len(), 2);
    }
}

#[test]
fn creation_never_replaces_existing_directories_or_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let directory = root.join("existing directory");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep.txt"), b"directory owner").unwrap();
    assert!(authoring::create(&options(&directory, "seed")).is_err());
    assert_eq!(
        fs::read(directory.join("keep.txt")).unwrap(),
        b"directory owner"
    );

    let file = root.join("existing file");
    fs::write(&file, b"file owner").unwrap();
    assert!(authoring::create(&options(&file, "seed")).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"file owner");
    assert_no_extra_entries(&root, &["existing directory", "existing file"]);
}

#[test]
fn invalid_paths_fail_before_creating_staging_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let parent = root.join("parent");
    fs::create_dir(&parent).unwrap();
    let invalid = [
        PathBuf::from("relative-room-project"),
        parent.join("missing-parent").join("project"),
        parent.join("..").join("traversal-project"),
        parent.join("project:stream"),
        parent.join("CON"),
        parent.join("trailing-dot."),
        parent.join("trailing-space "),
    ];
    for output in invalid {
        assert!(
            authoring::create(&options(&output, "seed")).is_err(),
            "invalid Windows output unexpectedly accepted: {}",
            output.display()
        );
    }
    assert_no_extra_entries(&root, &["parent"]);
    assert_no_extra_entries(&parent, &[]);
}

fn tick(simulation: &mut Simulation<RoomEscapeV1>, input: RoomInput, count: usize) {
    for _ in 0..count {
        let mut inputs = TickInputs::new(simulation.tick(), 1);
        inputs.set_input(PlayerSlot(0), input);
        simulation.step(&inputs);
    }
}

fn actor(frame: &Frame, kind: u32) -> orr_ecs::Entity {
    frame
        .entities()
        .find(|&entity| {
            frame
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == kind)
        })
        .expect("admitted room actor")
}

fn interact() -> RoomInput {
    RoomInput {
        buttons: INTERACT,
        ..RoomInput::default()
    }
}

#[test]
fn created_room_can_be_walked_to_key_and_then_exit() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let report = authoring::create(&options(&root.join("playable room"), "play-seed")).unwrap();
    let prepared = PreparedProject::open(&report.output).unwrap();
    let mut simulation = prepared.scene().simulation().unwrap();
    let initial_player_position = simulation
        .frame()
        .get::<Body>(actor(simulation.frame(), PLAYER))
        .unwrap()
        .pos;

    tick(
        &mut simulation,
        RoomInput {
            move_x: -1,
            ..RoomInput::default()
        },
        32,
    );
    tick(&mut simulation, interact(), 1);
    assert_eq!(simulation.frame().singleton::<RoomRun>().key_collected, 0);
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 0);
    tick(&mut simulation, RoomInput::default(), 1);
    tick(
        &mut simulation,
        RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        52,
    );
    tick(&mut simulation, interact(), 1);
    assert_eq!(simulation.frame().singleton::<RoomRun>().key_collected, 1);
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 0);
    tick(&mut simulation, RoomInput::default(), 1);
    tick(
        &mut simulation,
        RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        40,
    );
    tick(&mut simulation, interact(), 1);

    let final_frame = simulation.frame();
    let run = final_frame.singleton::<RoomRun>();
    assert_eq!(run.key_collected, 1);
    assert_eq!(run.won, 1);
    let player_position = final_frame
        .get::<Body>(actor(final_frame, PLAYER))
        .unwrap()
        .pos;
    let exit_position = final_frame
        .get::<Body>(actor(final_frame, EXIT))
        .unwrap()
        .pos;
    assert_ne!(player_position, initial_player_position);
    assert_eq!(player_position.y, initial_player_position.y);
    assert!((player_position.x - exit_position.x).abs() < FP::from_ratio(1, 10));
    let key_position = final_frame
        .get::<Body>(actor(final_frame, KEY))
        .unwrap()
        .pos;
    assert_eq!(key_position.y, initial_player_position.y);
}

#[test]
fn editor_undo_redo_save_and_reopen_preserve_room_admission() {
    let temp = tempfile::tempdir().unwrap();
    let root = ordinary_windows_path(temp.path());
    let report = authoring::create(&options(&root.join("editable room"), "edit-seed")).unwrap();
    let prepared = PreparedProject::open(&report.output).unwrap();
    let mut document = orr_edit::EditorDoc::from_yaml(
        prepared.scene().text(),
        room_project::types(),
        Simulation::<RoomEscapeV1>::build_registry(),
        room_project::SEED,
    )
    .unwrap();
    let expected_checksum = document.checksum();
    let key_guid = document
        .scene()
        .entities
        .iter()
        .find_map(|(guid, entity)| {
            entity
                .components
                .iter()
                .find(|(component, _)| component == room_project::ACTOR)
                .and_then(|(_, actor)| {
                    (actor.field("kind") == Some(&orr_reflect::Value::Int(i128::from(KEY))))
                        .then(|| guid.clone())
                })
        })
        .expect("key GUID in admitted scene");
    document
        .apply(
            orr_edit::Op::Rename {
                guid: key_guid,
                name: Some("author edited key".into()),
            },
            orr_edit::Origin::User,
        )
        .unwrap();
    document.undo().unwrap();
    document.redo().unwrap();
    assert_eq!(document.checksum(), expected_checksum);
    let saved = document.save_yaml();
    fs::write(prepared.path(), saved).unwrap();
    let reopened = PreparedProject::open(&report.output).unwrap();
    assert!(reopened.scene().text().contains("author edited key"));
    assert_eq!(reopened.models().document.bindings.len(), 2);
    assert_eq!(reopened.scene().frame().checksum(), expected_checksum);
}

#[test]
#[ignore = "requires an explicitly selected Windows creator binary and fresh evidence directory"]
fn actual_windows_creator_cli_persists_an_admitted_project() {
    let creator = std::env::var_os("ORR_WINDOWS_ROOM_CREATOR")
        .map(PathBuf::from)
        .expect("ORR_WINDOWS_ROOM_CREATOR must name the tested Windows creator executable");
    assert!(creator.is_absolute() && creator.is_file());
    let evidence = std::env::var_os("ORR_WIN_CREATE_EVIDENCE")
        .map(PathBuf::from)
        .expect("ORR_WIN_CREATE_EVIDENCE must name a fresh evidence directory");
    assert!(evidence.is_absolute() && evidence.is_dir());
    let evidence = ordinary_windows_path(&evidence);
    assert_eq!(
        fs::read_dir(&evidence).unwrap().count(),
        0,
        "evidence directory must be fresh"
    );
    let output = evidence.join("Created Room Ω");
    let result = Command::new(creator)
        .arg("--output")
        .arg(&output)
        .arg("--seed")
        .arg("cli-seed")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "creator failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("created project"), "{stdout}");
    assert!(output.is_dir());
    let prepared = PreparedProject::open(&output).unwrap();
    let mut simulation = prepared.scene().simulation().unwrap();
    let initial_checksum = simulation.frame().checksum();
    let initial_run = simulation.frame().singleton::<RoomRun>();
    assert_eq!(initial_run.key_collected, 0);
    assert_eq!(initial_run.won, 0);
    let mut stages = vec![serde_json::json!({
        "name": "initial",
        "ticks": simulation.tick(),
        "checksum": format!("{:016x}", initial_checksum),
        "key_collected": initial_run.key_collected,
        "won": initial_run.won,
    })];

    tick(
        &mut simulation,
        RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        20,
    );
    stages.push(serde_json::json!({
        "name": "approached_key",
        "ticks": simulation.tick(),
        "checksum": format!("{:016x}", simulation.frame().checksum()),
    }));
    tick(&mut simulation, interact(), 1);
    let after_key = simulation.frame().singleton::<RoomRun>();
    assert_eq!(after_key.key_collected, 1);
    assert_eq!(after_key.won, 0);
    stages.push(serde_json::json!({
        "name": "key_interacted",
        "ticks": simulation.tick(),
        "checksum": format!("{:016x}", simulation.frame().checksum()),
        "key_collected": after_key.key_collected,
        "won": after_key.won,
    }));
    tick(&mut simulation, RoomInput::default(), 1);
    tick(
        &mut simulation,
        RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        40,
    );
    stages.push(serde_json::json!({
        "name": "approached_exit",
        "ticks": simulation.tick(),
        "checksum": format!("{:016x}", simulation.frame().checksum()),
        "key_collected": simulation.frame().singleton::<RoomRun>().key_collected,
        "won": simulation.frame().singleton::<RoomRun>().won,
    }));
    tick(&mut simulation, interact(), 1);
    let completed = simulation.frame().singleton::<RoomRun>();
    assert_eq!(completed.key_collected, 1);
    assert_eq!(completed.won, 1);
    stages.push(serde_json::json!({
        "name": "exit_interacted",
        "ticks": simulation.tick(),
        "checksum": format!("{:016x}", simulation.frame().checksum()),
        "key_collected": completed.key_collected,
        "won": completed.won,
    }));
    let receipt = serde_json::json!({
        "schema": 1,
        "source": "fresh admitted PreparedProject Frame oracle",
        "initial_checksum": format!("{:016x}", initial_checksum),
        "stages": stages,
    });
    fs::write(
        evidence.join("room-session-oracle.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert!(
        stdout.contains(&format!("0x{:016x}", initial_checksum)),
        "CLI checksum differs from admitted project: {stdout}"
    );
}
