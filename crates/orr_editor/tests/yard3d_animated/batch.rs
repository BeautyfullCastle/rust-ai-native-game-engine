use super::*;
#[test]
fn batch_animated_bindings_share_asset_clip_and_host_timeline_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    app.models.kind = ModelKind::Animated;
    app.models.package = "sample-animation".into();
    app.models.asset = "animated.glb".into();
    app.models.animation.clip_index = 1;
    app.models.animation.playback = PlaybackMode::Loop;
    let guids: Vec<_> = ["box_left", "box_right"]
        .iter()
        .map(|name| {
            app.editor
                .rows()
                .iter()
                .find(|row| row.name.as_deref() == Some(name))
                .unwrap()
                .guid
                .clone()
                .unwrap()
        })
        .collect();
    let checksum = app.editor.checksum();
    app.models.assign_targets(&app.editor, &guids).unwrap();
    assert_eq!(app.editor.checksum(), checksum);
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    app.models.undo(&app.editor).unwrap();
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    app.models.redo(&app.editor).unwrap();
    app.models.save(&app.editor).unwrap();
    app.models = Default::default();
    app.models.open_for_editor(&app.editor, false).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    assert!(app.editor.start_play());
    sync_clock(&mut app.editor);
    app.editor.step(30);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        sync_clock(&mut app.editor);
        if app.editor.snapshot().unwrap().tick() >= 30 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "batch animation host must advance to tick30"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let poses = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(poses.len(), 2);
    assert!(std::sync::Arc::ptr_eq(&poses[0].model, &poses[1].model));
    assert_eq!(poses[0].pose, poses[1].pose);
    let clock = app.editor.snapshot().unwrap();
    assert_eq!(
        poses[0].pose,
        poses[0]
            .model
            .sample_clip(1, clock.tick() as f32 / clock.tick_rate() as f32)
            .unwrap()
    );
    assert!(app.models.assign_targets(&app.editor, &guids).is_err());
}
