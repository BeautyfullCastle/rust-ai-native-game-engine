#![cfg(all(
    feature = "project-create",
    feature = "room-playtest",
    target_os = "linux",
    target_arch = "x86_64"
))]
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn create(root: &Path, input: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
        .arg("--output")
        .arg(root)
        .args([
            "--template",
            "room-escape-3d-v1",
            "--seed",
            "detour",
            "--room-layout",
        ])
        .arg(input)
        .output()
        .unwrap()
}
fn step(t: u32, x: i8, z: i8, i: bool, k: bool, w: bool) -> Value {
    json!({"ticks":t,"move_x":x,"move_z":z,"interact":i,"expect_key":k,"expect_won":w})
}
fn play(root: &Path, path: &Path, steps: Vec<Value>) -> Output {
    fs::write(
        path,
        serde_json::to_vec(&json!({"schema":1,"steps":steps})).unwrap(),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_orr_playtest_room"))
        .arg("--project")
        .arg(root)
        .arg("--script")
        .arg(path)
        .arg("--json")
        .output()
        .unwrap()
}
#[test]
fn authored_wall_blocks_direct_route_and_saved_detour_wins() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("Puzzle Ω");
    let input = base.join("recipe Ω.json");
    let script = base.join("route.json");
    fs::write(&input,serde_json::to_vec(&json!({"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],"walls":[{"center":["0","0"],"half_extents":["0.25","1"]}]})).unwrap()).unwrap();
    let made = create(&root, &input);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let before = fs::read(root.join("room.scene.yaml")).unwrap();
    let direct = play(
        &root,
        &script,
        vec![
            step(1, 0, 0, true, true, false),
            step(120, 1, 0, false, true, false),
            step(1, 0, 0, true, true, true),
        ],
    );
    assert_eq!(direct.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&direct.stdout).unwrap();
    assert_eq!(report["error"]["kind"], "assertion");
    assert_eq!(report["steps"][2]["observed"]["won"], false);
    let route = vec![
        step(1, 0, 0, true, true, false),
        step(40, 0, 1, false, true, false),
        step(120, 1, 0, false, true, false),
        step(40, 0, -1, false, true, false),
        step(1, 0, 0, true, true, true),
    ];
    let won = play(&root, &script, route.clone());
    assert!(
        won.status.success(),
        "{} {}",
        String::from_utf8_lossy(&won.stdout),
        String::from_utf8_lossy(&won.stderr)
    );
    let repeat = play(&root, &script, route);
    assert_eq!(won.stdout, repeat.stdout);
    assert_eq!(before, fs::read(root.join("room.scene.yaml")).unwrap());
}
#[test]
fn cli_rejects_bad_recipe_and_incompatible_flags_without_output() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("new");
    let input = base.join("bad.json");
    fs::write(&input, b"{}").unwrap();
    assert_eq!(create(&root, &input).status.code(), Some(1));
    assert!(!root.exists());
    for extras in [
        vec!["--room-layout"],
        vec!["--room-layout", "missing", "--room-layout", "missing"],
        vec!["--room-layout", "missing", "--unknown", "value"],
        vec!["--room-layout", "missing", "--game-id", "bad"],
        vec!["--room-layout", "missing", "--sprite-package", "bad"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .arg("--output")
            .arg(&root)
            .args(["--template", "room-escape-3d-v1", "--seed", "test"])
            .args(extras)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!root.exists());
    }
    for template in ["arena-2d-v1", "room-escape-character-3d-v1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .arg("--output")
            .arg(&root)
            .args(["--template", template, "--seed", "test", "--room-layout"])
            .arg(&input)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!root.exists());
    }
}
