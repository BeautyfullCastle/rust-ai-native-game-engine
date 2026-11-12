use super::EditorApp;
use crate::mesh_animation::{Settings, Snapshot};
fn pump_coherent(editor: &mut crate::Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        editor.pump();
        if editor.snapshot().is_some_and(|snapshot| {
            snapshot.predicted().tick() == editor.sim().head_tick
                && snapshot.predicted().checksum() == editor.sim().checksum
                && snapshot.timeline().is_some_and(|timeline| timeline.epoch == editor.sim().epoch)
        }) { return; }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("host did not deliver a coherent play snapshot");
}
#[test]
fn actual_mesh_app_observes_transition_and_same_tick_seek_resets_without_sim_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("crossfade.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let view = orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo").canonicalize().unwrap();
    let snapshot = Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", view.graph.revision()).unwrap();
    let snapshot = snapshot.clone().with_settings(Settings { clip: 1, idle_clip: Some(0), crossfade_ticks: Some(4), ..snapshot.settings() }).unwrap();
    crate::mesh_animation_store::save(&scene, Some(&snapshot), None, view.graph.revision()).unwrap();
    let sidecar = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
    let saved = std::fs::read(&sidecar).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.mesh_animation.sync(&app.editor);
    assert!(app.editor.start_play());
    pump_coherent(&mut app.editor);
    app.mesh_animation.sync(&app.editor);
    let mut previous = app.mesh_animation_for_frame().unwrap().unwrap().pose;
    let mut previous_clip = snapshot.selected_clip_for_view(&view).1;
    let mut transitioned = false;
    for _ in 0..120 {
        app.editor.step(1);
        pump_coherent(&mut app.editor);
        let frame = app.editor.snapshot().unwrap().predicted();
        let view = orr_remote::mesh_navigation::mesh_from_view(frame).unwrap();
        let tick = frame.tick();
        let clip = snapshot.selected_clip_for_view(&view).1;
        let checksum = app.editor.checksum();
        let visible = app.mesh_animation_for_frame().unwrap().unwrap().pose;
        assert_eq!(app.editor.checksum(), checksum);
        if clip != previous_clip {
            assert_eq!(visible, previous);
            let revision = app.editor.mesh_animation_revision();
            app.editor.pause();
            assert_eq!(app.editor.mesh_animation_revision(), revision);
            assert_eq!(app.mesh_animation_for_frame().unwrap().unwrap().pose, visible);
            app.editor.seek(tick);
            assert_ne!(app.editor.mesh_animation_revision(), revision);
            assert_eq!(app.editor.checksum(), checksum);
            assert_eq!(app.mesh_animation_for_frame().unwrap().unwrap().pose,
                       snapshot.placement_for_view(&view, tick).unwrap().pose);
            let revision = app.editor.mesh_animation_revision();
            app.editor.seek(u64::MAX); // Rejected command must not invalidate presentation.
            assert_eq!(app.editor.mesh_animation_revision(), revision);
            assert_eq!(app.editor.checksum(), checksum);
            transitioned = true;
            break;
        }
        previous = visible;
        previous_clip = clip;
    }
    assert!(transitioned, "fixture must reach an idle/moving transition");
    let revision = app.editor.mesh_animation_revision();
    assert!(app.editor.stop().is_some());
    assert_ne!(app.editor.mesh_animation_revision(), revision);
    let revision = app.editor.mesh_animation_revision();
    assert!(app.editor.start_play());
    assert_ne!(app.editor.mesh_animation_revision(), revision);
    assert_eq!(std::fs::read(sidecar).unwrap(), saved);
}
