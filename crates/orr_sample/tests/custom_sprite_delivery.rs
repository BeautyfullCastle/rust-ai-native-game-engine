//! Real creator CLI and headless runtime coverage; no Editor UI or GPU claim.
#![cfg(all(
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::float_arithmetic)]

use orr_bridge::{Bridge, InProc, PlayHost, PlayerSlot};
use orr_fp::FP;
use orr_sample::{
    arena_game::ArenaInput,
    arena_view::arena_bridge_config,
    project_create::{
        create_custom_sprites, CreateOptions, CustomSpriteOptions, CUSTOM_SPRITE_TEMPLATE,
    },
    project_runtime::PreparedRuntime,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const PACKAGE: &str = "independent-hero";
const IDLE: &str = "rest-explicit";
const WALK: &str = "stride-explicit";

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(at).unwrap() {
            let e = e.unwrap();
            let path = e.path();
            let kind = e.file_type().unwrap();
            if kind.is_dir() {
                visit(root, &path, out);
            } else {
                let bytes = if kind.is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec()
                } else {
                    fs::read(&path).unwrap()
                };
                out.insert(path.strip_prefix(root).unwrap().into(), bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("external actor art");
        let output = temp.path().join("created game");
        fs::create_dir(&source).unwrap();
        let pixels: Vec<u8> = (0..12)
            .flat_map(|y| (0..32).flat_map(move |x| [x as u8 * 7, y as u8 * 19, 73, 255]))
            .collect();
        let mut bytes = Vec::new();
        {
            let mut encoder = png17::Encoder::new(&mut bytes, 32, 12);
            encoder.set_color(png17::ColorType::Rgba);
            encoder.set_depth(png17::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        fs::write(source.join("hero.png"), bytes).unwrap();
        fs::write(
            source.join("LICENSE.txt"),
            b"Original integration-test pixels, CC0-1.0\n",
        )
        .unwrap();
        write_json(
            &source.join("sprites.json"),
            &json!({"format":"orr_sprite","version":1,"atlas":{"image":"hero.png","width":32,"height":12},"regions":[{"id":31,"x":0,"y":0,"width":16,"height":12},{"id":47,"x":16,"y":0,"width":16,"height":12}],"clips":[{"id":IDLE,"mode":"loop","frames":[{"region":31,"duration_ms":16},{"region":47,"duration_ms":16}]},{"id":WALK,"mode":"loop","frames":[{"region":47,"duration_ms":16},{"region":31,"duration_ms":16}]}]}),
        );
        write_json(
            &source.join("orr.package.json"),
            &json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1","capabilities":["sprite"],"dependencies":{},"files":["sprites.json","hero.png","LICENSE.txt"]}),
        );
        Self {
            temp,
            source,
            output,
        }
    }
    fn args(&self, output: &Path) -> Vec<String> {
        vec![
            "--template".into(),
            CUSTOM_SPRITE_TEMPLATE.into(),
            "--seed".into(),
            "sprite-delivery".into(),
            "--output".into(),
            output.to_str().unwrap().into(),
            "--sprite-package".into(),
            self.source.to_str().unwrap().into(),
            "--idle-clip".into(),
            IDLE.into(),
            "--walk-clip".into(),
            WALK.into(),
            "--units-per-pixel".into(),
            "1.5".into(),
        ]
    }
    fn run(&self, args: &[String]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .args(args)
            .current_dir(self.temp.path())
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    }
    fn create(&self) {
        success(self.run(&self.args(&self.output)));
    }
    fn reject(&self, args: &[String]) {
        let before = files(self.temp.path());
        let output = self.run(args);
        assert!(!output.status.success(), "invalid input accepted: {args:?}");
        assert!(!self.output.exists());
        assert_eq!(files(self.temp.path()), before);
        assert!(!fs::read_dir(self.temp.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-create-")));
    }
}

#[test]
fn cli_is_reproducible_closed_and_preserves_baseline_gameplay_and_target() {
    let f = Fixture::new();
    let source = files(&f.source);
    f.create();
    assert_eq!(files(&f.source), source);
    let twin = f.temp.path().join("twin");
    success(f.run(&f.args(&twin)));
    assert_eq!(files(&f.output), files(&twin));
    let baseline = f.temp.path().join("baseline");
    success(f.run(&[
        "--template".into(),
        "arena-2d-v1".into(),
        "--seed".into(),
        "sprite-delivery".into(),
        "--output".into(),
        baseline.to_str().unwrap().into(),
    ]));
    assert_eq!(
        fs::read(f.output.join("arena.scene.yaml")).unwrap(),
        fs::read(baseline.join("arena.scene.yaml")).unwrap()
    );
    let sprites = read_json(&f.output.join("arena.sprites.json"));
    let original = read_json(&baseline.join("arena.sprites.json"));
    let hero = sprites["camera_follow"].as_str().unwrap();
    assert_eq!(sprites["camera_follow"], original["camera_follow"]);
    assert_eq!(sprites["bindings"].as_object().unwrap().len(), 2);
    for (guid, binding) in sprites["bindings"].as_object().unwrap() {
        if guid == hero {
            assert_eq!(binding["package"], PACKAGE);
            assert_eq!(
                binding["source"],
                json!({"Locomotion":{"idle":IDLE,"walk":WALK}})
            );
            assert_eq!(binding["units_per_pixel"], 1.5);
        } else {
            assert_eq!(binding, &original["bindings"][guid]);
        }
    }
    let lock = read_json(&f.output.join("orr.packages.lock.json"));
    assert_eq!(lock["direct"].as_object().unwrap().len(), 2);
    assert_eq!(lock["packages"].as_object().unwrap().len(), 2);
    let installed = f
        .output
        .join(".orr/packages/objects")
        .join(lock["packages"][PACKAGE]["digest"].as_str().unwrap());
    for name in ["sprites.json", "hero.png", "LICENSE.txt"] {
        assert_eq!(
            fs::read(installed.join(name)).unwrap(),
            source[Path::new(name)]
        );
    }
    fs::rename(&f.source, f.temp.path().join("source hidden")).unwrap();
    let custom = PreparedRuntime::open(&f.output).unwrap();
    let default = PreparedRuntime::open(&baseline).unwrap();
    assert_eq!(
        custom.initial_frame().checksum(),
        default.initial_frame().checksum()
    );
    let (session, mut presentation) = custom.into_parts().unwrap();
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    let mut baseline_bridge = default.bridge().unwrap();
    presentation
        .update(Some(&bridge.snapshot().unwrap()))
        .unwrap();
    assert_eq!(
        presentation
            .sprites()
            .iter()
            .filter(|s| s.instance.region == 31 && s.instance.size == [24.0, 18.0])
            .count(),
        1
    );
    // Idle playback advances to the second custom frame using real session snapshots.
    bridge.step(1);
    baseline_bridge.step(1);
    presentation
        .update(Some(&bridge.snapshot().unwrap()))
        .unwrap();
    assert_eq!(
        presentation
            .sprites()
            .iter()
            .filter(|s| s.instance.region == 47)
            .count(),
        1
    );
    for moving_tick in 0..4 {
        let input = ArenaInput::new(FP::from_int(1), FP::ZERO, true);
        bridge.set_input(PlayerSlot(0), input).unwrap();
        baseline_bridge.set_input(PlayerSlot(0), input).unwrap();
        bridge.step(1);
        baseline_bridge.step(1);
        let checksum = bridge.host().session().frame().checksum();
        presentation
            .update(Some(&bridge.snapshot().unwrap()))
            .unwrap();
        assert!(presentation.state(hero).moving);
        if moving_tick == 0 {
            assert_eq!(presentation.state(hero).elapsed_ms, 0);
            assert_eq!(
                presentation
                    .sprites()
                    .iter()
                    .filter(|sprite| sprite.instance.region == 47)
                    .count(),
                1,
                "the first moving update must use the explicitly selected walk clip"
            );
        }
        assert_eq!(checksum, bridge.host().session().frame().checksum());
        assert_eq!(
            checksum,
            baseline_bridge.host().session().frame().checksum()
        );
    }
    // Restore the source so no-overwrite rejection cannot be attributed to missing input.
    fs::rename(f.temp.path().join("source hidden"), &f.source).unwrap();
    let before = files(&f.output);
    assert!(!f.run(&f.args(&f.output)).status.success());
    assert_eq!(files(&f.output), before);
}

#[test]
fn same_clip_is_supported_by_cli_and_public_api() {
    let f = Fixture::new();
    let path = f.source.join("sprites.json");
    let mut document = read_json(&path);
    document["clips"]
        .as_array_mut()
        .unwrap()
        .retain(|clip| clip["id"] == IDLE);
    assert_eq!(document["clips"].as_array().unwrap().len(), 1);
    write_json(&path, &document);
    let mut args = f.args(&f.output);
    args[11] = IDLE.into();
    success(f.run(&args));
    let sprites = read_json(&f.output.join("arena.sprites.json"));
    let hero = sprites["camera_follow"].as_str().unwrap();
    assert_eq!(
        sprites["bindings"][hero]["source"],
        json!({"Locomotion":{"idle":IDLE,"walk":IDLE}})
    );
    let api_output = f.temp.path().join("api game");
    let report = create_custom_sprites(&CustomSpriteOptions {
        base: CreateOptions {
            output: api_output.clone(),
            template: CUSTOM_SPRITE_TEMPLATE.into(),
            seed: "sprite-delivery".into(),
        },
        package: f.source.clone(),
        idle: IDLE.into(),
        walk: IDLE.into(),
        units_per_pixel: 1.5,
    })
    .unwrap();
    assert_eq!(report.template, CUSTOM_SPRITE_TEMPLATE);
    assert_eq!(files(&f.output), files(&api_output));
    PreparedRuntime::open(&api_output).unwrap();
}

#[test]
fn invalid_cli_options_and_values_never_publish() {
    let f = Fixture::new();
    for option in [
        "--sprite-package",
        "--idle-clip",
        "--walk-clip",
        "--units-per-pixel",
    ] {
        let args = f.args(&f.output);
        let i = args.iter().position(|v| v == option).unwrap();
        let mut missing = args.clone();
        missing.drain(i..i + 2);
        f.reject(&missing);
        let mut duplicate = args.clone();
        duplicate.extend_from_slice(&args[i..i + 2]);
        f.reject(&duplicate);
        let mut missing_value = args.clone();
        missing_value.remove(i + 1);
        f.reject(&missing_value);
    }
    for (option, value) in [
        ("--sprite-package", "relative"),
        ("--idle-clip", "absent"),
        ("--walk-clip", "absent"),
        ("--idle-clip", ""),
        ("--walk-clip", ""),
    ] {
        let mut args = f.args(&f.output);
        let i = args.iter().position(|v| v == option).unwrap();
        args[i + 1] = value.into();
        f.reject(&args);
    }
    for value in [
        "0",
        "-1",
        "NaN",
        "inf",
        "-inf",
        "0.00001",
        "101",
        "not-a-number",
    ] {
        let mut args = f.args(&f.output);
        args[13] = value.into();
        f.reject(&args);
    }
    for template in ["arena-2d-v1", "arena-custom-tileset-2d-v1", "unknown"] {
        let mut args = f.args(&f.output);
        args[1] = template.into();
        f.reject(&args);
    }
    for (option, value) in [
        ("--tile-width", "16"),
        ("--game-id", "not-allowed"),
        ("--unknown", "value"),
    ] {
        let mut args = f.args(&f.output);
        args.extend([option.into(), value.into()]);
        f.reject(&args);
    }
}

#[test]
fn invalid_packages_are_rejected_without_source_changes_or_partial_output() {
    for case in [
        "malformed",
        "dependency",
        "capability",
        "collision",
        "extra",
        "missing-png",
        "bad-png",
        "wrong-dimensions",
        "missing-license",
        "empty-license",
        "oversize",
        "symlink",
        "root-symlink",
        "bad-clip",
    ] {
        let f = Fixture::new();
        match case {
            "malformed" => fs::write(f.source.join("orr.package.json"), b"not-json").unwrap(),
            "dependency" | "capability" | "collision" | "extra" => {
                let path = f.source.join("orr.package.json");
                let mut v = read_json(&path);
                match case {
                    "dependency" => v["dependencies"] = json!({"sample-sprites":"1.0.0"}),
                    "capability" => v["capabilities"] = json!(["sprite", "script"]),
                    "collision" => v["name"] = json!("sample-sprites"),
                    _ => {
                        v["files"].as_array_mut().unwrap().push(json!("extra.txt"));
                        fs::write(f.source.join("extra.txt"), b"extra").unwrap();
                    }
                }
                write_json(&path, &v);
            }
            "missing-png" => fs::remove_file(f.source.join("hero.png")).unwrap(),
            "bad-png" => fs::write(f.source.join("hero.png"), b"not-png").unwrap(),
            "wrong-dimensions" | "bad-clip" => {
                let path = f.source.join("sprites.json");
                let mut v = read_json(&path);
                if case == "wrong-dimensions" {
                    v["atlas"]["width"] = json!(33);
                } else {
                    v["clips"][0]["frames"][0]["region"] = json!(999);
                }
                write_json(&path, &v);
            }
            "missing-license" => fs::remove_file(f.source.join("LICENSE.txt")).unwrap(),
            "empty-license" => fs::write(f.source.join("LICENSE.txt"), b" \n").unwrap(),
            "oversize" => {
                fs::write(f.source.join("LICENSE.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap()
            }
            "symlink" => {
                let outside = f.temp.path().join("outside.png");
                fs::rename(f.source.join("hero.png"), &outside).unwrap();
                std::os::unix::fs::symlink(outside, f.source.join("hero.png")).unwrap();
            }
            "root-symlink" => {
                let outside = f.temp.path().join("real package");
                fs::rename(&f.source, &outside).unwrap();
                std::os::unix::fs::symlink(outside, &f.source).unwrap();
            }
            _ => unreachable!(),
        }
        f.reject(&f.args(&f.output));
    }
}

#[test]
fn existing_file_and_empty_directory_are_never_overwritten() {
    for directory in [false, true] {
        let f = Fixture::new();
        if directory {
            fs::create_dir(&f.output).unwrap();
        } else {
            fs::write(&f.output, b"existing destination").unwrap();
        }
        let before = files(f.temp.path());
        assert!(!f.run(&f.args(&f.output)).status.success());
        assert_eq!(files(f.temp.path()), before);
        assert_eq!(f.output.is_dir(), directory);
    }
}

#[test]
fn endpoint_scales_are_accepted_by_cli_and_runtime() {
    for (text, scale) in [("0.0001", 0.0001_f32), ("100", 100.0_f32)] {
        let f = Fixture::new();
        let mut args = f.args(&f.output);
        args[13] = text.into();
        success(f.run(&args));
        let sprites = read_json(&f.output.join("arena.sprites.json"));
        let hero = sprites["camera_follow"].as_str().unwrap();
        assert_eq!(
            sprites["bindings"][hero]["units_per_pixel"]
                .as_f64()
                .unwrap() as f32,
            scale
        );
        let (session, mut presentation) = PreparedRuntime::open(&f.output)
            .unwrap()
            .into_parts()
            .unwrap();
        let bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
        presentation
            .update(Some(&bridge.snapshot().unwrap()))
            .unwrap();
        let sprite = presentation
            .sprites()
            .iter()
            .find(|sprite| sprite.instance.region == 31)
            .unwrap();
        assert_eq!(sprite.instance.size, [16.0 * scale, 12.0 * scale]);
    }
}

#[test]
fn installed_png_tampering_is_rejected_on_reopen() {
    let f = Fixture::new();
    f.create();
    PreparedRuntime::open(&f.output).unwrap();
    let lock = read_json(&f.output.join("orr.packages.lock.json"));
    let installed = f
        .output
        .join(".orr/packages/objects")
        .join(lock["packages"][PACKAGE]["digest"].as_str().unwrap())
        .join("hero.png");
    let mut bytes = fs::read(&installed).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&installed, bytes).unwrap();
    assert!(PreparedRuntime::open(&f.output).is_err());
}

/// Actual exporter and CPU/headless runtime, not an Editor UI or GPU capture.
/// Moving owned sources proves path independence, not sandbox isolation.
#[cfg(feature = "project-export")]
#[test]
fn actual_export_runs_after_relocation_without_authoring_sources() {
    use sha2::{Digest, Sha256};

    let f = Fixture::new();
    f.create();
    let source_before = files(&f.source);
    let project_before = files(&f.output);
    let initial = PreparedRuntime::open(&f.output)
        .unwrap()
        .initial_frame()
        .checksum();
    let supplied = f.temp.path().join("supplied runtime");
    fs::create_dir(&supplied).unwrap();
    let runtime = supplied.join("arena");
    fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
    let digest: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let output = f.temp.path().join("export");
    success(
        Command::new(env!("CARGO_BIN_EXE_orr_export_arena"))
            .arg("--project")
            .arg(&f.output)
            .arg("--runtime")
            .arg(&runtime)
            .arg("--runtime-sha256")
            .arg(digest)
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap(),
    );
    assert_eq!(files(&f.source), source_before);
    assert_eq!(files(&f.output), project_before);
    let moved = f.temp.path().join("relocated export");
    fs::rename(&output, &moved).unwrap();
    let exported_project = moved.join("project");
    for name in [
        "arena.scene.yaml",
        "arena.sprites.json",
        "orr.packages.lock.json",
    ] {
        assert_eq!(
            fs::read(exported_project.join(name)).unwrap(),
            project_before[Path::new(name)]
        );
    }
    let lock = read_json(&exported_project.join("orr.packages.lock.json"));
    let object = exported_project
        .join(".orr/packages/objects")
        .join(lock["packages"][PACKAGE]["digest"].as_str().unwrap());
    for name in ["sprites.json", "hero.png", "LICENSE.txt"] {
        assert_eq!(
            fs::read(object.join(name)).unwrap(),
            source_before[Path::new(name)]
        );
    }
    let installed_manifest = object.join("orr.package.json");
    assert_eq!(
        fs::read(&installed_manifest).unwrap(),
        project_before[installed_manifest.strip_prefix(&exported_project).unwrap()]
    );
    fs::rename(&f.source, f.temp.path().join("package hidden")).unwrap();
    fs::rename(&f.output, f.temp.path().join("project hidden")).unwrap();
    fs::rename(&supplied, f.temp.path().join("runtime hidden")).unwrap();
    assert!(!output.exists() && !f.source.exists() && !f.output.exists() && !runtime.exists());
    assert_eq!(
        PreparedRuntime::open(&exported_project)
            .unwrap()
            .initial_frame()
            .checksum(),
        initial
    );
    let closure_before = files(&exported_project);
    let cwd = f.temp.path().join("unrelated empty cwd");
    fs::create_dir(&cwd).unwrap();
    let run = Command::new(moved.join("run-arena"))
        .args(["--headless", "--ticks", "60", "--hold", "right"])
        .current_dir(&cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    let text = String::from_utf8(run.stdout.clone()).unwrap();
    success(run);
    let states: Vec<_> = text
        .lines()
        .filter(|line| {
            line.starts_with("project initial checksum:") || line.starts_with("project tick:")
        })
        .collect();
    assert_eq!(states.len(), 2, "{text}");
    assert!(
        text.contains(&format!("project initial checksum: 0x{initial:016x}")),
        "{text}"
    );
    assert!(text.contains("project tick: 60 checksum:"), "{text}");
    assert_eq!(
        files(&exported_project),
        closure_before,
        "runtime changed the exported saved closure"
    );
}
