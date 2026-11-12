//! CPU-only producer -> package browser/assignment -> saved Room acceptance.
//! Shared import helper drives real preview/add/verify workers and egui Use;
//! import form gestures and GPU/window rendering are separate checks.
use super::*;
#[cfg(feature = "windows-export")]
use std::fs;
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn produced() -> orr_model::animation::AnimatedModel {
    let recipe = br#"{"format":"orr_animated_material_color","version":1,"source":"courier.animated.json","material":0,"linear_rgb":[0.2,0.7,0.4]}"#;
    let mut reads = 0;
    let model = orr_model::animated_recolor::recolor_model(
        recipe,
        "variants/courier.recolor.json",
        |path| {
            reads += 1;
            assert_eq!(path, "courier.animated.json");
            Ok(SOURCE.to_vec())
        },
    )
    .unwrap();
    assert_eq!(reads, 1);
    let old = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let before = old.source();
    let after = model.source();
    assert_eq!(after.asset_id, "variants/courier.recolor.json");
    assert_eq!(
        after.materials[0].base_color,
        [0.2, 0.7, 0.4, before.materials[0].base_color[3]]
    );
    // Compare immutable skeletal/geometry payload independent of rewritten IDs.
    let a = serde_json::to_value(before).unwrap();
    let b = serde_json::to_value(after).unwrap();
    for key in ["nodes", "skins", "clips", "images"] {
        assert_eq!(a[key], b[key], "{key}");
    }
    assert_eq!(
        a["primitives"].as_array().unwrap().len(),
        b["primitives"].as_array().unwrap().len()
    );
    assert_eq!(
        a["materials"].as_array().unwrap().len(),
        b["materials"].as_array().unwrap().len()
    );
    for index in 1..before.materials.len() {
        assert_eq!(a["materials"][index], b["materials"][index]);
    }
    for key in ["image", "linear_filter", "wrap_s", "wrap_t"] {
        assert_eq!(a["materials"][0][key], b["materials"][0][key]);
    }
    for (a, b) in a["primitives"]
        .as_array()
        .unwrap()
        .iter()
        .zip(b["primitives"].as_array().unwrap())
    {
        for key in ["node", "skin", "vertices", "indices", "material"] {
            assert_eq!(a[key], b[key], "{key}");
        }
    }
    for clip in 0..before.clips.len() as u32 {
        for time in [0.0, 0.25, 0.75] {
            let a = old.sample_clip(clip, time).unwrap();
            let b = model.sample_clip(clip, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
        }
    }
    model
}
fn open(root: &Path) -> orr_sample::room_project::PreparedProject {
    orr_sample::room_project::PreparedProject::open_with_capabilities(
        root,
        orr_sample::room_project::RoomProjectCapabilities {
            character: true,
            lighting: true,
        },
    )
    .unwrap()
}
fn assert_variant(prepared: &orr_sample::room_project::PreparedProject, bytes: &[u8]) {
    use sha2::{Digest, Sha256};
    let hash: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let player = prepared
        .scene()
        .frame()
        .entities()
        .find(|&entity| {
            prepared
                .scene()
                .frame()
                .get::<orr_sample::room_game::RoomActor>(entity)
                .is_some_and(|a| a.kind == orr_sample::room_game::PLAYER)
        })
        .unwrap();
    let guid = prepared.scene().index().guid(player).unwrap();
    let binding = &prepared.models().document.bindings[&guid.to_string()];
    assert_eq!(binding.package, "imported-courier");
    assert_eq!(binding.asset, "model.animated.json");
    assert_eq!(binding.kind, ModelKind::Animated);
    assert_eq!(binding.source_hash, hash);
    let loaded = &prepared.models().assets[&(binding.package.clone(), binding.asset.clone())];
    let model = loaded.animated_model().unwrap();
    assert_eq!(model.source().asset_id, "variants/courier.recolor.json");
    assert_eq!(model.source().materials[0].base_color, [0.2, 0.7, 0.4, 1.0]);
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    for clip in 0..3 {
        let a = original.sample_clip(clip, 0.5).unwrap();
        let b = model.sample_clip(clip, 0.5).unwrap();
        assert_eq!(a.skin_matrices(), b.skin_matrices());
    }
}
fn checksum_after_pickup(prepared: &orr_sample::room_project::PreparedProject) -> u64 {
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut inputs = orr_sim::TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(
            orr_sim::PlayerSlot(0),
            orr_sample::room_game::RoomInput {
                move_x: 1,
                buttons: orr_sample::room_game::INTERACT,
                ..Default::default()
            },
        );
        sim.step(&inputs);
    }
    assert_eq!(
        sim.frame()
            .singleton::<orr_sample::room_game::RoomRun>()
            .key_collected,
        1
    );
    assert_eq!(
        orr_sample::room_character::state(orr_bridge::FrameView::of(sim.frame())),
        orr_sample::room_character::State::Carrying
    );
    sim.frame().checksum()
}
#[test]
fn authored_animated_material_survives_real_package_selection_assignment_and_saved_room() {
    let bytes = produced().to_bytes().unwrap();
    let baseline = crate::package_manager::presentation_tests::fixture(true, true);
    let original = open(&baseline.root);
    let f = super::cooked_animated_import_tests::imported_player_with_source(&bytes);
    let prepared = open(&f.root);
    assert_variant(&prepared, &bytes);
    assert_eq!(
        prepared.scene().frame().to_bytes(),
        original.scene().frame().to_bytes()
    );
    assert_eq!(
        checksum_after_pickup(&prepared),
        checksum_after_pickup(&original)
    );
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "requires explicit trusted Room presentation runtime; CPU/headless only"]
fn authored_animated_material_survives_source_hidden_native_export() {
    use sha2::{Digest, Sha256};
    let bytes = produced().to_bytes().unwrap();
    let f = super::cooked_animated_import_tests::imported_player_with_source(&bytes);
    let expected = format!(
        "room tick: 8 checksum: 0x{:016x}",
        checksum_after_pickup(&open(&f.root))
    );
    let runtime = f.temp.path().join("trusted-runtime");
    fs::copy(
        std::env::var_os("ORR_ANIMATED_RECOLOR_RUNTIME").expect("explicit built runtime"),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.temp.path().join("recolored-game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    for name in [
        "orr.project.json",
        "room.scene.yaml",
        "room.models.json",
        "room.camera.json",
        "room.character.json",
        "room.lighting.json",
    ] {
        assert_eq!(
            fs::read(f.root.join(name)).unwrap(),
            fs::read(bundle.join("project").join(name)).unwrap()
        );
    }
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    fs::rename(
        f.temp.path().join("new animated package"),
        f.temp.path().join("hidden-package"),
    )
    .unwrap();
    fs::remove_file(f.temp.path().join("author.model.animated.json")).unwrap();
    fs::remove_file(f.temp.path().join("author-LICENSE.txt")).unwrap();
    assert_variant(&open(&bundle.join("project")), &bytes);
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(f.temp.path())
        .args([
            "--headless",
            "--ticks",
            "8",
            "--hold",
            "right,interact",
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
    assert!(text.contains(&expected), "{text}");
    assert!(
        text.contains("room model poses: 9 frames checked;"),
        "{text}"
    );
    assert!(text.contains("room lighting active: 1"), "{text}");
}
