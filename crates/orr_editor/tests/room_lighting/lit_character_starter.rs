#![cfg(feature = "room-character")]
use super::carried::{click, key, wait};
use super::*;
use orr_sample::room_project::RoomProjectCapabilities;

struct LitCharacterFixture {
    base: Fixture,
}
impl LitCharacterFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("new lit character room");
        let light = Document {
            follow_player: Some([1.0, 2.0, -1.0]),
            activation: Some(orr_sample::room_lighting::Activation::KeyMissing),
            ..Document::enabled_default()
        };
        let report = orr_sample::room_lit_starter::create_with_character(
            &root,
            "editor-lit-character",
            &light,
            true,
        )
        .unwrap();
        assert_eq!(
            report.template,
            orr_sample::room_lit_starter::CHARACTER_TEMPLATE
        );
        let prepared = PreparedProject::open_with_capabilities(
            &root,
            RoomProjectCapabilities {
                lighting: true,
                character: true,
            },
        )
        .unwrap();
        assert!(prepared.character().is_some());
        assert_eq!(prepared.lighting().unwrap().document, light);
        let original = project_files(&root);
        Self {
            base: Fixture {
                _temp: temp,
                root,
                original,
            },
        }
    }
    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open_with_capabilities(
            &self.base.root,
            RoomProjectCapabilities {
                lighting: true,
                character: true,
            },
        )
        .unwrap();
        let lighting = prepared.take_lighting().unwrap();
        let camera = prepared.take_camera().unwrap();
        let character = prepared.take_character().unwrap();
        let (_, scene_path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: scene_path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor
            .install_room_character(character.document.clone())
            .unwrap();
        editor
            .install_room_lighting(lighting.document.clone())
            .unwrap();
        editor.install_room_camera(camera.document.clone()).unwrap();
        let panel = Panel::new_for_editor(lighting, &editor).unwrap();
        Harness::builder()
            .with_size([1500.0, 1200.0])
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app.room_lighting = Some(panel);
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app.room_character = Some(
                    orr_editor::room_character_panel::Panel::new_for_editor(character, &app.editor)
                        .unwrap(),
                );
                app
            })
    }
}
fn edit(f: &LitCharacterFixture) {
    let mut h = f.app();
    settle(&mut h);
    assert!(h.state().editor.room_character().is_some());
    click(&mut h, "Room lighting");
    type_number(&mut h, "Intensity", "7");
    click(&mut h, "Apply lighting");
    click(&mut h, "Save lighting");
    assert_eq!(document(&h).point_light.unwrap().intensity, 7.0);
}
fn poses(app: &EditorApp) -> Vec<orr_model::animation::Matrix4> {
    let placements = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(placements.len(), 1);
    placements[0].pose.global().to_vec()
}
#[test]
fn lit_character_creator_opens_edits_reopens_and_animates_during_actual_pickup() {
    let f = LitCharacterFixture::new();
    edit(&f);
    let mut h = f.app();
    settle(&mut h);
    assert_eq!(document(&h).point_light.unwrap().intensity, 7.0);
    assert!(h.state().room_character.is_some());
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, "control", |a| a.editor.can_take_control());
    click(&mut h, "Take control");
    wait(&mut h, "active", |a| {
        a.editor.input_phase() == orr_editor::editor::input::Phase::Active
            && a.editor.yard_rows_coherent()
            && a.models
                .animated_placements(&a.editor)
                .is_ok_and(|p| p.len() == 1)
    });
    let searching = poses(h.state());
    wait(&mut h, "searching animation advances", |a| {
        a.editor.yard_rows_coherent() && poses(a) != searching
    });
    assert_eq!(
        orr_sample::room_character::state(h.state().editor.snapshot().unwrap().predicted()),
        orr_sample::room_character::State::Searching
    );
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
    assert_eq!(
        orr_sample::room_character::state(h.state().editor.snapshot().unwrap().predicted()),
        orr_sample::room_character::State::Carrying
    );
    let _carrying = poses(h.state());
    assert!(h
        .state()
        .editor
        .room_point_light_settings_for_presentation()
        .unwrap()
        .point_light
        .is_none());
    f.base.assert_preserved();
}
#[cfg(feature = "windows-export")]
#[test]
#[ignore = "explicit trusted character-and-lighting runtime required"]
fn lit_character_saved_export_runs_actual_animated_game_with_sources_hidden() {
    use orr_sim::{PlayerSlot, TickInputs};
    use sha2::{Digest, Sha256};
    let f = LitCharacterFixture::new();
    edit(&f);
    let saved = project_files(&f.base.root);
    let runtime = f.base._temp.path().join("trusted runtime");
    fs::copy(
        PathBuf::from(std::env::var_os("ORR_LIT_CHARACTER_RUNTIME").expect("explicit runtime")),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f.base._temp.path().join("deployed lit character game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.base.root.clone(),
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
            fs::read(bundle.join("project").join(name)).unwrap(),
            saved[Path::new(name)]
        );
    }
    fs::rename(&f.base.root, f.base.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open_with_capabilities(
        &bundle.join("project"),
        RoomProjectCapabilities {
            lighting: true,
            character: true,
        },
    )
    .unwrap();
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
    assert!(prepared.character().is_some());
    let mut sim = prepared.scene().simulation().unwrap();
    for _ in 0..8 {
        let mut input = TickInputs::new(sim.tick() + 1, 1);
        input.set_input(
            PlayerSlot(0),
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
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(f.base._temp.path())
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
    assert!(text.contains(&format!(
        "room tick: 8 checksum: 0x{:016x}",
        sim.frame().checksum()
    )));
    assert!(text.contains("room model poses: 9 frames checked;"));
    assert!(text.contains("room lighting active: 0"));
}
