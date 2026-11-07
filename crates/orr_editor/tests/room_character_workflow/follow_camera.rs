use super::*;
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
#[test]
fn follow_camera_widgets_save_reopen_real_movement_pan_reset_seek_and_stop() {
    let f = Fixture::new();
    let mut h = f.app(false);
    settle(&mut h);
    wait(&mut h, "coherent follow camera source", |app| {
        app.editor.yard_rows_coherent()
    });
    let before = frame_bytes(&mut h.state_mut().editor);
    let original = h.state().room_camera.as_ref().unwrap().document().clone();
    if h.query_by_label("Follow player during play").is_none() {
        click(&mut h, "Room camera");
    }
    click(&mut h, "Follow player during play");
    click(&mut h, "Apply camera");
    let saved = h.state().room_camera.as_ref().unwrap().document().clone();
    assert_eq!(saved.schema, 2);
    assert_eq!(saved.follow_player, Some([0.0; 3]));
    assert_eq!(
        h.state()
            .editor
            .presentation_camera3d((800, 600))
            .unwrap()
            .target,
        saved.target
    );
    click(&mut h, "Undo camera edit");
    assert_eq!(
        h.state().room_camera.as_ref().unwrap().document(),
        &original
    );
    click(&mut h, "Redo camera edit");
    click(&mut h, "Save camera");
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before);
    drop(h);
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(h.state().room_camera.as_ref().unwrap().document(), &saved);
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, "follow camera control available", |app| {
        app.editor.can_take_control()
    });
    click(&mut h, "Take control");
    wait(&mut h, "follow camera input active", |app| {
        app.editor.input_phase() == Phase::Active
    });
    key(&mut h, egui::Key::D, true);
    wait(&mut h, "actual movement updates camera target", |app| {
        if !app.editor.yard_rows_coherent() {
            return false;
        }
        let placed = app.models.animated_placements(&app.editor).unwrap();
        placed.len() == 1
            && placed[0].instance[3][0] > 0.1
            && app.editor.presentation_camera3d((800, 600)).unwrap().target[0]
                == placed[0].instance[3][0]
    });
    key(&mut h, egui::Key::D, false);
    h.state_mut().editor.pause();
    settle(&mut h);
    wait(&mut h, "follow camera paused coherent", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .timeline()
                .is_some_and(|timeline| !timeline.playing)
    });
    let centered = h.state().editor.presentation_camera3d((800, 600)).unwrap();
    h.state_mut().editor.pan3d([20.0, -10.0], (800, 600));
    let biased = h.state().editor.presentation_camera3d((800, 600)).unwrap();
    assert_ne!(biased.target, centered.target);
    for axis in 0..3 {
        let bias = h.state().editor.camera3d.target[axis] - saved.target[axis];
        assert!((biased.target[axis] - centered.target[axis] - bias).abs() < 0.0001);
    }
    h.state_mut().editor.reset_room_camera();
    assert_eq!(
        h.state()
            .editor
            .presentation_camera3d((800, 600))
            .unwrap()
            .target,
        centered.target
    );
    h.state_mut().editor.seek(0);
    wait(&mut h, "follow camera seek zero", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == 0)
    });
    assert_eq!(
        h.state()
            .editor
            .presentation_camera3d((800, 600))
            .unwrap()
            .target,
        saved.target
    );
    click(&mut h, orr_editor::app::LBL_STOP);
    wait(&mut h, "follow camera stop restores edit view", |app| {
        app.editor.mode() == Mode::Edit
            && app.editor.yard_rows_coherent()
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.timeline().is_none())
    });
    assert_eq!(
        h.state()
            .editor
            .presentation_camera3d((800, 600))
            .unwrap()
            .target,
        saved.target
    );
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "explicit CPU source-hidden export requires ORR_FOLLOW_CAMERA_RUNTIME"]
fn follow_camera_source_hidden_export_and_native_headless_consume_saved_offset_and_movement() {
    use orr_sim::{PlayerSlot, TickInputs};
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let mut h = f.app(false);
    settle(&mut h);
    wait(&mut h, "coherent camera export", |app| {
        app.editor.yard_rows_coherent()
    });
    let mut doc = h.state().room_camera.as_ref().unwrap().document().clone();
    doc.schema = 2;
    doc.follow_player = Some([1.0, 2.0, -1.0]);
    {
        let app = h.state_mut();
        app.room_camera
            .as_mut()
            .unwrap()
            .apply_document(doc.clone())
            .unwrap();
        app.editor.install_room_camera(doc.clone()).unwrap();
        app.room_camera.as_mut().unwrap().save().unwrap();
    }
    let camera_bytes = fs::read(f.root.join("room.camera.json")).unwrap();
    let built = PathBuf::from(
        std::env::var_os("ORR_FOLLOW_CAMERA_RUNTIME").expect("explicit built runtime required"),
    )
    .canonicalize()
    .unwrap();
    let runtime = f._temp.path().join("trusted follow runtime");
    fs::copy(built, &runtime).unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f._temp.path().join("deployed follow game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    assert_eq!(
        fs::read(bundle.join("project/room.camera.json")).unwrap(),
        camera_bytes
    );
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open_with_capabilities(
        &bundle.join("project"),
        orr_sample::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.camera().unwrap().document, doc);
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut input = TickInputs::new(sim.tick() + 1, 1);
        input.set_input(
            PlayerSlot(0),
            orr_sample::room_game::RoomInput {
                move_x: 1,
                ..Default::default()
            },
        );
        sim.step(&input);
    }
    let character = prepared.character().unwrap();
    assert_eq!(
        prepared.models().document.bindings[&character.document.player].transform,
        orr_model_bindings::model_bindings::LocalTransform::default()
    );
    let placed = orr_sample::room_view::character_placement(
        orr_bridge::FrameView::of(sim.frame()),
        prepared.scene().index(),
        prepared.models(),
        orr_sample::room_view::PresentationTime::default(),
    )
    .unwrap()
    .unwrap();
    let offset = doc.follow_player.unwrap();
    let expected = [
        placed.transform[3][0] + offset[0],
        placed.transform[3][1] + offset[1],
        placed.transform[3][2] + offset[2],
    ];
    assert!(expected[0] > offset[0]);
    let before = sim.frame().to_bytes();
    assert_eq!(
        doc.camera_for_frame(
            &doc.orbit(),
            (800, 600),
            orr_bridge::FrameView::of(sim.frame())
        )
        .unwrap()
        .target,
        expected
    );
    assert_eq!(sim.frame().to_bytes(), before);
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(f._temp.path())
        .args([
            "--headless",
            "--ticks",
            "8",
            "--hold",
            "right",
            "--check-model-poses",
        ])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("room model poses: 9 frames checked;"));
    let target: [f32; 3] = serde_json::from_str(
        text.lines()
            .find_map(|line| line.strip_prefix("room follow camera target: "))
            .expect("native runtime must consume saved follow camera"),
    )
    .unwrap();
    assert_eq!(target, expected);
}
