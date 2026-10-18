use super::*;

#[test]
fn default_mission_points_are_fixed_and_route_reachable_candidates() {
    let points = default_points();
    assert_eq!(points.len(), MAX_MISSION_POINTS);
    assert_eq!(points[0].join(","), "1.5,0,1,0.4");
    assert_eq!(points[1].join(","), "2,0,1,0.4");
    assert_eq!(points[7].join(","), "2,0,1,0.4");
    assert_eq!(
        parse_point(&points[0], "collectible").unwrap().position[0].raw(),
        98_304
    );
}

#[test]
fn point_parser_preserves_exact_fixed_point_and_limits() {
    let parsed = parse_point(
        &["-0.1".into(), "0.25".into(), "1.5".into(), "0.4".into()],
        "collectible",
    )
    .unwrap();
    assert_eq!(parsed.position[0], parse_fixed("-0.1", "x").unwrap());
    assert_eq!(parsed.position[1], parse_fixed("0.25", "y").unwrap());
    assert_eq!(
        format_point(parsed),
        ["-0.1", "0.25", "1.5", "0.4"].map(str::to_owned)
    );
    assert_eq!(
        parse_point(&format_point(parsed), "collectible").unwrap(),
        parsed
    );

    assert!(parse_point(
        &["256.0000153".into(), "0".into(), "0".into(), "0.5".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "0".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "4.0000153".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "not-a-number".into()],
        "collectible",
    )
    .is_err());
}

#[test]
fn disabled_mission_is_canonical_zero_and_count_is_bounded() {
    let mission = disabled_mission();
    assert_eq!(mission.enabled, 0);
    assert_eq!(mission.count, 0);
    assert!(mission
        .points
        .iter()
        .all(|point| *point == MissionPoint::default()));
    assert_eq!(mission.exit, MissionPoint::default());
    assert_eq!(mission.graph_revision, [0; 32]);

    assert_eq!(parse_count("1").unwrap(), 1);
    assert_eq!(parse_count("8").unwrap(), 8);
    for count in ["", "0", "9", "2.0", "-1", "123"] {
        assert!(parse_count(count).is_err());
    }
}
