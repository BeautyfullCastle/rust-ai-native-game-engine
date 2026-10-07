//! Actual EditorApp widgets on the opt-in point-navigation host.
#![cfg(feature = "navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    terrain_document::NewTerrain,
    Editor, EditorApp, Mode,
};
use orr_fp::FP;
use std::path::Path;

fn setup(root: &Path) -> Harness<'static, EditorApp> {
    let scene = root.join("navigation.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None))
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}
fn wait_for_playing_frame(h: &mut Harness<'_, EditorApp>) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < until {
        h.state_mut().editor.sync();
        h.run_steps(2);
        if h.state().editor.timeline().is_some_and(|t| t.playing) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the accepted host Play command did not publish a playing Frame within 3 seconds");
}
fn type_into(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
}

#[test]
fn author_build_save_reopen_play_pause_step_seek_stop_and_rebuild_stale() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("navigation.scene.yaml");
    let mut h = setup(&root_path);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, LBL_PLAY);
    assert_eq!(
        h.state().editor.mode(),
        Mode::Edit,
        "blank route cannot play"
    );
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    assert!(root_path.join("terrain.orrt").is_file());
    type_into(&mut h, "Navigation start X", "-3");
    type_into(&mut h, "Navigation goal X", "3");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    assert!(h.state().editor.admitted_navigation().admitted);
    assert_eq!(
        h.state().editor.history().entries.len(),
        1,
        "one atomic pin edit"
    );
    type_into(&mut h, "Navigation goal X", "2");
    type_into(&mut h, "Navigation distance per tick", "0.5");
    click(&mut h, LBL_PLAY);
    assert_eq!(
        h.state().editor.mode(),
        Mode::Edit,
        "unapplied route draft cannot play"
    );
    click(&mut h, "Build route");
    assert_eq!(h.state().editor.history().entries.len(), 2);
    assert_eq!(h.state().navigation.goal[0], "2");
    assert!(h.state_mut().editor.undo());
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert_eq!(
        h.state().navigation.goal[0],
        "3",
        "undo refreshes authored fields"
    );
    assert!(h.state_mut().editor.redo());
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert_eq!(
        h.state().navigation.goal[0],
        "2",
        "redo refreshes authored fields"
    );
    let other = root_path.join("other.scene.yaml");
    std::fs::write(
        &other,
        include_str!("../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    assert!(
        !h.state_mut().editor.open_path(&other),
        "host scene directory remains fixed"
    );
    assert_eq!(h.state().editor.path().as_deref(), Some(scene.as_path()));
    assert!(h.state_mut().editor.save());
    let start_checksum = h.state().editor.checksum();
    drop(h);

    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    assert!(editor.admitted_navigation().admitted);
    assert_eq!(editor.checksum(), start_checksum);
    let mut h = Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.run_steps(4);
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(
        h.state().editor.sim().playing,
        "Play widget must be accepted by the host"
    );
    wait_for_playing_frame(&mut h);
    assert!(
        h.state().editor.timeline().unwrap().playing,
        "Play widget must run the host clock"
    );
    click(&mut h, LBL_PAUSE);
    assert!(
        !h.state().editor.timeline().unwrap().playing,
        "Pause widget must stop the host clock"
    );
    let paused_tick = h.state().editor.timeline().unwrap().tick;
    click(&mut h, LBL_STEP);
    let widget_tick = h.state().editor.timeline().unwrap().tick;
    assert_eq!(
        widget_tick,
        paused_tick + 1,
        "Step widget advances exactly once"
    );
    h.state_mut().editor.step(10);
    h.state_mut().editor.sync();
    let tick = h.state().editor.timeline().unwrap().tick;
    assert_eq!(
        tick,
        widget_tick + 10,
        "explicit host Step advances ten ticks"
    );
    assert_eq!(
        h.state().editor.snapshot().unwrap().tick(),
        tick,
        "visible Frame matches the accepted host tick"
    );
    h.state_mut().editor.seek(0);
    h.state_mut().editor.sync();
    assert_eq!(h.state().editor.snapshot().unwrap().tick(), 0);
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);

    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, "Open terrain");
    click(&mut h, "Vertex");
    type_into(&mut h, "Terrain height", "0.5");
    click(&mut h, "Apply terrain height");
    assert!(h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    click(&mut h, "Terrain Undo");
    assert!(!h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    click(&mut h, "Terrain Redo");
    assert!(h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    click(&mut h, "Save terrain");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    assert!(!h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    assert!(h.state().editor.admitted_navigation().admitted);
}

#[test]
fn invalid_numeric_and_unreachable_route_do_not_change_host_document() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let mut h = setup(&root_path);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    let checksum = h.state().editor.checksum();
    type_into(&mut h, "Navigation maximum slope", "NaN");
    click(&mut h, "Build route");
    assert!(h.state().navigation.error.is_some());
    assert_eq!(h.state().editor.checksum(), checksum);
    type_into(&mut h, "Navigation maximum slope", "1");
    type_into(&mut h, "Navigation goal X", "1000");
    click(&mut h, "Build route");
    assert!(h.state().navigation.error.is_some());
    assert_eq!(h.state().editor.checksum(), checksum);
}

#[test]
fn another_saved_source_requires_explicit_build_and_replaces_full_identity() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let mut h = setup(&root_path);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    click(&mut h, "Build route");
    let original = h.state().editor.admitted_navigation().source.clone();
    assert_eq!(original, "terrain.orrt");
    {
        let app = h.state_mut();
        app.terrain.document.close().unwrap();
        app.terrain
            .document
            .new_local(
                "second.orrt",
                NewTerrain {
                    asset_id: "terrain/second.orrt".into(),
                    width: 9,
                    depth: 9,
                    origin: [FP::from_int(-4); 2],
                    spacing: FP::ONE,
                    height: FP::ZERO,
                },
            )
            .unwrap();
        app.terrain.document.save().unwrap();
    }
    h.run_steps(3);
    assert!(h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    assert_eq!(h.state().editor.admitted_navigation().source, "second.orrt");
    assert_eq!(
        h.state().editor.admitted_navigation().identity,
        "terrain/second.orrt"
    );
    assert!(!h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
}

#[test]
fn display_precision_rejection_keeps_scene_pin_and_history_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("navigation.scene.yaml");
    let mut h = setup(&root_path);
    {
        let app = h.state_mut();
        app.terrain.document.set_scene(Some(&scene)).unwrap();
        app.terrain
            .document
            .new_local(
                "large.orrt",
                NewTerrain {
                    asset_id: "terrain/large.orrt".into(),
                    width: 2,
                    depth: 2,
                    origin: [FP::from_int(1000); 2],
                    spacing: FP::ONE,
                    height: FP::ZERO,
                },
            )
            .unwrap();
        app.terrain.document.save().unwrap();
    }
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Point navigation");
    type_into(&mut h, "Navigation start X", "1000.0000152587890625");
    type_into(&mut h, "Navigation start Z", "1000.25");
    type_into(&mut h, "Navigation goal X", "1000.75");
    type_into(&mut h, "Navigation goal Z", "1000.25");
    click(&mut h, "Build route");
    assert!(
        h.state()
            .navigation
            .error
            .as_deref()
            .is_some_and(|e| e.contains("exact f32")),
        "{:?}",
        h.state().navigation.error
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!h.state().editor.admitted_navigation().admitted);
}

#[test]
fn saved_pinned_edit_survives_close_but_discard_and_other_source_do_not_latch() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let mut h = setup(&root_path);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    click(&mut h, "Build route");

    click(&mut h, "Vertex");
    type_into(&mut h, "Terrain height", "0.5");
    click(&mut h, "Apply terrain height");
    click(&mut h, "Save terrain");
    click(&mut h, "Close terrain");
    assert!(h.state().terrain.document.terrain().is_none());
    assert!(
        h.state()
            .navigation
            .route_stale(&h.state().editor, &h.state().terrain),
        "saved pinned-source mismatch must survive Close"
    );
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    click(&mut h, "Open terrain");
    click(&mut h, "Build route");
    assert!(!h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    wait_for_playing_frame(&mut h);
    click(&mut h, LBL_STOP);

    type_into(&mut h, "Terrain height", "0.25");
    click(&mut h, "Apply terrain height");
    assert!(h.state().terrain.document.dirty());
    click(&mut h, "Close terrain");
    click(&mut h, "Discard terrain changes and close");
    assert!(
        !h.state()
            .navigation
            .route_stale(&h.state().editor, &h.state().terrain),
        "discarding an unsaved edit restores the exact pinned source"
    );
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    wait_for_playing_frame(&mut h);
    click(&mut h, LBL_STOP);

    {
        let app = h.state_mut();
        app.terrain
            .document
            .new_local(
                "other.orrt",
                NewTerrain {
                    asset_id: "terrain/other.orrt".into(),
                    width: 9,
                    depth: 9,
                    origin: [FP::from_int(-4); 2],
                    spacing: FP::ONE,
                    height: FP::ZERO,
                },
            )
            .unwrap();
        app.terrain.document.save().unwrap();
    }
    h.run_steps(3);
    assert!(
        h.state()
            .navigation
            .route_stale(&h.state().editor, &h.state().terrain),
        "an open other-source terrain cannot borrow the pinned route"
    );
    click(&mut h, "Close terrain");
    assert!(
        !h.state()
            .navigation
            .route_stale(&h.state().editor, &h.state().terrain),
        "closing a different source does not mark the pinned file as changed"
    );
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    wait_for_playing_frame(&mut h);
    click(&mut h, LBL_STOP);
}
