//! Explicit source-hidden native CPU acceptance; no window or GPU claim.
#![allow(clippy::float_arithmetic)]
use orr_model::animation::AnimatedModel;
use orr_model_bindings::{animation_time::sample_clip_time_at_rate, model_bindings::PlaybackMode};
use orr_sample::room_project::{PreparedProject, RoomProjectCapabilities};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00002, "{a} != {b}");
}
#[test]
#[ignore = "explicit ORR_TRIM_ROOM_RUNTIME required; source-hidden CPU native acceptance"]
fn actual_trimmed_package_room_export_native_pose_and_fresh_restart() {
    let f = super::editor_animation_trim_tests::authored_room();
    let character_bytes = fs::read(f.root.join("room.character.json")).unwrap();
    let bindings_bytes = fs::read(f.root.join("room.models.json")).unwrap();
    let prepared = PreparedProject::open_with_capabilities(
        &f.root,
        RoomProjectCapabilities {
            character: true,
            lighting: true,
        },
    )
    .unwrap();
    let document = prepared.character().unwrap().document.clone();
    assert_eq!(document.searching, 3);
    assert_eq!(document.motion.unwrap().searching_idle, 3);
    assert!(document.motion.unwrap().pickup.is_some());
    assert_eq!(document.crossfade_ticks, Some(0));
    let binding = &prepared.models().document.bindings[&document.player];
    let package = binding.package.clone();
    let asset = binding.asset.clone();
    let model = prepared.models().assets[&(package.clone(), asset.clone())]
        .animated_model()
        .unwrap()
        .clone();
    assert_eq!(model.source().clips.len(), 4);
    assert_eq!(model.source().clips[3].name, "trimmed-search");
    assert_eq!(model.source().clips[3].duration(), 0.5);
    let original = AnimatedModel::from_bytes(include_bytes!(
        "../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    assert_eq!(model.source().clips[..3], original.source().clips[..]);
    let old_project =
        orr_package::Project::open(&f.root, orr_sample::room_project::compiled_runtime()).unwrap();
    let asset_bytes = old_project.read_asset(&package, &asset).unwrap();
    let trusted = f.temp.path().join("trusted trim runtime");
    fs::copy(
        PathBuf::from(std::env::var_os("ORR_TRIM_ROOM_RUNTIME").expect("explicit built runtime")),
        &trusted,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&trusted).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.temp.path().join("deployed trimmed Room");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: trusted.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    let deployed = bundle.join("project");
    assert_eq!(
        fs::read(deployed.join("room.character.json")).unwrap(),
        character_bytes
    );
    assert_eq!(
        fs::read(deployed.join("room.models.json")).unwrap(),
        bindings_bytes
    );
    let project =
        orr_package::Project::open(&deployed, orr_sample::room_project::compiled_runtime())
            .unwrap();
    assert_eq!(project.read_asset(&package, &asset).unwrap(), asset_bytes);
    for path in [
        f.root.clone(),
        f.temp.path().join("trim-source.animated.json"),
        f.temp.path().join("trimmed model package"),
        trusted,
    ] {
        assert!(path.exists(), "expected authoring input {}", path.display());
        fs::rename(&path, path.with_extension("hidden")).unwrap();
    }
    let prepared = PreparedProject::open_with_capabilities(
        &deployed,
        RoomProjectCapabilities {
            lighting: true,
            character: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.character().unwrap().document, document);
    let binding = &prepared.models().document.bindings[&document.player];
    let model = prepared.models().assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .unwrap();
    let mut sim = prepared.scene().simulation().unwrap();
    for tick in [0u64, 6, 15, 29, 30, 37] {
        while sim.tick() < tick {
            sim.step(&orr_sim::TickInputs::new(sim.tick() + 1, 1));
        }
        assert_eq!(
            sim.frame()
                .singleton::<orr_sample::room_game::RoomRun>()
                .key_collected,
            0
        );
        assert_eq!(
            sim.frame()
                .singleton::<orr_sample::room_game::RoomRun>()
                .won,
            0
        );
        let phase = sample_clip_time_at_rate(
            tick,
            60,
            0.5,
            PlaybackMode::Loop,
            document.speed(orr_sample::room_character::State::Searching),
        )
        .unwrap();
        let pose = model.sample_clip(3, phase).unwrap();
        let expected = original.sample_clip(2, 0.25 + phase).unwrap();
        for (a, b) in pose.local().iter().zip(expected.local()) {
            for i in 0..3 {
                close(a.translation[i], b.translation[i]);
                close(a.scale[i], b.scale[i]);
            }
            let dot: f32 = a.rotation.iter().zip(b.rotation).map(|(x, y)| x * y).sum();
            close(dot.abs(), 1.0);
        }
        for (a, b) in pose.global().iter().zip(expected.global()) {
            for (x, y) in a.iter().flatten().zip(b.iter().flatten()) {
                close(*x, *y);
            }
        }
        for (a, b) in pose.skin_matrices().iter().zip(expected.skin_matrices()) {
            for (a, b) in a.iter().zip(b) {
                for (x, y) in a.iter().flatten().zip(b.iter().flatten()) {
                    close(*x, *y);
                }
            }
        }
        let digest = orr_sample::room_app::character_pose_digest(&pose);
        let mut previous = None;
        for _ in 0..2 {
            let output = Command::new(bundle.join("run-room-escape"))
                .current_dir(f.temp.path())
                .args([
                    "--headless",
                    "--ticks",
                    &tick.to_string(),
                    "--check-model-poses",
                ])
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(
                text.contains(&format!(
                    "room tick: {tick} checksum: 0x{:016x}",
                    sim.frame().checksum()
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!("room model poses: {} frames checked;", tick + 1)),
                "{text}"
            );
            assert_eq!(
                text.lines()
                    .find_map(|l| l.strip_prefix("room character observed pose sha256: ")),
                Some(digest.as_str()),
                "{text}"
            );
            if let Some(old) = previous {
                assert_eq!(
                    text, old,
                    "fresh native process must reproduce phase and Frame"
                );
            }
            previous = Some(text);
        }
    }
    assert_eq!(
        fs::read(deployed.join("room.character.json")).unwrap(),
        character_bytes
    );
    assert_eq!(
        fs::read(deployed.join("room.models.json")).unwrap(),
        bindings_bytes
    );
    assert_eq!(
        orr_package::Project::open(&deployed, orr_sample::room_project::compiled_runtime())
            .unwrap()
            .read_asset(&package, &asset)
            .unwrap(),
        asset_bytes
    );
}
