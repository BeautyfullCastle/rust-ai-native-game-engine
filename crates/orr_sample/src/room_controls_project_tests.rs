use std::fs;
#[cfg(feature = "room-input-actions")]
use std::os::unix::fs::symlink;

fn room_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let options = crate::project_create::CreateOptions {
        output: temp.path().join("room"),
        template: crate::project_create::ROOM_TEMPLATE.into(),
        seed: "room-controls-test".into(),
    };
    crate::project_create::create(&options).unwrap();
    let manifest_path = options.output.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["controls"] = "room.controls.json".into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    (temp, options.output)
}

#[cfg(feature = "room-input-actions")]
#[test]
fn controls_reopen_custom_mapping_and_reject_unsafe_files() {
    let (temp, root) = room_fixture();
    let mut map = crate::room_controls::RoomControls::default_map();
    map.actions[0].bindings = vec![orr_input::Button::Keyboard {
        key: orr_input::Key::J,
    }];
    let bytes = serde_json::to_vec(&map).unwrap();
    let controls_path = root.join("room.controls.json");
    fs::write(&controls_path, &bytes).unwrap();

    let prepared = crate::room_project::PreparedProject::open(&root).unwrap();
    let admitted = prepared.controls().unwrap();
    assert_eq!(admitted.bytes, bytes);
    assert_eq!(admitted.map.input_from_keys([orr_input::Key::J]).move_x, -1);
    assert_eq!(admitted.map.input_from_keys([orr_input::Key::A]).move_x, 0);

    fs::write(&controls_path, vec![b' '; orr_input::MAX_FILE_BYTES + 1]).unwrap();
    assert!(crate::room_project::PreparedProject::open(&root).is_err());
    fs::remove_file(&controls_path).unwrap();
    let external = temp.path().join("external.controls.json");
    fs::write(&external, &bytes).unwrap();
    symlink(&external, &controls_path).unwrap();
    assert!(crate::room_project::PreparedProject::open(&root).is_err());
    fs::remove_file(&controls_path).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &controls_path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert!(crate::room_project::PreparedProject::open(&root).is_err());
    let manifest_path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["controls"] = "../outside.json".into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(crate::room_project::PreparedProject::open(&root).is_err());
}

#[cfg(not(feature = "room-input-actions"))]
#[test]
fn controls_fail_before_runtime_without_opt_in_feature() {
    let (_temp, root) = room_fixture();
    fs::write(root.join("room.controls.json"), b"{}").unwrap();
    let error = match crate::room_project::PreparedProject::open(&root) {
        Err(error) => error,
        Ok(_) => panic!("feature-off Room accepted controls"),
    };
    assert!(error.contains("room-input-actions"));
}

#[cfg(all(
    feature = "room-input-actions",
    feature = "project-create",
    feature = "project-export",
    target_arch = "x86_64"
))]
#[test]
fn relocated_export_closure_runs_identical_custom_and_default_room_trace() {
    use crate::project_export::{admission::ProjectSnapshot, profile::Profile};
    use orr_games::room_escape_game::{RoomInput, RoomRun};
    use orr_input::Key;
    use orr_sim::{PlayerSlot, TickInputs};

    fn trace(
        project: &crate::room_project::PreparedProject,
        controls: &crate::room_controls::RoomControls,
        events: &[Vec<Key>],
    ) -> Vec<(u64, u64, u32, u32)> {
        let mut sim = project.scene().simulation().unwrap();
        let mut output = Vec::new();
        for held in events {
            let input = controls.input_from_keys(held.iter().copied());
            let mut inputs = TickInputs::new(sim.tick() + 1, 1);
            inputs.set_input(PlayerSlot(0), input);
            sim.step(&inputs);
            let run = sim.frame().singleton::<RoomRun>();
            output.push((
                sim.tick(),
                sim.frame().checksum(),
                run.key_collected,
                run.won,
            ));
        }
        output
    }

    let (temp, root) = room_fixture();
    let mut map = crate::room_controls::RoomControls::default_map();
    for action in &mut map.actions {
        let key = match action.name.as_str() {
            "move_left" => Key::J,
            "move_right" => Key::L,
            "move_forward" => Key::I,
            "move_backward" => Key::K,
            "interact" => Key::O,
            _ => unreachable!(),
        };
        action.bindings = vec![orr_input::Button::Keyboard { key }];
    }
    let controls_bytes = serde_json::to_vec(&map).unwrap();
    fs::write(root.join("room.controls.json"), &controls_bytes).unwrap();

    let snapshot = ProjectSnapshot::open_profile(&root, Profile::Room).unwrap();
    let relocated = temp.path().join("relocated-export/project");
    for file in &snapshot.files {
        let destination = relocated.join(&file.relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, &file.bytes).unwrap();
    }
    snapshot.recheck().unwrap();
    let project = crate::room_project::PreparedProject::open(&relocated).unwrap();
    assert_eq!(project.controls().unwrap().bytes, controls_bytes);
    assert_eq!(
        project
            .controls()
            .unwrap()
            .map
            .input_from_keys([Key::D, Key::E]),
        RoomInput::default()
    );
    fs::rename(&root, temp.path().join("source-hidden")).unwrap();

    let custom = project.controls().unwrap().map.clone();
    let default = crate::room_controls::RoomControls::from_map(
        &crate::room_controls::RoomControls::default_map(),
    )
    .unwrap();
    let custom_events = std::iter::once(vec![Key::O])
        .chain(std::iter::repeat_n(vec![Key::L], 40))
        .chain(std::iter::once(vec![Key::O]))
        .collect::<Vec<_>>();
    let default_events = std::iter::once(vec![Key::E])
        .chain(std::iter::repeat_n(vec![Key::D], 40))
        .chain(std::iter::once(vec![Key::E]))
        .collect::<Vec<_>>();
    let custom_trace = trace(&project, &custom, &custom_events);
    let default_trace = trace(&project, &default, &default_events);
    assert_eq!(custom_trace, default_trace);
    assert_eq!(custom_trace.last().unwrap().2, 1);
    assert_eq!(custom_trace.last().unwrap().3, 1);
}
