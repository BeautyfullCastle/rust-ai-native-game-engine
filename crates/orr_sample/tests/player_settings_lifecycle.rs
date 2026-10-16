//! Fresh-process persistence and relocated, read-only export evidence.
//!
//! The child harness uses the production store, admitted project and input
//! adapter. Actual widget dispatch and App restart are separate App unit tests;
//! this harness does not claim native-window mouse or keyboard automation.
#![cfg(all(feature = "player-settings", target_os = "linux"))]
#![allow(clippy::disallowed_types)]

use orr_input::{Button, Key};
use orr_sample::{
    arena_input::ArenaControls,
    player_controls::PlayerSettingsSession,
    player_settings::{FireBinding, PlayerSettingsV1, SettingsPaths},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

const CHILD_MODE: &str = "ORR_PLAYER_SETTINGS_TEST_CHILD";

fn run(mut command: Command) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "lifecycle child timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn child(executable: &Path, cwd: &Path, profile: &Path, mode: &str) -> Command {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .current_dir(cwd)
        .args(["--exact", "settings_process_child", "--nocapture"])
        .env(CHILD_MODE, mode)
        .env("ORR_PLAYER_SETTINGS_TEST_DIR", profile);
    command
}

fn assert_effective_binding(controls: &mut ArenaControls, binding: FireBinding) {
    let mouse = Button::Mouse { button: 0 };
    let space = Button::Keyboard { key: Key::Space };
    controls.button(mouse, true, false, false);
    assert_eq!(controls.keys().fire, binding == FireBinding::LeftMouse);
    controls.button(mouse, false, false, false);
    assert!(!controls.keys().fire);
    controls.button(space, true, false, false);
    assert_eq!(controls.keys().fire, binding == FireBinding::Space);
    controls.button(space, false, false, false);
    assert!(!controls.keys().fire);
}

#[test]
fn settings_process_child() {
    let Ok(mode) = std::env::var(CHILD_MODE) else {
        return;
    };
    if let Some(project) = std::env::var_os("ORR_PLAYER_SETTINGS_TEST_PROJECT") {
        let prepared = orr_sample::project_runtime::PreparedRuntime::open(project).unwrap();
        let (_, _, ui) = prepared.into_launch_parts();
        assert!(
            ui.is_some(),
            "the relocated project must retain its admitted Korean UI"
        );
    }
    if let Some(root) = std::env::var_os("ORR_PLAYER_SETTINGS_TEST_READ_ONLY_BUNDLE") {
        let probe = Path::new(&root).join("must-not-create-settings-here");
        let attempted = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe);
        if attempted.is_ok() {
            let _ = fs::remove_file(&probe);
        }
        assert!(
            attempted.is_err(),
            "the exported bundle must actually reject writes"
        );
    }
    for source in [
        "ORR_PLAYER_SETTINGS_TEST_HIDDEN",
        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_PROJECT",
        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_RUNTIME",
        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_BINARY",
    ] {
        if let Some(hidden) = std::env::var_os(source) {
            assert!(
                !Path::new(&hidden).exists(),
                "original source is still visible: {source}"
            );
        }
    }
    let paths = if std::env::var_os("ORR_PLAYER_SETTINGS_TEST_USE_XDG").is_some() {
        SettingsPaths::from_environment().unwrap()
    } else {
        SettingsPaths::from_directory(PathBuf::from(
            std::env::var_os("ORR_PLAYER_SETTINGS_TEST_DIR").unwrap(),
        ))
        .unwrap()
    };
    let mut session = PlayerSettingsSession::open(Ok(paths.clone()));
    assert!(
        session.editable(),
        "unexpected read-only profile: {}",
        session.status()
    );
    let expected = match mode.as_str() {
        "read-default" | "commit-left" => FireBinding::Space,
        "read-left" | "commit-space" => FireBinding::LeftMouse,
        "read-space" => FireBinding::Space,
        other => panic!("unknown child mode {other}"),
    };
    assert_eq!(session.active(), expected);
    let mut controls = ArenaControls::new(session.action_map()).unwrap();
    assert_effective_binding(&mut controls, expected);
    if mode == "read-default" {
        assert!(
            !paths.directory().exists(),
            "loading defaults must not create a profile"
        );
    }
    if mode.starts_with("commit-") {
        let binding = if mode == "commit-left" {
            FireBinding::LeftMouse
        } else {
            FireBinding::Space
        };
        session.select(binding);
        assert!(session.apply(&mut controls), "{}", session.status());
        assert!(
            session.editable(),
            "unexpected durability uncertainty: {}",
            session.status()
        );
        assert_eq!(session.active(), binding);
        assert_effective_binding(&mut controls, binding);
        // Exit without running Rust destructors. Publication must not depend on
        // normal shutdown, a parent-process cache, or Drop flushing user data.
        std::process::exit(0);
    }
}

#[test]
fn settings_survive_distinct_processes_and_no_destructor_shutdown() {
    let scratch = tempfile::Builder::new()
        .prefix("orr settings process ")
        .tempdir()
        .unwrap();
    let cwd = scratch.path().join("empty cwd");
    fs::create_dir(&cwd).unwrap();
    let profile = scratch.path().join("external profile");
    let executable = std::env::current_exe().unwrap();
    for mode in [
        "read-default",
        "commit-left",
        "read-left",
        "commit-space",
        "read-space",
    ] {
        run(child(&executable, &cwd, &profile, mode));
    }
    let paths = SettingsPaths::from_directory(&profile).unwrap();
    let primary: PlayerSettingsV1 =
        serde_json::from_slice(&fs::read(paths.primary()).unwrap()).unwrap();
    let backup: PlayerSettingsV1 =
        serde_json::from_slice(&fs::read(paths.backup()).unwrap()).unwrap();
    assert_eq!(primary.fire_binding, FireBinding::Space);
    assert_eq!(backup.fire_binding, FireBinding::LeftMouse);
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 0);
}

#[cfg(all(feature = "project-export", target_arch = "x86_64"))]
#[path = "common/project.rs"]
mod project_fixture;

#[cfg(all(feature = "project-export", target_arch = "x86_64"))]
mod exported {
    use super::*;
    use orr_sample::project_export::{export, ExportOptions};
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, io::Read, os::unix::fs::PermissionsExt};

    fn digest(path: &Path) -> String {
        let mut source = fs::File::open(path).unwrap();
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let read = source.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
        fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, String>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let kind = entry.file_type().unwrap();
                assert!(!kind.is_symlink());
                if kind.is_dir() {
                    visit(root, &entry.path(), result);
                } else {
                    assert!(kind.is_file());
                    result.insert(
                        entry.path().strip_prefix(root).unwrap().to_owned(),
                        digest(&entry.path()),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }
    fn permissions(root: &Path, read_only: bool) {
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                permissions(&entry.path(), read_only);
            } else {
                let executable = entry.metadata().unwrap().permissions().mode() & 0o111 != 0;
                fs::set_permissions(
                    entry.path(),
                    fs::Permissions::from_mode(match (read_only, executable) {
                        (true, true) => 0o555,
                        (true, false) => 0o444,
                        (false, true) => 0o755,
                        (false, false) => 0o644,
                    }),
                )
                .unwrap();
            }
        }
        fs::set_permissions(
            root,
            fs::Permissions::from_mode(if read_only { 0o555 } else { 0o755 }),
        )
        .unwrap();
    }
    struct ReadOnly(PathBuf);
    impl ReadOnly {
        fn new(root: PathBuf) -> Self {
            permissions(&root, true);
            Self(root)
        }
    }
    impl Drop for ReadOnly {
        fn drop(&mut self) {
            permissions(&self.0, false);
        }
    }

    struct Isolation {
        use_bwrap: bool,
        scratch: PathBuf,
        bundle: PathBuf,
        hidden_root: PathBuf,
        repository: PathBuf,
        project_source: PathBuf,
        runtime_source: PathBuf,
        original_binary: PathBuf,
    }
    impl Isolation {
        fn command(&self, executable: &Path) -> Command {
            let mut command = if self.use_bwrap {
                let mut command = Command::new("bwrap");
                command
                    .args(["--ro-bind", "/", "/", "--bind"])
                    .arg(&self.scratch)
                    .arg(&self.scratch)
                    .arg("--ro-bind")
                    .arg(&self.bundle)
                    .arg(&self.bundle)
                    .arg("--tmpfs")
                    .arg(&self.hidden_root)
                    .arg("--tmpfs")
                    .arg(&self.project_source)
                    .arg("--tmpfs")
                    .arg(&self.runtime_source);
                if !self.original_binary.starts_with(&self.hidden_root) {
                    command
                        .arg("--tmpfs")
                        .arg(self.original_binary.parent().unwrap());
                }
                command.arg("--").arg(executable);
                command
            } else {
                Command::new(executable)
            };
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .current_dir(self.scratch.join("empty cwd"));
            command
        }
        fn settings(&self, executable: &Path, mode: &str) -> Command {
            let mut command = self.command(executable);
            command
                .args(["--exact", "settings_process_child", "--nocapture"])
                .env(CHILD_MODE, mode)
                .env("ORR_PLAYER_SETTINGS_TEST_USE_XDG", "1")
                .env("XDG_CONFIG_HOME", self.scratch.join("external config"))
                .env(
                    "ORR_PLAYER_SETTINGS_TEST_PROJECT",
                    self.bundle.join("project"),
                )
                .env("ORR_PLAYER_SETTINGS_TEST_READ_ONLY_BUNDLE", &self.bundle);
            if self.use_bwrap {
                command
                    .env(
                        "ORR_PLAYER_SETTINGS_TEST_HIDDEN",
                        self.repository.join("Cargo.toml"),
                    )
                    .env(
                        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_PROJECT",
                        self.project_source.join("orr.project.json"),
                    )
                    .env(
                        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_RUNTIME",
                        self.runtime_source.join("arena"),
                    )
                    .env(
                        "ORR_PLAYER_SETTINGS_TEST_HIDDEN_BINARY",
                        &self.original_binary,
                    );
            }
            command
        }
    }

    #[test]
    fn relocated_read_only_export_uses_external_shared_profile_without_bundle_changes() {
        let scratch = tempfile::Builder::new()
            .prefix("orr settings relocated ")
            .tempdir()
            .unwrap();
        let scratch_path = scratch.path().canonicalize().unwrap();
        fs::create_dir(scratch_path.join("empty cwd")).unwrap();
        fs::create_dir(scratch_path.join("supplied runtime")).unwrap();
        let mut project = project_fixture::ProjectFixture::new();
        let project_source = scratch_path.join("original project");
        fs::rename(&project.root, &project_source).unwrap();
        project.root = project_source.clone();
        let package = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap();
        orr_package::Project::open_for_install(
            &project.root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[package])
        .unwrap();
        let manifest_path = project.root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entry"]["ui"] = serde_json::json!({"profile": "arena-korean-v1", "font": {
            "package": "korean-game-ui", "asset": "OrreryKoreanUI.otf"
        }});
        fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let source_before = snapshot(&project.root);
        let runtime = scratch_path.join("supplied runtime/arena");
        fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
        let exported = export(&ExportOptions {
            project: project.root.clone(),
            runtime: runtime.clone(),
            runtime_sha256: digest(&runtime),
            output: scratch_path.join("original bundle"),
            trusted_runtime: true,
            source_revision: Some("declared-player-settings-lifecycle-test".into()),
        })
        .unwrap();
        let relocated = scratch_path.join("relocated read-only game");
        fs::rename(&exported.output, &relocated).unwrap();
        assert!(!exported.output.exists());
        let bundle_before = snapshot(&relocated);
        let _read_only = ReadOnly::new(relocated.clone());
        let executable = scratch_path.join("relocated lifecycle test harness");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let hidden_root = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.clone())
            .canonicalize()
            .unwrap();
        assert!(repository.starts_with(&hidden_root));
        assert!(!scratch_path.starts_with(&hidden_root));
        let use_bwrap = Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--", "/bin/true"])
            .output()
            .is_ok_and(|output| output.status.success());
        assert!(
            std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_none() || use_bwrap,
            "ORR_REQUIRE_PROJECT_ISOLATION=1 requires functional bwrap source hiding"
        );
        if !use_bwrap {
            eprintln!("bwrap unavailable: relocated read-only subprocess checks run without source hiding");
        }
        let isolation = Isolation {
            use_bwrap,
            scratch: scratch_path.clone(),
            bundle: relocated.clone(),
            hidden_root,
            repository,
            project_source,
            runtime_source: runtime.parent().unwrap().into(),
            original_binary: Path::new(env!("CARGO_BIN_EXE_arena"))
                .canonicalize()
                .unwrap(),
        };
        for mode in ["read-default", "commit-left", "read-left"] {
            run(isolation.settings(&executable, mode));
        }
        let profile = SettingsPaths::from_directory(
            scratch_path.join("external config/orrery/arena-controls-v1"),
        )
        .unwrap();
        let persisted = fs::read(profile.primary()).unwrap();
        let mut previous_stdout = None;
        for _ in 0..2 {
            let mut command = isolation.command(&relocated.join("run-arena"));
            command
                .args(["--headless", "--ticks", "3", "--hold", "fire"])
                .env("XDG_CONFIG_HOME", scratch_path.join("external config"));
            let output = run(command);
            assert!(String::from_utf8_lossy(&output.stdout).contains("project tick: 3"));
            if let Some(previous) = previous_stdout.replace(output.stdout) {
                assert_eq!(
                    previous_stdout.as_ref().unwrap(),
                    &previous,
                    "relaunch must retain the same deterministic headless output"
                );
            }
        }
        run(isolation.settings(&executable, "read-left"));
        assert_eq!(
            fs::read(profile.primary()).unwrap(),
            persisted,
            "headless export launch altered the external profile"
        );
        assert_eq!(
            snapshot(&relocated),
            bundle_before,
            "a bundle byte changed during external settings persistence"
        );
        assert_eq!(snapshot(&project.root), source_before);
        assert_eq!(
            fs::read_dir(scratch_path.join("empty cwd"))
                .unwrap()
                .count(),
            0
        );
        eprintln!("fresh-process LeftMouse profile retained outside read-only relocated bundle; all bundle bytes unchanged; source_hiding={use_bwrap}");
    }
}
