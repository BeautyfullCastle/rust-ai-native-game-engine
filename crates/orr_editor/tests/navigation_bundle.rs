//! Actual Editor Save -> real exporter -> relocated launcher acceptance.
//! Owned source paths are hidden by rename, not by sandbox isolation. No UI/GPU claim.
#![cfg(all(feature = "navigation", target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::disallowed_types)]
use orr_ecs::Frame;
use orr_editor::{
    game::EditorGame, navigation_panel::NavigationPanel, terrain_panel::TerrainPanel, Editor, Mode,
};
use orr_navigation_view::authored::AuthoredPlayer;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn wait(editor: &mut Editor, tick: Option<u64>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        let ready = editor
            .snapshot()
            .is_some_and(|s| tick.is_none_or(|t| s.predicted().tick() == t));
        if ready && editor.can_mutate() && editor.yard_rows_coherent() && tick.is_none() {
            return;
        }
        if ready
            && tick.is_some()
            && editor.mode() == Mode::Play
            && editor.timeline().is_some_and(|t| !t.playing)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor did not reach expected state: tick={tick:?}, mode={:?}, log={:?}",
            editor.mode(),
            editor.log()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn author(root: &Path) -> (Editor, PathBuf) {
    let scene = root.join("navigation.scene.yaml");
    fs::write(
        &scene,
        include_str!("../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    wait(&mut editor, None);
    let mut terrain = TerrainPanel::default();
    terrain.local_path = "authored.orrt".into();
    terrain.asset_id = "navigation/editor-parity".into();
    terrain.dimensions = [9, 9];
    terrain.origin = ["-4".into(), "-4".into()];
    terrain.spacing = "1".into();
    terrain.initial_height = "0".into();
    terrain.create_for_editor(&editor).unwrap();
    assert!(terrain.document.dirty());
    terrain.save_for_editor(&editor).unwrap();
    assert!(!terrain.document.dirty());
    assert_eq!(
        fs::read(root.join("authored.orrt")).unwrap(),
        terrain.document.bytes().unwrap()
    );
    let mut navigation = NavigationPanel::default();
    navigation.start = ["-3".into(), "0".into()];
    navigation.goal = ["3".into(), "0".into()];
    navigation.distance_per_tick = "0.25".into();
    navigation.build_route(&mut editor, &terrain).unwrap();
    editor.sync();
    assert!(
        editor.save(),
        "actual ERP scene.save must write the authored pin"
    );
    let saved = fs::read_to_string(&scene).unwrap();
    assert!(saved.contains("terrain_revision:"));
    assert!(saved.contains("graph_revision:"));
    (editor, scene)
}
fn authoritative(editor: &mut Editor, tick: u64) -> Frame {
    wait(editor, Some(tick));
    let checksum = editor.snapshot().unwrap().predicted().checksum();
    let client = editor.agent_client("authored-navigation-parity").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(received) = client.local_frames.pop_front() {
            if received.frame.tick() == tick && received.frame.checksum() == checksum {
                return (*received.frame).clone();
            }
        }
        assert!(
            Instant::now() < deadline,
            "authoritative Frame {tick} was not delivered"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn executable(variable: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is required")),
    );
    assert!(
        path.is_absolute() && path.is_file(),
        "{variable} must name an actual absolute binary"
    );
    path
}
#[test]
#[ignore = "requires actual exporter/player and recorded player SHA-256; no fake smoke or runtime"]
fn editor_saved_navigation_exports_and_relocates_without_original_sources() {
    let exporter = executable("ORR_NAVIGATION_EXPORT");
    let runtime = executable("ORR_NAVIGATION_BUNDLE_PLAYER");
    let runtime_hash = std::env::var("ORR_NAVIGATION_BUNDLE_PLAYER_SHA256")
        .expect("recorded actual runtime SHA-256 required");
    assert!(
        runtime_hash.len() == 64
            && runtime_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("author source with spaces");
    fs::create_dir(&source).unwrap();
    let (mut editor, scene) = author(&source);
    let scene_bytes = fs::read(&scene).unwrap();
    let terrain_bytes = fs::read(source.join("authored.orrt")).unwrap();
    // The supplied runtime is copied into the source tree, so its export-input path
    // also disappears before launching the bundle. The exporter checks the actual hash.
    let copied_runtime = source.join("trusted navigation player");
    fs::copy(&runtime, &copied_runtime).unwrap();
    assert_eq!(
        fs::read(&copied_runtime).unwrap(),
        fs::read(&runtime).unwrap()
    );
    let mut player = AuthoredPlayer::open(&scene).unwrap();
    editor.step(0);
    let mut checksums = Vec::new();
    for tick in 0..=60 {
        if tick > 0 {
            editor.step(1);
            player.step().unwrap();
        }
        let frame = authoritative(&mut editor, tick);
        assert_eq!(
            frame.to_bytes(),
            player.frame().to_bytes(),
            "saved authored player tick {tick}"
        );
        checksums.push(frame.checksum());
    }
    let expected = checksums[60];
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(source.join("authored.orrt")).unwrap(),
        terrain_bytes
    );
    drop(editor);
    let output = scratch.path().join("export output with spaces");
    let exported = Command::new(&exporter)
        .arg("--scene")
        .arg(&scene)
        .arg("--runtime")
        .arg(&copied_runtime)
        .arg("--runtime-sha256")
        .arg(&runtime_hash)
        .arg("--trusted-runtime")
        .arg("--output")
        .arg(&output)
        .args(["--source-revision", "operator-test-claim-not-attestation"])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "export stdout={} stderr={}",
        String::from_utf8_lossy(&exported.stdout),
        String::from_utf8_lossy(&exported.stderr)
    );
    assert_eq!(
        fs::read(output.join("project/game.scene.yaml")).unwrap(),
        scene_bytes
    );
    assert_eq!(
        fs::read(output.join("project/authored.orrt")).unwrap(),
        terrain_bytes
    );
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(source.join("authored.orrt")).unwrap(),
        terrain_bytes
    );
    let hidden = scratch.path().join("hidden author backup");
    fs::rename(&source, &hidden).unwrap();
    assert!(!source.exists() && !scene.exists() && !copied_runtime.exists());
    let relocated = scratch.path().join("moved bundle with spaces");
    fs::rename(&output, &relocated).unwrap();
    assert!(!output.exists());
    let cwd = scratch.path().join("empty working directory");
    fs::create_dir(&cwd).unwrap();
    for ticks in [0, 17, 60] {
        let run = Command::new(relocated.join("run-navigation"))
            .current_dir(&cwd)
            .args(["--ticks", &ticks.to_string(), "--restart-check"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "launcher stdout={} stderr={}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        let receipt: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
        assert_eq!(receipt["tick"].as_u64(), Some(ticks as u64));
        assert_eq!(receipt["checksum"].as_u64(), Some(checksums[ticks]));
    }
    assert_eq!(expected, player.checksum());
    assert!(!source.exists() && !scene.exists() && !copied_runtime.exists());
    assert_eq!(
        fs::read(hidden.join("navigation.scene.yaml")).unwrap(),
        scene_bytes
    );
    assert_eq!(
        fs::read(hidden.join("authored.orrt")).unwrap(),
        terrain_bytes
    );
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 0);
}
