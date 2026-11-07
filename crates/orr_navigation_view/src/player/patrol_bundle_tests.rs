use super::*;
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(|value| OsString::from(*value)).collect()
}
#[test]
fn explicit_patrol_bundle_profile_advertises_exact_new_capabilities() {
    assert_eq!(
        capabilities(),
        "ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1"
    );
    assert_eq!(
        crate::bundle::compiled_profile().unwrap(),
        "NavigationYard3DPatrolV1"
    );
}
#[test]
fn patrol_bundle_dispatch_preserves_strict_duplicate_and_ambiguous_input_rejection() {
    let command = parse_command(args(&[
        "--bundle",
        "/missing-bundle",
        "--ticks",
        "200",
        "--restart-check",
    ]))
    .unwrap()
    .unwrap();
    assert!(matches!(
        command,
        Command::Bundle(Options {
            ticks: 200,
            restart_check: true,
            ..
        })
    ));
    for bad in [
        vec!["--bundle", "x", "--scene", "y", "--ticks", "1"],
        vec!["--bundle", "x", "--bundle", "y", "--ticks", "1"],
        vec![
            "--bundle",
            "x",
            "--ticks",
            "1",
            "--restart-check",
            "--restart-check",
        ],
        vec!["--bundle", "x", "--ticks", "100001"],
        vec!["--bundle", "x", "--ticks", "1", "--unknown"],
    ] {
        assert!(parse_command(args(&bad)).is_err(), "{bad:?}");
    }
}
#[test]
fn bundle_capture_is_refused_before_any_bundle_file_open() {
    let options = Options {
        scene: "/nonexistent-patrol-bundle".into(),
        ticks: 1,
        restart_check: false,
        capture: Some("unused.png".into()),
    };
    assert!(
        run_bundle(options)
            .unwrap_err()
            .contains("does not support capture")
    );
}
