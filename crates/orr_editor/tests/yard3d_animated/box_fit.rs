use super::*;
#[test]
fn fitted_rest_model_retains_clip_and_saved_transform_drives_host_poses() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    app.models.save(&app.editor).unwrap();
    app.models.load_candidate(&app.editor).unwrap();
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    let original_bytes = frame_bytes(&mut app.editor);
    let animation = app.models.animation;
    app.models.fit_selected_box(&app.editor).unwrap();
    let fit = app.models.transform;
    assert_eq!(app.models.animation, animation);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    assert_eq!(frame_bytes(&mut app.editor), original_bytes);
    app.models.assign(&app.editor).unwrap();
    app.models.save(&app.editor).unwrap();
    let mut reopened = orr_editor::model_panel::ModelPanel::default();
    reopened.open_for_editor(&app.editor, false).unwrap();
    app.models = reopened;
    let rest = app.models.animated_placements(&app.editor).unwrap();
    let bounds = rest[0].model.bounds(&rest[0].pose).unwrap();
    for axis in 0..3 {
        assert!(bounds.min[axis] * fit.scale[axis] + fit.translation[axis] >= -0.5);
        assert!(bounds.max[axis] * fit.scale[axis] + fit.translation[axis] <= 0.5);
    }
    app.editor.step(20);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.editor.sync();
        if app
            .editor
            .snapshot()
            .is_some_and(|snapshot| snapshot.tick() >= 20)
        {
            break;
        }
        assert!(Instant::now() < deadline, "stepped frame missing");
        std::thread::sleep(Duration::from_millis(1));
    }
    sync_clock(&mut app.editor);
    let poses = app.models.animated_placements(&app.editor).unwrap();
    let model = &poses[0].model;
    let clip = &model.source().clips[1];
    let snapshot = app.editor.snapshot().unwrap();
    let time = orr_model_bindings::animation_time::sample_clip_time(
        snapshot.tick(),
        snapshot.tick_rate(),
        clip.duration(),
        animation.playback,
    )
    .unwrap();
    assert_eq!(poses[0].pose, model.sample_clip(1, time).unwrap());
    assert_eq!(
        app.models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .values()
            .next()
            .unwrap()
            .transform,
        fit
    );
}
