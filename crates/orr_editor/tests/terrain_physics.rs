//! Optional admitted terrain through the production editor/ERP controls.
//! This is CPU/egui evidence, not a native-window or GPU acceptance claim.
#![cfg(feature = "terrain-physics")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode, Owner,
};
use orr_fp::{fp, FPVec3};
use orr_reflect::Value;
use std::path::Path;
fn fixture(root: &Path) -> std::path::PathBuf {
    std::fs::create_dir(root.join("terrain")).unwrap();
    std::fs::write(
        root.join("terrain/sphere_demo.orrt"),
        include_bytes!("../../../scenes/terrain/sphere_demo.orrt"),
    )
    .unwrap();
    let scene = root.join("terrain.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/terrain_sphere.scene.yaml"),
    )
    .unwrap();
    scene
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}
#[test]
fn actual_play_controls_use_pinned_frame_and_disable_terrain_play_edits() {
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path());
    let mut editor = Editor::open_game(&scene, EditorGame::TerrainYard3D).unwrap();
    editor.sync();
    assert!(editor.admitted_terrain().admitted);
    assert!(editor.admitted_terrain().model.is_some());
    assert_eq!(editor.yard_frame().items.len(), 2, "no hidden floor proxy");
    let revision = editor.admitted_terrain().revision;
    let initial = editor.checksum();
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.run_steps(3);
    click(&mut h, "Terrain collision");
    h.get_by_label("Admitted Frame-owned terrain · dynamic spheres only");
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    click(&mut h, LBL_PAUSE);
    std::fs::remove_file(root.path().join("terrain/sphere_demo.orrt")).unwrap();
    {
        let app = h.state_mut();
        assert!(app.terrain.viewport_model(&app.editor).is_some());
        assert!(app.terrain.apply_height(&app.editor).is_err());
        assert!(app.terrain.apply_hole(&app.editor).is_err());
        assert!(app.terrain.undo(&app.editor).is_err());
        assert!(app.terrain.redo(&app.editor).is_err());
        assert!(!app.editor.set_field(
            &Owner::Singleton("TerrainScenePin".into()),
            "friction",
            Value::Fixed(fp!(1))
        ));
    }
    click(&mut h, LBL_STEP);
    h.state_mut().editor.step(120);
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert_eq!(h.state().editor.admitted_terrain().revision, revision);
    assert!(h.state().editor.admitted_terrain().error.is_none());
    assert!(h
        .state()
        .terrain
        .viewport_model(&h.state().editor)
        .is_some());
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), initial);
    // Source no longer exists: the next edit is refused, preserving admission.
    assert!(!h
        .state_mut()
        .editor
        .spawn_yard_body(FPVec3::new(fp!(0), fp!(5), fp!(0))));
    assert_eq!(h.state().editor.checksum(), initial);
}

#[test]
fn terrain_game_spawns_only_spheres_and_cannot_open_without_admission() {
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path());
    let mut editor = Editor::open_game(&scene, EditorGame::TerrainYard3D).unwrap();
    editor.sync();
    assert!(editor.spawn_yard_body(FPVec3::new(fp!(0), fp!(5), fp!(0))));
    editor.sync();
    let selected = editor.selection().unwrap().clone();
    assert!(
        matches!(editor.field_of(&selected,"orr_physics3d::Collider","shape").unwrap(), Value::Variant(kind,_) if kind == "sphere")
    );
    let checksum = editor.checksum();
    assert!(!editor.set_field(
        &Owner::Component("orr_physics3d::Collider".into()),
        "shape",
        Value::Variant(
            "box".into(),
            vec![("half_extents".into(), Value::Vec3(FPVec3::splat(fp!(0.5))))]
        )
    ));
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    std::fs::write(root.path().join("terrain/sphere_demo.orrt"), b"bad").unwrap();
    assert!(Editor::open_game(&scene, EditorGame::TerrainYard3D).is_err());
}

#[test]
fn same_revision_source_change_refreshes_panel_label_and_reuses_mesh() {
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path());
    let original_source = "terrain/sphere_demo.orrt";
    let new_source = "terrain/sphere_copy.orrt";
    assert_eq!(new_source.len(), original_source.len());
    std::fs::copy(
        root.path().join(original_source),
        root.path().join(new_source),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::TerrainYard3D).unwrap();
    editor.sync();
    let revision = editor.admitted_terrain().revision;
    let model = editor.admitted_terrain().model.clone().unwrap();
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.run_steps(3);
    click(&mut h, "Terrain collision");
    h.get_by_label("Scene-relative source: terrain/sphere_demo.orrt");

    let mut path_bytes = [0u8; 256];
    path_bytes[..new_source.len()].copy_from_slice(new_source.as_bytes());
    let path_value = Value::Array(
        path_bytes
            .into_iter()
            .map(|byte| Value::Int(i128::from(byte)))
            .collect(),
    );
    assert!(h.state_mut().editor.set_field(
        &Owner::Singleton("TerrainScenePin".into()),
        "source_path",
        path_value,
    ));
    h.state_mut().editor.sync();
    h.run_steps(3);

    let admitted = h.state().editor.admitted_terrain();
    assert!(admitted.admitted);
    assert!(admitted.error.is_none());
    assert_eq!(admitted.source, new_source);
    assert_eq!(admitted.revision, revision);
    assert!(std::sync::Arc::ptr_eq(
        &model,
        admitted.model.as_ref().unwrap()
    ));
    h.get_by_label("Scene-relative source: terrain/sphere_copy.orrt");
}
