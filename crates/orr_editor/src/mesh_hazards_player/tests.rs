use super::{
    MAX_CAPTURE_PIXELS, MAX_STARTUP_TICKS, Options, Request, screenshot_path, start_playback,
};
use crate::{Editor, game::EditorGame};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "orr-mesh-player-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create unique mesh-player test directory");
        Self(
            path.canonicalize()
                .expect("canonicalize the owned test directory"),
        )
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn parse(arguments: &[&str]) -> Result<Request, String> {
    Options::parse(arguments.iter().map(OsString::from))
}

fn playback_editor() -> (TestDirectory, Editor) {
    let directory = TestDirectory::new();
    let scene = directory.path().join("player-startup.scene.yaml");
    let mut editor =
        crate::mesh_hazards_creator::prepare(&scene, &Default::default(), true).unwrap();
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.sync(&editor);
    editor.set_mesh_navigation_blocker(panel.play_blocker(&editor));
    (directory, editor)
}

#[test]
fn zero_tick_playback_starts_without_a_refused_step_rpc() {
    let (_directory, mut editor) = playback_editor();
    start_playback(&mut editor, Some(0), false).unwrap();
    assert_eq!(editor.timeline().unwrap().tick, 0);
    assert!(!editor.timeline().unwrap().playing);
    assert!(editor.status().is_none_or(|message| !message.error));
}

#[test]
fn screenshot_without_tick_override_starts_paused_at_zero() {
    let (_directory, mut editor) = playback_editor();
    start_playback(&mut editor, None, true).unwrap();
    assert_eq!(editor.timeline().unwrap().tick, 0);
    assert!(!editor.timeline().unwrap().playing);
    assert!(editor.status().is_none_or(|message| !message.error));
}

#[test]
fn counted_playback_is_exact_and_default_playback_runs() {
    let (_directory, mut editor) = playback_editor();
    start_playback(&mut editor, Some(30), false).unwrap();
    assert_eq!(editor.timeline().unwrap().tick, 30);
    assert!(!editor.timeline().unwrap().playing);
    assert!(editor.status().is_none_or(|message| !message.error));
    editor.stop().unwrap();
    editor.sync();
    start_playback(&mut editor, None, false).unwrap();
    assert!(editor.timeline().unwrap().playing);
    editor.pause();
}

#[test]
fn default_and_bounded_player_options_parse() {
    let Request::Launch(defaults) = parse(&[]).expect("default launch options") else {
        panic!("default invocation must launch the player");
    };
    assert_eq!(defaults.frames, 30);
    assert_eq!(defaults.size, (960, 600));
    assert!(defaults.bundle.is_none());
    assert!(defaults.ticks.is_none());
    assert!(defaults.screenshot.is_none());

    let Request::Launch(options) = parse(&[
        "--bundle",
        "game bundle",
        "--ticks",
        "10000",
        "--screenshot",
        "capture.png",
        "--frames",
        "1",
        "--size",
        "1024x768",
    ])
    .expect("valid bounded launch options") else {
        panic!("valid arguments must launch the player");
    };
    assert_eq!(options.bundle.as_deref(), Some(Path::new("game bundle")));
    assert_eq!(options.ticks, Some(MAX_STARTUP_TICKS));
    assert_eq!(
        options.screenshot.as_deref(),
        Some(Path::new("capture.png"))
    );
    assert_eq!(options.frames, 1);
    assert_eq!(options.size, (1024, 768));

    assert!(matches!(
        parse(&["--capabilities"]).expect("capability request"),
        Request::Capabilities
    ));
    assert!(matches!(
        parse(&["--help"]).expect("help request"),
        Request::Help
    ));
}

#[test]
fn invalid_or_conflicting_player_options_are_rejected() {
    for arguments in [
        vec!["--unknown"],
        vec!["--bundle"],
        vec!["--bundle", ""],
        vec!["--bundle", "one", "--bundle", "two"],
        vec!["--ticks", "-1"],
        vec!["--ticks", "10001"],
        vec!["--ticks", "1", "--ticks", "2"],
        vec!["--screenshot"],
        vec!["--frames", "1"],
        vec!["--screenshot", "capture.png", "--frames", "0"],
        vec!["--screenshot", "capture.png", "--frames", "10001"],
        vec!["--size", "960"],
        vec!["--size", "319x600"],
        vec!["--size", "2049x600"],
        vec!["--size", "2048x600"],
        vec!["--size", "960x239"],
        vec!["--capabilities", "--bundle", "game"],
        vec!["--help", "--ticks", "1"],
    ] {
        assert!(
            parse(&arguments).is_err(),
            "accepted invalid arguments: {arguments:?}"
        );
    }

    assert!(u64::from(960_u32) * u64::from(600_u32) <= MAX_CAPTURE_PIXELS);
}

#[test]
fn capabilities_report_hazards_and_interactive_input_profiles() {
    assert_eq!(
        super::CAPABILITIES,
        "ORR_MESH_HAZARDS_PLAYER_V1\nORR_MESH_INPUT_V1"
    );
}

#[test]
fn screenshot_preflight_requires_existing_parent_and_outside_bundle() {
    let directory = TestDirectory::new();
    let bundle = directory.path().join("bundle");
    let outside = directory.path().join("captures");
    fs::create_dir(&bundle).expect("create bundle directory");
    fs::create_dir(&outside).expect("create capture directory");

    let accepted = screenshot_path(&outside.join("capture.png"), &bundle)
        .expect("new screenshot outside the bundle");
    assert_eq!(accepted, outside.join("capture.png"));
    assert!(screenshot_path(&bundle.join("capture.png"), &bundle).is_err());
    assert!(screenshot_path(&outside.join("missing/capture.png"), &bundle).is_err());
}

#[test]
fn screenshot_preflight_rejects_existing_files_and_directories() {
    let directory = TestDirectory::new();
    let bundle = directory.path().join("bundle");
    let outside = directory.path().join("captures");
    fs::create_dir(&bundle).expect("create bundle directory");
    fs::create_dir(&outside).expect("create capture directory");
    let existing_file = outside.join("existing.png");
    let existing_directory = outside.join("existing-directory");
    fs::write(&existing_file, b"preserve this file").expect("create existing file");
    fs::create_dir(&existing_directory).expect("create existing directory");

    assert!(screenshot_path(&existing_file, &bundle).is_err());
    assert!(screenshot_path(&existing_directory, &bundle).is_err());
    assert_eq!(
        fs::read(existing_file).expect("read preserved file"),
        b"preserve this file"
    );
}

#[test]
fn executable_identity_checks_platform_and_actual_bytes() {
    let dir = TestDirectory::new();
    let executable = dir.path().join("runtime");
    fs::write(&executable, b"test runtime bytes").unwrap();
    let native = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    };
    let mut bundle = crate::mesh_hazards_export::PreparedBundle {
        root: dir.path().to_owned(),
        scene: dir.path().join("scene"),
        runtime: executable.clone(),
        checksum: 0,
        graph_revision: [0; 32],
        runtime_sha256: super::sha256_file_bounded(&executable).unwrap(),
        platform: native,
    };
    super::verify_executable(&executable, &bundle).unwrap();
    fs::write(&executable, b"changed runtime").unwrap();
    assert!(
        super::verify_executable(&executable, &bundle)
            .unwrap_err()
            .contains("hash")
    );
    bundle.platform = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    };
    assert!(
        super::verify_executable(&executable, &bundle)
            .unwrap_err()
            .contains("platform")
    );
}

#[test]
fn fresh_session_rejects_legacy_profile_before_typed_hazard_access() {
    let dir = TestDirectory::new();
    let mut editor = crate::mesh_workspace::mission::prepare(
        &dir.path().join("legacy.scene.yaml"),
        &Default::default(),
    )
    .unwrap();
    assert!(editor.start_play());
    editor.sync();
    assert_eq!(editor.game(), EditorGame::MeshNavigation);
    assert!(crate::app::mesh_hazards_player::validate_fresh_session(&editor).is_err());
}
