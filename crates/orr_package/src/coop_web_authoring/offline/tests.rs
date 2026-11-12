use super::*;
#[test]
fn explicit_output_and_closed_options_required() {
    let base = ["--build-dir", "dist", "--output", "new.html"];
    let args = |extra: &[&str]| {
        base.into_iter()
            .chain(extra.iter().copied())
            .map(OsString::from)
            .collect::<Vec<_>>()
    };
    assert!(options(&args(&[])).is_ok());
    assert!(options(&[]).is_err());
    for extra in [
        &["--node", "other"][..],
        &["--output", "old"],
        &["--scene"],
        &["--overwrite", "true"],
    ] {
        assert!(options(&args(extra)).is_err());
    }
}
#[test]
fn bounded_snapshots_recheck_and_budget_count() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input");
    std::fs::write(&path, b"1234").unwrap();
    let mut total = 0;
    let mut inputs = ExtraInputs::default();
    assert_eq!(inputs.get(&path, 4, &mut total).unwrap(), b"1234");
    assert_eq!(total, 4);
    assert_eq!(inputs.get(&path, 4, &mut total).unwrap(), b"1234");
    assert_eq!(total, 4);
    assert!(inputs.get(&path, 3, &mut total).is_err());
    std::fs::write(&path, b"changed").unwrap();
    assert!(inputs.recheck().is_err());
    total = MAX_INPUT - 1;
    assert!(take(&path, 20, &mut total).is_err());
    assert_eq!(total, MAX_INPUT - 1);
}
#[test]
fn fixed_build_snapshot_opaque_bytes_and_changed_source_refused() {
    let temp = tempfile::tempdir().unwrap();
    for name in bundle::ASSETS {
        let p = temp.path().join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"opaque").unwrap();
    }
    let mut total = 0;
    let build = BuildSnapshot::open(temp.path(), &mut total).unwrap();
    assert_eq!(total, 12 * 6);
    assert_eq!(build.file("game.wasm"), b"opaque");
    assert!(build.recheck().is_ok());
    std::fs::write(temp.path().join("game.wasm"), b"changed").unwrap();
    assert!(build.recheck().is_err());
}
#[test]
fn embedded_json_and_binary_preserve_raw_bytes_without_script_breakout() {
    let raw = r#" {"x":1,"x":2,"big":9007199254740993,"text":"</script>한글"} "#;
    let mut html = String::new();
    json_script(&mut html, "metadata", raw).unwrap();
    assert!(html.contains("\\u003c/script>한글"));
    assert!(html.contains("9007199254740993"));
    assert!(html.contains("\"x\":1,\"x\":2"));
    let mut binary_html = String::new();
    binary(&mut binary_html, "fixed", b"\0\xff").unwrap();
    assert!(binary_html.contains("AP8="));
}
