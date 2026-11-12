//! Real Editor/host Save and standalone consumer parity, without a window or UI-widget claim.
#![cfg(all(feature = "navigation", target_os = "linux"))]
#![allow(clippy::disallowed_types)]
use orr_ecs::Frame;
use orr_editor::{
    game::EditorGame, navigation_panel::NavigationPanel, terrain_panel::TerrainPanel, Editor, Mode,
};
use orr_navigation_view::authored::AuthoredPlayer;
use std::{
    fs,
    path::{Path, PathBuf},
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
#[test]
fn real_editor_terrain_route_save_matches_standalone_sixty_ticks_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let (mut editor, scene) = author(root.path());
    let scene_before = fs::read(&scene).unwrap();
    let terrain_before = fs::read(root.path().join("authored.orrt")).unwrap();
    let mut player = AuthoredPlayer::open(&scene).unwrap();
    editor.step(0); // Starts the actual host Play session paused, without wall-clock advancement.
    let initial_position = orr_remote::navigation_yard3d::navigation_from_view(
        orr_bridge::FrameView::of(player.frame()),
    )
    .unwrap()
    .navigator
    .position();
    let mut moved = false;
    let mut trace = Vec::new();
    for tick in 0..=60 {
        if tick > 0 {
            editor.step(1);
            player.step().unwrap();
        }
        let frame = authoritative(&mut editor, tick);
        assert_eq!(player.tick(), tick);
        assert_eq!(player.checksum(), frame.checksum(), "tick {tick}");
        assert_eq!(
            player.frame().to_bytes(),
            frame.to_bytes(),
            "authoritative bytes at tick {tick}"
        );
        let nav =
            orr_remote::navigation_yard3d::navigation_from_view(orr_bridge::FrameView::of(&frame))
                .unwrap();
        moved |= nav.navigator.position() != initial_position;
        assert_eq!(
            frame
                .singleton::<orr_remote::navigation_yard3d::NavigationStepStatus>()
                .failed,
            0
        );
        trace.push(frame.to_bytes());
    }
    assert_ne!(trace[0], trace[1]);
    assert!(
        moved,
        "the authored route must actually advance the point agent"
    );
    player.restart().unwrap();
    for (tick, expected) in trace.iter().enumerate() {
        if tick > 0 {
            player.step().unwrap();
        }
        assert_eq!(&player.frame().to_bytes(), expected, "restart tick {tick}");
    }
    assert_eq!(fs::read(&scene).unwrap(), scene_before);
    assert_eq!(
        fs::read(root.path().join("authored.orrt")).unwrap(),
        terrain_before
    );
}

#[test]
#[ignore = "requires explicit actual navigation_player binary; no mocked CLI success"]
fn actual_cli_consumes_relocated_editor_saved_scene() {
    let binary = PathBuf::from(
        std::env::var_os("ORR_NAVIGATION_PLAYER").expect("ORR_NAVIGATION_PLAYER required"),
    );
    assert!(
        binary.is_absolute() && binary.is_file(),
        "actual absolute executable required"
    );
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    let (editor, scene) = author(&source);
    drop(editor);
    let mut expected = AuthoredPlayer::open(&scene).unwrap();
    for _ in 0..60 {
        expected.step().unwrap();
    }
    let moved = root.path().join("relocated");
    fs::rename(&source, &moved).unwrap();
    assert!(!source.exists());
    let cwd = root.path().join("empty");
    fs::create_dir(&cwd).unwrap();
    let result = std::process::Command::new(&binary)
        .current_dir(&cwd)
        .args(["--scene"])
        .arg(moved.join("navigation.scene.yaml"))
        .args(["--ticks", "60", "--restart-check"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(receipt["tick"].as_u64(), Some(60));
    assert_eq!(receipt["checksum"].as_u64(), Some(expected.checksum()));
    assert!(!source.exists());
}
