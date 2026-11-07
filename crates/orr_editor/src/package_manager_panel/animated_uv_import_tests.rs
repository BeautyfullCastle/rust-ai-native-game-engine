//! CPU-only producer -> package browser/assignment -> saved Room acceptance.
//! Shared import helper drives real preview/add/verify workers and egui Use;
//! import form gestures and GPU/window rendering are separate checks.
use super::*;
#[cfg(feature = "windows-export")]
use std::fs;
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn assert_payload(model: &orr_model::animation::AnimatedModel) {
    let old = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let before = old.source();
    let after = model.source();
    assert_eq!(after.asset_id, "variants/courier.uv.json");
    assert_eq!(after.images, before.images);
    assert_eq!(after.materials, before.materials);
    assert_eq!(before.nodes, after.nodes);
    assert_eq!(before.skins, after.skins);
    assert_eq!(before.clips, after.clips);
    assert_eq!(before.primitives.len(), after.primitives.len());
    let mut selected_vertices = 0;
    let mut other_vertices = 0;
    for (a, b) in before.primitives.iter().zip(&after.primitives) {
        let mut restored = b.clone();
        restored.id = a.id.clone();
        assert_eq!(a.vertices.len(), b.vertices.len());
        for ((original, actual), restore) in a
            .vertices
            .iter()
            .zip(&b.vertices)
            .zip(&mut restored.vertices)
        {
            if a.material == 0 {
                // Independent oracle for scale [2,.5], +90deg then offset [.25,-.5].
                let [u, v] = original.vertex.uv;
                let expected = [0.25 - v * 0.5, -0.5 + u * 2.0];
                assert_eq!(actual.vertex.uv, expected);
                restore.vertex.uv = original.vertex.uv;
                selected_vertices += 1;
            } else {
                assert_eq!(actual.vertex.uv, original.vertex.uv);
                other_vertices += 1;
            }
        }
        // Restores only authored UV and namespaced ID; influences and every
        // geometry attribute must still match the source exactly.
        assert_eq!(a, &restored);
    }
    assert!(selected_vertices > 0 && other_vertices > 0);
    for clip in 0..before.clips.len() as u32 {
        for time in [0.0, 0.25, 0.75] {
            let a = old.sample_clip(clip, time).unwrap();
            let b = model.sample_clip(clip, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
            let original = old.deform(&a).unwrap();
            let transformed = model.deform(&b).unwrap();
            assert_eq!(original.len(), transformed.len());
            for (a, b) in original.iter().zip(&transformed) {
                assert_eq!(a.vertices.len(), b.vertices.len());
                for (a, b) in a.vertices.iter().zip(&b.vertices) {
                    assert_eq!(a.position, b.position);
                    assert_eq!(a.normal, b.normal);
                }
            }
        }
    }
}
fn produced() -> orr_model::animation::AnimatedModel {
    let recipe = br#"{"format":"orr_animated_material_uv","version":1,"source":"courier.animated.json","material":0,"offset":[0.25,-0.5],"scale":[2.0,0.5],"quarter_turns":1}"#;
    let mut reads = Vec::new();
    let model =
        orr_model::animated_uv::transform_model_uv(recipe, "variants/courier.uv.json", |path| {
            reads.push(path.to_owned());
            assert_eq!(path, "courier.animated.json");
            Ok(SOURCE.to_vec())
        })
        .unwrap();
    assert_eq!(reads, ["courier.animated.json"]);
    assert_payload(&model);
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
    assert_payload(model);
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
fn authored_animated_uv_survives_real_package_selection_assignment_and_saved_room() {
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
fn authored_animated_uv_survives_source_hidden_native_export() {
    use sha2::{Digest, Sha256};
    let bytes = produced().to_bytes().unwrap();
    let f = super::cooked_animated_import_tests::imported_player_with_source(&bytes);
    let expected = format!(
        "room tick: 8 checksum: 0x{:016x}",
        checksum_after_pickup(&open(&f.root))
    );
    let runtime = f.temp.path().join("trusted-runtime");
    fs::copy(
        std::env::var_os("ORR_ANIMATED_UV_RUNTIME").expect("explicit built runtime"),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.temp.path().join("samplerd-game");
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
