//! Actual local trusted-runtime delivery. No window, GPU, network or portability claim.
#![cfg(all(
    feature = "authored-bundle",
    target_os = "linux",
    target_arch = "x86_64"
))]
use orr_navigation_view::{authored::AuthoredPlayer, player};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};
const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    scene: PathBuf,
    empty: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("navigation")).unwrap();
        let scene = source.join("game.scene.yaml");
        fs::write(&scene, YAML).unwrap();
        fs::write(source.join("navigation/point_demo.orrt"), TERRAIN).unwrap();
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        Self {
            temp,
            source,
            scene,
            empty,
        }
    }
    fn export(&self, output: &Path, runtime: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_navigation_export"));
        c.current_dir(&self.empty)
            .arg("--scene")
            .arg(&self.scene)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(hash(&fs::read(runtime).unwrap()))
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(output);
        c
    }
    fn bundle(&self) -> PathBuf {
        let out = self.temp.path().join("delivery");
        success(
            self.export(&out, Path::new(env!("CARGO_BIN_EXE_navigation_player")))
                .arg("--source-revision")
                .arg("user claim, not verified provenance")
                .output()
                .unwrap(),
        );
        out
    }
    fn launch(&self, root: &Path) -> Output {
        Command::new(root.join("run-navigation"))
            .current_dir(&self.empty)
            .args(["--ticks", "48", "--restart-check"])
            .output()
            .unwrap()
    }
}
fn success(out: Output) -> Output {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn failure(out: Output) {
    assert!(
        !out.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
#[test]
fn closed_bundle_arguments_and_direct_call_guards() {
    for args in [
        vec!["--bundle", "x", "--scene", "y", "--ticks", "1"],
        vec!["--bundle", "x", "--bundle", "y", "--ticks", "1"],
        vec!["--bundle", "x", "--ticks", "100001"],
        vec!["--bundle", "x", "--ticks", "0", "--capture", "x.png"],
        vec!["--capabilities", "--ticks", "0"],
    ] {
        assert!(player::parse_command(args.into_iter().map(OsString::from)).is_err());
    }
    let opts = player::Options {
        scene: "missing".into(),
        ticks: 0,
        restart_check: false,
        capture: Some("must-not-create.png".into()),
    };
    assert!(player::run_bundle(opts).unwrap_err().contains("capture"));
    let actual = success(
        Command::new(env!("CARGO_BIN_EXE_navigation_player"))
            .arg("--capabilities")
            .output()
            .unwrap(),
    );
    assert_eq!(
        actual.stdout,
        b"ORR_NAVIGATION_PLAYER_V1\nORR_NAVIGATION_BUNDLE_V1\n"
    );
}
#[test]
fn actual_export_relocated_launcher_replays_without_original_sources() {
    let f = Fixture::new();
    let mut expected = AuthoredPlayer::open(&f.scene).unwrap();
    for _ in 0..48 {
        expected.step().unwrap();
    }
    let bundle = f.bundle();
    let manifest = fs::read(bundle.join("navigation.bundle.json")).unwrap();
    let moved = f.temp.path().join("moved delivery");
    fs::rename(&bundle, &moved).unwrap();
    fs::rename(&f.source, f.temp.path().join("hidden-source")).unwrap();
    let out = success(f.launch(&moved));
    let receipt: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(receipt["tick"], 48);
    assert_eq!(receipt["checksum"].as_u64(), Some(expected.checksum()));
    assert_eq!(receipt["restart_checked"], true);
    assert_eq!(receipt["point_agent_only"], true);
    assert!(receipt["capture"].is_null());
    assert_eq!(
        manifest,
        fs::read(moved.join("navigation.bundle.json")).unwrap()
    );
    assert_eq!(fs::read_dir(&f.empty).unwrap().count(), 0);
    // The original build executable cannot impersonate the bundled executable.
    failure(
        Command::new(env!("CARGO_BIN_EXE_navigation_player"))
            .arg("--bundle")
            .arg(&moved)
            .args(["--ticks", "0"])
            .output()
            .unwrap(),
    );
}
#[test]
fn bundle_tampering_modes_inventory_and_symlinks_fail_closed() {
    let f = Fixture::new();
    let bundle = f.bundle();
    for path in [
        "project/game.scene.yaml",
        "project/navigation/point_demo.orrt",
    ] {
        let path = bundle.join(path);
        let original = fs::read(&path).unwrap();
        let mut bad = original.clone();
        bad.push(b' ');
        fs::write(&path, bad).unwrap();
        failure(f.launch(&bundle));
        fs::write(&path, original).unwrap();
    }
    let manifest_path = bundle.join("navigation.bundle.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let mut whitespace = manifest_bytes.clone();
    whitespace.push(b' ');
    fs::write(&manifest_path, whitespace).unwrap();
    success(f.launch(&bundle)); // Integrity binds canonical payload, not JSON whitespace.
    let mut changed: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let checksum = changed["payload"]["initial_checksum"].as_u64().unwrap();
    changed["payload"]["initial_checksum"] = Value::from(checksum ^ 1);
    fs::write(&manifest_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let rejected = f.launch(&bundle);
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("digest"));
    failure(rejected);
    fs::write(&manifest_path, manifest_bytes).unwrap();
    let terrain = bundle.join("project/navigation/point_demo.orrt");
    let mode = fs::metadata(&terrain).unwrap().permissions();
    fs::set_permissions(&terrain, fs::Permissions::from_mode(0o600)).unwrap();
    failure(f.launch(&bundle));
    fs::set_permissions(&terrain, mode).unwrap();
    let extra = bundle.join("unexpected");
    fs::write(&extra, b"not admitted").unwrap();
    failure(f.launch(&bundle));
    fs::remove_file(extra).unwrap();
    let original = fs::read(&terrain).unwrap();
    fs::remove_file(&terrain).unwrap();
    symlink(f.source.join("navigation/point_demo.orrt"), &terrain).unwrap();
    failure(f.launch(&bundle));
    fs::remove_file(&terrain).unwrap();
    fs::write(&terrain, original).unwrap();
    fs::set_permissions(&terrain, fs::Permissions::from_mode(0o644)).unwrap();
    success(f.launch(&bundle));
}
#[test]
fn export_rejects_existing_outputs_bad_digest_pin_and_overlap() {
    let f = Fixture::new();
    let runtime = Path::new(env!("CARGO_BIN_EXE_navigation_player"));
    for kind in ["file", "directory", "dangling"] {
        let path = f.temp.path().join(kind);
        match kind {
            "file" => fs::write(&path, b"keep").unwrap(),
            "directory" => fs::create_dir(&path).unwrap(),
            _ => symlink("missing", &path).unwrap(),
        };
        failure(f.export(&path, runtime).output().unwrap());
        match kind {
            "file" => assert_eq!(fs::read(&path).unwrap(), b"keep"),
            "directory" => assert_eq!(fs::read_dir(&path).unwrap().count(), 0),
            _ => assert_eq!(fs::read_link(&path).unwrap(), Path::new("missing")),
        };
    }
    let bad = f.temp.path().join("bad-digest");
    let mut command = Command::new(env!("CARGO_BIN_EXE_navigation_export"));
    command
        .arg("--scene")
        .arg(&f.scene)
        .arg("--runtime")
        .arg(runtime)
        .arg("--runtime-sha256")
        .arg("0".repeat(64))
        .arg("--trusted-runtime")
        .arg("--output")
        .arg(&bad);
    failure(command.output().unwrap());
    assert!(!bad.exists());
    let inside = f.source.join("delivery");
    failure(f.export(&inside, runtime).output().unwrap());
    assert!(!inside.exists());
    let hidden = f.temp.path().join(".orr");
    fs::create_dir(&hidden).unwrap();
    let out = hidden.join("delivery");
    success(f.export(&out, runtime).output().unwrap());
    success(f.launch(&out)); // Absolute ancestry is not a descriptor-relative path.
                             // Retain a syntactically valid YAML source, but break its exact admitted terrain revision pin.
    let altered = YAML; // Locate the literal revision by its admitted value, not YAML formatting.
    let player = AuthoredPlayer::open(&f.scene).unwrap();
    let revision = player.view().unwrap().terrain.revision();
    let revision = format!("{revision:?}");
    assert!(altered.contains(&revision));
    fs::write(
        &f.scene,
        altered.replace(&revision, &format!("{:?}", [0u8; 32])),
    )
    .unwrap();
    let out = f.temp.path().join("bad-pin");
    failure(f.export(&out, runtime).output().unwrap());
    assert!(!out.exists());
}
#[test]
#[ignore = "requires retained pre-bundle feature-off executable ORR_NAVIGATION_FEATURE_OFF_PLAYER"]
fn feature_off_runtime_is_not_admitted_even_with_matching_hash() {
    let f = Fixture::new();
    let runtime = PathBuf::from(
        std::env::var_os("ORR_NAVIGATION_FEATURE_OFF_PLAYER")
            .expect("retained feature-off runtime required"),
    );
    let capabilities = success(
        Command::new(&runtime)
            .arg("--capabilities")
            .output()
            .unwrap(),
    );
    assert_eq!(capabilities.stdout, b"ORR_NAVIGATION_PLAYER_V1\n");
    failure(
        Command::new(&runtime)
            .args(["--bundle", "missing", "--ticks", "0"])
            .output()
            .unwrap(),
    );
    let out = f.temp.path().join("feature-off");
    failure(f.export(&out, &runtime).output().unwrap());
    assert!(!out.exists());
}
