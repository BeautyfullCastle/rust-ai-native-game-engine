//! CPU acceptance: real package workers/browser and editor binding persistence.
//! Import form gestures are covered separately by model_package_import_panel tests.
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::{
    fs,
    time::{Duration, Instant},
};

fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() && Instant::now() < deadline {
        panel.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.busy(), "worker timeout: {}", panel.status);
}

fn imported_player() -> crate::package_manager::presentation_tests::Fixture {
    imported_player_with_source(include_bytes!(
        "../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
}

pub(super) fn imported_player_with_source(
    bytes: &[u8],
) -> crate::package_manager::presentation_tests::Fixture {
    use crate::model_package_import::{import_package, Request, SourceFormat};
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let source = f.temp.path().join("author.model.animated.json");
    fs::write(&source, bytes).unwrap();
    let license = f.temp.path().join("author-LICENSE.txt");
    fs::write(&license, "Acceptance fixture: redistribution acknowledged").unwrap();
    let destination = f.temp.path().join("new animated package");
    let report = import_package(Request {
        source,
        license,
        destination,
        name: "imported-courier".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    })
    .unwrap();
    assert_eq!(report.asset, "model.animated.json");
    assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
    let prepared = open(&f.root);
    let character = prepared.character().unwrap().document.clone();
    let light = prepared.lighting().unwrap().document.clone();
    let player = prepared
        .scene()
        .frame()
        .entities()
        .find(|&e| {
            prepared
                .scene()
                .frame()
                .get::<orr_sample::room_game::RoomActor>(e)
                .is_some_and(|a| a.kind == orr_sample::room_game::PLAYER)
        })
        .unwrap();
    let guid = prepared.scene().index().guid(player).unwrap().clone();
    let old = prepared.models().document.bindings[&guid.to_string()].clone();
    assert_ne!(old.package, report.package);
    drop(prepared);
    let mut panel = Panel {
        root: Some(f.root.clone()),
        source: report.root.display().to_string(),
        source_identity: report.root.display().to_string(),
        ..Default::default()
    };
    assert!(!package_manager::inspect(&f.root)
        .unwrap()
        .packages
        .contains_key(&report.package));
    panel.start_preview(report.root.clone());
    wait(&mut panel);
    assert!(panel.preview.is_some(), "{}", panel.status);
    // Preview is read-only; adding is an explicit second production operation.
    assert!(!package_manager::inspect(&f.root)
        .unwrap()
        .packages
        .contains_key(&report.package));
    panel.start_add();
    wait(&mut panel);
    panel.start_verify();
    wait(&mut panel);
    let lock = panel.verified.as_ref().expect("installed verified lock");
    assert!(lock.direct.contains_key(&report.package));
    let mut ui = Harness::builder()
        .with_size(egui::vec2(1100.0, 1200.0))
        .build_ui_state(
            |ui, panel: &mut Panel| {
                model_asset_buttons(
                    ui,
                    ModelBrowser {
                        lock: panel.verified.as_ref().unwrap(),
                        filter: &mut panel.model_filter,
                        preview_request: &mut panel.preview_request,
                    },
                    true,
                    &mut panel.model_request,
                    panel.root.as_ref(),
                    &panel.source,
                    (panel.project_generation, panel.source_generation),
                );
            },
            panel,
        );
    ui.run_steps(3);
    ui.get_by_label("Preview animated model imported-courier/model.animated.json")
        .click();
    ui.run_steps(3);
    let preview = ui.state_mut().take_preview_request().unwrap();
    assert_eq!(preview.package_id, report.package);
    assert_eq!(preview.asset_path, report.asset);
    assert_eq!(preview.kind, ModelKind::Animated);
    ui.get_by_label("Use animated model imported-courier/model.animated.json")
        .click();
    ui.run_steps(3);
    let (package, asset, kind) = ui.state_mut().take_model_asset_request().unwrap();
    assert_eq!(
        (&package, &asset, kind),
        (&report.package, &report.asset, ModelKind::Animated)
    );
    // This calls the same validated editor transaction used by assignment, not
    // a synthetic JSON rewrite or a native-window/GPU playtest.
    let loaded =
        crate::model_bindings::load_asset_for_kind(&f.root, &package, &asset, kind).unwrap();
    let binding = crate::model_bindings::Binding::from_animated_asset(
        package.clone(),
        asset.clone(),
        &loaded,
        old.animation.unwrap(),
        old.transform,
    )
    .unwrap();
    assert_eq!(binding.source_hash, report.source_sha256);
    assert!(loaded.animated_model().unwrap().source().clips.len() >= 3);
    let mut bindings =
        crate::model_bindings::Bindings::open(f.root.join("room.models.json")).unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&guid), &binding, &loaded)
        .unwrap();
    assert!(bindings.dirty());
    bindings.save().unwrap();
    assert!(!bindings.dirty());
    let reopened = open(&f.root);
    let saved = &reopened.models().document.bindings[&guid.to_string()];
    assert_eq!(saved, &binding);
    assert_eq!(saved.package, "imported-courier");
    assert_eq!(saved.asset, "model.animated.json");
    assert_eq!(reopened.character().unwrap().document, character);
    assert_eq!(reopened.lighting().unwrap().document, light);
    assert!(reopened.models().assets.contains_key(&(package, asset)));
    f
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

#[test]
fn imported_cooked_animation_is_explicitly_installed_used_saved_and_reopened() {
    let f = imported_player();
    let prepared = open(&f.root);
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut input = orr_sim::TickInputs::new(sim.tick() + 1, 1);
        input.set_input(
            orr_sim::PlayerSlot(0),
            orr_sample::room_game::RoomInput {
                move_x: 1,
                buttons: orr_sample::room_game::INTERACT,
                ..Default::default()
            },
        );
        sim.step(&input);
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
}

#[cfg(feature = "windows-export")]
#[test]
#[ignore = "requires explicit trusted presentation-capable runtime; CPU/headless only"]
fn imported_cooked_animation_survives_source_hidden_native_export() {
    use sha2::{Digest, Sha256};
    let f = imported_player();
    let prepared = open(&f.root);
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut input = orr_sim::TickInputs::new(sim.tick() + 1, 1);
        input.set_input(
            orr_sim::PlayerSlot(0),
            orr_sample::room_game::RoomInput {
                move_x: 1,
                buttons: orr_sample::room_game::INTERACT,
                ..Default::default()
            },
        );
        sim.step(&input);
    }
    let expected = format!("room tick: 8 checksum: 0x{:016x}", sim.frame().checksum());
    let runtime = f.temp.path().join("trusted-runtime");
    fs::copy(
        std::env::var_os("ORR_COOKED_ANIMATED_IMPORT_RUNTIME").expect("explicit built runtime"),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.temp.path().join("exported-game");
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
    let deployed = open(&bundle.join("project"));
    assert!(deployed
        .models()
        .document
        .bindings
        .values()
        .any(|binding| binding.package == "imported-courier"
            && binding.asset == "model.animated.json"
            && binding.kind == ModelKind::Animated));
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
