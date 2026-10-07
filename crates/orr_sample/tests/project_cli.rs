//! Real standalone Arena argument/admission path, without a native window.
#![allow(clippy::disallowed_types)]
#[cfg(feature = "project")]
#[path = "common/project.rs"]
mod project_fixture;

use std::{
    path::Path,
    process::{Command, Output},
};
fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arena"))
        .args(args)
        .current_dir(cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap()
}
fn diagnostic(output: Output) -> String {
    assert!(!output.status.success());
    String::from_utf8(output.stderr).unwrap()
}
#[test]
fn conflicting_project_routes_reject_before_opening_a_missing_root() {
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    for option in [
        "--sprite-project",
        "--game-ui-project",
        "--save-input-bindings",
    ] {
        assert!(diagnostic(run(
            &["--project", "definitely-missing", option, "also-missing"],
            cwd
        ))
        .contains("cannot be combined"));
    }
    for (option, value) in [
        ("--connect", "localhost:1"),
        ("--latency", "0"),
        ("--jitter", "0"),
        ("--tau", "0"),
        ("--remote", "none"),
        ("--room", "1"),
        ("--slot", "0"),
    ] {
        assert!(
            diagnostic(run(
                &["--project", "definitely-missing", option, value],
                cwd
            ))
            .contains("cannot use relay/loopback/interpolation"),
            "{option}"
        );
    }
    assert!(diagnostic(run(&["--project", "missing", "--bot"], cwd)).contains("cannot use relay"));
    assert!(
        diagnostic(run(&["--project", "missing", "--headless"], cwd)).contains("requires --ticks")
    );
    assert!(diagnostic(run(&["--ticks", "1"], cwd)).contains("require --project --headless"));
    assert!(diagnostic(run(
        &["--project", "missing", "--headless", "--ticks", "6001"],
        cwd
    ))
    .contains("between 0 and 6000"));
    assert!(
        diagnostic(run(&["--project", "missing", "--project", "other"], cwd)).contains("only once")
    );
}
#[cfg(not(feature = "project"))]
#[test]
fn project_option_has_an_explicit_default_off_feature_guard() {
    assert!(diagnostic(run(
        &["--project", "missing"],
        Path::new(env!("CARGO_MANIFEST_DIR"))
    ))
    .contains("--features project"));
}
#[test]
fn optional_project_path_has_no_editor_remote_or_edit_production_dependency() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(manifest.contains("default = []"));
    assert!(manifest.contains("project = [\"sprites\", \"orr_reflect/scene\", \"dep:serde\", \"dep:serde_json\", \"dep:png17\"]"));
    let production = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[dev-dependencies]")
        .next()
        .unwrap();
    for forbidden in ["orr_editor =", "orr_remote =", "orr_edit ="] {
        assert!(!production.contains(forbidden), "{forbidden}");
    }
}
#[cfg(feature = "project")]
mod enabled {
    use super::*;
    use crate::project_fixture::ProjectFixture;
    use std::{fs, path::PathBuf};
    struct Fixture(ProjectFixture);
    impl Fixture {
        fn new() -> Self {
            let fixture = ProjectFixture::new();
            fs::create_dir(fixture.root.join("empty")).unwrap();
            Self(fixture)
        }
        fn project(&self) -> PathBuf {
            self.0.root.clone()
        }
        fn args(&self, held: &str) -> Output {
            run(
                &[
                    "--project",
                    self.project().to_str().unwrap(),
                    "--headless",
                    "--ticks",
                    "3",
                    "--hold",
                    held,
                ],
                &self.0.root.join("empty"),
            )
        }
    }
    #[test]
    fn headless_authored_scene_runs_without_a_window_and_relaunch_resets() {
        let fixture = Fixture::new();
        let scene = fs::read(fixture.project().join("arena.scene.yaml")).unwrap();
        let sprites = fs::read(fixture.project().join("arena.sprites.json")).unwrap();
        let a = fixture.args("right,fire");
        assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
        let b = fixture.args("right,fire");
        assert!(b.status.success());
        assert_eq!(a.stdout, b.stdout);
        let idle = fixture.args("idle");
        assert!(idle.status.success());
        assert_ne!(a.stdout, idle.stdout);
        let stdout = String::from_utf8(a.stdout).unwrap();
        assert!(stdout.contains("project tick: 3"));
        assert!(stdout.contains("e_00000001"));
        assert!(stdout.contains("e_00000002"));
        assert_eq!(
            fs::read(fixture.project().join("arena.scene.yaml")).unwrap(),
            scene
        );
        assert_eq!(
            fs::read(fixture.project().join("arena.sprites.json")).unwrap(),
            sprites
        );
    }
    #[test]
    fn missing_installed_assets_fail_even_in_cpu_headless_mode() {
        let fixture = Fixture::new();
        fs::remove_dir_all(fixture.project().join(".orr")).unwrap();
        let out = fixture.args("idle");
        assert!(!out.status.success());
        assert!(!String::from_utf8(out.stdout)
            .unwrap()
            .contains("initial checksum"));
        assert!(String::from_utf8(out.stderr).unwrap().contains("package"));
    }
    #[test]
    fn copied_standalone_binary_runs_without_repository_paths() {
        let fixture = Fixture::new();
        let expected = fixture.args("right,fire");
        assert!(expected.status.success());
        let executable = fixture.0.root.join("arena-relocated");
        fs::copy(env!("CARGO_BIN_EXE_arena"), &executable).unwrap();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let isolated = std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_some();
        let mut command = if isolated {
            assert_eq!(
                std::env::consts::OS,
                "linux",
                "repository-hidden acceptance needs Linux bubblewrap"
            );
            let mut c = Command::new("bwrap");
            c.args(["--ro-bind", "/", "/", "--bind"]).arg(&fixture.0.root).arg(&fixture.0.root)
                .arg("--tmpfs").arg(&repository)
                .args(["--", "/bin/sh", "-c", "test ! -e \"$1/Cargo.toml\" && test ! -e \"$1/crates/orr_sample\" && shift && exec \"$@\"", "sh"])
                .arg(&repository).arg(&executable);
            c
        } else {
            Command::new(&executable)
        };
        let output = command
            .args([
                "--project",
                fixture.project().to_str().unwrap(),
                "--headless",
                "--ticks",
                "3",
                "--hold",
                "right,fire",
            ])
            .current_dir(fixture.0.root.join("empty"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout, expected.stdout,
            "relocated executable must use the same admitted authored scene and input path"
        );
    }
}
