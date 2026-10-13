mod common;
use common::*;
use orr_fp::FP;
use orr_terrain::Terrain;
use orr_terrain_demo::{
    course::{MAX_COURSE_BYTES, MAX_TERRAIN_BYTES},
    Course, Game, DEFAULT_COURSE, DEFAULT_TERRAIN,
};
use std::fs;

#[test]
fn builtin_canonical_snapshot_is_asymmetric_bounded_and_has_a_hole() {
    let c = Course::builtin().unwrap();
    assert_eq!(c.terrain().cook(), DEFAULT_TERRAIN);
    assert_eq!(
        Terrain::load(DEFAULT_TERRAIN).unwrap().revision(),
        c.terrain().revision()
    );
    assert_eq!((c.terrain().width(), c.terrain().depth()), (17, 17));
    assert_eq!(c.terrain().holes().iter().filter(|x| **x).count(), 4);
    assert!(c.terrain().sample(FP::ZERO, FP::from_int(1)).is_none());
    assert_ne!(
        c.terrain().sample(FP::from_int(4), FP::from_int(-4)),
        c.terrain().sample(FP::from_int(-4), FP::from_int(4))
    );
    assert!(c.finish()[1] > c.spawn()[1]);
    for a in [c.spawn(), c.finish()]
        .into_iter()
        .chain(c.beacons().iter().copied())
    {
        assert_eq!(c.terrain().sample(a[0], a[2]), Some(a[1]));
    }
}

#[test]
fn strict_json_rejects_duplicate_unknown_missing_and_wrong_types() {
    let original = std::str::from_utf8(DEFAULT_COURSE).unwrap();
    for json in [
        original.replacen("\"version\": 1", "\"version\": 1, \"version\": 1", 1),
        original.replacen("\"version\": 1", "\"version\": 1, \"extra\": true", 1),
        original.replace("\"version\": 1,", ""),
        original.replace("\"version\": 1", "\"version\": \"1\""),
        original.replace("\"radius\": \"0.5\"", "\"radius\": 0.5"),
        format!("{original} null"),
    ] {
        assert!(
            Course::from_bytes(json.as_bytes(), DEFAULT_TERRAIN).is_err(),
            "{json}"
        );
    }
}

#[test]
fn schema_limits_and_filenames_fail_closed() {
    let original: serde_json::Value = serde_json::from_slice(DEFAULT_COURSE).unwrap();
    let variants = [
        ("version", serde_json::json!(2)),
        ("title", serde_json::json!("")),
        ("title", serde_json::json!("x".repeat(49))),
        ("title", serde_json::json!("bad\nname")),
        ("terrain", serde_json::json!("../hill_course.orrt")),
        ("terrain", serde_json::json!("/hill_course.orrt")),
        ("terrain", serde_json::json!("other\\hill_course.orrt")),
        ("terrain", serde_json::json!(".hidden.orrt")),
        ("terrain", serde_json::json!("hill_course.png")),
        ("beacons", serde_json::json!([])),
        ("beacons", serde_json::json!(vec![["0", "0"]; 5])),
        ("radius", serde_json::json!("0.249")),
        ("radius", serde_json::json!("1.01")),
        ("radius", serde_json::json!("NaN")),
        ("radius", serde_json::json!("9".repeat(33))),
    ];
    for (field, value) in variants {
        let mut v = original.clone();
        v[field] = value;
        assert!(
            Course::from_bytes(&serde_json::to_vec(&v).unwrap(), DEFAULT_TERRAIN).is_err(),
            "{v}"
        );
    }
}

#[test]
fn malformed_terrain_and_byte_caps_are_rejected() {
    for bytes in [
        &[][..],
        &DEFAULT_TERRAIN[..100],
        &vec![0; MAX_TERRAIN_BYTES + 1][..],
    ] {
        assert!(Course::from_bytes(DEFAULT_COURSE, bytes).is_err());
    }
    let mut bad = DEFAULT_TERRAIN.to_vec();
    bad.push(0);
    assert!(Course::from_bytes(DEFAULT_COURSE, &bad).is_err());
    bad = DEFAULT_TERRAIN.to_vec();
    bad[0] ^= 1;
    assert!(Course::from_bytes(DEFAULT_COURSE, &bad).is_err());
    assert!(Course::from_bytes(&vec![b' '; MAX_COURSE_BYTES + 1], DEFAULT_TERRAIN).is_err());
}

#[test]
fn terrain_profile_limits_are_checked_before_view_creation() {
    let json = json(["0", "0"], [["0", "0"]; 4], ["0", "0"]);
    for t in [
        terrain(18, 2, [0, 0], FP::from_int(1), vec![false; 17]),
        terrain(2, 2, [0, 0], FP::from_raw(16383), vec![false]),
        terrain(2, 2, [0, 0], FP::from_raw(524289), vec![false]),
        terrain(2, 2, [128, 0], FP::from_int(1), vec![false]),
        Terrain::new(
            "high".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::from_int(1),
            vec![FP::from_int(129); 4],
            vec![false],
        )
        .unwrap(),
    ] {
        assert!(Course::from_bytes(&json, &t.cook()).is_err());
    }
}

#[test]
fn minimum_spacing_and_inclusive_coordinate_limits_are_admitted() {
    let t = terrain(2, 2, [-128, -128], FP::from_raw(16384), vec![false]);
    assert!(Course::from_bytes(
        &json(
            ["-128", "-128"],
            [["-127.75", "-127.75"]; 4],
            ["-128", "-128"]
        ),
        &t.cook()
    )
    .is_ok());
}

#[test]
fn unsupported_spawn_beacon_finish_and_all_holes_reject() {
    let mut v: serde_json::Value = serde_json::from_slice(DEFAULT_COURSE).unwrap();
    for field in ["spawn", "finish"] {
        v[field] = serde_json::json!(["0", "1"]);
        assert!(Course::from_bytes(&serde_json::to_vec(&v).unwrap(), DEFAULT_TERRAIN).is_err());
        v = serde_json::from_slice(DEFAULT_COURSE).unwrap();
        v[field] = serde_json::json!(["9000000000", "0"]);
        assert!(Course::from_bytes(&serde_json::to_vec(&v).unwrap(), DEFAULT_TERRAIN).is_err());
        v = serde_json::from_slice(DEFAULT_COURSE).unwrap();
    }
    v["beacons"][2] = serde_json::json!(["0", "1"]);
    assert!(Course::from_bytes(&serde_json::to_vec(&v).unwrap(), DEFAULT_TERRAIN).is_err());
    let t = terrain(2, 2, [0, 0], FP::from_int(1), vec![true]);
    assert!(Course::from_bytes(&json(["0", "0"], [["0", "0"]; 4], ["0", "0"]), &t.cook()).is_err());
}

#[test]
fn file_snapshot_survives_source_changes_and_restart_never_reopens() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let p = root.join("course.json");
    let t = root.join("hill_course.orrt");
    fs::write(&p, DEFAULT_COURSE).unwrap();
    fs::write(&t, DEFAULT_TERRAIN).unwrap();
    let c = Course::load(&p).unwrap();
    let revision = c.terrain().revision();
    let old = c.terrain();
    let changed = Terrain::new(
        old.asset_id().into(),
        old.width(),
        old.depth(),
        old.origin(),
        old.spacing(),
        old.heights()
            .iter()
            .map(|h| FP::from_raw(h.raw() + 65536))
            .collect(),
        old.holes().to_vec(),
    )
    .unwrap();
    fs::write(&t, changed.cook()).unwrap();
    let new = Course::load(&p).unwrap();
    assert_ne!(new.terrain().revision(), revision);
    assert_eq!(new.spawn()[1].raw(), c.spawn()[1].raw() + 65536);
    fs::remove_file(p).unwrap();
    fs::remove_file(t).unwrap();
    let mut g = Game::new(c);
    let before = g.state();
    g.tick(Default::default());
    g.restart();
    assert_eq!(g.state(), before);
    assert_eq!(g.course().terrain().revision(), revision);
}

#[test]
fn loader_failures_preserve_source_bytes_and_reject_oversize_missing_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let p = root.join("course.json");
    let t = root.join("hill_course.orrt");
    fs::write(&p, DEFAULT_COURSE).unwrap();
    assert!(Course::load(&p).is_err());
    assert!(Course::load(&root).is_err());
    fs::write(&t, vec![0; MAX_TERRAIN_BYTES + 1]).unwrap();
    assert!(Course::load(&p).is_err());
    assert_eq!(fs::read(&p).unwrap(), DEFAULT_COURSE);
    assert_eq!(
        fs::metadata(&t).unwrap().len(),
        (MAX_TERRAIN_BYTES + 1) as u64
    );
    fs::write(&p, vec![b' '; MAX_COURSE_BYTES + 1]).unwrap();
    assert!(Course::load(&p).is_err());
    assert!(Course::load(&root.join("../course.json")).is_err());
}

#[cfg(unix)]
#[test]
fn loader_rejects_symlink_files_ancestors_and_special_devices() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let p = root.join("course.json");
    let t = root.join("hill_course.orrt");
    fs::write(&p, DEFAULT_COURSE).unwrap();
    fs::write(&t, DEFAULT_TERRAIN).unwrap();
    symlink(&p, root.join("linked.json")).unwrap();
    assert!(Course::load(&root.join("linked.json")).is_err());
    symlink(&root, root.join("alias")).unwrap();
    assert!(Course::load(&root.join("alias/course.json")).is_err());
    fs::remove_file(&t).unwrap();
    symlink("/dev/null", &t).unwrap();
    assert!(Course::load(&p).is_err());
    assert!(Course::load(std::path::Path::new("/dev/null")).is_err());
    assert_eq!(fs::read(&p).unwrap(), DEFAULT_COURSE);
}

#[cfg(windows)]
#[test]
fn windows_canonical_and_ordinary_absolute_paths_load_the_same_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let ordinary = temp.path().join("course.json");
    let terrain = temp.path().join("hill_course.orrt");
    assert!(ordinary.is_absolute());
    fs::write(&ordinary, DEFAULT_COURSE).unwrap();
    fs::write(&terrain, DEFAULT_TERRAIN).unwrap();
    let canonical = ordinary.canonicalize().unwrap();
    // Windows canonicalize supplies the actual extended-length prefix; never
    // simulate it with string substitution in the production loader.
    assert!(canonical.to_string_lossy().starts_with(r"\\?\"));
    let first = Course::load(&ordinary).unwrap();
    let second = Course::load(&canonical).unwrap();
    assert_eq!(first.terrain_revision(), second.terrain_revision());
    assert_eq!(first.spawn(), second.spawn());
    assert_eq!(fs::read(&ordinary).unwrap(), DEFAULT_COURSE);
    assert_eq!(fs::read(&terrain).unwrap(), DEFAULT_TERRAIN);
}

#[cfg(windows)]
#[test]
fn windows_drive_relative_policy_rejects_before_filesystem_access() {
    for path in [r"C:course.json", r"C:", r"Z:course.json", r"Z:"] {
        let error = Course::load(std::path::Path::new(path)).unwrap_err();
        assert_eq!(error, "prefixed paths must be absolute");
    }
}
