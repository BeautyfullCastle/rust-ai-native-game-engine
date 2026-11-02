use super::*;
use serde_json::json;
fn args(extra: &[&str]) -> Vec<OsString> {
    ["--wasm", "game.wasm", "--output", "scene.yaml"]
        .into_iter()
        .chain(extra.iter().copied())
        .map(Into::into)
        .collect()
}
fn receipt(wasm: &[u8], scene: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"sceneBytes":scene.len(),"sceneSha256":digest(scene),"wasmSha256":digest(wasm),"initialChecksum":"0123456789abcdef"})).unwrap()
}
#[test]
fn exact_default_yaml_and_pacing_omission() {
    let o = options(&args(&[])).unwrap();
    let text = scene_text(&o).unwrap();
    assert_eq!(text,b"schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario:\n    starting_gold: 150\n    reactor_hp: 30\n  CoopDefense::Campaign:\n    last_wave: 10\n");
    for (gold, hp, wave, pacing) in [(0, 1, 1, 35), (10000, 1000, 10, 350)] {
        let mut o = options(&args(&[])).unwrap();
        o.gold = gold;
        o.hp = hp;
        o.wave = wave;
        o.pacing = Some(pacing);
        let t = String::from_utf8(scene_text(&o).unwrap()).unwrap();
        assert!(t.ends_with(&format!(
            "  CoopDefense::WavePacing:\n    spawn_interval_ticks: {pacing}\n"
        )));
    }
}
#[test]
fn strict_options_bounds_and_decimal_compatibility() {
    for extra in [
        vec!["--unknown", "x"],
        vec!["--wasm", "other"],
        vec!["--starting-gold", "-1"],
        vec!["--reactor-hp", "0"],
        vec!["--last-wave", "11"],
        vec!["--spawn-interval-ticks", "34"],
        vec!["--spawn-interval-ticks", "351"],
        vec!["--last-wave", "1.0"],
        vec!["--last-wave", "true"],
        vec!["--last-wave", "0x1"],
        vec!["--last-wave", "1__0"],
        vec!["--last-wave", "١"],
    ] {
        assert!(options(&args(&extra)).is_err(), "{extra:?}");
    }
    assert!(options(&[]).is_err());
    assert!(options(&args(&["--last-wave"])).is_err());
    let o = options(&args(&[
        "--starting-gold",
        " +0_0150 ",
        "--last-wave",
        "01",
    ]))
    .unwrap();
    assert_eq!(o.gold, 150);
    assert_eq!(o.wave, 1);
    assert!(options(&args(&["--starting-gold", &"9".repeat(4097)])).is_err());
}
#[test]
fn receipt_requires_exact_fields_types_hashes_and_checksum() {
    let wasm = b"fixture";
    let scene = scene_text(&options(&args(&[])).unwrap()).unwrap();
    let good = receipt(wasm, &scene);
    assert_eq!(admit(scene.clone(), wasm, &good).unwrap().bytes(), scene);
    let value: serde_json::Value = serde_json::from_slice(&good).unwrap();
    for key in [
        "version",
        "sceneBytes",
        "sceneSha256",
        "wasmSha256",
        "initialChecksum",
    ] {
        let mut v = value.clone();
        v.as_object_mut().unwrap().remove(key);
        assert!(admit(scene.clone(), wasm, &serde_json::to_vec(&v).unwrap()).is_err());
    }
    for (key, bad) in [
        ("version", json!(true)),
        ("version", json!(1.0)),
        ("version", json!(2)),
        ("sceneBytes", json!(0)),
        ("sceneBytes", json!(true)),
        ("sceneSha256", json!("0".repeat(64))),
        ("wasmSha256", json!("0".repeat(64))),
        ("initialChecksum", json!("0123456789ABCDEf")),
        ("initialChecksum", json!("123")),
        ("extra", json!(null)),
    ] {
        let mut v = value.clone();
        v[key] = bad;
        assert!(
            admit(scene.clone(), wasm, &serde_json::to_vec(&v).unwrap()).is_err(),
            "{key}"
        );
    }
    let text = String::from_utf8(good).unwrap();
    let duplicate = text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert!(admit(scene, wasm, duplicate.as_bytes()).is_err());
}
#[test]
fn failed_gate_and_changed_original_wasm_never_publish() {
    for change in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let wasm = temp.path().join("game.wasm");
        std::fs::write(&wasm, b"original").unwrap();
        let output = temp.path().join("scene.yaml");
        let mut o = options(&args(&[])).unwrap();
        o.wasm = wasm.clone();
        o.output = output.clone();
        let result = create_with(&o, |bytes, scene| {
            if change {
                std::fs::write(&wasm, b"changed").unwrap();
                Ok(receipt(bytes, scene))
            } else {
                Err("Node validation timed out (injected test gate)".into())
            }
        });
        assert!(result.is_err());
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
#[test]
fn existing_output_refused_before_any_gate() {
    let temp = tempfile::tempdir().unwrap();
    let mut o = options(&args(&[])).unwrap();
    o.output = temp.path().join("keep");
    std::fs::write(&o.output, b"existing").unwrap();
    assert!(create_with(&o, |_, _| panic!("must not execute gate")).is_err());
    assert_eq!(std::fs::read(&o.output).unwrap(), b"existing");
}
