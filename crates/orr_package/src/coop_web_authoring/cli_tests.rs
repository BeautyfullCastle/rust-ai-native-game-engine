use super::*;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn create_args(extra: &[&str]) -> Vec<OsString> {
    let mut values = args(&[
        "--assets-dir",
        "assets",
        "--output",
        "new.json",
        "--role",
        "unit:0",
        "--image",
        "a.png",
        "--frame",
        "attack",
        "0",
        "0",
        "1",
        "2",
    ]);
    values.extend(args(extra));
    values
}

fn authored(values: &[OsString]) -> Result<Vec<u8>, String> {
    let o = options(values, false)?;
    manifest::create(
        &o.role,
        &o.image,
        &o.frames,
        &o.clips,
        o.pivot,
        o.attack_timing,
    )
}

#[test]
fn attack_timing_is_optional_and_does_not_require_clip_settings() {
    let defaults = options(&create_args(&[]), false).unwrap();
    assert!(defaults.attack_timing.is_none());
    let bytes = authored(&create_args(&[])).unwrap();
    let v = manifest::parse(&bytes).unwrap();
    let attack = &v["unit:0"]["clips"]["attack"];
    assert_eq!(attack.as_object().unwrap().len(), 1);

    let values = create_args(&["--attack-timing", "2000", "1.25", "1"]);
    let o = options(&values, false).unwrap();
    assert_eq!(o.attack_timing, Some([2000.0, 1.25, 1.0]));
    assert!(o.clips.is_empty());
    let bytes = authored(&values).unwrap();
    let v = manifest::parse(&bytes).unwrap();
    let attack = &v["unit:0"]["clips"]["attack"];
    assert_eq!(attack["windupMs"].as_f64(), Some(2000.0));
    assert_eq!(attack["releaseMs"].as_f64(), Some(1.25));
    assert_eq!(attack["recoveryMs"].as_f64(), Some(1.0));
    assert!(attack.get("fps").is_none());
    assert!(attack.get("loop").is_none());
}

#[test]
fn attack_timing_supports_options_and_any_flag_order() {
    let mut values = args(&["--attack-timing", "1", "2000", "2.5"]);
    values.extend(create_args(&[
        "--clip", "attack", "12.5", "once", "--pivot", "0.5", "2",
    ]));
    let bytes = authored(&values).unwrap();
    let v = manifest::parse(&bytes).unwrap();
    let attack = &v["unit:0"]["clips"]["attack"];
    assert_eq!(attack["fps"].as_f64(), Some(12.5));
    assert_eq!(attack["loop"].as_bool(), Some(false));
    assert_eq!(attack["windupMs"].as_f64(), Some(1.0));
    assert_eq!(attack["releaseMs"].as_f64(), Some(2000.0));
    assert_eq!(attack["recoveryMs"].as_f64(), Some(2.5));
    assert_eq!(v["unit:0"]["pivot"], serde_json::json!([0.5, 2.0]));
}

#[test]
fn attack_timing_rejects_duplicate_missing_extra_and_malformed_arguments() {
    for extra in [
        vec!["--attack-timing"],
        vec!["--attack-timing", "1"],
        vec!["--attack-timing", "1", "2"],
        vec!["--attack-timing", "1", "2", "3", "4"],
        vec![
            "--attack-timing",
            "1",
            "2",
            "3",
            "--attack-timing",
            "4",
            "5",
            "6",
        ],
    ] {
        assert!(options(&create_args(&extra), false).is_err(), "{extra:?}");
    }
    for index in 0..3 {
        for bad in [
            "true", "oops", "", "1ms", "NaN", "inf", "-inf", "1e999", "0", "-1", "0.999",
            "2000.001",
        ] {
            let mut values = ["--attack-timing", "1", "2", "3"];
            values[index + 1] = bad;
            assert!(
                authored(&create_args(&values)).is_err(),
                "duration {index}: {bad}"
            );
        }
    }
}

#[test]
fn attack_timing_requires_attack_frames_and_is_not_a_compose_flag() {
    let mut values = create_args(&["--attack-timing", "1", "2", "3"]);
    let attack = values.iter_mut().find(|v| **v == "attack").unwrap();
    *attack = "idle".into();
    assert!(authored(&values).is_err());
    let values = args(&[
        "--assets-dir",
        "assets",
        "--output",
        "new.json",
        "--manifest",
        "old.json",
        "--attack-timing",
        "1",
        "2",
        "3",
    ]);
    assert!(options(&values, true).is_err());
}
