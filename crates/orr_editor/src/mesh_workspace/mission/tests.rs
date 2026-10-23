use super::{configured_scene, prepare, scene_yaml, Settings};
use crate::game::EditorGame;
use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, mesh_from_view, mission};
use std::fs;

#[test]
fn generated_scene_admits_the_floor_agent_and_requested_mission() {
    let settings = Settings {
        width: 12,
        depth: 8,
        collectible_count: 3,
        distance_per_tick: "0.25".into(),
    };
    let yaml = scene_yaml(&settings).unwrap();
    let document = mesh_doc_from_yaml(&yaml).unwrap();
    let frame = FrameView::of(document.frame());
    let view = mesh_from_view(frame).unwrap();
    let mission = mission::from_view(frame).unwrap().unwrap();

    assert_eq!(view.pin.start[0].raw(), orr_fp::FP::SCALE);
    assert_eq!(view.pin.start[2].raw(), 4 * orr_fp::FP::SCALE);
    assert_eq!(view.pin.goal[0].raw(), 11 * orr_fp::FP::SCALE);
    assert_eq!(view.pin.goal[2], view.pin.start[2]);
    assert_eq!(view.pin.distance_per_tick, orr_fp::fp!(0.25));
    assert_eq!(mission.config.count, 3);
    assert_eq!(mission.config.points[0].position[0], orr_fp::fp!(3.5));
    assert_eq!(mission.config.points[1].position[0], orr_fp::fp!(6));
    assert_eq!(mission.config.points[2].position[0], orr_fp::fp!(8.5));
    assert_eq!(mission.config.exit.position, view.pin.goal);
    assert_eq!(mission.run, Default::default());
}

#[test]
fn invalid_dimensions_count_and_speed_are_rejected_before_output() {
    let cases = [
        Settings {
            width: 5,
            ..Settings::default()
        },
        Settings {
            depth: 65,
            ..Settings::default()
        },
        Settings {
            collectible_count: 0,
            ..Settings::default()
        },
        Settings {
            collectible_count: 9,
            ..Settings::default()
        },
        Settings {
            distance_per_tick: "0".into(),
            ..Settings::default()
        },
        Settings {
            distance_per_tick: "0.5001".into(),
            ..Settings::default()
        },
        Settings {
            distance_per_tick: "1e999999999999999999999999".into(),
            ..Settings::default()
        },
    ];
    for settings in cases {
        assert!(configured_scene(&settings).is_err());
        assert!(scene_yaml(&settings).is_err());
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let path = root.join("invalid.scene.yaml");
    assert!(prepare(
        &path,
        &Settings {
            width: 5,
            ..Settings::default()
        }
    )
    .is_err());
    assert!(!path.exists());
}

#[test]
fn wizard_preserves_existing_scene_and_orphan_sidecars() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let existing = root.join("existing.scene.yaml");
    let original_scene = b"existing scene bytes";
    fs::write(&existing, original_scene).unwrap();
    assert!(prepare(&existing, &Settings::default()).is_err());
    assert_eq!(fs::read(&existing).unwrap(), original_scene);

    let blocked = root.join("blocked.scene.yaml");
    let sidecar = blocked.with_file_name("blocked.scene.yaml.mesh-camera.json");
    let original_sidecar = b"orphan sidecar bytes";
    fs::write(&sidecar, original_sidecar).unwrap();
    assert!(prepare(&blocked, &Settings::default()).is_err());
    assert!(!blocked.exists());
    assert_eq!(fs::read(sidecar).unwrap(), original_sidecar);
}

#[test]
fn new_workspace_opens_and_plays_the_requested_collect_and_exit_route() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let path = root.join("playable.scene.yaml");
    let settings = Settings {
        width: 12,
        depth: 8,
        collectible_count: 3,
        distance_per_tick: "0.25".into(),
    };

    let mut editor = prepare(&path, &settings).unwrap();
    assert_eq!(editor.game(), EditorGame::MeshNavigation);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        scene_yaml(&settings).unwrap()
    );
    assert!(editor.start_play());
    editor.step(64);
    editor.sync();
    let frame = editor.snapshot().unwrap().predicted();
    let mission = mission::from_view(frame).unwrap().unwrap();
    assert_eq!(mission.run.collected, (1 << settings.collectible_count) - 1);
    assert_eq!(mission.run.won, 1);
}
