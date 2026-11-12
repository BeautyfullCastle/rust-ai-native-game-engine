#![cfg(all(
    feature = "room-playtest",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
use serde_json::{json, Value};
#[cfg(feature = "room-input-actions")]
use std::collections::BTreeMap;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
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
#[cfg(feature = "room-input-actions")]
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
#[cfg(feature = "room-input-actions")]
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
fn run_json(root: &Path, script: &Path) -> (Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_orr_playtest_room"))
        .arg("--project")
        .arg(root)
        .arg("--script")
        .arg(script)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() < 256 * 1024);
    let report = serde_json::from_slice(&output.stdout).unwrap();
    (output, report)
}

#[cfg(not(feature = "room-input-actions"))]
#[test]
fn key_scripts_require_explicit_compiled_support() {
    let (temp, root) = fixture();
    let path = temp.path().join("keys.json");
    write(
        &path,
        &json!({"schema":2,"steps":[{"ticks":1,"held":[],"expect_key":false,"expect_won":false}]}),
    );
    let (failed, report) = run_json(&root, &path);
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(report["error"]["kind"], "script");
    assert!(report.get("script_sha256").is_none());
}

#[cfg(feature = "room-input-actions")]
mod enabled {
    use super::*;
    use orr_input::{Button, Key};
    fn keys(ticks: u32, held: &[&str], key: bool, won: bool) -> Value {
        json!({"ticks":ticks,"held":held,"expect_key":key,"expect_won":won})
    }
    fn key_script(steps: Vec<Value>) -> Value {
        json!({"schema":2,"steps":steps})
    }
    fn controls(root: &Path) -> Vec<u8> {
        let mut map = orr_sample::room_controls::RoomControls::default_map();
        for action in &mut map.actions {
            if action.name == "interact" {
                action.bindings = vec![Button::Keyboard { key: Key::Space }];
            }
            if action.name == "move_right" {
                action.bindings = vec![
                    Button::Keyboard { key: Key::L },
                    Button::Keyboard {
                        key: Key::ArrowRight,
                    },
                ];
            }
        }
        let bytes = serde_json::to_vec(&map).unwrap();
        let manifest_path = root.join("orr.project.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entry"]["controls"] = json!("room.controls.json");
        write(&manifest_path, &manifest);
        fs::write(root.join("room.controls.json"), &bytes).unwrap();
        bytes
    }
    #[test]
    fn mapped_keys_win_repeat_and_report_exact_admitted_map_without_writes() {
        use sha2::{Digest, Sha256};
        let (temp, root) = fixture();
        let bytes = controls(&root);
        let path = temp.path().join("keys Ω.json");
        write(
            &path,
            &key_script(vec![
                keys(1, &["e", "d"], false, false), // Old defaults no longer act.
                keys(1, &["space"], true, false),
                keys(10, &["l", "arrow_right", "a"], true, false), // Aliases do not double movement; opposition cancels.
                keys(60, &["l", "arrow_right", "space"], true, false), // Held interact cannot retrigger.
                keys(0, &[], true, false), // Zero ticks must not release interact.
                keys(1, &["space"], true, false),
                keys(1, &[], true, false),
                keys(1, &["space"], true, true),
            ]),
        );
        let before = tree(&root);
        let (first, report) = run_json(&root, &path);
        assert!(first.status.success(), "{report}");
        assert_eq!(report["schema"], 2);
        assert_eq!(report["input_mode"], "supplied_physical_key_identities");
        assert_eq!(
            report["controls_sha256"],
            Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(
            report["steps"][0]["input"],
            json!({"move_x":0,"move_z":0,"interact":false})
        );
        assert_eq!(report["steps"][2]["input"]["move_x"], 0);
        assert_eq!(report["steps"][3]["input"]["move_x"], 1);
        assert_eq!(report["steps"][7]["observed"]["won"], true);
        let (repeat, _) = run_json(&root, &path);
        assert_eq!(first.stdout, repeat.stdout);
        let text = run(&root, &path);
        assert!(text.status.success());
        assert!(String::from_utf8_lossy(&text.stdout).contains("room playtest schema=2"));
        assert_eq!(before, tree(&root));
    }
    #[test]
    fn key_script_validation_is_strict_before_project_admission() {
        let (temp, root) = fixture();
        let path = temp.path().join("keys.json");
        let good = key_script(vec![keys(1, &[], false, false)]);
        let mut bad = vec![];
        for held in [
            json!(["unknown"]),
            json!([{ "space": null }]),
            json!([1]),
            json!([true]),
            json!(["r"]),
            json!(["escape"]),
            json!(["a", "a"]),
            json!(["a", "b", "c", "d", "e", "f", "g", "h", "i"]),
            json!(null),
        ] {
            let mut value = good.clone();
            value["steps"][0]["held"] = held;
            bad.push(value);
        }
        for field in ["held", "ticks", "expect_key", "expect_won"] {
            let mut value = good.clone();
            value["steps"][0].as_object_mut().unwrap().remove(field);
            bad.push(value);
        }
        for (field, value) in [
            ("move_x", json!(0)),
            ("interact", json!(false)),
            ("extra", json!(0)),
            ("ticks", json!(6001)),
        ] {
            let mut item = good.clone();
            item["steps"][0][field] = value;
            bad.push(item);
        }
        bad.push(key_script(vec![]));
        bad.push(key_script(vec![keys(0, &[], false, false); 257]));
        bad.push(key_script(vec![keys(3001, &[], false, false); 2]));
        for value in bad {
            write(&path, &value);
            let (failed, report) = run_json(&root.join("missing"), &path);
            assert_eq!(failed.status.code(), Some(1), "{value}");
            assert_eq!(report["error"]["kind"], "script", "{report}");
            assert!(report.get("script_sha256").is_none());
        }
        let encoded = good.to_string();
        for duplicate in [
            encoded.replacen("\"schema\":2", "\"schema\":2,\"schema\":2", 1),
            encoded.replacen("\"held\":[]", "\"held\":[],\"held\":[]", 1),
        ] {
            assert_ne!(duplicate, encoded);
            fs::write(&path, duplicate).unwrap();
            let (_, report) = run_json(&root, &path);
            assert_eq!(report["error"]["kind"], "script");
            assert!(report["error"]["message"]
                .as_str()
                .unwrap()
                .contains("duplicate field"));
        }
    }
    #[test]
    fn controls_are_required_read_fresh_and_never_fallback_on_invalid_project() {
        let (temp, root) = fixture();
        let path = temp.path().join("keys.json");
        write(&path, &key_script(vec![keys(1, &["space"], true, false)]));
        let (_, report) = run_json(&root, &path);
        assert_eq!(report["schema"], 2);
        assert_eq!(report["error"]["kind"], "unsupported_project");
        assert!(report.get("controls_sha256").is_none());
        controls(&root);
        let (_, original) = run_json(&root, &path);
        assert_eq!(original["status"], "passed");
        let map_path = root.join("room.controls.json");
        fs::write(
            &map_path,
            serde_json::to_vec(&orr_sample::room_controls::RoomControls::default_map()).unwrap(),
        )
        .unwrap();
        let (failed, changed) = run_json(&root, &path);
        assert_eq!(failed.status.code(), Some(1));
        assert_eq!(changed["error"]["kind"], "assertion");
        assert_ne!(changed["controls_sha256"], original["controls_sha256"]);
        assert_eq!(changed["steps"][0]["input"]["interact"], false);
        for bad in [b"invalid".as_slice(), b"{}".as_slice()] {
            fs::write(&map_path, bad).unwrap();
            let (_, report) = run_json(&root, &path);
            assert_eq!(report["error"]["kind"], "project");
            assert_eq!(report["steps"], json!([]));
            assert!(report.get("controls_sha256").is_none());
            assert!(report.get("initial_checksum").is_none());
        }
        controls(&root);
        fs::write(root.join("room.scene.yaml"), "invalid scene").unwrap();
        let (_, report) = run_json(&root, &path);
        assert_eq!(report["error"]["kind"], "project");
        assert!(report.get("controls_sha256").is_none());
    }

    #[test]
    fn maximum_key_script_report_is_bounded() {
        let (temp, root) = fixture();
        controls(&root);
        let path = temp.path().join("maximum-keys.json");
        let mut steps = vec![keys(0, &["a", "b", "c", "d", "f", "g", "h", "i"], false, false); 256];
        steps[1] = keys(0, &["w"], false, false);
        steps[2] = keys(0, &["s", "arrow_up"], false, false);
        write(&path, &key_script(steps));
        let (output, report) = run_json(&root, &path);
        assert!(output.status.success(), "{report}");
        assert_eq!(report["steps"].as_array().unwrap().len(), 256);
        assert_eq!(report["steps"][255]["tick"], 0);
        assert_eq!(report["steps"][1]["input"]["move_z"], -1);
        assert_eq!(report["steps"][2]["input"]["move_z"], 0);
        assert!(output.stdout.len() < 256 * 1024);
    }
}
