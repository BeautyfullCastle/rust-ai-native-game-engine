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
fn explicit_wave_pairs_emit_exact_array_without_changing_omitted_bytes() {
    let default = scene_text(&options(&args(&[])).unwrap()).unwrap();
    for (input, expected) in [
        ("1,1,1,1,1,1,1,1,1,1", [1; 10]),
        ("28,28,28,28,28,28,28,28,28,28", [28; 10]),
        (
            " +01 ,0_2,3,4,5,6,7,8,9,28 ",
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 28],
        ),
        (
            "10,12,14,16,18,20,22,24,26,28",
            [10, 12, 14, 16, 18, 20, 22, 24, 26, 28],
        ),
    ] {
        let o = options(&args(&["--wave-pairs", input])).unwrap();
        assert_eq!(o.wave_pairs, Some(expected));
        let values = expected.map(|n| n.to_string()).join(", ");
        let mut bytes = default.clone();
        bytes.extend_from_slice(
            format!("  CoopDefense::WaveCounts:\n    pair_counts: [{values}]\n").as_bytes(),
        );
        assert_eq!(scene_text(&o).unwrap(), bytes);
    }
    let all = args(&[
        "--starting-gold",
        "250",
        "--reactor-hp",
        "75",
        "--last-wave",
        "3",
        "--spawn-interval-ticks",
        "70",
        "--wave-pairs",
        "1,2,3,4,5,6,7,8,9,28",
    ]);
    assert_eq!(all.len(), 14);
    let text = scene_text(&options(&all).unwrap()).unwrap();
    assert_eq!(text, b"schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario:\n    starting_gold: 250\n    reactor_hp: 75\n  CoopDefense::Campaign:\n    last_wave: 3\n  CoopDefense::WavePacing:\n    spawn_interval_ticks: 70\n  CoopDefense::WaveCounts:\n    pair_counts: [1, 2, 3, 4, 5, 6, 7, 8, 9, 28]\n");
}
#[test]
fn malformed_or_duplicate_wave_pairs_are_usage_errors() {
    for value in [
        "",
        "1",
        "1,2,3,4,5,6,7,8,9",
        "1,2,3,4,5,6,7,8,9,10,11",
        "1,2,3,4,5,6,7,8,9,",
        ",2,3,4,5,6,7,8,9,10",
        "1,2,3,4,,6,7,8,9,10",
        "1,2,3,4,5,6,7,8,9,0",
        "1,2,3,4,5,6,7,8,9,29",
        "-1,2,3,4,5,6,7,8,9,10",
        "1.0,2,3,4,5,6,7,8,9,10",
        "true,2,3,4,5,6,7,8,9,10",
        "0x1,2,3,4,5,6,7,8,9,10",
        "1__0,2,3,4,5,6,7,8,9,10",
        "١,2,3,4,5,6,7,8,9,10",
        "1,2,3,4,5,6,7,8,9,18446744073709551616",
        "[1,2,3,4,5,6,7,8,9,10]",
        "1 2 3 4 5 6 7 8 9 10",
        "1,2,3,4,5,6,7,8,9,10\nother: 1",
    ] {
        assert!(
            options(&args(&["--wave-pairs", value])).is_err(),
            "{value:?}"
        );
    }
    let valid = "1,2,3,4,5,6,7,8,9,10";
    assert!(options(&args(&["--wave-pairs"])).is_err());
    assert!(options(&args(&["--wave-pairs", valid, "--wave-pairs", valid])).is_err());
    assert!(options(&args(&[
        "--starting-gold",
        "250",
        "--reactor-hp",
        "75",
        "--last-wave",
        "3",
        "--spawn-interval-ticks",
        "70",
        "--wave-pairs",
        valid,
        "--extra",
        "1",
    ]))
    .is_err());
    let mut o = options(&args(&["--wave-pairs", valid])).unwrap();
    for count in [0, 29, u32::MAX] {
        o.wave_pairs = Some([count; 10]);
        assert!(scene_text(&o).is_err());
    }
}
#[test]
fn enemy_health_is_optional_bounded_and_uses_existing_integer_grammar() {
    let default = scene_text(&options(&args(&[])).unwrap()).unwrap();
    for (input, multiplier) in [("1", 1), ("2", 2), ("3", 3), ("4", 4), (" +0_4 ", 4)] {
        let o = options(&args(&["--enemy-health-multiplier", input])).unwrap();
        assert_eq!(o.enemy_health, Some(multiplier));
        let mut expected = default.clone();
        expected.extend_from_slice(
            format!("  CoopDefense::EnemyHealth:\n    multiplier: {multiplier}\n").as_bytes(),
        );
        assert_eq!(scene_text(&o).unwrap(), expected);
    }
    for input in [
        "",
        "0",
        "5",
        "-1",
        "1.0",
        "true",
        "0x1",
        "1__0",
        "١",
        "18446744073709551616",
        "1\nother: 2",
    ] {
        assert!(
            options(&args(&["--enemy-health-multiplier", input])).is_err(),
            "{input:?}"
        );
    }
    assert!(options(&args(&["--enemy-health-multiplier"])).is_err());
    assert!(options(&args(&[
        "--enemy-health-multiplier",
        "1",
        "--enemy-health-multiplier",
        "4"
    ]))
    .is_err());
    let mut o = options(&args(&[])).unwrap();
    for invalid in [0, 5, i64::MAX] {
        o.enemy_health = Some(invalid);
        assert!(scene_text(&o).is_err());
    }
}
#[test]
fn all_eight_options_emit_health_after_existing_fields() {
    let all = args(&[
        "--starting-gold",
        "250",
        "--reactor-hp",
        "75",
        "--last-wave",
        "3",
        "--spawn-interval-ticks",
        "70",
        "--wave-pairs",
        "1,2,3,4,5,6,7,8,9,28",
        "--enemy-health-multiplier",
        "4",
    ]);
    assert_eq!(all.len(), 16);
    let mut expected = scene_text(&options(&all[..14]).unwrap()).unwrap();
    expected.extend_from_slice(b"  CoopDefense::EnemyHealth:\n    multiplier: 4\n");
    assert_eq!(scene_text(&options(&all).unwrap()).unwrap(), expected);
    let mut excess = all;
    excess.extend([OsString::from("--unknown"), OsString::from("1")]);
    assert!(options(&excess).is_err());
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
