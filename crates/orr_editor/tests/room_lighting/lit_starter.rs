use super::carried::{click, key, wait};
use super::*;
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("new lit room");
    let d = Document {
        follow_player: Some([1.0, 2.0, -1.0]),
        activation: Some(orr_sample::room_lighting::Activation::KeyMissing),
        ..Document::enabled_default()
    };
    orr_sample::room_lit_starter::create(&root, "editor-lit-starter", &d).unwrap();
    let original = project_files(&root);
    Fixture {
        _temp: temp,
        root,
        original,
    }
}
fn edit(f: &Fixture) {
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(
        document(&h).activation,
        Some(orr_sample::room_lighting::Activation::KeyMissing)
    );
    click(&mut h, "Room lighting");
    type_number(&mut h, "Intensity", "7");
    click(&mut h, "Apply lighting");
    click(&mut h, "Save lighting");
    assert_eq!(document(&h).point_light.unwrap().intensity, 7.0);
}
#[test]
fn created_lit_project_opens_edits_saves_reopens_and_plays_without_manifest_surgery() {
    let f = fixture();
    let manifest = fs::read(f.root.join("orr.project.json")).unwrap();
    edit(&f);
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(document(&h).point_light.unwrap().intensity, 7.0);
    assert!(h
        .state()
        .editor
        .room_point_light_settings_for_presentation()
        .unwrap()
        .point_light
        .is_some());
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, "control", |a| a.editor.can_take_control());
    click(&mut h, "Take control");
    wait(&mut h, "active", |a| {
        a.editor.input_phase() == orr_editor::editor::input::Phase::Active
    });
    key(&mut h, egui::Key::E, true);
    wait(&mut h, "actual pickup", |a| {
        a.editor.yard_rows_coherent()
            && a.editor
                .snapshot()
                .unwrap()
                .predicted()
                .singleton::<orr_sample::room_game::RoomRun>()
                .key_collected
                == 1
    });
    key(&mut h, egui::Key::E, false);
    settle(&mut h);
    assert!(h
        .state()
        .editor
        .room_point_light_settings_for_presentation()
        .unwrap()
        .point_light
        .is_none());
    assert_eq!(fs::read(f.root.join("orr.project.json")).unwrap(), manifest);
    f.assert_preserved();
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "explicit trusted built runtime required"]
fn created_edited_lit_project_exports_and_runs_with_creator_and_sources_absent() {
    use orr_sim::{PlayerSlot, TickInputs};
    use sha2::{Digest, Sha256};
    let f = fixture();
    edit(&f);
    let document = fs::read(f.root.join("room.lighting.json")).unwrap();
    let source =
        PathBuf::from(std::env::var_os("ORR_LIT_STARTER_RUNTIME").expect("explicit runtime"));
    let runtime = f._temp.path().join("trusted runtime");
    fs::copy(source, &runtime).unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    let bundle = f._temp.path().join("exported lit game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    assert_eq!(
        fs::read(bundle.join("project/room.lighting.json")).unwrap(),
        document
    );
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open_with_lighting(&bundle.join("project"), true).unwrap();
    assert_eq!(
        prepared
            .lighting()
            .unwrap()
            .document
            .point_light
            .unwrap()
            .intensity,
        7.0
    );
    for ticks in [0, 1] {
        let mut sim = prepared.scene().simulation().unwrap();
        for _ in 0..ticks {
            let mut input = TickInputs::new(sim.tick() + 1, 1);
            input.set_input(
                PlayerSlot(0),
                orr_sample::room_game::RoomInput {
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
            u8::from(ticks != 0) as u32
        );
        let output = std::process::Command::new(bundle.join("run-room-escape"))
            .current_dir(f._temp.path())
            .args([
                "--headless",
                "--ticks",
                &ticks.to_string(),
                "--hold",
                "interact",
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
        assert!(text.contains(&format!(
            "room tick: {ticks} checksum: 0x{:016x}",
            sim.frame().checksum()
        )));
        assert!(text.contains(&format!("room lighting active: {}", u8::from(ticks == 0))));
    }
}
