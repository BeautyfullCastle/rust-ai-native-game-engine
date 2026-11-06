use super::*;
#[test]
fn copied_animation_settings_save_reopen_and_drive_actual_host_play_pose() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    app.models.transform.translation = [0.25, 0.5, 0.0];
    app.models.transform.scale = [0.6; 3];
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    app.models.save(&app.editor).unwrap();
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    let checksum = app.editor.checksum();
    app.models.package = "draft".into();
    app.models.asset = "wrong.glb".into();
    app.models.kind = ModelKind::Static;
    app.models.transform = Default::default();
    app.models.animation.clip_index = 0;
    app.models.animation.playback = PlaybackMode::Once;
    app.models.use_selected_settings(&app.editor).unwrap();
    assert_eq!(app.models.kind, ModelKind::Animated);
    assert_eq!(app.models.package, "sample-animation");
    assert_eq!(app.models.asset, "animated.glb");
    assert_eq!(app.models.animation.clip_index, 1);
    assert_eq!(app.models.animation.playback, PlaybackMode::Loop);
    assert_eq!(app.models.transform.translation, [0.25, 0.5, 0.0]);
    assert_eq!(app.models.transform.scale, [0.6; 3]);
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    app.models.animation.playback = PlaybackMode::Once;
    app.models.assign(&app.editor).unwrap();
    app.models.save(&app.editor).unwrap();
    let mut reopened = orr_editor::model_panel::ModelPanel::default();
    reopened.open_for_editor(&app.editor, false).unwrap();
    app.models = reopened;
    let saved = app
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(saved.animation.unwrap().clip_index, 1);
    assert_eq!(saved.animation.unwrap().playback, PlaybackMode::Once);
    let rest = app.models.animated_placements(&app.editor).unwrap();
    let duration = rest[0].model.source().clips[1].duration();
    assert!(duration > 0.0 && duration < 10.0);
    app.models.animation.clip_index = 0;
    app.models.animation.playback = PlaybackMode::Loop;
    let rate = app.editor.snapshot().unwrap().tick_rate();
    let target = (duration * rate as f32).ceil() as u32 + rate / 2;
    app.editor.step(target);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.editor.sync();
        if app
            .editor
            .snapshot()
            .is_some_and(|s| s.tick() >= u64::from(target))
        {
            break;
        }
        assert!(Instant::now() < deadline, "stepped snapshot missing");
        std::thread::sleep(Duration::from_millis(1));
    }
    sync_clock(&mut app.editor);
    let poses = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(poses.len(), 1);
    assert_eq!(
        poses[0].pose,
        poses[0].model.sample_clip(1, duration).unwrap()
    );
}
