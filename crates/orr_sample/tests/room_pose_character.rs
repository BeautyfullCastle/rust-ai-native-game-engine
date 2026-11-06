#![cfg(all(
    feature = "room-character",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types)]
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), out);
            } else {
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn character_crossfade_cpu_check_preserves_game_output_and_all_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("character game");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: orr_sample::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "pose-check".into(),
    })
    .unwrap();
    let path = root.join("room.character.json");
    let mut document =
        orr_sample::room_character::Document::parse(&fs::read(&path).unwrap()).unwrap();
    document.schema = 3;
    document.crossfade_ticks = Some(120);
    document.speeds = Some(orr_sample::room_character::PlaybackRates {
        searching: orr_sample::room_character::PlaybackRate::Half,
        carrying: orr_sample::room_character::PlaybackRate::Double,
        escaped: orr_sample::room_character::PlaybackRate::Quarter,
    });
    fs::write(path, document.to_bytes().unwrap()).unwrap();
    let before = snapshot(&root);
    for ticks in [0, 8] {
        let run = |checked: bool| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_room_escape"));
            command.arg("--project").arg(&root).args([
                "--headless",
                "--ticks",
                &ticks.to_string(),
                "--hold",
                "interact",
            ]);
            if checked {
                command.arg("--check-model-poses");
            }
            let output = command
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        let plain = run(false);
        let checked = run(true);
        assert!(checked.contains(&format!("room model poses: {} frames checked;", ticks + 1)));
        assert!(checked.contains("animated: 1"));
        let normalized = checked
            .lines()
            .filter(|line| !line.starts_with("room model poses:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(plain.trim_end(), normalized);
        assert_eq!(before, snapshot(&root));
    }
}
