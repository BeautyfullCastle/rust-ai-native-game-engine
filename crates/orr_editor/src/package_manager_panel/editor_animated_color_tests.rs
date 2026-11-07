//! Linux CPU acceptance of actual import form -> worker -> installed binding.
use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest, Role};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{
    fs,
    time::{Duration, Instant},
};
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() && Instant::now() < deadline {
        panel.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.busy(), "worker timeout: {}", panel.status);
}
fn imported_player(
    variant: bool,
) -> (crate::package_manager::presentation_tests::Fixture, Vec<u8>) {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let source = f.temp.path().join("author.model.animated.json");
    fs::write(&source, SOURCE).unwrap();
    let license = f.temp.path().join("author-LICENSE.txt");
    let license_bytes = b"Acceptance fixture: redistribution acknowledged";
    fs::write(&license, license_bytes).unwrap();
    let destination = f.temp.path().join("new animated package");
    let mut h = Harness::builder()
        .with_size([1400.0, 1800.0])
        .build_ui_state(
            |ui,
             state: &mut (
                crate::model_package_import_panel::Panel,
                PathBuf,
                Option<crate::model_package_import::Report>,
            )| {
                if let Some(report) = state.0.poll(Some(&state.1), 4, 7) {
                    state.2 = Some(report);
                }
                state.0.show(ui, true, Some(&state.1), 4, 7);
            },
            (
                crate::model_package_import_panel::Panel::default(),
                f.root.clone(),
                None,
            ),
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Cooked animated model").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        (
            "Cooked animated model file (.animated.json)",
            source.to_str().unwrap(),
        ),
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", "imported-courier"),
        ("Package version", "1.0.0"),
    ] {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(3);
    }
    if variant {
        h.get_by_label("Create animated color variant")
            .click_accesskit();
        h.run_steps(3);
        for (label, value) in [
            ("Variant material index", 0.0),
            ("Variant linear R (0..1)", 0.2),
            ("Variant linear G (0..1)", 0.7),
            ("Variant linear B (0..1)", 0.4),
        ] {
            let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
            h.event(egui::Event::AccessKitActionRequest(ActionRequest {
                target_node,
                target_tree,
                action: Action::SetValue,
                data: Some(ActionData::NumericValue(value)),
            }));
            h.run_steps(3);
        }
        h.get_by_role_and_label(Role::TextInput, "Variant asset ID")
            .focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, "Variant asset ID")
            .type_text("variants/courier.recolor.json");
        h.run_steps(3);
    }
    h.get_by_label("I have reviewed rights to distribute this content")
        .click_accesskit();
    h.run_steps(3);
    assert!(!destination.exists());
    h.get_by_label("Import cooked animated package")
        .click_accesskit();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().2.is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = h.state_mut().2.take().unwrap();
    let payload = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(fs::read(&license).unwrap(), license_bytes);
    assert_eq!(report.asset, "model.animated.json");
    if !variant {
        assert_eq!(payload, SOURCE);
    }
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
    (f, payload)
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
fn actual_editor_color_widgets_import_install_assign_save_and_reopen() {
    let (f, bytes) = imported_player(true);
    let prepared = open(&f.root);
    assert_variant(&prepared, &bytes);
    let baseline = crate::package_manager::presentation_tests::fixture(true, true);
    let original = open(&baseline.root);
    assert_eq!(
        prepared.scene().frame().to_bytes(),
        original.scene().frame().to_bytes()
    );
    assert_eq!(
        checksum_after_pickup(&prepared),
        checksum_after_pickup(&original)
    );
    let old = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let new = orr_model::animation::AnimatedModel::from_bytes(&bytes).unwrap();
    let mut restored = new.source().clone();
    restored.asset_id = old.source().asset_id.clone();
    restored.dependencies = old.source().dependencies.clone();
    restored.materials[0].base_color = old.source().materials[0].base_color;
    for (a, b) in old.source().primitives.iter().zip(&mut restored.primitives) {
        b.id = a.id.clone();
    }
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(old.source()).unwrap()
    );
    use sha2::{Digest, Sha256};
    let hash = |raw: &[u8]| -> String {
        Sha256::digest(raw)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let recipe=serde_json::to_vec(&serde_json::json!({"format":"orr_animated_material_color","version":1,"source":"input.animated.json","material":0,"linear_rgb":[0.2,0.7,0.4]})).unwrap();
    assert_eq!(
        new.source().dependencies,
        vec![
            orr_model::Dependency {
                uri: "$source".into(),
                sha256: hash(&recipe)
            },
            orr_model::Dependency {
                uri: "input.animated.json".into(),
                sha256: hash(SOURCE)
            }
        ]
    );
}
#[test]
fn unchecked_editor_color_option_preserves_original_cooked_import_route() {
    let (f, bytes) = imported_player(false);
    assert_eq!(bytes, SOURCE);
    let prepared = open(&f.root);
    let loaded = prepared.models().assets.values().find_map(|asset| {
        asset.animated_model().filter(|m| {
            m.source().asset_id
                == orr_model::animation::AnimatedModel::from_bytes(SOURCE)
                    .unwrap()
                    .source()
                    .asset_id
        })
    });
    assert!(loaded.is_some());
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "requires explicit trusted Room presentation runtime; CPU/headless only"]
fn editor_authored_color_survives_source_hidden_native_export() {
    use sha2::{Digest, Sha256};
    let (f, bytes) = imported_player(true);
    let expected = format!(
        "room tick: 8 checksum: 0x{:016x}",
        checksum_after_pickup(&open(&f.root))
    );
    let runtime = f.temp.path().join("trusted-runtime");
    fs::copy(
        std::env::var_os("ORR_EDITOR_ANIMATED_COLOR_RUNTIME").expect("explicit built runtime"),
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
