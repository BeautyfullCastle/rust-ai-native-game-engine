use super::*;
use std::path::{Path, PathBuf};
const ANIMATED_GLB: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/animation_demo/animated.glb"
));
const LICENSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/animation_demo/LICENSE.txt"
));

fn animated_package(root: &Path) -> PathBuf {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
    };
    let package = root.join("animated-package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("animated.glb"), ANIMATED_GLB).unwrap();
    fs::write(package.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "sample-animation".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["animation".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["animated.glb".into(), "LICENSE.txt".into()]),
    };
    fs::write(
        package.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package
}

fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("candidate.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let package = animated_package(dir.path());
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.sync(&editor);
    panel.package_root = package.to_str().unwrap().into();
    panel.license_acknowledged = true;
    panel.load_candidate();
    assert!(panel.candidate.is_some(), "{:?}", panel.error);
    (dir, editor, panel)
}

#[test]
fn candidate_audition_before_apply_preserves_authoritative_state_and_invalid_pose() {
    let (_dir, editor, mut panel) = fixture();
    assert!(panel.current_snapshot().is_none());
    assert!(!panel.license_acknowledged);
    let bytes = std::fs::read(panel.scene.as_ref().unwrap()).unwrap();
    let checksum = editor.checksum();
    let tick = editor.snapshot().unwrap().predicted().tick();
    let history_id = panel.current_entry().unwrap().id;
    let host_history_len = editor.history().entries.len();
    let candidate_settings = panel.candidate.as_ref().unwrap().settings();
    request(&mut panel, &editor, "1", "0");
    let first = panel.placement(&editor).unwrap().unwrap().pose;
    request(&mut panel, &editor, "1", "16");
    let accepted = panel.placement(&editor).unwrap().unwrap().pose;
    assert_ne!(first, accepted);
    for (clip, tick) in [("9999", "16"), ("1", "3601"), ("-1", "16"), ("1", "nan")] {
        request(&mut panel, &editor, clip, tick);
        assert_eq!(panel.placement(&editor).unwrap().unwrap().pose, accepted);
    }
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.snapshot().unwrap().predicted().tick(), tick);
    assert_eq!(panel.current_entry().unwrap().id, history_id);
    assert_eq!(editor.history().entries.len(), host_history_len);
    assert_eq!(
        panel.candidate.as_ref().unwrap().settings(),
        candidate_settings
    );
    assert!(panel.current_snapshot().is_none());
    assert_eq!(std::fs::read(panel.scene.as_ref().unwrap()).unwrap(), bytes);
    assert!(
        !crate::mesh_animation_store::sidecar_path(panel.scene.as_ref().unwrap())
            .unwrap()
            .exists()
    );
    clear(&mut panel);
    assert!(panel.placement(&editor).unwrap().is_none());
}

#[test]
fn candidate_clear_restores_applied_pose_and_apply_changes_source_identity() {
    let (_dir, editor, mut panel) = fixture();
    panel.apply(&editor);
    let applied = panel.placement(&editor).unwrap().unwrap().pose;
    panel.license_acknowledged = true;
    panel.load_candidate();
    request(&mut panel, &editor, "1", "16");
    assert_ne!(panel.placement(&editor).unwrap().unwrap().pose, applied);
    clear(&mut panel);
    assert_eq!(panel.placement(&editor).unwrap().unwrap().pose, applied);
    request(&mut panel, &editor, "1", "16");
    panel.apply(&editor);
    assert!(!has_preview(&panel));
    assert!(matches!(
        context(&panel, &editor).unwrap().source,
        SourceKey::Applied(_)
    ));
    panel.undo();
    assert_eq!(panel.placement(&editor).unwrap().unwrap().pose, applied);
    panel.redo();
    assert!(!has_preview(&panel));
}

#[test]
fn candidate_reload_and_failed_load_invalidate_preview_and_require_rights() {
    let (_dir, editor, mut panel) = fixture();
    request(&mut panel, &editor, "1", "16");
    let generation = panel.candidate_generation;
    panel.license_acknowledged = true;
    panel.load_candidate();
    assert_ne!(panel.candidate_generation, generation);
    assert!(!has_preview(&panel));
    request(&mut panel, &editor, "1", "16");
    panel.asset = "missing.glb".into();
    panel.license_acknowledged = true;
    panel.load_candidate();
    assert!(panel.candidate.is_none());
    assert!(!has_preview(&panel));
    panel.asset = "animated.glb".into();
    panel.license_acknowledged = false;
    panel.load_candidate();
    assert!(panel.candidate.is_none());
}

#[test]
fn stale_candidate_epoch_generation_and_form_cannot_present_or_accept() {
    let (_dir, editor, mut panel) = fixture();
    request(&mut panel, &editor, "1", "16");
    panel.candidate_generation += 1;
    assert!(panel.placement(&editor).unwrap().is_none());
    sync(&mut panel, &editor);
    assert!(!has_preview(&panel));
    request(&mut panel, &editor, "1", "16");
    panel.scene_epoch = Some(editor.scene_load_epoch().wrapping_add(1));
    request(&mut panel, &editor, "1", "16");
    assert!(!has_preview(&panel));
    panel.scene_epoch = Some(editor.scene_load_epoch());
    request(&mut panel, &editor, "1", "16");
    panel.form_dirty = true;
    sync(&mut panel, &editor);
    assert!(!has_preview(&panel));
}

#[test]
fn candidate_play_transition_hides_and_clears_preview() {
    let (_dir, mut editor, mut panel) = fixture();
    request(&mut panel, &editor, "1", "16");
    assert!(has_preview(&panel));
    // The app normally blocks Play for an unapplied candidate. Exercise the
    // lower-level transition too, so an overlay cannot survive another caller.
    assert!(editor.start_play());
    assert!(panel.placement(&editor).unwrap().is_none());
    sync(&mut panel, &editor);
    assert!(!has_preview(&panel));
    assert!(panel.current_snapshot().is_none());
}

#[test]
fn live_candidate_clock_changes_only_pose_and_pause_preserves_pixels() {
    let (_dir, editor, mut panel) = fixture();
    let checksum = editor.checksum();
    let tick = editor.snapshot().unwrap().predicted().tick();
    let history = editor.history().entries.len();
    let settings = panel.candidate.as_ref().unwrap().settings();
    let bytes = std::fs::read(panel.scene.as_ref().unwrap()).unwrap();
    let mut state = std::mem::take(&mut panel.preview);
    state.clip = "1".into();
    state.tick = "0".into();
    state.play(&panel, &editor, 0.0, 0);
    assert!(state.transport.running);
    state.advance(0.25, 1, true);
    assert_eq!(state.accepted.as_ref().unwrap().tick, 15);
    panel.preview = state;
    let animated = panel.placement(&editor).unwrap().unwrap().pose;
    panel.preview.transport.pause();
    panel.preview.advance(0.5, 2, true);
    assert_eq!(panel.placement(&editor).unwrap().unwrap().pose, animated);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.snapshot().unwrap().predicted().tick(), tick);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(panel.candidate.as_ref().unwrap().settings(), settings);
    assert_eq!(std::fs::read(panel.scene.as_ref().unwrap()).unwrap(), bytes);
}
#[test]
fn live_invalid_tick_preserves_pose_and_clear_removes_transport() {
    let (_dir, editor, mut panel) = fixture();
    let mut state = std::mem::take(&mut panel.preview);
    state.clip = "1".into();
    state.tick = "0".into();
    state.play(&panel, &editor, 0.0, 0);
    state.advance(0.25, 1, true);
    let accepted = state.accepted.as_ref().unwrap().tick;
    state.tick = "not a tick".into();
    state.play(&panel, &editor, 0.25, 1);
    assert!(state.error.is_some());
    assert!(!state.transport.running);
    assert_eq!(state.accepted.as_ref().unwrap().tick, accepted);
    state.clear_accepted();
    assert!(state.accepted.is_none());
    assert!(!state.transport.running);
}
#[test]
fn live_play_and_candidate_generation_changes_clear_transport() {
    let (_dir, mut editor, mut panel) = fixture();
    let mut state = std::mem::take(&mut panel.preview);
    state.clip = "1".into();
    state.tick = "0".into();
    state.play(&panel, &editor, 0.0, 0);
    panel.preview = state;
    panel.candidate_generation += 1;
    sync(&mut panel, &editor);
    assert!(panel.preview.accepted.is_none());
    assert!(!panel.preview.transport.running);
    let mut state = std::mem::take(&mut panel.preview);
    state.play(&panel, &editor, 0.0, 0);
    panel.preview = state;
    assert!(editor.start_play());
    editor.sync();
    sync(&mut panel, &editor);
    assert!(panel.preview.accepted.is_none());
    assert!(!panel.preview.transport.running);
}
#[test]
fn live_endpoint_uses_authored_rate_and_leaves_candidate_settings_unchanged() {
    let (_dir, editor, mut panel) = fixture();
    let mut candidate_settings = panel.candidate.as_ref().unwrap().settings();
    candidate_settings.rate = 0.5;
    panel.candidate = Some(
        panel
            .candidate
            .take()
            .unwrap()
            .with_settings(candidate_settings)
            .unwrap(),
    );
    let mut state = std::mem::take(&mut panel.preview);
    state.request(&panel, &editor, "1", "0");
    let duration = state
        .accepted
        .as_ref()
        .unwrap()
        .snapshot
        .clips()
        .into_iter()
        .find(|(index, _, _)| *index == 1)
        .unwrap()
        .2;
    assert_eq!(
        state.end_tick(),
        Some(
            (f64::from(duration / 0.5) * f64::from(orr_sample::yard3d_game::TICK_RATE)).ceil()
                as u64
        )
    );
    state.transport.speed = Speed::Double;
    assert_eq!(
        state.accepted.as_ref().unwrap().snapshot.settings().rate,
        0.5
    );
    assert_eq!(
        panel.candidate.as_ref().unwrap().settings(),
        candidate_settings
    );
}
