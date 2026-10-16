//! The persistent player profile belongs only to opted-in interactive Arena.
//! These subprocess checks never inherit the user's HOME or configuration.
#![allow(clippy::disallowed_types)]

use std::{fs, path::Path};

#[test]
fn player_settings_stays_opt_in_and_out_of_editor_and_simulation_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sample = fs::read_to_string(root.join("crates/orr_sample/Cargo.toml")).unwrap();
    let features = sample
        .split("[features]")
        .nth(1)
        .unwrap()
        .split("[dependencies]")
        .next()
        .unwrap();
    assert!(features.lines().any(|line| line == "default = []"));
    assert!(features
        .lines()
        .any(|line| line.starts_with("player-settings =")));
    for feature in ["project", "project-export", "game-ui", "input-actions"] {
        let declaration = features
            .lines()
            .find(|line| line.starts_with(&format!("{feature} =")))
            .unwrap();
        assert!(
            !declaration.contains("player-settings"),
            "{feature} must not silently opt into persistent player settings"
        );
    }
    let production = sample
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[dev-dependencies]")
        .next()
        .unwrap();
    for dependency in ["orr_editor =", "orr_remote =", "orr_edit ="] {
        assert!(!production.contains(dependency), "{dependency}");
    }
    for name in [
        "orr_editor",
        "orr_fp",
        "orr_ecs",
        "orr_sim",
        "orr_session",
        "orr_package",
    ] {
        let manifest = fs::read_to_string(root.join(format!("crates/{name}/Cargo.toml"))).unwrap();
        assert!(
            !manifest.contains("player-settings"),
            "{name} must not opt into the standalone player's writable profile"
        );
    }
}

#[cfg(not(feature = "player-settings"))]
#[test]
fn explicit_profile_directory_requires_the_default_off_feature() {
    for args in [
        [
            "--project",
            "/missing-player-settings-project",
            "--player-settings-dir",
            "/missing-player-settings-profile",
        ],
        [
            "--player-settings-dir",
            "/missing-player-settings-profile",
            "--project",
            "/missing-player-settings-project",
        ],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_arena"))
            .env_clear()
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("--features player-settings"), "{error}");
    }
}

#[cfg(all(feature = "player-settings", target_os = "linux"))]
#[path = "common/project.rs"]
mod project_fixture;

#[cfg(all(feature = "player-settings", target_os = "linux"))]
mod enabled {
    use super::*;
    use crate::project_fixture::ProjectFixture;
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        process::{Command, Output, Stdio},
        time::{Duration, Instant},
    };

    struct Fixture {
        scratch: tempfile::TempDir,
        project: ProjectFixture,
    }
    impl Fixture {
        fn new() -> Self {
            let scratch = tempfile::Builder::new()
                .prefix("orr player settings cli ")
                .tempdir()
                .unwrap();
            fs::create_dir(scratch.path().join("empty cwd")).unwrap();
            let project = ProjectFixture::new();
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
            let path = project.root.join("orr.project.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            manifest["entry"]["ui"] = serde_json::json!({
                "profile": "arena-korean-v1", "font": {
                    "package": "korean-game-ui", "asset": "OrreryKoreanUI.otf"
                }
            });
            fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            Self { scratch, project }
        }
        fn root(&self) -> &Path {
            self.scratch.path()
        }
        fn arena(&self) -> Command {
            let mut command = Command::new(env!("CARGO_BIN_EXE_arena"));
            command
                .env_clear()
                .current_dir(self.root().join("empty cwd"))
                .arg("--project")
                .arg(&self.project.root);
            command
        }
        fn fifo(&self) -> PathBuf {
            let path = self.root().join("profile directory must not be opened");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &path,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            path
        }
        fn assert_empty_cwd(&self) {
            assert_eq!(
                fs::read_dir(self.root().join("empty cwd")).unwrap().count(),
                0
            );
        }
    }

    fn run(mut command: Command) -> Output {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "subprocess timed out, possibly opening a settings FIFO: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().unwrap()
    }
    fn success(output: Output) -> Output {
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
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
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }

    #[test]
    fn headless_ignores_profile_discovery_and_explicit_invalid_paths() {
        let fixture = Fixture::new();
        let before = snapshot(&fixture.project.root);
        let mut baseline = fixture.arena();
        baseline.args(["--headless", "--ticks", "3", "--hold", "right,fire"]);
        let baseline = success(run(baseline));
        assert!(String::from_utf8_lossy(&baseline.stdout).contains("project tick: 3"));
        let fifo = fixture.fifo();
        let mut reordered = Command::new(env!("CARGO_BIN_EXE_arena"));
        reordered
            .env_clear()
            .current_dir(fixture.root().join("empty cwd"))
            .arg("--player-settings-dir")
            .arg(&fifo)
            .arg("--project")
            .arg(&fixture.project.root)
            .args(["--headless", "--ticks", "3", "--hold", "right,fire"]);
        assert_eq!(success(run(reordered)).stdout, baseline.stdout);
        for path in [
            fifo.as_path(),
            Path::new("relative-directory-must-not-be-resolved"),
        ] {
            let mut command = fixture.arena();
            command
                .env("HOME", &fifo)
                .env("XDG_CONFIG_HOME", &fifo)
                .args(["--headless", "--ticks", "3", "--hold", "right,fire"])
                .arg("--player-settings-dir")
                .arg(path);
            let output = success(run(command));
            assert_eq!(output.stdout, baseline.stdout);
            assert_eq!(
                output.stderr, baseline.stderr,
                "headless must not diagnose ignored profiles"
            );
        }
        let missing = fixture.root().join("must stay absent");
        let mut command = fixture.arena();
        command
            .env("HOME", &missing)
            .env("XDG_CONFIG_HOME", &missing)
            .args(["--headless", "--ticks", "0"])
            .arg("--player-settings-dir")
            .arg(&missing);
        success(run(command));
        assert!(!missing.exists(), "headless created profile directories");
        fixture.assert_empty_cwd();
        assert_eq!(snapshot(&fixture.project.root), before);
    }

    #[test]
    fn explicit_directory_requires_project_and_interactive_korean_ui() {
        let fixture = Fixture::new();
        let profile = fixture.root().join("profile must stay absent");
        let mut command = Command::new(env!("CARGO_BIN_EXE_arena"));
        command
            .env_clear()
            .current_dir(fixture.root().join("empty cwd"))
            .arg("--player-settings-dir")
            .arg(&profile);
        let output = run(command);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires --project"));

        let manifest_path = fixture.project.root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entry"].as_object_mut().unwrap().remove("ui");
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut command = fixture.arena();
        command.arg("--player-settings-dir").arg(&profile);
        let output = run(command);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("requires an authored Korean Arena UI"),
            "{error}"
        );
        assert!(!profile.exists());

        let fifo = fixture.fifo();
        let mut command = fixture.arena();
        command
            .env("HOME", &fifo)
            .env("XDG_CONFIG_HOME", &fifo)
            .args(["--bridge", "inproc", "--audio", "off"]);
        let output = run(command);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("event loop"), "{error}");
        assert!(
            !error.contains("player settings:"),
            "plain project should not discover a profile: {error}"
        );
        fixture.assert_empty_cwd();
    }

    #[test]
    fn capture_ignores_profile_and_writes_only_the_requested_image() {
        use orr_render::orr_rhi::{Wgpu, WgpuOptions};
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        });
        if gpu.is_err() {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required capture has no software GPU"
            );
            eprintln!(
                "software GPU unavailable; set ORR_REQUIRE_GPU=1 to require capture isolation"
            );
            return;
        }
        drop(gpu);
        let fixture = Fixture::new();
        let before = snapshot(&fixture.project.root);
        let fifo = fixture.fifo();
        let capture = fixture.root().join("requested capture.png");
        let mut command = fixture.arena();
        command
            .env("HOME", &fifo)
            .env("XDG_CONFIG_HOME", &fifo)
            .args(["--headless", "--ticks", "0"])
            .arg("--player-settings-dir")
            .arg(&fifo)
            .arg("--capture")
            .arg(&capture);
        success(run(command));
        assert!(fs::metadata(capture).unwrap().len() > 8);
        fixture.assert_empty_cwd();
        assert_eq!(snapshot(&fixture.project.root), before);
    }

    #[test]
    fn external_input_bindings_bypass_profile_before_window_creation() {
        let fixture = Fixture::new();
        let fifo = fixture.fifo();
        let input = fixture.root().join("session-only bindings.json");
        orr_sample::arena_input::default_map()
            .save_new(&input)
            .unwrap();
        let input_before = fs::read(&input).unwrap();
        let mut command = fixture.arena();
        command
            .env("HOME", &fifo)
            .env("XDG_CONFIG_HOME", &fifo)
            .args(["--bridge", "inproc", "--audio", "off"])
            .arg("--player-settings-dir")
            .arg(&fifo)
            .arg("--input-bindings")
            .arg(&input);
        let output = run(command);
        assert!(
            !output.status.success(),
            "env_clear must provide no native display"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("event loop"), "the session override must reach window creation without resolving the profile: {error}");
        assert!(
            error.contains("외부 입력 파일 사용 중"),
            "missing read-only external-session notice: {error}"
        );
        assert!(
            error.contains("설정을 저장하지 않습니다"),
            "external override must clearly disable persistence: {error}"
        );
        assert!(
            !error.contains("설정 저장 불가"),
            "external override tried to resolve an unavailable profile: {error}"
        );
        assert_eq!(fs::read(input).unwrap(), input_before);
        assert_eq!(
            fs::read_dir(fixture.root()).unwrap().count(),
            3,
            "only the input file, FIFO and empty cwd may exist"
        );
        fixture.assert_empty_cwd();
    }

    #[cfg(all(feature = "project-export", target_arch = "x86_64"))]
    #[test]
    fn exporter_smoke_runs_with_no_home_or_xdg_and_no_player_profile() {
        use sha2::{Digest, Sha256};
        let fixture = Fixture::new();
        let before = snapshot(&fixture.project.root);
        let runtime = Path::new(env!("CARGO_BIN_EXE_arena"));
        let hash: String = Sha256::digest(fs::read(runtime).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let output = fixture.root().join("exported with cleared environment");
        let mut command = Command::new(env!("CARGO_BIN_EXE_orr_export_arena"));
        command
            .env_clear()
            .current_dir(fixture.root().join("empty cwd"))
            .arg("--project")
            .arg(&fixture.project.root)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(hash)
            .arg("--output")
            .arg(&output)
            .arg("--trusted-runtime");
        success(run(command));
        let manifest: orr_sample::project_export::ExportManifest =
            serde_json::from_slice(&fs::read(output.join("orr.export.json")).unwrap()).unwrap();
        assert_eq!(
            manifest.payload.files.len(),
            17,
            "only 15 content files, runtime and launcher are exported"
        );
        assert!(manifest
            .payload
            .files
            .iter()
            .all(|file| !file.path.contains("player-settings")
                && !file.path.contains("arena-controls")));
        assert_eq!(snapshot(&fixture.project.root), before);
        fixture.assert_empty_cwd();
    }
}
