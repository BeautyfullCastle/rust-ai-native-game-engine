use super::*;
use serde_json::json;

fn options(extra: &[&str]) -> scene::Options {
    let args: Vec<OsString> = ["--output", "unused.scene.yaml"]
        .into_iter()
        .chain(extra.iter().copied())
        .map(Into::into)
        .collect();
    scene::options_for(&args, true).unwrap()
}
#[test]
fn native_and_web_authoring_share_exact_bytes_and_omission() {
    for extra in [
        vec![],
        vec!["--spawn-interval-ticks", "70"],
        vec![
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
            "2",
            "--wave-kinds",
            "3,2,1,0;0,1,2,3;1,2,3,0;2,3,0,1;3,0,1,2;0,0,0,0;1,1,1,1;2,2,2,2;3,3,3,3;0,1,2,3",
            "--opening-bosses",
            "1,0,0,0,1,0,0,0,0,1",
        ],
    ] {
        let native = scene::scene_text(&options(&extra)).unwrap();
        let web: Vec<OsString> = ["--wasm", "not-opened.wasm", "--output", "unused.scene.yaml"]
            .into_iter()
            .chain(extra.iter().copied())
            .map(Into::into)
            .collect();
        assert_eq!(
            native,
            scene::scene_text(&scene::options_for(&web, false).unwrap()).unwrap()
        );
        let admitted = admit(native.clone()).unwrap();
        let receipt = serde_json::to_value(admitted.receipt).unwrap();
        assert_eq!(receipt["kind"], "orr.coop-native-scene/1");
        assert_eq!(receipt["sceneSha256"], digest(&native));
        assert_eq!(receipt["sceneBytes"], native.len());
        assert!(receipt.get("wasmSha256").is_none());
        assert!(receipt.get("initialChecksum").is_none());
        assert_eq!(
            receipt["admitted"]["enemyHealthMultiplier"],
            if extra.len() > 2 { json!(2) } else { json!(1) }
        );
    }
    assert_eq!(scene::scene_text(&options(&[])).unwrap(), b"schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario:\n    starting_gold: 150\n    reactor_hp: 30\n  CoopDefense::Campaign:\n    last_wave: 10\n");
}
#[test]
fn native_receipt_requires_real_strict_admission() {
    for bytes in [
        vec![0xff], vec![b' '; 65537],
        b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 150, reactor_hp: 0}}\n".to_vec(),
        b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::EnemyMotion: {progress_per_tick: [4,7,4,4,2]}}\n".to_vec(),
    ] {
        assert!(admit(bytes).is_err());
    }
}
#[test]
fn native_cannot_accept_wasm_and_web_cannot_omit_it() {
    let native: Vec<OsString> = ["--output", "new.yaml"].map(Into::into).into();
    assert!(scene::options_for(&native, true).is_ok());
    assert!(scene::options_for(&native, false).is_err());
    let web: Vec<OsString> = ["--output", "new.yaml", "--wasm", "game.wasm"]
        .map(Into::into)
        .into();
    assert!(scene::options_for(&web, true).is_err());
    assert!(scene::options_for(&web, false).is_ok());
}
#[test]
fn native_publication_is_no_replace_and_refuses_symlink_ancestry() {
    let temp = tempfile::tempdir().unwrap();
    let mut opts = options(&[]);
    opts.output = temp.path().join("new.yaml");
    let (verified, warning) = create(&opts).unwrap();
    assert!(warning.is_none());
    assert_eq!(std::fs::read(&opts.output).unwrap(), verified.bytes());
    assert!(create(&opts).is_err());
    assert_eq!(std::fs::read(&opts.output).unwrap(), verified.bytes());
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(temp.path(), &link).unwrap();
    opts.output = link.join("foreign.yaml");
    assert!(create(&opts).is_err());
    assert!(!temp.path().join("foreign.yaml").exists());
    let dangling = temp.path().join("dangling");
    std::os::unix::fs::symlink(temp.path().join("absent"), &dangling).unwrap();
    opts.output = dangling;
    assert!(create(&opts).is_err());
    assert!(!temp.path().join("absent").exists());
}
