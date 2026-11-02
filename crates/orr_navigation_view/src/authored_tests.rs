use super::*;
use std::os::unix::fs::symlink;
const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
const BLANK: &str = include_str!("../../../scenes/navigation_blank.scene.yaml");
const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");
struct Fixture {
    temp: tempfile::TempDir,
    root: std::path::PathBuf,
    scene: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("navigation")).unwrap();
        let scene = root.join("point.scene.yaml");
        fs::write(&scene, YAML).unwrap();
        fs::write(root.join("navigation/point_demo.orrt"), TERRAIN).unwrap();
        Self { temp, root, scene }
    }
}
#[test]
fn saved_scene_matches_actual_simulation_each_tick_and_restart_after_sources_hidden() {
    let f = Fixture::new();
    let mut player = AuthoredPlayer::open(&f.scene).unwrap();
    let admission = LocalNavigationAdmission::for_scene(&f.scene).unwrap();
    let doc = navigation_yard3d_doc_from_yaml(YAML, admission).unwrap();
    let mut oracle =
        Simulation::<NavigationYard3D>::from_frame(doc.frame(), YARD_TICK_RATE, YARD_BUILD_ID)
            .unwrap();
    assert_eq!(player.tick(), 0);
    let initial = player.frame().to_bytes();
    let checksum = player.checksum();
    assert_eq!(player.view().unwrap().terrain.cook(), TERRAIN);
    assert!(player.view().unwrap().navigator.path().is_some());
    fs::rename(&f.root, f.temp.path().join("hidden")).unwrap();
    for _ in 0..48 {
        player.step().unwrap();
        oracle.step(&TickInputs::new(oracle.tick() + 1, YARD_PLAYERS));
        assert_eq!(player.tick(), oracle.tick());
        assert_eq!(player.checksum(), oracle.checksum());
        assert_eq!(player.frame().to_bytes(), oracle.frame().to_bytes());
        player.view().unwrap();
    }
    player.restart().unwrap();
    assert_eq!(player.tick(), 0);
    assert_eq!(player.checksum(), checksum);
    assert_eq!(player.frame().to_bytes(), initial);
    player.step().unwrap();
}
#[test]
fn moving_complete_authoring_root_preserves_initial_bytes() {
    let f = Fixture::new();
    let initial = AuthoredPlayer::open(&f.scene).unwrap().frame().to_bytes();
    let moved = f.temp.path().join("relocated");
    fs::rename(&f.root, &moved).unwrap();
    let player = AuthoredPlayer::open(&moved.join("point.scene.yaml")).unwrap();
    assert_eq!(player.frame().to_bytes(), initial);
}
#[test]
fn missing_blank_stale_pins_and_missing_terrain_are_refused() {
    let f = Fixture::new();
    assert!(AuthoredPlayer::open(&f.root.join("missing.yaml")).is_err());
    for text in ["", BLANK, "schema: orr.scene/1\nentities: {}\n"] {
        fs::write(&f.scene, text).unwrap();
        assert!(AuthoredPlayer::open(&f.scene).is_err());
    }
    for (from, to) in [
        ("terrain_revision: [205,", "terrain_revision: [204,"),
        ("graph_revision: [76,", "graph_revision: [77,"),
    ] {
        let stale = YAML.replacen(from, to, 1);
        assert_ne!(stale, YAML);
        fs::write(&f.scene, stale).unwrap();
        assert!(AuthoredPlayer::open(&f.scene).is_err());
    }
    fs::write(&f.scene, YAML).unwrap();
    fs::remove_file(f.root.join("navigation/point_demo.orrt")).unwrap();
    assert!(AuthoredPlayer::open(&f.scene).is_err());
    fs::write(
        f.root.join("navigation/point_demo.orrt"),
        b"malformed terrain",
    )
    .unwrap();
    assert!(AuthoredPlayer::open(&f.scene).is_err());
}
#[test]
fn source_symlinks_ancestors_and_bounded_scene_input_are_refused() {
    let f = Fixture::new();
    let alias = f.temp.path().join("alias");
    symlink(&f.root, &alias).unwrap();
    assert!(AuthoredPlayer::open(&alias.join("point.scene.yaml")).is_err());
    let scene_link = f.root.join("link.yaml");
    symlink(&f.scene, &scene_link).unwrap();
    assert!(AuthoredPlayer::open(&scene_link).is_err());
    let terrain = f.root.join("navigation/point_demo.orrt");
    let backup = f.temp.path().join("terrain.orrt");
    fs::rename(&terrain, &backup).unwrap();
    symlink(&backup, &terrain).unwrap();
    assert!(AuthoredPlayer::open(&f.scene).is_err());
    fs::remove_file(&terrain).unwrap();
    fs::rename(backup, terrain).unwrap();
    fs::write(&f.scene, [0xff]).unwrap();
    assert!(AuthoredPlayer::open(&f.scene).is_err());
    let file = File::create(&f.scene).unwrap();
    file.set_len(MAX_SCENE as u64 + 1).unwrap();
    assert!(AuthoredPlayer::open(&f.scene).is_err());
    assert!(AuthoredPlayer::open(Path::new(&"x".repeat(4097))).is_err());
}
#[test]
fn authoritative_failure_and_tick_overflow_refuse_step_until_restart() {
    let f = Fixture::new();
    let mut player = AuthoredPlayer::open(&f.scene).unwrap();
    let initial = player.checksum();
    let mut status = *player.frame().singleton::<NavigationStepStatus>();
    status.failed = 1;
    status.failed_tick = 7;
    player.simulation.frame_mut().set_singleton(status);
    assert!(player.step().unwrap_err().contains("failed at tick 7"));
    assert!(player.view().is_err());
    assert_eq!(player.tick(), 0);
    player.restart().unwrap();
    assert_eq!(player.checksum(), initial);
    player.simulation.frame_mut().set_tick(u64::MAX);
    assert!(player.step().is_err());
    assert_eq!(player.tick(), u64::MAX);
    player.restart().unwrap();
    player.step().unwrap();
    assert_eq!(player.tick(), 1);
}
