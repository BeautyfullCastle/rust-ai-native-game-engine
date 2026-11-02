//! Actual saved-scene CLI from an empty working directory. No sockets or editor.
#![cfg(all(feature = "authored", target_os = "linux"))]
use orr_navigation_view::{authored::AuthoredPlayer, player};
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");
struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    empty: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("authored");
        fs::create_dir_all(root.join("navigation")).unwrap();
        let scene = root.join("point.scene.yaml");
        fs::write(&scene, YAML).unwrap();
        fs::write(root.join("navigation/point_demo.orrt"), TERRAIN).unwrap();
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        Self {
            temp,
            root,
            scene,
            empty,
        }
    }
    fn command(&self, scene: &Path, ticks: u64) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_navigation_player"));
        command
            .current_dir(&self.empty)
            .arg("--scene")
            .arg(scene)
            .arg("--ticks")
            .arg(ticks.to_string());
        command
    }
}
fn receipt(result: Output) -> Value {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn parse(args: &[&str]) -> Result<Option<player::Options>, String> {
    player::parse(args.iter().map(OsString::from))
}
#[test]
fn bounded_arguments_and_closed_options() {
    for count in ["0", "100000"] {
        assert_eq!(
            parse(&["--scene", "scene.yaml", "--ticks", count])
                .unwrap()
                .unwrap()
                .ticks,
            count.parse::<u64>().unwrap()
        );
    }
    for count in ["-1", "100001", "18446744073709551616", "1.5", "+1", ""] {
        assert!(parse(&["--scene", "scene.yaml", "--ticks", count]).is_err());
    }
    for args in [
        vec![],
        vec!["--scene", "x"],
        vec!["--ticks", "1"],
        vec!["--scene", "x", "--ticks", "1", "--ticks", "2"],
        vec![
            "--scene",
            "x",
            "--ticks",
            "1",
            "--restart-check",
            "--restart-check",
        ],
        vec!["--scene", "x", "--ticks", "1", "--connect", "localhost:1"],
    ] {
        assert!(parse(&args).is_err());
    }
    assert!(parse(&["--help"]).unwrap().is_none());
    #[cfg(not(feature = "gpu"))]
    assert!(parse(&["--scene", "x", "--ticks", "0", "--capture", "new.png"]).is_err());
}
#[test]
fn public_run_cannot_bypass_argument_limits() {
    assert!(player::run(player::Options {
        scene: PathBuf::from("missing"),
        ticks: 100001,
        restart_check: false,
        capture: None
    })
    .unwrap_err()
    .contains("ticks"));
    assert!(player::run(player::Options {
        scene: PathBuf::new(),
        ticks: 0,
        restart_check: false,
        capture: None
    })
    .unwrap_err()
    .contains("scene path"));
    #[cfg(not(feature = "gpu"))]
    assert!(player::run(player::Options {
        scene: PathBuf::from("missing"),
        ticks: 0,
        restart_check: false,
        capture: Some(PathBuf::from("capture.png"))
    })
    .unwrap_err()
    .contains("gpu"));
}

#[test]
fn actual_cli_relocation_empty_cwd_restart_and_authoritative_checksum() {
    let f = Fixture::new();
    let mut expected = AuthoredPlayer::open(&f.scene).unwrap();
    for _ in 0..48 {
        expected.step().unwrap();
    }
    let original = receipt(
        f.command(&f.scene, 48)
            .arg("--restart-check")
            .output()
            .unwrap(),
    );
    assert_eq!(original["checksum"].as_u64(), Some(expected.checksum()));
    assert_eq!(original["tick"], 48);
    assert_eq!(original["restart_checked"], true);
    assert_eq!(original["point_agent_only"], true);
    assert_eq!(
        original["position_raw"],
        serde_json::json!(expected
            .view()
            .unwrap()
            .navigator
            .position()
            .map(|v| v.raw()))
    );
    assert_eq!(
        original["status"],
        format!("{:?}", expected.view().unwrap().navigator.status())
    );
    assert_eq!(
        original["terrain_revision"],
        original["graph_terrain_revision"]
    );
    assert_eq!(original["terrain_revision"].as_str().unwrap().len(), 64);
    assert_eq!(original["graph_revision"].as_str().unwrap().len(), 64);
    assert!(original["capture"].is_null());
    assert_eq!(fs::read(&f.scene).unwrap(), YAML.as_bytes());
    assert_eq!(
        fs::read(f.root.join("navigation/point_demo.orrt")).unwrap(),
        TERRAIN
    );
    let relocated = f.temp.path().join("relocated project");
    fs::rename(&f.root, &relocated).unwrap();
    let moved = receipt(
        f.command(&relocated.join("point.scene.yaml"), 48)
            .arg("--restart-check")
            .output()
            .unwrap(),
    );
    assert_eq!(moved, original);
    assert_eq!(
        fs::read_dir(&f.empty).unwrap().count(),
        0,
        "read-only run wrote to cwd"
    );
    assert_eq!(fs::read_dir(&relocated).unwrap().count(), 2);
}
#[test]
fn invalid_scene_and_usage_do_not_publish_success() {
    let f = Fixture::new();
    fs::write(&f.scene, "bad authored scene").unwrap();
    let result = f.command(&f.scene, 0).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let result = Command::new(env!("CARGO_BIN_EXE_navigation_player"))
        .current_dir(&f.empty)
        .args(["--scene", "missing", "--ticks", "100001"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert_eq!(fs::read(&f.scene).unwrap(), b"bad authored scene");
    assert_eq!(fs::read_dir(&f.empty).unwrap().count(), 0);
}
#[cfg(feature = "gpu")]
#[test]
fn capture_preflight_preserves_existing_files_and_rejects_parent_links_before_scene() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let missing = f.root.join("missing.yaml");
    for kind in ["file", "directory", "link"] {
        let output = f.temp.path().join(format!("{kind}.png"));
        match kind {
            "file" => fs::write(&output, b"keep").unwrap(),
            "directory" => fs::create_dir(&output).unwrap(),
            _ => symlink("absent", &output).unwrap(),
        }
        let result = f
            .command(&missing, 0)
            .arg("--capture")
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains("capture output already exists"));
        assert!(result.stdout.is_empty());
        match kind {
            "file" => assert_eq!(fs::read(&output).unwrap(), b"keep"),
            "directory" => assert!(output.is_dir()),
            _ => assert_eq!(fs::read_link(&output).unwrap(), Path::new("absent")),
        }
    }
    let alias = f.temp.path().join("alias");
    symlink(&f.empty, &alias).unwrap();
    let result = f
        .command(&missing, 0)
        .arg("--capture")
        .arg(alias.join("new.png"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("capture parent"));
    assert_eq!(fs::read_dir(&f.empty).unwrap().count(), 0);
}
#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires actual headless GPU/software adapter; no physical GPU or window claim"]
fn actual_offscreen_capture_preserves_authoritative_receipt() {
    let f = Fixture::new();
    let output = f.temp.path().join("capture.png");
    let plain = receipt(
        f.command(&f.scene, 4)
            .arg("--restart-check")
            .output()
            .unwrap(),
    );
    let captured = receipt(
        f.command(&f.scene, 4)
            .arg("--restart-check")
            .arg("--capture")
            .arg(&output)
            .output()
            .unwrap(),
    );
    for key in [
        "tick",
        "checksum",
        "position_raw",
        "status",
        "terrain_revision",
        "graph_revision",
    ] {
        assert_eq!(plain[key], captured[key]);
    }
    assert_eq!(captured["capture"]["width"], 1000);
    assert_eq!(captured["capture"]["height"], 750);
    assert_eq!(captured["capture"]["rgba_bytes"], 3_000_000);
    let data = fs::read(&output).unwrap();
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(data[16..20].try_into().unwrap()), 1000);
    assert_eq!(u32::from_be_bytes(data[20..24].try_into().unwrap()), 750);
    assert_eq!(fs::read(&f.scene).unwrap(), YAML.as_bytes());
    assert_eq!(
        fs::read(f.root.join("navigation/point_demo.orrt")).unwrap(),
        TERRAIN
    );
}
