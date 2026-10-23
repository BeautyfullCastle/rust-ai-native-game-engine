//! Actual grid CLI -> immutable installation -> editor binding/Save/reopen/Play.
//! CPU/API acceptance only: no widget clicks, framebuffer or native input proof.
#![cfg(all(feature = "sprites", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_editor::{
    editor::input::{Keys, Phase},
    game::EditorGame,
    sprite_bindings::{self, Source},
    sprite_panel::SpritePanel,
    Editor,
};
use orr_package::{Project, Runtime};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

const HERO: &str = "e_00000001";
const PACKAGE: &str = "grid-editor";
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";

fn wait_for(
    editor: &mut Editor,
    panel: &mut SpritePanel,
    label: &str,
    ready: impl Fn(&Editor, &SpritePanel) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.pump();
        panel.update(editor);
        if ready(editor, panel) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {label}: {:?}",
            editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires built grid CLI; explicit gate must run --ignored --exact and require one test"]
fn grid_cli_install_editor_save_reopen_and_locomotion() {
    let binary = std::env::var_os("ORR_PNG_GRID_PACKAGE_BIN")
        .expect("explicit grid consumer requires ORR_PNG_GRID_PACKAGE_BIN");
    let binary = Path::new(&binary)
        .canonicalize()
        .expect("grid CLI must exist");
    assert!(binary.is_file());
    let temp_root = fs::canonicalize(std::env::temp_dir()).unwrap();
    let temp = tempfile::tempdir_in(temp_root).unwrap();
    let source = temp.path().join("source");
    let output = temp.path().join("package");
    let project = temp.path().join("project");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&project).unwrap();
    // Four asymmetric, differently coloured cells, including transparency.
    let rgba: [u8; 16] = [
        231, 17, 39, 255, 13, 211, 47, 127, 29, 61, 229, 255, 89, 101, 113, 0,
    ];
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    fs::write(source.join("sheet.png"), &encoded).unwrap();
    fs::write(source.join("LICENSE.txt"), b"Synthetic fixture, CC0-1.0\n").unwrap();
    fs::write(source.join("grid.json"), serde_json::to_vec(&json!({
        "schema":1,"name":PACKAGE,"version":"1.0.0","license":"LICENSE.txt",
        "png":"sheet.png","cell_width":1,"cell_height":1,"columns":2,"rows":2,
        "clips":[
            {"id":"idle","mode":"loop","frames":[{"cell":0,"duration_ms":100},{"cell":1,"duration_ms":100}]},
            {"id":"walk","mode":"loop","frames":[{"cell":2,"duration_ms":100},{"cell":3,"duration_ms":100}]}
        ]
    })).unwrap()).unwrap();
    let result = Command::new(binary)
        .arg("--recipe")
        .arg(source.join("grid.json"))
        .arg("--out")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "grid CLI failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(output.join("atlas.png")).unwrap(), encoded);
    Project::open_for_install(&project, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&output))
        .unwrap();
    // Neither the recipe nor published source path remains available to consumers.
    fs::rename(&source, temp.path().join("hidden-source")).unwrap();
    fs::rename(&output, temp.path().join("hidden-package")).unwrap();
    sprite_bindings::open_project(&project)
        .unwrap()
        .verify()
        .unwrap();
    let asset = sprite_bindings::load_asset(&project, PACKAGE, "sprites.json").unwrap();
    assert_eq!(asset.rgba, rgba);
    for (id, x, y) in [(0, 0, 0), (1, 1, 0), (2, 0, 1), (3, 1, 1)] {
        let region = asset.document.region(id).unwrap();
        assert_eq!(
            (region.x, region.y, region.width, region.height),
            (x, y, 1, 1)
        );
    }
    let scene = project.join("arena.scene.yaml");
    let sidecar = project.join("arena.sprites.json");
    fs::write(&scene, SCENE).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.select_named("hero"));
    let checksum = editor.checksum();
    let context = egui::Context::default();
    let mut panel = SpritePanel::default();
    panel
        .create(sidecar.clone(), "arena.scene.yaml".into(), ".".into())
        .unwrap();
    panel.package = PACKAGE.into();
    panel.document = "sprites.json".into();
    panel.source = Source::Locomotion {
        idle: "idle".into(),
        walk: "walk".into(),
    };
    panel.assign(&context, &editor).unwrap();
    panel.bindings.as_mut().unwrap().save().unwrap();
    let saved = panel.bindings.as_ref().unwrap().document().clone();
    assert_eq!(
        editor.checksum(),
        checksum,
        "presentation authoring must not edit simulation"
    );
    assert_eq!(fs::read_to_string(&scene).unwrap(), SCENE);
    drop(panel);
    drop(editor);

    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    let mut panel = SpritePanel::default();
    panel.open(&context, sidecar.clone()).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &saved);
    assert_eq!(editor.checksum(), checksum);
    editor.play();
    wait_for(
        &mut editor,
        &mut panel,
        "idle playback and realtime control",
        |e, p| e.can_take_control() && matches!(p.sampled_region(e, HERO), Some(0 | 1)),
    );
    assert!(editor.take_control(0, true));
    wait_for(&mut editor, &mut panel, "managed input grant", |e, _| {
        e.input_phase() == Phase::Active
    });
    editor.arena_keys(
        true,
        Keys {
            x: 1,
            y: 0,
            fire: false,
        },
    );
    wait_for(
        &mut editor,
        &mut panel,
        "actual hero movement and walk clip",
        |e, p| p.playback.state(HERO).moving && matches!(p.sampled_region(e, HERO), Some(2 | 3)),
    );
    editor.arena_keys(true, Keys::default());
    wait_for(
        &mut editor,
        &mut panel,
        "neutral input returns to idle clip",
        |e, p| !p.playback.state(HERO).moving && matches!(p.sampled_region(e, HERO), Some(0 | 1)),
    );
    editor.release_control();
    wait_for(&mut editor, &mut panel, "managed input release", |e, _| {
        e.input_phase() == Phase::Off
    });
    editor.pause();
    editor.sync();
    editor.stop().expect("play recording remains available");
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &saved);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(fs::read_to_string(&scene).unwrap(), SCENE);
    sprite_bindings::open_project(&project)
        .unwrap()
        .verify()
        .unwrap();
}
