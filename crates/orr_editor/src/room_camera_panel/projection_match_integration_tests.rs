//! CPU UI/editor/persistence acceptance; never requests a GPU or native window.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use std::fs;
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
fn switched_project() -> (
    crate::package_manager::presentation_tests::Fixture,
    Document,
) {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let mut source = Document::readable_default();
    source.schema = 3;
    source.follow_player = Some([0.0, 1.0, -0.0]);
    source.target_bounds = Some(orr_sample::room_camera::TargetBounds {
        min: [-8.0, 0.0, -6.0],
        max: [8.0, 4.0, 6.0],
    });
    fs::write(f.root.join("room.camera.json"), source.to_bytes().unwrap()).unwrap();
    let mut prepared = open(&f.root);
    let frame = prepared.scene().frame().to_bytes();
    let character = prepared.character().unwrap().document.clone();
    let lighting = prepared.lighting().unwrap().document.clone();
    let camera = prepared.take_camera().unwrap();
    let original = fs::read(&camera.path).unwrap();
    let manifest = fs::read(&camera.manifest_path).unwrap();
    let scene_bytes = fs::read(f.root.join("room.scene.yaml")).unwrap();
    let (_, path, scene, _models) = prepared.into_parts();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.install_room_camera(source.clone()).unwrap();
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let live = editor.camera3d;
    let mut h = Harness::builder()
        .with_size([1200.0, 1800.0])
        .build_ui_state(
            |ui, state: &mut (Panel, crate::Editor, usize)| {
                if state.0.show(ui, true) {
                    state
                        .1
                        .install_room_camera(state.0.document().clone())
                        .unwrap();
                    state.2 += 1;
                }
            },
            (Panel::new(camera), editor, 0),
        );
    h.run_steps(3);
    h.get_by_label("Room camera").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Match projection scale").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Match projection aspect width", "900"),
        ("Match projection aspect height", "1600"),
    ] {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(3);
    }
    h.get_by_label("Switch projection keeping target-plane scale")
        .click_accesskit();
    h.run_steps(3);
    let expected = projection_match::match_projection(&source, (900, 1600)).unwrap();
    assert_eq!(h.state().0.candidate, expected);
    assert_eq!(h.state().0.document(), &source);
    assert!(h.state().0.undo.is_empty());
    assert!(h.state().0.redo.is_empty());
    assert_eq!(h.state().2, 0);
    assert_eq!(h.state().1.camera3d, live);
    assert_eq!(fs::read(f.root.join("room.camera.json")).unwrap(), original);
    for (label, document, count) in [
        ("Apply camera", expected.clone(), 1),
        ("Undo camera edit", source.clone(), 2),
        ("Redo camera edit", expected.clone(), 3),
    ] {
        h.get_by_label(label).click_accesskit();
        h.run_steps(3);
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        assert_eq!(h.state().0.document(), &document);
        assert_eq!(h.state().0.candidate, document);
        assert_eq!(h.state().2, count);
        let actual = h.state().1.presentation_camera3d((900, 1600)).unwrap();
        let expected_camera = document.camera(&document.orbit(), (900, 1600)).unwrap();
        assert_eq!(actual, expected_camera);
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(fs::read(f.root.join("room.camera.json")).unwrap(), original);
    }
    h.get_by_label("Save camera").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    assert!(!h.state().0.dirty());
    let reopened = open(&f.root);
    assert_eq!(reopened.camera().unwrap().document, expected);
    assert_eq!(reopened.scene().frame().to_bytes(), frame);
    assert_eq!(reopened.character().unwrap().document, character);
    assert_eq!(reopened.lighting().unwrap().document, lighting);
    assert_eq!(fs::read(f.root.join("orr.project.json")).unwrap(), manifest);
    assert_eq!(
        fs::read(f.root.join("room.scene.yaml")).unwrap(),
        scene_bytes
    );
    let camera = reopened.camera().unwrap();
    assert_eq!(
        camera
            .document
            .camera(&camera.document.orbit(), (900, 1600))
            .unwrap(),
        h.state().1.presentation_camera3d((900, 1600)).unwrap()
    );
    (f, expected)
}
#[test]
fn explicit_projection_switch_stages_applies_undoes_redoes_saves_and_reopens() {
    let _ = switched_project();
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "explicit trusted presentation runtime; CPU/headless only"]
fn room_projection_switch_survives_source_hidden_native_export() {
    use sha2::{Digest, Sha256};
    let (f, expected_camera) = switched_project();
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
        std::env::var_os("ORR_ROOM_PROJECTION_RUNTIME").expect("explicit built runtime"),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.temp.path().join("projection-game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    assert_eq!(
        open(&bundle.join("project")).camera().unwrap().document,
        expected_camera
    );
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
}
