use super::{parse_fixed_bounded, parse_index, Panel};
use crate::{editor::Editor, game::EditorGame, mesh_navigation};
use orr_fp::{fp, FP};
use std::{fs, path::PathBuf};

#[test]
fn geometry_fields_use_exact_bounded_fixed_point_and_unsigned_indices() {
    assert_eq!(parse_fixed_bounded("0.125").unwrap(), fp!(0.125));
    assert!(parse_fixed_bounded(&"1".repeat(49)).is_err());
    assert_eq!(parse_index("128").unwrap(), 128);
    assert!(parse_index("-1").is_err());
    assert!(parse_index("4294967296").is_err());
}

#[test]
fn acknowledge_clears_draft_block_and_reset_restores_defaults() {
    let mut panel = Panel {
        dirty: true,
        ..Default::default()
    };
    panel.acknowledge();
    assert!(!panel.dirty());
    panel.quad_rise = "invalid".into();
    panel.dirty = true;
    panel.reset();
    assert!(!panel.dirty());
    assert_eq!(panel.quad_rise, "0");
}

struct TestScene {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl TestScene {
    fn create() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("geometry.scene.yaml");
        fs::write(
            &scene,
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
        )
        .unwrap();
        Self {
            _directory: directory,
            path: scene,
        }
    }
}

fn placed_editor(scene: &TestScene) -> (Editor, mesh_navigation::Panel) {
    let mut editor = Editor::open_game(&scene.path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut navigation = mesh_navigation::Panel::default();
    navigation.sync(&editor);
    navigation.create_ramp_and_bridge().unwrap();
    navigation.place_agent(&mut editor).unwrap();
    navigation.sync(&editor);
    (editor, navigation)
}

fn appended_quad() -> super::Action {
    super::Action::AppendQuad(crate::mesh_navigation::geometry::Quad {
        origin: [fp!(10), FP::ZERO, FP::ZERO],
        width: fp!(2),
        depth: fp!(2),
        rise: FP::ZERO,
    })
}

#[cfg(feature = "mesh-mission")]
#[test]
fn graph_bound_mission_blocks_geometry_without_mutating_scene_or_history() {
    use orr_remote::mesh_navigation::{mesh_from_view, mission};

    let scene = TestScene::create();
    let (mut editor, mut navigation) = placed_editor(&scene);
    let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
    let mut config = mission::MeshMission {
        enabled: 1,
        count: 1,
        graph_revision: view.graph.revision(),
        ..mission::MeshMission::default()
    };
    config.points[0] = mission::MissionPoint {
        position: view.agent.spec.start,
        radius: fp!(0.5),
    };
    config.exit = mission::MissionPoint {
        position: view.agent.spec.goal,
        radius: fp!(0.5),
    };
    editor
        .host_call("world.singleton.patch", mission::patch_request(&config, &view).unwrap())
        .unwrap();
    editor.sync();
    navigation.sync(&editor);
    assert!(mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .is_some());
    let checksum = editor.checksum();
    let history_len = editor.history().entries.len();
    let revision = navigation.graph.as_ref().unwrap().revision();
    let error = navigation
        .stage_geometry(appended_quad(), &editor)
        .unwrap_err();
    assert!(error.contains("Remove the mesh mission"));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history_len);
    assert_eq!(navigation.graph.as_ref().unwrap().revision(), revision);
    assert!(!navigation.geometry_staged);
    for replacement in [
        navigation.create_ramp_and_bridge(),
        navigation.open_mesh_asset(),
        navigation.open_model(),
    ] {
        assert!(replacement.unwrap_err().contains("Remove the mesh mission"));
        assert_eq!(editor.checksum(), checksum);
        assert_eq!(editor.history().entries.len(), history_len);
        assert_eq!(navigation.graph.as_ref().unwrap().revision(), revision);
    }
    let super::Action::AppendQuad(quad) = appended_quad() else {
        panic!("fixture must append a quad");
    };
    navigation.graph = Some(
        crate::mesh_navigation::geometry::append_quad(
            navigation.graph.as_ref(),
            navigation.geometry_profile().unwrap(),
            quad,
        )
        .unwrap(),
    );
    navigation.draft_dirty = true;
    navigation.mission_bound = false;
    assert!(!navigation.geometry_staged);
    assert!(navigation
        .place_agent(&mut editor)
        .unwrap_err()
        .contains("Remove the mesh mission"));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history_len);
    editor.undo();
    editor.sync();
    navigation.sync(&editor);
    assert!(!navigation.mission_bound);
    assert!(navigation.create_ramp_and_bridge().is_ok());
}

#[test]
fn presentation_sidecars_block_geometry_staging_without_mutating_the_host() {
    let scene = TestScene::create();
    let (editor, mut navigation) = placed_editor(&scene);
    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();
    let original_revision = navigation.graph.as_ref().unwrap().revision();

    for suffix in [
        ".mesh-art.json",
        ".mesh-props.json",
        ".mesh-camera.json",
        ".mesh-animation.json",
        ".mesh-lighting.json",
        ".mesh-hud.json",
    ] {
        let sidecar = PathBuf::from(format!("{}{}", scene.path.display(), suffix));
        fs::write(&sidecar, b"opaque sidecar").unwrap();
        let error = navigation
            .stage_geometry(appended_quad(), &editor)
            .unwrap_err();
        assert!(error.contains(
            "Remove and save existing scene-bound art, props, camera, lighting and animation"
        ));
        fs::remove_file(sidecar).unwrap();
        assert_eq!(
            navigation.graph.as_ref().unwrap().revision(),
            original_revision
        );
        assert_eq!(editor.checksum(), original_checksum);
        assert_eq!(editor.history().entries.len(), original_history_len);
        assert!(!navigation.geometry_staged);
    }
}

#[test]
fn new_sidecars_or_pending_presentation_edits_block_place_after_geometry_is_staged() {
    let scene = TestScene::create();
    let (mut editor, mut navigation) = placed_editor(&scene);
    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();

    navigation.stage_geometry(appended_quad(), &editor).unwrap();
    let staged_revision = navigation.graph.as_ref().unwrap().revision();
    for suffix in [
        ".mesh-art.json",
        ".mesh-props.json",
        ".mesh-camera.json",
        ".mesh-animation.json",
        ".mesh-lighting.json",
        ".mesh-hud.json",
    ] {
        let sidecar = PathBuf::from(format!("{}{}", scene.path.display(), suffix));
        fs::write(&sidecar, b"new binding after staging").unwrap();
        let error = navigation.place_agent(&mut editor).unwrap_err();
        assert!(error.contains("Remove and save existing scene-bound"));
        assert_eq!(fs::read(&sidecar).unwrap(), b"new binding after staging");
        fs::remove_file(sidecar).unwrap();
        assert_eq!(
            navigation.graph.as_ref().unwrap().revision(),
            staged_revision
        );
        assert_eq!(editor.checksum(), original_checksum);
        assert_eq!(editor.history().entries.len(), original_history_len);
    }
    navigation.set_geometry_presentation_pending(true);

    let error = navigation.place_agent(&mut editor).unwrap_err();
    assert!(error.contains("Save or discard camera, lighting and animation edits before geometry"));
    assert_eq!(
        navigation.graph.as_ref().unwrap().revision(),
        staged_revision
    );
    assert!(navigation.geometry_staged);
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(editor.history().entries.len(), original_history_len);
}

#[test]
fn externally_placed_agent_invalidates_staged_geometry_without_overwriting_host() {
    let scene = TestScene::create();
    let (mut editor, mut first) = placed_editor(&scene);
    first.stage_geometry(appended_quad(), &editor).unwrap();
    let staged_revision = first.graph.as_ref().unwrap().revision();

    let mut second = mesh_navigation::Panel::default();
    second.sync(&editor);
    second.create_ramp_and_bridge().unwrap();
    second.distance_per_tick = "0.0625".into();
    second.place_agent(&mut editor).unwrap();
    let external_checksum = editor.checksum();
    let external_history_len = editor.history().entries.len();
    assert_ne!(external_checksum, first.geometry_host_context.unwrap().1);

    let error = first.place_agent(&mut editor).unwrap_err();
    assert!(error.contains("Host mesh changed while geometry was staged"));
    assert_eq!(first.graph.as_ref().unwrap().revision(), staged_revision);
    assert!(first.geometry_staged);
    assert_eq!(editor.checksum(), external_checksum);
    assert_eq!(editor.history().entries.len(), external_history_len);
}
