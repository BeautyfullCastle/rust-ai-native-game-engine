//! Real relocated executable, not the internal CLI function. The default build
//! also exercises explicit feature-off rejection before any output is opened.
use std::process::Command;
#[test]
fn native_scene_command_feature_contract() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("creator");
    std::fs::copy(env!("CARGO_BIN_EXE_orr_pkg"), &binary).unwrap();
    let out = dir.path().join("scene.yaml");
    let result = Command::new(&binary)
        .env("PATH", "")
        .current_dir(dir.path())
        .args(["create-coop-native-scene", "--output"])
        .arg(&out)
        .output()
        .unwrap();
    if cfg!(all(feature = "coop-native-authoring", target_os = "linux")) {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(out.is_file());
        let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(receipt["kind"], "orr.coop-native-scene/1");
        assert_eq!(receipt["admitted"]["reactorHp"], 30);
    } else {
        assert_eq!(result.status.code(), Some(2));
        assert!(!out.exists());
    }
}
#[cfg(all(feature = "coop-native-authoring", target_os = "linux"))]
#[test]
fn native_cli_rejects_invalid_duplicate_and_web_flags_without_publication() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("scene.yaml");
    for flags in [
        vec!["--starting-gold", "10001"],
        vec!["--reactor-hp", "0"],
        vec!["--last-wave", "11"],
        vec!["--spawn-interval-ticks", "34"],
        vec!["--enemy-health-multiplier", "5"],
        vec!["--wave-pairs", "1,2"],
        vec!["--wave-kinds", "0,1,2,4"],
        vec!["--opening-bosses", "0,0,0,0,0,0,0,0,0,2"],
        vec!["--last-wave", "1", "--last-wave", "2"],
        vec!["--wasm", "anything.wasm"],
        vec!["--output", "other.yaml"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
            .env("PATH", "")
            .args(["create-coop-native-scene", "--output"])
            .arg(&out)
            .args(&flags)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{flags:?}");
        assert!(!out.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    let web = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .env("PATH", "")
        .args(["create-coop-scene", "--output"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(web.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&web.stderr).contains("--wasm required"));
    assert!(!out.exists());
}

#[cfg(all(feature = "coop-native-authoring", target_os = "linux"))]
#[test]
#[ignore = "requires ORR_NATIVE_SCENE_WASM: trusted exact-source WASM and Node on PATH"]
fn existing_web_command_executes_real_wasm_and_matches_native_yaml() {
    use sha2::{Digest, Sha256};
    let wasm = std::env::var_os("ORR_NATIVE_SCENE_WASM").expect("ORR_NATIVE_SCENE_WASM required");
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("native.yaml");
    let web = dir.path().join("web.yaml");
    let flags = [
        "--starting-gold",
        "250",
        "--reactor-hp",
        "75",
        "--last-wave",
        "3",
        "--spawn-interval-ticks",
        "70",
        "--wave-pairs",
        "2,1,3,4,5,6,7,8,9,10",
        "--enemy-health-multiplier",
        "3",
        "--wave-kinds",
        "3,2,1,0;0,1,2,3;1,2,3,0;2,3,0,1;3,0,1,2;0,0,0,0;1,1,1,1;2,2,2,2;3,3,3,3;0,1,2,3",
        "--opening-bosses",
        "1,0,0,0,1,0,0,0,0,1",
    ];
    let n = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .env("PATH", "")
        .args(["create-coop-native-scene", "--output"])
        .arg(&native)
        .args(flags)
        .output()
        .unwrap();
    assert!(n.status.success(), "{}", String::from_utf8_lossy(&n.stderr));
    let w = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["create-coop-scene", "--wasm"])
        .arg(&wasm)
        .arg("--output")
        .arg(&web)
        .args(flags)
        .output()
        .unwrap();
    assert!(w.status.success(), "{}", String::from_utf8_lossy(&w.stderr));
    assert_eq!(
        std::fs::read(&native).unwrap(),
        std::fs::read(&web).unwrap()
    );
    let receipt: serde_json::Value = serde_json::from_slice(&w.stdout).unwrap();
    assert_eq!(
        receipt["wasmSha256"],
        Sha256::digest(std::fs::read(&wasm).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(receipt["initialChecksum"].as_str().unwrap().len(), 16);
    let refused = dir.path().join("refused.yaml");
    let no_node = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .env("PATH", "")
        .args(["create-coop-scene", "--wasm"])
        .arg(&wasm)
        .arg("--output")
        .arg(&refused)
        .output()
        .unwrap();
    assert_eq!(no_node.status.code(), Some(1));
    assert!(!refused.exists());
    let bad_wasm = dir.path().join("bad.wasm");
    std::fs::write(&bad_wasm, b"not WASM").unwrap();
    let bad = Command::new(env!("CARGO_BIN_EXE_orr_pkg"))
        .args(["create-coop-scene", "--wasm"])
        .arg(&bad_wasm)
        .arg("--output")
        .arg(&refused)
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));
    assert!(!refused.exists());
}
