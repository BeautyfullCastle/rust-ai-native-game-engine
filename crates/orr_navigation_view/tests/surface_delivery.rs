//! Independent CPU parity and opt-in actual Linux surface-delivery acceptance.
#![cfg(all(
    feature = "authored-surface-bundle",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[path = "support/surface_fixture.rs"]
mod support;
use orr_navigation_view::{
    authored::AuthoredPlayer,
    surface_player::{self, SurfacePlayer},
};
use std::{ffi::OsString, fs};

#[test]
fn surface_is_presentation_only_and_preserves_every_frame_and_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let scene = support::fixture(temp.path());
    let mut original = AuthoredPlayer::open(&scene).unwrap();
    let mut surface = SurfacePlayer::open(&scene).unwrap();
    let initial = original.frame().to_bytes();
    fs::rename(
        temp.path().join("navigation"),
        temp.path().join("hidden navigation"),
    )
    .unwrap();
    fs::rename(&scene, temp.path().join("hidden scene")).unwrap();
    fs::rename(
        temp.path().join("source appearance.png"),
        temp.path().join("hidden image"),
    )
    .unwrap();
    for tick in 0..=60 {
        assert_eq!(
            original.frame().to_bytes(),
            surface.frame().to_bytes(),
            "tick {tick}"
        );
        assert_eq!(original.checksum(), surface.checksum());
        if tick == 0 || tick == 60 {
            let view = original.view().unwrap();
            let baseline = orr_navigation_view::to_static_model(
                &view.terrain,
                view.navigator.path(),
                view.navigator.position(),
                orr_navigation_view::PathDisplay::Current,
            )
            .unwrap();
            let visual = surface.model().unwrap();
            assert_eq!(visual.source().images[0], *surface.surface().image());
            assert_ne!(visual.source().images[0], baseline.source().images[0]);
            assert_eq!(
                visual.source().primitives.len(),
                baseline.source().primitives.len()
            );
            for (expected, actual) in baseline
                .source()
                .primitives
                .iter()
                .zip(&visual.source().primitives)
            {
                assert_eq!(expected.vertices, actual.vertices);
                assert_eq!(expected.indices, actual.indices);
                assert_eq!(expected.transform, actual.transform);
            }
            for (expected, actual) in baseline
                .source()
                .primitives
                .iter()
                .zip(&visual.source().primitives)
                .skip(1)
            {
                let a = &baseline.source().materials[expected.material as usize];
                let b = &visual.source().materials[actual.material as usize];
                assert_eq!(
                    baseline.source().images[a.image as usize],
                    visual.source().images[b.image as usize]
                );
                assert_eq!(a.base_color, b.base_color);
            }
        }
        if tick < 60 {
            original.step().unwrap();
            surface.step().unwrap();
        }
    }
    original.restart().unwrap();
    surface.restart().unwrap();
    assert_eq!(surface.frame().to_bytes(), initial);
    assert_eq!(original.frame().to_bytes(), surface.frame().to_bytes());
}

#[test]
fn surface_parser_is_closed_and_legacy_capabilities_and_capture_refusal_stay_unchanged() {
    assert!(!surface_player::capabilities().contains("_V1"));
    assert_eq!(
        orr_navigation_view::player::capabilities(),
        "ORR_NAVIGATION_PLAYER_V1\nORR_NAVIGATION_BUNDLE_V1"
    );
    for args in [
        vec!["--bundle", "x", "--scene", "y", "--ticks", "0"],
        vec!["--bundle", "x", "--bundle", "y", "--ticks", "0"],
        vec!["--bundle", "x", "--ticks", "100001"],
        vec!["--bundle", "x", "--ticks", "-1"],
        vec!["--capabilities", "--ticks", "0"],
        vec!["--bundle", "x", "--ticks", "0", "--unknown"],
    ] {
        assert!(surface_player::parse_command(args.into_iter().map(OsString::from)).is_err());
    }
    let args = ["--bundle", "x", "--ticks", "0", "--capture", "x.png"];
    assert!(orr_navigation_view::player::parse_command(args.map(OsString::from)).is_err());
    assert_eq!(
        surface_player::parse_command(args.map(OsString::from)).is_ok(),
        cfg!(feature = "gpu")
    );
}

#[cfg(feature = "gpu")]
mod actual {
    use super::*;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        path::{Path, PathBuf},
        process::{Command, Output},
    };
    fn success(output: Output) -> Output {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn failure(output: Output) {
        assert!(
            !output.status.success(),
            "unexpected success: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect()
    }
    fn export(exporter: &Path, scene: &Path, runtime: &Path, output: &Path, cwd: &Path) -> Command {
        let mut command = Command::new(exporter);
        command
            .current_dir(cwd)
            .arg("--scene")
            .arg(scene)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(hash(&fs::read(runtime).unwrap()))
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(output);
        command
    }
    fn launch(
        runtime: &Path,
        root: &Path,
        tick: u64,
        capture: Option<&Path>,
        cwd: &Path,
    ) -> Command {
        let mut command = Command::new(runtime);
        command
            .current_dir(cwd)
            .arg("--bundle")
            .arg(root)
            .arg("--ticks")
            .arg(tick.to_string())
            .arg("--restart-check");
        if let Some(path) = capture {
            command.arg("--capture").arg(path);
        }
        command
    }
    fn rgba(path: &Path) -> Vec<u8> {
        let mut decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
        let info = decoder.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (1000, 750));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        pixels
    }
    // Literal UI-import fixture colors, independent of SurfacePlayer's model builder.
    // None overlaps the legacy green checker, cyan route, or orange agent colors.
    const PATTERN: [[u8; 4]; 4] = [
        [245, 210, 40, 255],
        [40, 70, 220, 255],
        [220, 40, 150, 255],
        [50, 220, 70, 255],
    ];
    #[derive(serde::Serialize)]
    struct PixelEvidence {
        authored_color_counts: [u64; 4],
        cyan_route_pixels: u64,
        orange_agent_pixels: u64,
        orange_x_sum: u64,
        orange_y_sum: u64,
    }
    fn pixel_evidence(pixels: &[u8]) -> PixelEvidence {
        let mut result = PixelEvidence {
            authored_color_counts: [0; 4],
            cyan_route_pixels: 0,
            orange_agent_pixels: 0,
            orange_x_sum: 0,
            orange_y_sum: 0,
        };
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            assert_eq!(pixel[3], 255);
            for (color, count) in PATTERN.iter().zip(&mut result.authored_color_counts) {
                if pixel[..3]
                    .iter()
                    .zip(&color[..3])
                    .all(|(a, b)| a.abs_diff(*b) <= 3)
                {
                    *count += 1;
                }
            }
            if pixel[0] < 100 && pixel[1] > 160 && pixel[2] > 220 {
                result.cyan_route_pixels += 1;
            }
            if pixel[0] > 220 && pixel[1] > 60 && pixel[1] < 120 && pixel[2] < 50 {
                result.orange_agent_pixels += 1;
                result.orange_x_sum += (index % 1000) as u64;
                result.orange_y_sum += (index / 1000) as u64;
            }
        }
        for (color, count) in PATTERN.iter().zip(result.authored_color_counts) {
            assert!(count > 1000, "literal imported color {color:?} missing: {count} pixels; checker fallback must fail");
        }
        assert!(result.cyan_route_pixels > 10, "route cyan missing");
        assert!(result.orange_agent_pixels > 30, "agent orange missing");
        result
    }
    fn assert_agent_moved(a: &PixelEvidence, b: &PixelEvidence) {
        let denominator = i128::from(a.orange_agent_pixels) * i128::from(b.orange_agent_pixels);
        let dx = (i128::from(a.orange_x_sum) * i128::from(b.orange_agent_pixels)
            - i128::from(b.orange_x_sum) * i128::from(a.orange_agent_pixels))
        .abs();
        let dy = (i128::from(a.orange_y_sum) * i128::from(b.orange_agent_pixels)
            - i128::from(b.orange_y_sum) * i128::from(a.orange_agent_pixels))
        .abs();
        assert!(
            dx + dy > denominator * 2,
            "orange agent centroid must move more than two pixels independently of terrain colors"
        );
    }
    #[test]
    #[ignore = "requires explicit trusted actual binaries and working offscreen GPU; no skip on failure"]
    fn relocated_actual_surface_bundle_capture_and_refusals() {
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        let runtime = PathBuf::from(
            std::env::var_os("ORR_NAV_SURFACE_PLAYER_BIN").expect("trusted player required"),
        );
        let exporter = PathBuf::from(
            std::env::var_os("ORR_NAV_SURFACE_EXPORT_BIN").expect("actual exporter required"),
        );
        let evidence = PathBuf::from(
            std::env::var_os("ORR_NAV_SURFACE_EVIDENCE_DIR")
                .expect("fresh evidence directory required"),
        );
        assert!(runtime.is_absolute() && exporter.is_absolute() && evidence.is_absolute());
        fs::create_dir(&evidence).expect("evidence directory must not exist");
        let source = evidence.join("supplied source copy");
        let scene = if let Some(saved) = std::env::var_os("ORR_NAV_SURFACE_AUTHORED_SCENE") {
            let saved = PathBuf::from(saved);
            assert!(saved.is_absolute());
            let captured =
                orr_navigation_view::surface_bundle::SavedSource::capture(&saved).unwrap();
            let admitted = AuthoredPlayer::open(&saved).unwrap();
            let relative = admitted
                .frame()
                .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>()
                .source()
                .unwrap();
            let parent = saved.parent().unwrap();
            fs::create_dir_all(source.join(relative).parent().unwrap()).unwrap();
            let scene = source.join("point.scene.yaml");
            fs::write(&scene, captured.scene_bytes()).unwrap();
            fs::copy(parent.join(relative), source.join(relative)).unwrap();
            fs::write(
                source.join(format!("{relative}.surface.json")),
                captured.surface_bytes(),
            )
            .unwrap();
            captured.recheck().unwrap();
            let copied = orr_navigation_view::surface_bundle::SavedSource::capture(&scene).unwrap();
            assert_eq!(copied.scene_bytes(), captured.scene_bytes());
            assert_eq!(copied.surface_bytes(), captured.surface_bytes());
            assert_eq!(copied.initial_frame_bytes(), captured.initial_frame_bytes());
            assert_eq!(
                fs::read(parent.join(relative)).unwrap(),
                fs::read(source.join(relative)).unwrap()
            );
            fs::write(evidence.join("editor-handoff.json"),serde_json::to_vec_pretty(&serde_json::json!({
                "saved_scene":saved,"copied_for_export":true,"editor_originals_preserved":true,"scene_sha256":hash(copied.scene_bytes()),
                "terrain_sha256":hash(&fs::read(source.join(relative)).unwrap()),
                "surface_sha256":hash(copied.surface_bytes()),"initial_frame_sha256":hash(copied.initial_frame_bytes()),
                "initial_checksum":copied.initial_checksum(),"terrain_relative":relative
            })).unwrap()).unwrap();
            scene
        } else {
            support::fixture(&source)
        };
        let empty = evidence.join("empty working directory");
        fs::create_dir(&empty).unwrap();
        // Only this supplied executable is the export source. Hide it after publishing.
        let supplied = evidence.join("supplied executable copy");
        fs::copy(&runtime, &supplied).unwrap();
        assert!(fs::metadata(&supplied).unwrap().len() <= 64 * 1024 * 1024);
        let capabilities = success(
            Command::new(&supplied)
                .arg("--capabilities")
                .output()
                .unwrap(),
        );
        assert_eq!(
            capabilities.stdout,
            b"ORR_NAVIGATION_SURFACE_PLAYER_V2\nORR_NAVIGATION_SURFACE_CAPTURE_V2\n"
        );
        let mut original = AuthoredPlayer::open(&scene).unwrap();
        let mut wrapped = SurfacePlayer::open(&scene).unwrap();
        let terrain_relative = original
            .frame()
            .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>()
            .source()
            .unwrap()
            .to_owned();
        assert_eq!(
            (
                wrapped.surface().image().width,
                wrapped.surface().image().height
            ),
            (2, 2)
        );
        assert_eq!(
            wrapped.surface().image().rgba8,
            PATTERN.concat(),
            "acceptance requires the exact literal Editor import pattern"
        );
        let initial_frame = original.frame().to_bytes();
        fs::write(evidence.join("initial.frame"), &initial_frame).unwrap();
        let mut trajectory = Vec::new();
        for tick in 0..=48 {
            assert_eq!(original.frame().to_bytes(), wrapped.frame().to_bytes());
            trajectory.push(original.checksum());
            if tick != 48 {
                original.step().unwrap();
                wrapped.step().unwrap();
            }
        }
        fs::write(
            evidence.join("trajectory.json"),
            serde_json::to_vec_pretty(&trajectory).unwrap(),
        )
        .unwrap();
        fs::write(evidence.join("final.frame"), original.frame().to_bytes()).unwrap();
        fs::write(evidence.join("frame-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "frame_comparison":"CPU SurfacePlayer versus unchanged AuthoredPlayer: all Frame bytes and checksums through tick 48. Actual binaries: checksum/restart receipts and pixel oracles.",
            "hidden_inputs":"Export-supplied copies only; original Editor handoff evidence remains unchanged.",
            "initial_frame_sha256":hash(&initial_frame),"final_frame_sha256":hash(&original.frame().to_bytes()),
            "initial_checksum":trajectory[0],"final_checksum":original.checksum(),"ticks":48,
            "scene_sha256":hash(&fs::read(&scene).unwrap()),
            "terrain_sha256":hash(&fs::read(source.join(&terrain_relative)).unwrap()),
            "surface_sha256":hash(&wrapped.surface().to_bytes().unwrap()),
            "surface_digest":wrapped.surface().digest().iter().map(|b|format!("{b:02x}")).collect::<String>()
        })).unwrap()).unwrap();
        for tick in [0, 48] {
            let output = success(
                Command::new(&supplied)
                    .current_dir(&empty)
                    .arg("--scene")
                    .arg(&scene)
                    .arg("--ticks")
                    .arg(tick.to_string())
                    .arg("--capture")
                    .arg(evidence.join(format!("source-{tick}.png")))
                    .output()
                    .unwrap(),
            );
            fs::write(evidence.join(format!("source-{tick}.json")), output.stdout).unwrap();
        }
        let bundle = evidence.join("delivery");
        success(
            export(&exporter, &scene, &supplied, &bundle, &empty)
                .output()
                .unwrap(),
        );
        // Existing outputs, overlapping source output, bad digest and non-player capability refuse.
        failure(
            export(&exporter, &scene, &supplied, &bundle, &empty)
                .output()
                .unwrap(),
        );
        failure(
            export(
                &exporter,
                &scene,
                &supplied,
                &source.join("overlap"),
                &empty,
            )
            .output()
            .unwrap(),
        );
        let bad_cap = evidence.join("bad capability output");
        failure(
            export(&exporter, &scene, &exporter, &bad_cap, &empty)
                .output()
                .unwrap(),
        );
        assert!(!bad_cap.exists());
        let bad_hash = evidence.join("bad hash output");
        let mut bad = Command::new(&exporter);
        bad.arg("--scene")
            .arg(&scene)
            .arg("--runtime")
            .arg(&supplied)
            .args([
                "--runtime-sha256",
                &"0".repeat(64),
                "--trusted-runtime",
                "--output",
            ])
            .arg(&bad_hash);
        failure(bad.output().unwrap());
        assert!(!bad_hash.exists());
        let moved = evidence.join("relocated bundle with spaces");
        fs::rename(&bundle, &moved).unwrap();
        fs::rename(&source, evidence.join("hidden supplied source copy")).unwrap();
        fs::rename(&supplied, evidence.join("hidden supplied executable copy")).unwrap();
        assert!(
            !scene.exists()
                && !source.join("source appearance.png").exists()
                && !source.join(&terrain_relative).exists()
                && !source
                    .join(format!("{terrain_relative}.surface.json"))
                    .exists()
                && !supplied.exists()
        );
        let bundled = moved.join("bin/navigation_surface_player");
        let mut pixel_proofs = Vec::new();
        for tick in [0, 48] {
            let capture = evidence.join(format!("relocated-{tick}.png"));
            let output = success(
                launch(&bundled, &moved, tick, Some(&capture), &empty)
                    .output()
                    .unwrap(),
            );
            let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(receipt["schema"], 2);
            assert_eq!(receipt["tick"], tick);
            assert_eq!(
                receipt["checksum"].as_u64(),
                Some(trajectory[tick as usize])
            );
            assert_eq!(receipt["restart_checked"], true);
            assert_eq!(receipt["capture"]["offscreen"], true);
            fs::write(
                evidence.join(format!("relocated-{tick}.json")),
                output.stdout,
            )
            .unwrap();
            assert_eq!(
                rgba(&capture),
                rgba(&evidence.join(format!("source-{tick}.png")))
            );
            let proof = pixel_evidence(&rgba(&capture));
            fs::write(
                evidence.join(format!("pixels-{tick}.json")),
                serde_json::to_vec_pretty(&proof).unwrap(),
            )
            .unwrap();
            pixel_proofs.push(proof);
            let before = fs::read(&capture).unwrap();
            failure(
                launch(&bundled, &moved, tick, Some(&capture), &empty)
                    .output()
                    .unwrap(),
            );
            assert_eq!(fs::read(&capture).unwrap(), before);
        }
        assert_agent_moved(&pixel_proofs[0], &pixel_proofs[1]);
        assert_ne!(
            rgba(&evidence.join("relocated-0.png")),
            rgba(&evidence.join("relocated-48.png"))
        );
        assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
        failure(launch(&runtime, &moved, 0, None, &empty).output().unwrap());
        let inside = moved.join("forbidden.png");
        failure(
            launch(&bundled, &moved, 0, Some(&inside), &empty)
                .output()
                .unwrap(),
        );
        assert!(!inside.exists());
        let launcher = success(
            Command::new(moved.join("run-navigation-surface"))
                .current_dir(&empty)
                .args(["--ticks", "0"])
                .output()
                .unwrap(),
        );
        let receipt: Value = serde_json::from_slice(&launcher.stdout).unwrap();
        assert_eq!(receipt["checksum"].as_u64(), Some(trajectory[0]));
        for relative in [
            "project/game.scene.yaml".to_owned(),
            format!("project/{terrain_relative}"),
            format!("project/{terrain_relative}.surface.json"),
            "run-navigation-surface".to_owned(),
        ] {
            let path = moved.join(relative);
            let original = fs::read(&path).unwrap();
            let mut bad = original.clone();
            bad.push(b' ');
            fs::write(&path, bad).unwrap();
            failure(launch(&bundled, &moved, 0, None, &empty).output().unwrap());
            fs::write(path, original).unwrap();
        }
        let extra = moved.join("extra.txt");
        fs::write(&extra, b"unlisted").unwrap();
        failure(launch(&bundled, &moved, 0, None, &empty).output().unwrap());
        fs::remove_file(extra).unwrap();
        let sidecar = moved.join(format!("project/{terrain_relative}.surface.json"));
        let hidden = evidence.join("hidden bundled surface");
        fs::rename(&sidecar, &hidden).unwrap();
        failure(launch(&bundled, &moved, 0, None, &empty).output().unwrap());
        fs::rename(hidden, sidecar).unwrap();
        success(launch(&bundled, &moved, 48, None, &empty).output().unwrap());
        fs::write(evidence.join("acceptance.txt"), b"Actual supplied binaries: export, relocation, hidden export-input copies, tick 0/48 literal imported-color counts, cyan route, orange-agent centroid movement, source/relocated pixel equality, checksum/restart receipts and refusals passed. Full Frame byte parity is independently proven by CPU SurfacePlayer versus unchanged AuthoredPlayer. Original Editor handoff files are preserved. Offscreen only; no native window or hardware-only claim.\n").unwrap();
    }
}
