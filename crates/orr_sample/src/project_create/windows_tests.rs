//! Starter creation on Windows: real directories, no replacement, no stage left behind.
use super::*;

fn options(parent: &Path, leaf: &str, seed: &str) -> CreateOptions {
    CreateOptions {
        output: parent.join(leaf),
        template: TEMPLATE.into(),
        seed: seed.into(),
    }
}

fn stage_left_behind(parent: &Path) -> bool {
    fs::read_dir(parent)
        .unwrap()
        .any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".orr-create-"))
}

#[test]
fn an_arena_starter_is_published_once_and_opens_as_a_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let first = create(&options(temp.path(), "arena one", "win-seed")).unwrap();
    assert!(first.output.join("orr.project.json").is_file());
    assert!(first.output.join("arena.scene.yaml").is_file());
    assert!(first.output.join(".orr").join("packages").join("objects").is_dir());
    assert!(!first.entity_guids.is_empty());
    assert_eq!(
        PreparedRuntime::open(&first.output).unwrap().initial_frame().checksum(),
        first.initial_checksum
    );
    // The same seed makes the same project in another place.
    let second = create(&options(temp.path(), "arena two", "win-seed")).unwrap();
    assert_eq!(second.initial_checksum, first.initial_checksum);
    assert_eq!(
        fs::read(second.output.join("arena.scene.yaml")).unwrap(),
        fs::read(first.output.join("arena.scene.yaml")).unwrap()
    );
    // An existing destination is never replaced and no stage remains.
    let before = fs::read(first.output.join("orr.project.json")).unwrap();
    assert!(create(&options(temp.path(), "arena one", "other")).is_err());
    assert_eq!(fs::read(first.output.join("orr.project.json")).unwrap(), before);
    assert!(!stage_left_behind(temp.path()));
}

#[test]
fn bad_outputs_and_seeds_are_rejected_before_anything_is_written() {
    let temp = tempfile::tempdir().unwrap();
    assert!(create(&options(temp.path(), "x", "bad seed!")).is_err());
    let relative = CreateOptions { output: PathBuf::from("relative-dir"), template: TEMPLATE.into(), seed: "s".into() };
    assert!(create(&relative).is_err());
    let missing_parent = options(&temp.path().join("absent"), "x", "s");
    assert!(create(&missing_parent).is_err());
    let traversal = CreateOptions {
        output: temp.path().join("a").join("..").join("b"),
        template: TEMPLATE.into(),
        seed: "s".into(),
    };
    assert!(create(&traversal).is_err());
    let reserved = options(temp.path(), "CON", "s");
    assert!(create(&reserved).is_err());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[cfg(feature = "collect-dodge")]
#[test]
fn a_collect_starter_needs_an_explicit_game_id() {
    let temp = tempfile::tempdir().unwrap();
    let mut collect = options(temp.path(), "collect", "win-seed");
    collect.template = COLLECT_TEMPLATE.into();
    assert!(create_collect(&collect, "not-a-uuid").is_err());
    let report = create_collect(&collect, "123e4567-e89b-42d3-a456-426614174000").unwrap();
    let manifest = fs::read_to_string(report.output.join("orr.project.json")).unwrap();
    assert!(manifest.contains("123e4567-e89b-42d3-a456-426614174000"));
    assert!(report.output.join("level.scene.yaml").is_file());
    assert!(!stage_left_behind(temp.path()));
}
