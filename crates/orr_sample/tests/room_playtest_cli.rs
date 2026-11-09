#![cfg(all(
    feature = "room-playtest",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Output},
};
fn step(ticks: u32, x: i8, interact: bool, key: bool, won: bool) -> Value {
    json!({"ticks":ticks,"move_x":x,"move_z":0,"interact":interact,"expect_key":key,"expect_won":won})
}
fn script(steps: Vec<Value>) -> Value {
    json!({"schema":1,"steps":steps})
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("Room Ω");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "room-escape-3d-v1".into(),
        seed: "playtest".into(),
    })
    .unwrap();
    (temp, root)
}
fn run(root: &Path, script: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orr_playtest_room"))
        .arg("--project")
        .arg(root)
        .arg("--script")
        .arg(script)
        .output()
        .unwrap()
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn tree(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
#[test]
fn saved_completion_repeats_exactly_without_writes_and_requires_release() {
    let (temp, root) = fixture();
    let path = temp.path().join("route Ω.json");
    let before = tree(&root);
    write(
        &path,
        &script(vec![
            step(0, 0, false, false, false),
            step(1, 0, true, true, false),
            step(60, 1, true, true, false),
            step(1, 0, false, true, false),
            step(1, 0, true, true, true),
            step(5, -1, false, true, true),
        ]),
    );
    let first = run(&root, &path);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("passed steps=6 ticks=68"));
    let repeated = run(&root, &path);
    assert!(repeated.status.success());
    assert_eq!(first.stdout, repeated.stdout);
    assert_eq!(before, tree(&root));
    write(
        &path,
        &script(vec![
            step(1, 0, true, true, false),
            step(60, 1, true, true, true),
        ]),
    );
    let failed = run(&root, &path);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("step 2 tick 61"));
    assert_eq!(before, tree(&root));
}
#[test]
fn actual_wall_blocks_route_and_false_assertions_fail() {
    let (temp, root) = fixture();
    let path = temp.path().join("route.json");
    // Outer wall at X=5 stops the player; 240 forward then 30 backward returns to interaction range of the exit (an unblocked trajectory would miss it).
    write(
        &path,
        &script(vec![
            step(1, 0, true, true, false),
            step(240, 1, false, true, false),
            step(30, -1, false, true, false),
            step(1, 0, true, true, true),
        ]),
    );
    let good = run(&root, &path);
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    write(&path, &script(vec![step(0, 0, false, true, false)]));
    assert!(!run(&root, &path).status.success());
}
#[test]
fn rejects_schema_fields_ranges_budget_and_non_regular_inputs() {
    let (temp, root) = fixture();
    let path = temp.path().join("bad.json");
    let good = script(vec![step(1, 0, false, false, false)]);
    let mut extra = good.clone();
    extra["extra"] = json!(1);
    let mut missing = good.clone();
    missing["steps"][0]
        .as_object_mut()
        .unwrap()
        .remove("interact");
    let mut wrong = good.clone();
    wrong["schema"] = json!(2);
    let mut unknown_step = good.clone();
    unknown_step["steps"][0]["extra"] = json!(1);
    let mut wrong_ticks = good.clone();
    wrong_ticks["steps"][0]["ticks"] = json!(true);
    let mut negative_ticks = good.clone();
    negative_ticks["steps"][0]["ticks"] = json!(-1);
    let mut wrong_interact = good.clone();
    wrong_interact["steps"][0]["interact"] = json!("false");
    let mut wrong_z = good.clone();
    wrong_z["steps"][0]["move_z"] = json!(2);
    for invalid in [
        unknown_step,
        wrong_ticks,
        negative_ticks,
        wrong_interact,
        wrong_z,
        script(vec![step(6001, 0, false, false, false)]),
        extra,
        missing,
        wrong,
        script(vec![]),
        script(vec![step(1, 2, false, false, false)]),
        script(vec![
            step(6000, 0, false, false, false),
            step(1, 0, false, false, false),
        ]),
        script(vec![step(0, 0, false, false, false); 257]),
    ] {
        write(&path, &invalid);
        assert!(!run(&root, &path).status.success(), "{invalid}");
    }
    fs::write(&path, vec![b' '; 65537]).unwrap();
    let oversized = run(&root, &path);
    assert!(!oversized.status.success());
    assert!(String::from_utf8_lossy(&oversized.stderr).contains("at most 65536 bytes"));
    let encoded = serde_json::to_string(&good).unwrap();
    for duplicate in [
        encoded.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1),
        encoded.replacen("\"ticks\":1", "\"ticks\":1,\"ticks\":1", 1),
    ] {
        assert_ne!(duplicate, encoded);
        fs::write(&path, duplicate).unwrap();
        let failure = run(&root, &path);
        assert!(!failure.status.success());
        assert!(String::from_utf8_lossy(&failure.stderr).contains("duplicate field"));
    }
    write(&path, &good);
    assert!(run(&root, &path).status.success());
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(!run(&root, &link).status.success());
    let fifo = temp.path().join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
    assert!(!run(&root, &fifo).status.success());
    assert!(!run(&root, temp.path()).status.success());
}
#[test]
fn cli_usage_and_project_admission_fail_closed() {
    let help = Command::new(env!("CARGO_BIN_EXE_orr_playtest_room"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let usage = Command::new(env!("CARGO_BIN_EXE_orr_playtest_room"))
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2));
    let (temp, root) = fixture();
    let path = temp.path().join("route.json");
    write(&path, &script(vec![step(1, 0, false, false, false)]));
    fs::write(root.join("room.scene.yaml"), "invalid scene").unwrap();
    assert!(!run(&root, &path).status.success());
}

#[cfg(feature = "room-input-actions")]
#[test]
fn unified_controls_feature_does_not_grant_consumer_support() {
    let (temp, root) = fixture();
    let path = temp.path().join("route.json");
    write(&path, &script(vec![step(1, 0, false, false, false)]));
    let manifest_path = root.join("orr.project.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["controls"] = json!("room.controls.json");
    write(&manifest_path, &manifest);
    fs::write(
        root.join("room.controls.json"),
        serde_json::to_vec(&orr_sample::room_controls::RoomControls::default_map()).unwrap(),
    )
    .unwrap();
    assert!(orr_sample::room_project::PreparedProject::open(&root).is_ok());
    let result = run(&root, &path);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("do not consume authored physical controls"));
}
