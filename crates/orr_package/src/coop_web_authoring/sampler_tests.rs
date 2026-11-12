use super::*;
fn prepared() -> (tempfile::TempDir, Prepared) {
    let temp = tempfile::tempdir().unwrap();
    let mut png = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[1, 2, 3, 255])
            .unwrap();
    }
    std::fs::write(temp.path().join("atlas.png"), png).unwrap();
    let bytes =
        br#"{"unit:0":{"image":"atlas.png","clips":{"idle":{"frames":[[0,0,1,1]]}}}}"#.to_vec();
    let p = crate::coop_web_authoring::prepared::prepare(bytes, temp.path()).unwrap();
    (temp, p)
}
#[test]
fn exact_receipt_gate_rejects_extra_duplicate_missing_wrong_hash() {
    for bad in [
        br#"{}"#.as_slice(),
        br#"{"version":1,"version":1,"manifestBytes":1,"manifestSha256":"bad"}"#,
        br#"{"version":1,"manifestBytes":1,"manifestSha256":"bad","extra":0}"#,
        br#"{"version":1,"manifestBytes":1,"manifestSha256":"bad"}"#,
    ] {
        let (_, p) = prepared();
        assert!(admit(p, bad).is_err());
    }
    let (_, p) = prepared();
    let receipt = serde_json::to_vec(&Receipt {
        version: 1,
        manifest_bytes: p.bytes().len(),
        manifest_sha256: digest(p.bytes()),
    })
    .unwrap();
    assert!(admit(p, &receipt).is_ok());
}
#[test]
fn image_recheck_detects_changed_snapshot_and_embedded_sources_are_exact() {
    let (temp, p) = prepared();
    std::fs::write(temp.path().join("atlas.png"), b"changed").unwrap();
    assert!(p.recheck().is_err());
    // Downloaded test binaries can be replayed against the independently
    // verified source tree without recreating the build machine's directories.
    // Missing or changed files still fail the same byte-for-byte assertions.
    let source_root = std::env::var_os("ORR_PACKAGE_TEST_SOURCE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    assert_eq!(
        VERIFIER,
        std::fs::read(source_root.join("tools/validate_coop_defense_animation.mjs")).unwrap()
    );
    assert_eq!(
        SAMPLER,
        std::fs::read(source_root.join("crates/orr_coop_defense_web/web/combat-presentation.mjs"))
            .unwrap()
    );
}
#[test]
fn bounded_runner_missing_failure_timeout_and_pipe_caps() {
    let t = tempfile::tempdir().unwrap();
    assert!(execute(
        Command::new(t.path().join("missing-node")),
        Duration::from_millis(20)
    )
    .is_err());
    let mut bad = Command::new("node");
    bad.args(["-e", "process.exit(3)"]);
    assert!(execute(bad, Duration::from_secs(2)).is_err());
    let mut slow = Command::new("node");
    slow.args(["-e", "setInterval(()=>{},1000)"]);
    assert!(execute(slow, Duration::from_millis(50))
        .unwrap_err()
        .contains("timed out"));
    for stream in ["stdout", "stderr"] {
        let mut flood = Command::new("node");
        flood.args(["-e", &format!("process.{stream}.write('x'.repeat(100000))")]);
        assert!(execute(flood, Duration::from_secs(2)).is_err());
    }
}
