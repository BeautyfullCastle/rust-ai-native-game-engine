//! Real CLI export and ordinary relocation, NOT hostile-source sandbox isolation.
#![cfg(all(
    feature = "project-create",
    feature = "project-export",
    feature = "tilemap",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::float_arithmetic)]
use orr_sample::project_runtime::PreparedRuntime;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
#[allow(dead_code)]
#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::copy_tree;
const ATLAS: &[u8] = include_bytes!("../../../assets/sprite_demo/lantern_keeper.rgba");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
struct Fixture {
    temp: tempfile::TempDir,
    project: PathBuf,
    runtime: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("source project");
        success(
            Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
                .args([
                    "--template",
                    "arena-tilemap-2d-v1",
                    "--seed",
                    "tile-export-regression",
                    "--output",
                ])
                .arg(&project)
                .output()
                .unwrap(),
        );
        let mut map: orr_tilemap::Document =
            serde_json::from_slice(&fs::read(project.join("arena.tilemap.json")).unwrap()).unwrap();
        map.width = 3;
        map.height = 2;
        map.origin = [-200.0, 160.0];
        map.cells = vec![Some(10), None, Some(21), Some(20), Some(11), None];
        fs::write(
            project.join("arena.tilemap.json"),
            serde_json::to_vec(&map).unwrap(),
        )
        .unwrap();
        let runtime_dir = temp.path().join("supplied runtime");
        fs::create_dir(&runtime_dir).unwrap();
        let runtime = runtime_dir.join("arena");
        fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
        fs::create_dir(temp.path().join("empty cwd")).unwrap();
        Self {
            temp,
            project,
            runtime,
        }
    }
    fn export(&self, project: &Path, output: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_export_arena"))
            .arg("--project")
            .arg(project)
            .arg("--runtime")
            .arg(&self.runtime)
            .arg("--runtime-sha256")
            .arg(hash(&fs::read(&self.runtime).unwrap()))
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    }
    fn run(&self, bundle: &Path, capture: Option<&Path>) -> String {
        let mut command = Command::new(bundle.join("run-arena"));
        let (ticks, hold) = if capture.is_some() {
            ("0", "idle")
        } else {
            ("60", "right")
        };
        command
            .args(["--headless", "--ticks", ticks, "--hold", hold])
            .current_dir(self.temp.path().join("empty cwd"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        if let Some(path) = capture {
            command.arg("--capture").arg(path);
        }
        success(command.output().unwrap())
    }
}
fn state(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| {
            line.starts_with("project initial checksum:") || line.starts_with("project tick:")
        })
        .collect()
}
fn edit_manifest(root: &Path) {
    let path = root.join("orr.project.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["entry"].as_object_mut().unwrap().remove("tilemap");
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
#[test]
fn cli_exports_saved_tilemap_and_relocated_runtime_preserves_checksum() {
    let fixture = Fixture::new();
    let output = fixture.temp.path().join("export");
    let initial = PreparedRuntime::open(&fixture.project)
        .unwrap()
        .initial_frame()
        .checksum();
    let map = fs::read(fixture.project.join("arena.tilemap.json")).unwrap();
    success(fixture.export(&fixture.project, &output));
    let moved = fixture.temp.path().join("relocated game");
    fs::rename(&output, &moved).unwrap();
    assert_eq!(
        fs::read(moved.join("project/arena.tilemap.json")).unwrap(),
        map
    );
    fs::rename(
        &fixture.project,
        fixture
            .temp
            .path()
            .join("retained source not at original path"),
    )
    .unwrap();
    fs::rename(
        fixture.runtime.parent().unwrap(),
        fixture
            .temp
            .path()
            .join("retained runtime not at original path"),
    )
    .unwrap();
    assert!(!fixture.project.exists());
    assert!(!fixture.runtime.exists());
    assert!(!output.exists());
    assert_eq!(
        PreparedRuntime::open(moved.join("project"))
            .unwrap()
            .initial_frame()
            .checksum(),
        initial
    );
    let stdout = fixture.run(&moved, None);
    assert_eq!(state(&stdout).len(), 2);
    assert!(stdout.contains(&format!("project initial checksum: 0x{initial:016x}")));
}
#[test]
fn bad_tilemap_or_atlas_never_creates_export_or_overwrites_existing_output() {
    let fixture = Fixture::new();
    let output = fixture.temp.path().join("rejected");
    let map = fixture.project.join("arena.tilemap.json");
    let good = fs::read(&map).unwrap();
    for bytes in [b"not json".to_vec(), b"{}".to_vec()] {
        fs::write(&map, bytes).unwrap();
        assert!(!fixture.export(&fixture.project, &output).status.success());
        assert!(!output.exists());
    }
    fs::remove_file(&map).unwrap();
    assert!(!fixture.export(&fixture.project, &output).status.success());
    assert!(!output.exists());
    fs::write(&map, &good).unwrap();
    fn atlas(root: &Path) -> PathBuf {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                let found = atlas(&path);
                if found.exists() {
                    return found;
                }
            } else if path.file_name().unwrap() == "lantern_keeper.png" {
                return path;
            }
        }
        PathBuf::new()
    }
    let object = atlas(&fixture.project.join(".orr/packages/objects"));
    assert!(object.is_file());
    let bytes = fs::read(&object).unwrap();
    fs::write(&object, b"tampered atlas").unwrap();
    assert!(!fixture.export(&fixture.project, &output).status.success());
    assert!(!output.exists());
    fs::remove_file(&object).unwrap();
    assert!(!fixture.export(&fixture.project, &output).status.success());
    assert!(!output.exists());
    fs::write(&object, bytes).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"existing delivery").unwrap();
    assert!(!fixture.export(&fixture.project, &output).status.success());
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"existing delivery");
    assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
}
fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = png17::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!((info.width, info.height), (512, 512));
    assert_eq!(info.color_type, png17::ColorType::Rgba);
    data.truncate(info.buffer_size());
    data
}
#[test]
#[ignore = "explicit real headless GPU capture of relocated tilemap export"]
fn relocated_cli_capture_matches_literal_atlas_and_control_exports() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_EXPORT_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let fixture = Fixture::new();
    let mut bundles = Vec::new();
    for name in ["saved", "no-map", "wrong-region"] {
        let source = fixture.temp.path().join(name);
        fs::create_dir(&source).unwrap();
        copy_tree(&fixture.project, &source);
        if name == "no-map" {
            edit_manifest(&source);
        }
        if name == "wrong-region" {
            let p = source.join("arena.tilemap.json");
            let mut map: orr_tilemap::Document =
                serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            map.cells[0] = Some(11);
            fs::write(p, serde_json::to_vec(&map).unwrap()).unwrap();
        }
        let export = fixture.temp.path().join(format!("{name}-export"));
        success(fixture.export(&source, &export));
        let moved = fixture.temp.path().join(format!("{name}-relocated"));
        fs::rename(export, &moved).unwrap();
        fs::rename(
            &source,
            fixture.temp.path().join(format!("{name}-source-hidden")),
        )
        .unwrap();
        assert!(!source.exists());
        bundles.push(moved);
    }
    fs::write(
        evidence.join("source-tilemap.json"),
        fs::read(fixture.project.join("arena.tilemap.json")).unwrap(),
    )
    .unwrap();
    fs::rename(
        &fixture.project,
        fixture.temp.path().join("original-source-hidden"),
    )
    .unwrap();
    fs::rename(
        fixture.runtime.parent().unwrap(),
        fixture.temp.path().join("original-runtime-hidden"),
    )
    .unwrap();
    assert!(!fixture.project.exists());
    assert!(!fixture.runtime.exists());
    let mut images = Vec::new();
    let mut states = Vec::new();
    let mut stepped_states = Vec::new();
    for (name, bundle) in ["saved", "no-map", "wrong-region"].into_iter().zip(bundles) {
        let capture = evidence.join(format!("{name}.png"));
        let stdout = fixture.run(&bundle, Some(&capture));
        assert!(stdout.contains("project capture adapter:"));
        states.push(
            state(&stdout)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        fs::write(evidence.join(format!("{name}.stdout")), stdout).unwrap();
        images.push(pixels(&capture));
        let stepped = fixture.run(&bundle, None);
        assert!(stepped.contains("project tick: 60 checksum:"));
        stepped_states.push(
            state(&stepped)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        fs::write(
            evidence.join(format!("{name}-tick60-right.stdout")),
            stepped,
        )
        .unwrap();
    }
    assert_eq!(states[0], states[1]);
    assert_eq!(states[0], states[2]);
    assert_eq!(states[0].len(), 2);
    assert_eq!(stepped_states[0], stepped_states[1]);
    assert_eq!(stepped_states[0], stepped_states[2]);
    assert_eq!(stepped_states[0].len(), 2);
    // Literal default camera follow [-60,0], half-height 240, 512-square capture;
    // literal first tile [-200,160] and atlas first 16x16 region in 64-wide RGBA.
    // Only opaque interiors: no production tile UV/extraction helper is an oracle.
    let mut checked = 0;
    let mut distinguished = 0;
    for y in 0..512 {
        for x in 0..512 {
            let wx = (x as f64 + 0.5 - 256.0) * 480.0 / 512.0 - 60.0;
            let wy = 240.0 - (y as f64 + 0.5) * 480.0 / 512.0;
            let tx = wx + 200.0;
            let ty = 160.0 - wy;
            if !(1.0..15.0).contains(&tx) || !(1.0..15.0).contains(&ty) {
                continue;
            }
            let offset = (ty.floor() as usize * 64 + tx.floor() as usize) * 4;
            let expected = &ATLAS[offset..offset + 4];
            if expected[3] != 255 {
                continue;
            }
            let p = (y * 512 + x) * 4;
            let actual = &images[0][p..p + 4];
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 3),
                "tile texel ({x},{y}) {actual:?} != {expected:?}"
            );
            checked += 1;
            if images[1][p..p + 4] != images[0][p..p + 4]
                && images[2][p..p + 4] != images[0][p..p + 4]
            {
                distinguished += 1;
            }
        }
    }
    assert!(checked >= 10, "insufficient opaque texels {checked}");
    assert!(
        distinguished >= 5,
        "controls not discriminated {distinguished}"
    );
    fs::write(evidence.join("oracle.txt"),format!("opaque_checked={checked}\ncontrols_distinguished={distinguished}\nordinary owned-source rename; not sandbox isolation\n")).unwrap();
}

#[test]
#[ignore = "explicit feature-off exporter and runtime native admission regression"]
fn feature_off_exporter_and_runtime_reject_tilemap_without_publication() {
    let binary = |name: &str| {
        let path = PathBuf::from(
            std::env::var_os(name).unwrap_or_else(|| panic!("{name} actual binary required")),
        );
        assert!(
            path.is_absolute() && path.is_file(),
            "{name} must name an absolute actual binary"
        );
        path
    };
    let off_arena = binary("ORR_TILEMAP_FEATURE_OFF_ARENA");
    let off_exporter = binary("ORR_TILEMAP_FEATURE_OFF_EXPORTER");
    let fixture = Fixture::new();
    // Positive admission of this same fixture rules out using malformed content
    // as a substitute for the intended compiled-capability refusal.
    PreparedRuntime::open(&fixture.project).unwrap();
    for (label, exporter, runtime) in [
        (
            "feature-off-exporter",
            off_exporter.as_path(),
            fixture.runtime.as_path(),
        ),
        (
            "feature-off-runtime",
            Path::new(env!("CARGO_BIN_EXE_orr_export_arena")),
            off_arena.as_path(),
        ),
    ] {
        let output = fixture.temp.path().join(label);
        let result = Command::new(exporter)
            .arg("--project")
            .arg(&fixture.project)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(hash(&fs::read(runtime).unwrap()))
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(&output)
            .current_dir(fixture.temp.path().join("empty cwd"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "{label} accepted unsupported tilemap"
        );
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            diagnostic.to_ascii_lowercase().contains("tilemap"),
            "{label} must identify tilemap refusal, not unrelated failure: {diagnostic}"
        );
        assert!(!output.exists(), "{label} left a published output");
        eprintln!("{label}: {diagnostic}");
    }
}
