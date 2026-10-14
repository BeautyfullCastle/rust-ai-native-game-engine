//! Opt-in unchanged production session/panel acceptance. No native-window claim.
mod common;
use common::*;
use orr_editor::{
    game::EditorGame, terrain_document::TerrainSession, terrain_panel::TerrainPanel,
    terrain_pick::TerrainSelectionMode, Editor,
};
use orr_fp::FP;
use orr_package::Project;
use orr_terrain::{Edit, Terrain};
use orr_terrain_view::package::{self, install_and_reload};
use std::{fs, path::Path};
fn editor(root: &Path) -> Editor {
    let scene = root.join("yard.scene.yaml");
    fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    assert!(editor.yard_rows_coherent());
    editor
}
#[test]
fn imported_local_open_query_edit_undo_save_reopen_preserves_host_frame_history() {
    let root = temp();
    let raw_path = root.path().join("input.raw");
    fs::write(&raw_path, raw(&SAMPLES, orr_terrain_import::Endian::Little)).unwrap();
    orr_terrain_import::publish::import_file(
        &raw_path,
        &root.path().join("imported.orrt"),
        &recipe(),
    )
    .unwrap();
    let mut host = editor(root.path());
    let checksum = host.checksum();
    let history = host.history().clone();
    let frame = format!(
        "{:?}/{:?}",
        host.yard_frame().items,
        host.yard_frame().poses
    );
    let mut panel = TerrainPanel::default();
    panel.local_path = "imported.orrt".into();
    panel.open_for_editor(&host).unwrap();
    assert_eq!(panel.document.terrain(), Some(&terrain()));
    panel.query_xz = ["-1".into(), "-1".into()];
    panel.query_for_editor(&host).unwrap();
    assert_eq!(panel.document.surface().unwrap().height, FP::from_int(-1));
    panel.selection_mode = TerrainSelectionMode::Vertex;
    panel.selected_vertex = [0, 0];
    panel.height = "0.5".into();
    panel.apply_height(&host).unwrap();
    let edited = panel.document.bytes().unwrap().to_vec();
    assert!(panel.document.dirty());
    assert!(panel.open_for_editor(&host).is_err());
    assert_eq!(panel.document.bytes(), Some(edited.as_slice()));
    assert!(panel.undo(&host).unwrap());
    assert_eq!(panel.document.terrain(), Some(&terrain()));
    assert!(panel.redo(&host).unwrap());
    panel.save_for_editor(&host).unwrap();
    panel.close_for_editor(&host).unwrap();
    panel.open_for_editor(&host).unwrap();
    assert_eq!(panel.document.bytes(), Some(edited.as_slice()));
    host.sync();
    assert_eq!(host.checksum(), checksum);
    assert_eq!(host.history(), &history);
    assert_eq!(
        format!(
            "{:?}/{:?}",
            host.yard_frame().items,
            host.yard_frame().poses
        ),
        frame
    );
}
#[test]
fn installed_import_opens_read_only_and_copy_edit_does_not_change_package() {
    let root = temp();
    let project_root = root.path().join("project");
    install_and_reload(
        &terrain(),
        &project_root,
        &root.path().join("source"),
        "1.0.0",
    )
    .unwrap();
    let mut host = editor(root.path());
    let checksum = host.checksum();
    let history = host.history().clone();
    let mut panel = TerrainPanel::default();
    panel.project = project_root.to_str().unwrap().into();
    panel.package = package::PACKAGE_NAME.into();
    panel.asset = package::ASSET_PATH.into();
    panel.local_path = "copy.orrt".into();
    panel.open_package_for_editor(&host).unwrap();
    assert!(panel.document.read_only());
    panel.selection_mode = TerrainSelectionMode::Vertex;
    panel.selected_vertex = [0, 0];
    panel.height = "2".into();
    assert!(panel.apply_height(&host).is_err());
    panel.copy_to_scene(&host).unwrap();
    assert!(!panel.document.read_only());
    panel.apply_height(&host).unwrap();
    panel.save_for_editor(&host).unwrap();
    assert_ne!(
        fs::read(root.path().join("copy.orrt")).unwrap(),
        terrain().cook()
    );
    let project = Project::open(&project_root, package::runtime()).unwrap();
    project.verify().unwrap();
    assert_eq!(
        project
            .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
            .unwrap(),
        terrain().cook()
    );
    host.sync();
    assert_eq!(host.checksum(), checksum);
    assert_eq!(host.history(), &history);
}
#[test]
fn production_editor_rejects_missing_terrain_declaration_and_precision_transactionally() {
    let root = temp();
    let scene = root.path().join("yard.scene.yaml");
    fs::write(&scene, b"session-only scene marker").unwrap();
    fs::write(root.path().join("good.orrt"), terrain().cook()).unwrap();
    let mut session = TerrainSession::default();
    session.set_scene(Some(&scene)).unwrap();
    session.open_local("good.orrt").unwrap();
    let original = session.bytes().unwrap().to_vec();
    let project_root = root.path().join("project");
    fs::create_dir(&project_root).unwrap();
    let source_root = root.path().join("source");
    source(&source_root, "undeclared-terrain", &[], &["lab.orrt"]);
    let project = Project::open(&project_root, package::runtime()).unwrap();
    project.install(&[source_root]).unwrap();
    // Generic content reads are legal, but this actual terrain consumer refuses it.
    assert_eq!(
        project
            .read_asset("undeclared-terrain", "lab.orrt")
            .unwrap(),
        terrain().cook()
    );
    assert!(session
        .open_package(&project_root, "undeclared-terrain", "lab.orrt")
        .unwrap_err()
        .contains("terrain_v1"));
    assert_eq!(session.bytes(), Some(original.as_slice()));
    let witness = Terrain::new(
        "precision".into(),
        2,
        2,
        [FP::from_raw(-1); 2],
        FP::from_raw(16_777_217),
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    fs::write(root.path().join("precision.orrt"), witness.cook()).unwrap();
    assert!(session
        .open_local("precision.orrt")
        .unwrap_err()
        .contains("spacing"));
    assert_eq!(session.bytes(), Some(original.as_slice()));
    session
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    let dirty = session.bytes().unwrap().to_vec();
    assert!(session.open_local("good.orrt").is_err());
    assert_eq!(session.bytes(), Some(dirty.as_slice()));
}
