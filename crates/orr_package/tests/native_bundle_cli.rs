//! Feature contract and explicit fresh-binary delivery acceptance. Fixture tests
//! do not substitute for this real copied ELF/native simulation execution.
use std::process::Command;

#[test]
fn native_bundle_feature_contract_rejects_before_opening_files() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("not-created");
    let result = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["pack-coop-native", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(!output.exists());
    let help = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["pack-coop-native", "--help"])
        .output()
        .unwrap();
    assert_eq!(
        help.status.success(),
        cfg!(all(
            feature = "coop-native-authoring",
            target_os = "linux",
            target_arch = "x86_64"
        ))
    );
}

#[cfg(all(
    feature = "coop-native-authoring",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[test]
#[ignore = "requires trusted ORR_NATIVE_BUNDLE_RUNTIME_BIN built from this exact source with coop-defense-run-save"]
fn relocated_native_bundle_runs_without_original_inputs_and_enforces_save_boundary() {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};
    fn inventory(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().into_string().unwrap(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }
    let supplied = std::env::var_os("ORR_NATIVE_BUNDLE_RUNTIME_BIN")
        .expect("ORR_NATIVE_BUNDLE_RUNTIME_BIN required; no missing-input skip");
    assert!(Path::new(&supplied).is_absolute());
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("inputs");
    fs::create_dir(&source).unwrap();
    let player = source.join("player");
    fs::copy(supplied, &player).unwrap();
    fs::set_permissions(&player, fs::Permissions::from_mode(0o755)).unwrap();
    let runtime_sha: String = Sha256::digest(fs::read(&player).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let creator = root.path().join("creator");
    fs::copy(env!("CARGO_BIN_EXE_orr_pkg"), &creator).unwrap();
    let scene = source.join("authored.yaml");
    let authored = Command::new(&creator)
        .env("PATH", "")
        .args(["create-coop-native-scene", "--output"])
        .arg(&scene)
        .args([
            "--starting-gold",
            "250",
            "--reactor-hp",
            "75",
            "--last-wave",
            "3",
            "--spawn-interval-ticks",
            "70",
            "--enemy-health-multiplier",
            "3",
            "--wave-pairs",
            "2,1,3,4,5,6,7,8,9,10",
        ])
        .output()
        .unwrap();
    assert!(
        authored.status.success(),
        "{}",
        String::from_utf8_lossy(&authored.stderr)
    );
    let expected = Command::new(&player)
        .env("PATH", "")
        .args(["--headless", "--ticks", "420", "--scene"])
        .arg(&scene)
        .output()
        .unwrap();
    assert!(expected.status.success());
    let output = root.path().join("original bundle");
    let packaged = Command::new(&creator)
        .env("PATH", "")
        .args(["pack-coop-native", "--runtime"])
        .arg(&player)
        .args(["--runtime-sha256", &runtime_sha, "--scene"])
        .arg(&scene)
        .arg("--output")
        .arg(&output)
        .arg("--trusted-runtime")
        .output()
        .unwrap();
    assert!(
        packaged.status.success(),
        "{}",
        String::from_utf8_lossy(&packaged.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&packaged.stdout).unwrap();
    assert_eq!(receipt["runtime"]["sha256"], runtime_sha);
    let before = inventory(&output);
    let moved = root.path().join("relocated 한글 ' $; delivery");
    fs::rename(output, &moved).unwrap();
    fs::rename(source, root.path().join("hidden-inputs")).unwrap();
    fs::rename(&creator, root.path().join("hidden-creator")).unwrap();
    let unrelated = root.path().join("unrelated cwd");
    fs::create_dir(&unrelated).unwrap();
    let result = Command::new(moved.join("run-game"))
        .env("PATH", "")
        .current_dir(&unrelated)
        .args(["--headless", "--ticks", "420"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, expected.stdout);
    assert_eq!(inventory(&moved), before);
    assert_eq!(fs::read_dir(&unrelated).unwrap().count(), 0);
    let saves = moved.join("saves");
    fs::create_dir(&saves).unwrap();
    fs::set_permissions(&saves, fs::Permissions::from_mode(0o700)).unwrap();
    let guard = Command::new(moved.join("coop_defense"))
        .env("PATH", "")
        .current_dir(&unrelated)
        .arg("--scene")
        .arg(moved.join("game.scene.yaml"))
        .arg("--run-directory")
        .arg(&saves)
        .output()
        .unwrap();
    assert!(!guard.status.success());
    assert!(
        String::from_utf8_lossy(&guard.stderr)
            .contains("save directory is inside a source or executable bundle root"),
        "{}",
        String::from_utf8_lossy(&guard.stderr)
    );
    assert_eq!(fs::read_dir(&saves).unwrap().count(), 0);
}
