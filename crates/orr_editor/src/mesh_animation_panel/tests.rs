use super::{frame_gate, parse_settings, Panel, PlaybackFields, MAX_HISTORY};
use crate::{backend::HostSpec, game::EditorGame, mesh_animation::Snapshot, model::Mode};
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

fn playback_fields<'fields>(
    moving_clip: &'fields str,
    idle_clip: Option<&'fields str>,
    rate: &'fields str,
    looped: bool,
    face_route: bool,
) -> PlaybackFields<'fields> {
    PlaybackFields {
        moving_clip,
        idle_clip,
        rate,
        looped,
        face_route,
    }
}

#[test]
fn parses_finite_animation_settings_and_rejects_bad_values() {
    let settings = parse_settings(
        &["1.25".into(), "-2".into(), "0.5".into()],
        "0.75",
        &["90".into(), "0".into(), "0".into()],
        playback_fields("1", None, "1.5", false, false),
    )
    .unwrap();
    assert_eq!(settings.offset, [1.25, -2.0, 0.5]);
    assert_eq!(settings.scale, 0.75);
    assert_eq!(settings.clip, 1);
    assert_eq!(settings.rate, 1.5);
    assert!(!settings.looped);
    assert_eq!(settings.facing, crate::mesh_facing::Facing::Fixed);
    let route_facing = parse_settings(
        &["0".into(), "0".into(), "0".into()],
        "1",
        &["0".into(), "0".into(), "0".into()],
        playback_fields("0", None, "1", true, true),
    )
    .unwrap();
    assert_eq!(route_facing.facing, crate::mesh_facing::Facing::Route);
    let zeros = || std::array::from_fn(|_| "0".to_owned());
    assert!(parse_settings(
        &["NaN".into(), "0".into(), "0".into()],
        "1",
        &zeros(),
        playback_fields("0", None, "1", true, false)
    )
    .is_err());
    assert!(parse_settings(
        &zeros(),
        "inf",
        &zeros(),
        playback_fields("0", None, "1", true, false)
    )
    .is_err());
    assert!(parse_settings(
        &zeros(),
        "1",
        &zeros(),
        playback_fields("0", None, "inf", true, false)
    )
    .is_err());
    assert!(parse_settings(
        &zeros(),
        "1",
        &zeros(),
        playback_fields("-1", None, "1", true, false)
    )
    .is_err());
}

#[test]
fn idle_clip_form_is_optional_and_validated() {
    let zeros = || std::array::from_fn(|_| "0".to_owned());
    let moving_only = parse_settings(
        &zeros(),
        "1",
        &zeros(),
        playback_fields("1", None, "1", true, false),
    )
    .unwrap();
    assert_eq!(moving_only.idle_clip, None);

    let idle = parse_settings(
        &zeros(),
        "1",
        &zeros(),
        playback_fields("1", Some("0"), "1", true, false),
    )
    .unwrap();
    assert_eq!(idle.idle_clip, Some(0));
    assert!(parse_settings(
        &zeros(),
        "1",
        &zeros(),
        playback_fields("1", Some("invalid"), "1", true, false)
    )
    .is_err());
}

#[test]
fn animated_snapshot_roundtrips_clip_settings_and_private_history() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = animated_package(&root);
    let initial =
        Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", [0x35; 32]).unwrap();
    let clips = initial.clips();
    assert!(clips.len() >= 2);
    let mut settings = initial.settings();
    settings.clip = clips[1].0;
    settings.idle_clip = Some(clips[0].0);
    settings.rate = 1.5;
    settings.looped = false;
    let first = initial.with_settings(settings).unwrap();
    let mut changed = first.settings();
    changed.offset[0] = 1.0;
    let second = first.clone().with_settings(changed).unwrap();
    let mut panel = Panel::default();
    panel.push_state(Some(first.clone()));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    panel.push_state(Some(second));
    panel.undo();
    let restored = panel.current_snapshot().unwrap().settings();
    assert_eq!(restored.clip, settings.clip);
    assert_eq!(restored.idle_clip, settings.idle_clip);
    assert_eq!(restored.rate, settings.rate);
    assert_eq!(restored.looped, settings.looped);
    assert_eq!(restored.facing, settings.facing);
    panel.redo();
    assert_eq!(panel.current_snapshot().unwrap().settings().offset[0], 1.0);
    for _ in 0..MAX_HISTORY + 2 {
        let current = panel.current_snapshot().cloned();
        panel.push_state(current);
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
}

#[test]
fn frame_gate_rejects_incoherent_scene_and_edit_frame() {
    let scene = PathBuf::from(r"C:\workspace\mesh.scene.yaml");
    let spec = HostSpec::local_game(scene.clone(), EditorGame::MeshNavigation);
    assert!(frame_gate(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Play,
        true,
        1,
        2
    ));
    assert!(!frame_gate(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Edit,
        true,
        1,
        2
    ));
    assert!(!frame_gate(
        EditorGame::MeshNavigation,
        &spec,
        Some(Path::new(r"C:\workspace\other.scene.yaml")),
        Mode::Edit,
        true,
        1,
        1
    ));
    assert!(!frame_gate(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Edit,
        false,
        1,
        1
    ));
}

#[cfg(feature = "mesh-mission")]
#[test]
fn workspace_save_persists_only_applied_animation_and_blocks_form_drafts() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("animation-workspace.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let frame = editor.snapshot().unwrap().predicted();
    let revision = orr_remote::mesh_navigation::mesh_from_view(frame)
        .unwrap()
        .graph
        .revision();
    let package = animated_package(directory.path());
    let snapshot =
        Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", revision).unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.push_state(Some(snapshot.clone()));
    assert!(panel.workspace_save_blocker(&editor).is_none());
    assert!(panel.save_workspace_changes(&editor).unwrap());
    let sidecar = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
    let saved_bytes = std::fs::read(&sidecar).unwrap();
    let mut form_panel = Panel::default();
    form_panel.sync(&editor);
    form_panel.candidate = Some(snapshot.clone());
    assert!(form_panel.workspace_save_blocker(&editor).is_some());
    form_panel.candidate = None;
    form_panel.form_dirty = true;
    assert!(form_panel.workspace_save_blocker(&editor).is_some());
    assert!(form_panel.save_workspace_changes(&editor).is_err());
    assert_eq!(std::fs::read(sidecar).unwrap(), saved_bytes);
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
fn preview_fixture() -> (
    tempfile::TempDir,
    crate::Editor,
    Panel,
    Snapshot,
    Vec<u8>,
    Vec<u8>,
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let scene = root.join("animation-preview.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let frame = editor.snapshot().unwrap().predicted();
    let revision = orr_remote::mesh_navigation::mesh_from_view(frame)
        .unwrap()
        .graph
        .revision();
    let package = animated_package(&root);
    let initial =
        Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", revision).unwrap();
    let clips = initial.clips();
    assert!(clips.len() > 1);
    let mut settings = initial.settings();
    settings.clip = clips[1].0;
    settings.idle_clip = Some(clips[0].0);
    let snapshot = initial.with_settings(settings).unwrap();
    crate::mesh_animation_store::save(&scene, Some(&snapshot), None, revision).unwrap();
    let sidecar = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
    let sidecar_bytes = std::fs::read(sidecar).unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.sync(&editor);
    let scene_bytes = std::fs::read(scene).unwrap();
    (
        directory,
        editor,
        panel,
        snapshot,
        scene_bytes,
        sidecar_bytes,
    )
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
#[test]
fn animation_preview_validates_clip_and_tick_without_clamping() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let package = animated_package(&root);
    let snapshot =
        Snapshot::from_package(&package, "animated.glb", "LICENSE.txt", [0x73; 32]).unwrap();
    let clips = snapshot.clips();
    let (preview_snapshot, tick) =
        super::preview::prepare_snapshot(&snapshot, &clips[1].0.to_string(), "3600").unwrap();
    assert_eq!(tick, 3600);
    assert_eq!(preview_snapshot.settings().clip, clips[1].0);
    assert_eq!(preview_snapshot.settings().idle_clip, None);
    for (clip, requested_tick) in [
        ("-1", "0"),
        ("+1", "0"),
        ("99", "0"),
        ("1", "-1"),
        ("1", "3601"),
        ("1", "1.5"),
    ] {
        assert!(
            super::preview::prepare_snapshot(&snapshot, clip, requested_tick).is_err(),
            "clip {clip:?}, tick {requested_tick:?}"
        );
    }
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
#[test]
fn animation_preview_changes_pose_without_mutating_editor_history_or_saved_bytes() {
    let (_directory, editor, mut panel, snapshot, scene_bytes, sidecar_bytes) = preview_fixture();
    let before_checksum = editor.checksum();
    let before_tick = editor.snapshot().unwrap().predicted().tick();
    let before_cursor = panel.cursor;
    let before_history_id = panel.current_entry().unwrap().id;
    let before_dirty = panel.dirty();
    let frame = editor.snapshot().unwrap().predicted();
    let view = orr_remote::mesh_navigation::mesh_from_view(frame).unwrap();
    let original = snapshot.placement_for_view(&view, frame.tick()).unwrap();

    super::preview::request(&mut panel, &editor, "1", "16");
    assert!(super::preview::has_preview(&panel));
    let accepted = panel.placement(&editor).unwrap().unwrap();
    assert_ne!(accepted.pose, original.pose);
    let accepted_pose = accepted.pose;
    super::preview::request(&mut panel, &editor, "1", "-1");
    assert!(super::preview::has_preview(&panel));
    assert_eq!(
        panel.placement(&editor).unwrap().unwrap().pose,
        accepted_pose
    );

    assert_eq!(editor.checksum(), before_checksum);
    assert_eq!(editor.snapshot().unwrap().predicted().tick(), before_tick);
    assert_eq!(panel.cursor, before_cursor);
    assert_eq!(panel.current_entry().unwrap().id, before_history_id);
    assert_eq!(panel.dirty(), before_dirty);
    assert_eq!(
        std::fs::read(panel.scene.as_ref().unwrap()).unwrap(),
        scene_bytes
    );
    let sidecar = crate::mesh_animation_store::sidecar_path(panel.scene.as_ref().unwrap()).unwrap();
    assert_eq!(std::fs::read(sidecar).unwrap(), sidecar_bytes);
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
#[test]
fn animation_preview_clear_and_play_restore_authoritative_frame_pose() {
    let (_directory, mut editor, mut panel, snapshot, _scene_bytes, _sidecar_bytes) =
        preview_fixture();
    let frame = editor.snapshot().unwrap().predicted();
    let view = orr_remote::mesh_navigation::mesh_from_view(frame).unwrap();
    let original = snapshot.placement_for_view(&view, frame.tick()).unwrap();
    super::preview::request(&mut panel, &editor, "1", "16");
    assert_ne!(
        panel.placement(&editor).unwrap().unwrap().pose,
        original.pose
    );

    super::preview::clear(&mut panel);
    assert!(!super::preview::has_preview(&panel));
    assert_eq!(
        panel.placement(&editor).unwrap().unwrap().pose,
        original.pose
    );

    super::preview::request(&mut panel, &editor, "1", "16");
    assert!(editor.start_play());
    panel.sync(&editor);
    assert!(!super::preview::has_preview(&panel));
    let play_frame = editor.snapshot().unwrap().predicted();
    let play_view = orr_remote::mesh_navigation::mesh_from_view(play_frame).unwrap();
    let authoritative = snapshot
        .placement_for_view(&play_view, play_frame.tick())
        .unwrap();
    assert_eq!(
        panel.placement(&editor).unwrap().unwrap().pose,
        authoritative.pose
    );
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
#[test]
fn animation_preview_clears_when_applied_history_changes() {
    let (_directory, editor, mut panel, snapshot, _scene_bytes, _sidecar_bytes) = preview_fixture();
    super::preview::request(&mut panel, &editor, "1", "16");
    assert!(super::preview::has_preview(&panel));
    let mut settings = snapshot.settings();
    settings.offset[0] = 0.5;
    panel.push_state(Some(snapshot.with_settings(settings).unwrap()));
    assert!(!super::preview::has_preview(&panel));
}

#[cfg(all(feature = "mesh-mission", feature = "mesh-animated-models"))]
#[test]
fn animation_preview_clears_when_scene_context_changes() {
    let (directory, editor, mut panel, _snapshot, _scene_bytes, _sidecar_bytes) = preview_fixture();
    super::preview::request(&mut panel, &editor, "1", "16");
    assert!(super::preview::has_preview(&panel));

    let second_scene = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("another-animation-preview.scene.yaml");
    let second_editor = crate::mesh_workspace::mission::prepare(
        &second_scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    panel.sync(&second_editor);
    assert!(!super::preview::has_preview(&panel));
    assert_eq!(panel.scene.as_deref(), Some(second_scene.as_path()));
}
