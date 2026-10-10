//! Optional orientation consumer acceptance. This fixture module is also used by
//! the actual-editor target; runtime tests below only compile in orr_sample.
#![cfg(feature = "project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/sprite_orientation.rs"]
mod fixture;

#[cfg(feature = "project")]
mod tests {
    use super::fixture::*;
    use orr_bridge::{Bridge, SimControl};
    use orr_sample::project_sprites::{Document, Orientation};
    use std::fs;

    fn descriptor(version: u8, orientation: &str) -> String {
        format!(
            r#"{{"version":{version},"scene":"scene.yaml","project":".","bindings":{{"e_00000001":{{"package":"test","document":"sprites.json","source":{{"Region":7}},"units_per_pixel":1{orientation}}}}}}}"#
        )
    }
    #[test]
    fn strict_schema_rejects_old_version_presence_and_malformed_orientation() {
        for version in [1, 2] {
            let legacy = Document::from_bytes(descriptor(version, "").as_bytes()).unwrap();
            assert!(!serde_json::to_string(&legacy)
                .unwrap()
                .contains("orientation"));
            for value in [
                r#"{"quarter_turns":0,"flip_x":false,"flip_y":false}"#,
                "null",
            ] {
                assert!(Document::from_bytes(
                    descriptor(version, &format!(",\"orientation\":{value}")).as_bytes()
                )
                .is_err());
            }
        }
        for value in [
            "null",
            "[]",
            "[0,false,false]",
            "{}",
            "false",
            "0",
            r#"{"quarter_turns":0,"flip_x":false}"#,
            r#"{"quarter_turns":4,"flip_x":false,"flip_y":false}"#,
            r#"{"quarter_turns":-1,"flip_x":false,"flip_y":false}"#,
            r#"{"quarter_turns":1.0,"flip_x":false,"flip_y":false}"#,
            r#"{"quarter_turns":0,"flip_x":0,"flip_y":false}"#,
            r#"{"quarter_turns":0,"flip_x":false,"flip_y":null}"#,
            r#"{"quarter_turns":0,"flip_x":false,"flip_y":false,"tint":[1,1,1,1]}"#,
            r#"{"quarter_turns":0,"quarter_turns":1,"flip_x":false,"flip_y":false}"#,
            r#"{"quarter_turns":0,"flip_x":false,"flip_x":true,"flip_y":false}"#,
            r#"{"quarter_turns":0,"flip_x":false,"flip_y":false,"flip_y":true}"#,
        ] {
            let bytes = descriptor(3, &format!(",\"orientation\":{value}"));
            assert!(Document::from_bytes(bytes.as_bytes()).is_err(), "{value}");
        }
        for orientation in combinations() {
            let value = serde_json::to_string(&orientation).unwrap();
            let original = Document::from_bytes(
                descriptor(3, &format!(",\"orientation\":{value}")).as_bytes(),
            )
            .unwrap();
            assert_eq!(
                original.bindings["e_00000001"].orientation,
                Some(orientation)
            );
            assert_eq!(
                Document::from_bytes(&serde_json::to_vec(&original).unwrap()).unwrap(),
                original
            );
            let duplicate = descriptor(
                3,
                &format!(",\"orientation\":{value},\"orientation\":{value}"),
            );
            assert!(Document::from_bytes(duplicate.as_bytes()).is_err());
        }
        let mut programmatic = Document::from_bytes(descriptor(3, "").as_bytes()).unwrap();
        programmatic
            .bindings
            .get_mut("e_00000001")
            .unwrap()
            .orientation = Some(Orientation {
            quarter_turns: 255,
            ..Default::default()
        });
        assert!(programmatic.validate().is_err());
    }
    #[test]
    fn sixteen_authored_combinations_have_eight_distinct_geometric_transforms() {
        let mut signatures = std::collections::BTreeSet::new();
        for orientation in combinations() {
            let equivalent = Orientation {
                quarter_turns: (orientation.quarter_turns + 2) % 4,
                flip_x: !orientation.flip_x,
                flip_y: !orientation.flip_y,
            };
            let mut signature = Vec::new();
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let at = texel_offset(x, y, orientation);
                    assert_eq!(at, texel_offset(x, y, equivalent));
                    signature.push([at[0] as i32, at[1] as i32]);
                }
            }
            signatures.insert(signature);
        }
        assert_eq!(signatures.len(), 8);
    }
    #[test]
    fn arena_runtime_preserves_frame_and_owned_orientation_through_pause_seek_restart() {
        for orientation in combinations() {
            let fixture = Fixture::new(false);
            let legacy =
                orr_sample::project_runtime::PreparedRuntime::open(&fixture.project).unwrap();
            let initial = legacy.initial_frame().to_bytes();
            let initial_checksum = legacy.initial_frame().checksum();
            fixture.orientation(Some(orientation));
            let project =
                orr_sample::project_runtime::PreparedRuntime::open(&fixture.project).unwrap();
            assert_eq!(project.initial_frame().to_bytes(), initial);
            let (seed, mut presentation, _) = project.into_launch_parts();
            let mut bridge = seed.bridge().unwrap();
            let before = snapshot(&fixture.project);
            bridge.step(36);
            for tick in [0, 18, 18, 0, 3] {
                bridge.control(orr_session::ControlOp::Seek(tick)).unwrap();
                presentation.update(bridge.snapshot().as_ref()).unwrap();
                let draw = &presentation.sprites()[0].instance;
                assert_eq!(
                    (draw.flip_x, draw.flip_y),
                    (orientation.flip_x, orientation.flip_y)
                );
                assert_eq!(draw.rotation, orientation.radians());
                assert_eq!(draw.tint, [1.0; 4]);
                assert_eq!(presentation.sprites()[1].instance.rotation, 0.0);
                assert!(!presentation.sprites()[1].instance.flip_x);
                let checksum = bridge.snapshot().unwrap().predicted().checksum();
                presentation.update(bridge.snapshot().as_ref()).unwrap();
                assert_eq!(bridge.snapshot().unwrap().predicted().checksum(), checksum);
            }
            assert_eq!(snapshot(&fixture.project), before);
            fs::remove_dir_all(&fixture.project).unwrap();
            presentation.reset();
            let bridge = seed.bridge().unwrap();
            presentation.update(bridge.snapshot().as_ref()).unwrap();
            assert_eq!(
                presentation.sprites()[0].instance.rotation,
                orientation.radians()
            );
            assert_eq!(
                bridge.snapshot().unwrap().predicted().checksum(),
                initial_checksum
            );
        }
    }
    #[cfg(feature = "collect-sprites")]
    #[test]
    fn collect_orientation_does_not_change_gameplay_collection_or_restart() {
        use orr_bridge::{BridgeConfig, InProc, PlayHost, PlayerSlot};
        use orr_sample::collect_game::{CollectInput, CollectRun, RESTART, WON};
        use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
        let fixture = Fixture::new(true);
        let open = || {
            PreparedProject::open_with_presentation(
                &fixture.project,
                ProgressSupport::MetadataOnly,
                SpriteSupport::Supported,
            )
            .unwrap()
        };
        let legacy = open();
        let before = legacy.scene().frame().to_bytes();
        let orientation = Orientation {
            quarter_turns: 1,
            flip_x: true,
            flip_y: false,
        };
        fixture.orientation(Some(orientation));
        let project = open();
        assert_eq!(project.scene().frame().to_bytes(), before);
        let mut presentation = project.presentation();
        let mut baseline = InProc::new(
            PlayHost::new(legacy.scene().session().unwrap(), PlayerSlot(0)),
            BridgeConfig::default(),
        );
        let mut bridge = InProc::new(
            PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
            BridgeConfig::default(),
        );
        fs::remove_dir_all(&fixture.project).unwrap();
        for step in 0..122 {
            let input = if step == 121 {
                CollectInput {
                    buttons: RESTART,
                    ..Default::default()
                }
            } else {
                CollectInput {
                    x: orr_fp::FP::ONE,
                    ..Default::default()
                }
            };
            baseline.set_input(PlayerSlot(0), input).unwrap();
            bridge.set_input(PlayerSlot(0), input).unwrap();
            baseline.step(1);
            bridge.step(1);
            assert_eq!(
                bridge.snapshot().unwrap().predicted().checksum(),
                baseline.snapshot().unwrap().predicted().checksum()
            );
            presentation.update(bridge.snapshot().as_ref()).unwrap();
            assert_eq!(
                presentation.sprites()[0].instance.rotation,
                orientation.radians()
            );
            assert!(presentation.sprites()[0].instance.flip_x);
            if step == 120 {
                assert_eq!(
                    bridge
                        .snapshot()
                        .unwrap()
                        .predicted()
                        .singleton::<CollectRun>()
                        .phase,
                    WON
                );
                assert_eq!(presentation.sprites().len(), 1);
            }
        }
        assert_eq!(presentation.sprites().len(), 2);
    }
    #[test]
    #[ignore = "mandatory installed non-square atlas runtime software-GPU proof"]
    fn all_orientations_render_asymmetric_non_square_runtime_texels() {
        use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
        let gpu = Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        })
        .expect("mandatory software GPU");
        assert!(gpu.is_software());
        eprintln!("orientation runtime adapter: {}", gpu.adapter_name());
        let size = (960, 640);
        let target =
            orr_render::OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
        for orientation in combinations() {
            let fixture = Fixture::new(false);
            fixture.orientation(Some(orientation));
            let project =
                orr_sample::project_runtime::PreparedRuntime::open(&fixture.project).unwrap();
            let (seed, mut p, _) = project.into_launch_parts();
            p.update(seed.bridge().unwrap().snapshot().as_ref())
                .unwrap();
            let mut compositor = orr_sample::project_compositor::ProjectCompositor::new(
                gpu.clone(),
                TextureFormat::Rgba8UnormSrgb,
                p.assets(),
            )
            .unwrap();
            fs::remove_dir_all(&fixture.project).unwrap();
            compositor
                .draw(
                    target.render_view(),
                    size,
                    p.shapes(),
                    p.sprites(),
                    &p.camera,
                )
                .unwrap();
            let rgba = target.read_rgba8();
            assert_texels(orientation, fixture.scale, [-60.0, 0.0], false, |world| {
                let screen = p.camera.world_to_screen(world, size);
                let at =
                    ((screen[1].floor() as u32 * size.0 + screen[0].floor() as u32) * 4) as usize;
                rgba[at..at + 4].try_into().unwrap()
            });
            assert_texels(
                Orientation::default(),
                fixture.scale,
                [60.0, 0.0],
                false,
                |world| {
                    let screen = p.camera.world_to_screen(world, size);
                    let at = ((screen[1].floor() as u32 * size.0 + screen[0].floor() as u32) * 4)
                        as usize;
                    rgba[at..at + 4].try_into().unwrap()
                },
            );
            if let Some(path) = std::env::var_os("ORR_ORIENTATION_CAPTURE_DIR") {
                let path = std::path::PathBuf::from(path);
                fs::create_dir_all(&path).unwrap();
                let file = fs::File::create(path.join(format!(
                    "runtime-{}-{}-{}.png",
                    orientation.quarter_turns, orientation.flip_x, orientation.flip_y
                )))
                .unwrap();
                let mut encoder = png::Encoder::new(file, size.0, size.1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&rgba)
                    .unwrap();
            }
        }
    }
}

#[cfg(all(
    feature = "image-reimport",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[test]
fn reimport_preserves_exact_v3_sidecar_and_immutable_original_package() {
    use fixture::*;
    use orr_sample::{
        image_reimport::{prepare, Consumer, Options},
        project_sprites::Orientation,
    };
    use std::fs;
    for collect in [false, true] {
        let fixture = Fixture::new(collect);
        let orientation = Orientation {
            quarter_turns: 3,
            flip_x: true,
            flip_y: false,
        };
        fixture.orientation(Some(orientation));
        let sidecar = fs::read(fixture.project.join("view.json")).unwrap();
        let scene = fs::read(fixture.project.join("scene.yaml")).unwrap();
        let source = snapshot(&fixture.source);
        let replacement = fixture.temp.path().join("replacement.png");
        write_png(&replacement, &pixels(true));
        let transaction = prepare(&Options {
            project: fixture.project.clone(),
            package: PACKAGE.into(),
            document: "sprites.json".into(),
            image: replacement,
            version: "1.0.1".into(),
            consumer: Consumer { collect, ui: false },
        })
        .unwrap();
        assert_eq!(
            transaction.document().bindings["e_00000001"].orientation,
            Some(orientation)
        );
        let old_objects = snapshot(&fixture.project.join(".orr/packages/objects"));
        transaction.commit().unwrap();
        assert_eq!(
            fs::read(fixture.project.join("view.json")).unwrap(),
            sidecar
        );
        assert_eq!(fs::read(fixture.project.join("scene.yaml")).unwrap(), scene);
        assert_eq!(snapshot(&fixture.source), source);
        let objects = snapshot(&fixture.project.join(".orr/packages/objects"));
        for (path, bytes) in old_objects {
            assert_eq!(objects[&path], bytes);
        }
        let doc = orr_sample::project_sprites::Document::from_bytes(&sidecar).unwrap();
        let manager = orr_package::Project::open(
            &fixture.project,
            orr_sample::project_runtime::compiled_runtime(),
        )
        .unwrap();
        let asset =
            orr_sample::project_sprites::load_project_asset(&manager, PACKAGE, "sprites.json")
                .unwrap();
        assert_eq!(asset.rgba, pixels(true));
        assert_eq!(doc.bindings["e_00000001"].orientation, Some(orientation));
    }
}

/// Production exporters/runtimes are copied into owned temporary storage. The
/// namespace must hide source checkout, authored project, package sources and
/// copied tools, and mount the relocated bundle read-only. No fallback proof.
#[cfg(all(
    feature = "project-export",
    feature = "collect-sprites",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[test]
#[ignore = "requires trusted production Arena/Collect binaries, source hiding, read-only bundles and software GPU"]
fn authored_orientation_export_relocates_with_sources_hidden_and_read_only() {
    use fixture::*;
    use orr_sample::project_sprites::Orientation;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
    };
    fn good(output: Output) -> String {
        assert!(
            output.status.success(),
            "{}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    for collect in [false, true] {
        let fixture = Fixture::new(collect);
        fixture.orientation(Some(Orientation {
            quarter_turns: 3,
            flip_x: true,
            flip_y: false,
        }));
        let base = fixture.temp.path().canonicalize().unwrap();
        let tools = base.join("trusted tools");
        let empty = base.join("empty cwd");
        fs::create_dir(&tools).unwrap();
        fs::create_dir(&empty).unwrap();
        let prefix = if collect { "COLLECT" } else { "ARENA" };
        let mut copied = Vec::new();
        for role in ["RUNTIME", "EXPORTER"] {
            let key = format!("ORR_ORIENTATION_{prefix}_{role}");
            let source = PathBuf::from(
                std::env::var_os(&key)
                    .unwrap_or_else(|| panic!("mandatory acceptance requires {key}")),
            )
            .canonicalize()
            .unwrap();
            let destination = tools.join(role.to_ascii_lowercase());
            fs::copy(source, &destination).unwrap();
            copied.push(destination);
        }
        let runtime = &copied[0];
        let exporter = &copied[1];
        let baseline = base.join("source.png");
        let run = |runtime: &Path, project: &Path, capture: &Path| {
            good(
                Command::new(runtime)
                    .arg("--project")
                    .arg(project)
                    .args(["--headless", "--ticks", "0", "--capture"])
                    .arg(capture)
                    .current_dir(&empty)
                    .env_remove("DISPLAY")
                    .env_remove("WAYLAND_DISPLAY")
                    .output()
                    .unwrap(),
            )
        };
        let before = snapshot(&fixture.project);
        let package_before = snapshot(&fixture.source);
        let baseline_stdout = run(runtime, &fixture.project, &baseline);
        assert!(baseline_stdout.contains("software: true"));
        let output = base.join("original export");
        use std::fmt::Write as _;
        let mut runtime_hash = String::with_capacity(64);
        for byte in Sha256::digest(fs::read(runtime).unwrap()) {
            write!(&mut runtime_hash, "{byte:02x}").unwrap();
        }
        good(
            Command::new(exporter)
                .arg("--project")
                .arg(&fixture.project)
                .arg("--runtime")
                .arg(runtime)
                .args([
                    "--runtime-sha256",
                    &runtime_hash,
                    "--trusted-runtime",
                    "--output",
                ])
                .arg(&output)
                .current_dir(&empty)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap(),
        );
        let relocated = base.join("relocated read only bundle");
        fs::rename(&output, &relocated).unwrap();
        assert!(!output.exists());
        assert_eq!(
            fs::read(relocated.join("project/view.json")).unwrap(),
            before[&PathBuf::from("view.json")]
        );
        let bundle_before = snapshot(&relocated);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let hidden = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.parent().unwrap().into())
            .canonicalize()
            .unwrap();
        assert!(repository.starts_with(&hidden));
        assert!(!base.starts_with(&hidden));
        let capture = base.join("isolated.png");
        let mut command = Command::new("bwrap");
        command.args(["--die-with-parent","--ro-bind","/","/","--dev-bind","/dev/null","/dev/null","--bind"]).arg(&base).arg(&base)
            .arg("--tmpfs").arg(&hidden).arg("--tmpfs").arg(&fixture.project).arg("--tmpfs").arg(&fixture.source).arg("--tmpfs").arg(&tools)
            .arg("--ro-bind").arg(&relocated).arg(&relocated)
            .args(["--","/bin/sh","-c","test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" || exit 91; shift 3; exec \"$@\"","sh"])
            .arg(repository.join("Cargo.toml")).arg(fixture.project.join("orr.project.json")).arg(runtime)
            .arg(relocated.join(if collect {"run-collect-dodge"} else {"run-arena"}))
            .args(["--headless","--ticks","0","--capture"]).arg(&capture).current_dir(&empty)
            .env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        assert_eq!(
            baseline_stdout,
            good(
                command
                    .output()
                    .expect("mandatory source hiding requires bubblewrap")
            )
        );
        assert_eq!(fs::read(&capture).unwrap(), fs::read(&baseline).unwrap());
        assert_eq!(snapshot(&relocated), bundle_before);
        assert_eq!(snapshot(&fixture.project), before);
        assert_eq!(snapshot(&fixture.source), package_before);
        if let Some(path) = std::env::var_os("ORR_ORIENTATION_CAPTURE_DIR") {
            let path = PathBuf::from(path);
            fs::create_dir_all(&path).unwrap();
            fs::copy(&baseline, path.join(format!("export-{prefix}-source.png"))).unwrap();
            fs::copy(&capture, path.join(format!("export-{prefix}-isolated.png"))).unwrap();
            fs::copy(
                relocated.join("orr.export.json"),
                path.join(format!("export-{prefix}-manifest.json")),
            )
            .unwrap();
        }
    }
}
