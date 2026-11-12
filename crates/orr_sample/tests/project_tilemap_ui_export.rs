//! Real CLI export and ordinary relocation, NOT hostile-source sandbox isolation.
#![cfg(all(
    feature = "project-create",
    feature = "project-export",
    feature = "tilemap",
    feature = "game-ui",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::float_arithmetic)]
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
                    "arena-2d-v1",
                    "--seed",
                    "tile-export-regression",
                    "--output",
                ])
                .arg(&project)
                .output()
                .unwrap(),
        );
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
#[ignore = "explicit real GPU UI plus tilemap relocated export acceptance"]
fn relocated_korean_ui_and_tilemap_both_paint_with_control_exports() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_UI_EXPORT_EVIDENCE").expect("evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let fixture = Fixture::new();
    let font = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        &fixture.project,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer.install(&[font]).unwrap();
    // Build this private authored combination only after installing dependencies.
    // Existing tilemap projects deliberately refuse arbitrary package mutations.
    let map = orr_tilemap::Document {
        version: 1,
        sprite: orr_tilemap::SpriteRef {
            package: "sample-sprites".into(),
            document: "sprites.json".into(),
        },
        width: 3,
        height: 2,
        tile_size: [16, 16],
        origin: [-200.0, 160.0],
        units_per_pixel: 1.0,
        cells: vec![Some(10), None, Some(21), Some(20), Some(11), None],
    };
    fs::write(
        fixture.project.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let manifest = fixture.project.join("orr.project.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["entry"]["tilemap"] = serde_json::json!("arena.tilemap.json");
    value["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut bundles = Vec::new();
    for name in ["ui-map", "no-ui-map", "ui-no-map"] {
        let source = fixture.temp.path().join(name);
        fs::create_dir(&source).unwrap();
        copy_tree(&fixture.project, &source);
        if name == "ui-no-map" {
            edit_manifest(&source);
        }
        if name == "no-ui-map" {
            let path = source.join("orr.project.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            v["entry"].as_object_mut().unwrap().remove("ui");
            fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let exported = fixture.temp.path().join(format!("{name}-export"));
        success(fixture.export(&source, &exported));
        let moved = fixture.temp.path().join(format!("{name}-relocated"));
        fs::rename(exported, &moved).unwrap();
        fs::rename(
            &source,
            fixture.temp.path().join(format!("{name}-source-hidden")),
        )
        .unwrap();
        assert!(!source.exists());
        bundles.push(moved);
    }
    fs::write(
        evidence.join("source-ui-project.json"),
        fs::read(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence.join("source-tilemap.json"),
        fs::read(fixture.project.join("arena.tilemap.json")).unwrap(),
    )
    .unwrap();
    fs::rename(
        &fixture.project,
        fixture.temp.path().join("original-project-hidden"),
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
    for (name, bundle) in ["ui-map", "no-ui-map", "ui-no-map"]
        .into_iter()
        .zip(bundles)
    {
        let path = evidence.join(format!("{name}.png"));
        let stdout = fixture.run(&bundle, Some(&path));
        assert!(stdout.contains("project capture adapter:"));
        states.push(
            state(&stdout)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        fs::write(evidence.join(format!("{name}.stdout")), stdout).unwrap();
        images.push(pixels(&path));
    }
    assert_eq!(states[0], states[1]);
    assert_eq!(states[0], states[2]);
    assert_eq!(states[0].len(), 2);
    let readable = |pixels: &[u8]| {
        (260..335)
            .flat_map(|y| (90..175).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let p = &pixels[(y * 512 + x) * 4..][..3];
                p.iter().all(|v| *v > 110) && p.iter().max().unwrap() - p.iter().min().unwrap() < 12
            })
            .count()
    };
    let labels = readable(&images[0]);
    assert!(labels > 100, "actual UI labels unreadable: {labels}");
    assert!(
        labels > readable(&images[1]) + 100,
        "UI-off control must not supply readable title rows"
    );
    let mut opaque = 0;
    let mut map_under_ui = 0;
    for y in 0..512 {
        for x in 0..512 {
            let wx = (x as f64 + 0.5 - 256.0) * 480.0 / 512.0 - 60.0;
            let wy = 240.0 - (y as f64 + 0.5) * 480.0 / 512.0;
            let tx = wx + 200.0;
            let ty = 160.0 - wy;
            if !(1.0..15.0).contains(&tx) || !(1.0..15.0).contains(&ty) {
                continue;
            }
            let at = (ty.floor() as usize * 64 + tx.floor() as usize) * 4;
            let expected = &ATLAS[at..at + 4];
            if expected[3] != 255 {
                continue;
            }
            let p = (y * 512 + x) * 4;
            assert!(
                images[1][p..p + 4]
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 3),
                "literal tile oracle failed ({x},{y})"
            );
            opaque += 1;
            // Existing UI may shade underlying content; compare like-for-like UI
            // controls rather than misidentifying its shading as wrong atlas data.
            if images[0][p..p + 3]
                .iter()
                .zip(&images[2][p..p + 3])
                .any(|(a, b)| a.abs_diff(*b) > 3)
            {
                map_under_ui += 1;
            }
        }
    }
    assert!(opaque >= 10, "insufficient literal atlas texels");
    assert!(
        map_under_ui >= 5,
        "tilemap vanished beneath actual UI: {map_under_ui}"
    );
    fs::write(evidence.join("oracle.txt"),format!("readable_title_pixels={labels}\nopaque_tile_pixels={opaque}\ntile_different_under_ui={map_under_ui}\nordinary relocation, not sandbox isolation; real installed Korean font\n")).unwrap();
}
