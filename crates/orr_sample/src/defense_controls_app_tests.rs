//! Actual CPU App/session commands and private settings; no native window/device.
use super::*;
use orr_input::{ActionMap, Button, Key as InputKey};
fn frame(a: &mut App, events: Vec<egui::Event>, delta: f64) {
    audio_tests::frame(a, events, delta);
}
fn key(k: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        })
        .collect()
}
fn click(a: &mut App, x: f32, y: f32) {
    for pressed in [true, false] {
        frame(
            a,
            vec![egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }],
            0.0,
        );
    }
}
fn rebind(map: &mut ActionMap, name: &str, key: InputKey) {
    map.actions
        .iter_mut()
        .find(|a| a.name == name)
        .unwrap()
        .bindings = vec![Button::Keyboard { key }];
}
fn directory() -> (tempfile::TempDir, PathBuf) {
    #[cfg(windows)]
    {
        crate::defense_audio_settings::windows_test_directory()
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().to_owned();
        (dir, path)
    }
}
fn sentinel(path: &std::path::Path) {
    #[cfg(windows)]
    crate::defense_audio_settings::windows_test_write(
        path,
        b"audio sentinel must remain untouched",
    );
    #[cfg(target_os = "linux")]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap()
            .write_all(b"audio sentinel must remain untouched")
            .unwrap();
    }
}
#[test]
fn controls_app_custom_keys_produce_identical_session_frames_through_pause_focus_and_restart() {
    let mut a = audio_tests::app();
    let mut b = audio_tests::app();
    for (name, key) in ["summon", "upgrade", "invest", "restart"].into_iter().zip([
        InputKey::K,
        InputKey::L,
        InputKey::M,
        InputKey::N,
    ]) {
        rebind(&mut b.ui.controls.draft, name, key);
    }
    b.ui.controls.apply().unwrap();
    audio_tests::start(&mut a);
    audio_tests::start(&mut b);
    for tick in 0..100 {
        let (mut left, mut right) = match tick % 12 {
            0 => (key(egui::Key::Space), key(egui::Key::K)),
            3 => (key(egui::Key::U), key(egui::Key::L)),
            6 => (key(egui::Key::I), key(egui::Key::M)),
            9 => (key(egui::Key::R), key(egui::Key::N)),
            _ => (vec![], vec![]),
        };
        if tick == 30 {
            left = key(egui::Key::P);
            right = left.clone();
        }
        if tick == 32 {
            left.extend(key(egui::Key::Enter));
            right.extend(key(egui::Key::Enter));
        }
        if tick == 50 {
            a.set_focused(false);
            b.set_focused(false);
        }
        if tick == 52 {
            a.set_focused(true);
            b.set_focused(true);
        }
        if tick == 54 {
            left = key(egui::Key::Enter);
            right = left.clone();
        }
        frame(&mut a, left, 1.0 / 60.0);
        frame(&mut b, right, 1.0 / 60.0);
        assert_eq!(
            a.session.predicted_frame().to_bytes(),
            b.session.predicted_frame().to_bytes(),
            "tick {tick}"
        );
    }
}
#[test]
fn controls_app_modal_capture_apply_reset_close_freeze_game_and_require_continue() {
    let mut a = audio_tests::app();
    audio_tests::start(&mut a);
    frame(&mut a, vec![], 1.0 / 60.0);
    click(&mut a, 1100.0, 30.0);
    assert!(a.ui.controls.open);
    assert!(a.ui.paused);
    let before = a.session.predicted_frame().to_bytes();
    let mut events = key(egui::Key::Space);
    events.extend(key(egui::Key::R));
    events.extend(key(egui::Key::Enter));
    frame(&mut a, events, 10.0);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    assert!(a.commands.is_empty());
    click(&mut a, 500.0, 370.0);
    frame(&mut a, key(egui::Key::K), 10.0);
    click(&mut a, 400.0, 630.0);
    assert_eq!(a.ui.controls.hint(0, false), "K");
    click(&mut a, 640.0, 630.0);
    assert_eq!(a.ui.controls.hint(0, true), "SPACE");
    frame(&mut a, key(egui::Key::Escape), 10.0);
    assert!(!a.ui.controls.open);
    assert!(a.ui.paused);
    assert_eq!(a.ui.controls.hint(0, false), "K");
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    let mut events = key(egui::Key::Enter);
    events.extend(key(egui::Key::K));
    frame(&mut a, events, 10.0);
    assert!(!a.ui.paused);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    assert!(a.commands.is_empty());
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(game::run(a.session.predicted_frame()).last_sequence, 0);
}
#[test]
fn controls_app_real_ui_save_restores_into_new_app_without_touching_audio_or_starting_game() {
    let (_root, path) = directory();
    sentinel(&path.join("audio-settings.json"));
    let options = Options {
        controls_directory: Some(path.clone()),
        ..Default::default()
    };
    assert!(prepare_audio_settings(&options).unwrap().is_none());
    let mut a = audio_tests::app();
    a.ui.controls = prepare_controls(&options).unwrap().unwrap();
    let before = a.session.predicted_frame().to_bytes();
    click(&mut a, 1100.0, 30.0);
    click(&mut a, 500.0, 370.0);
    frame(&mut a, key(egui::Key::K), 0.0);
    click(&mut a, 400.0, 630.0);
    assert!(!path.join("controls-settings.json").exists());
    click(&mut a, 520.0, 630.0);
    assert!(a.ui.controls.status.contains("saved"));
    assert!(!a.ui.started);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    let mut b = audio_tests::app();
    b.ui.controls = prepare_controls(&options).unwrap().unwrap();
    assert_eq!(b.ui.controls.hint(0, false), "K");
    frame(&mut b, key(egui::Key::K), 0.0);
    assert!(!b.ui.started);
    audio_tests::start(&mut b);
    frame(&mut b, key(egui::Key::K), 1.0 / 60.0);
    for _ in 0..3 {
        frame(&mut b, vec![], 1.0 / 60.0);
    }
    assert!(game::units(b.session.predicted_frame())
        .iter()
        .any(|u| u.board == 0 && u.active != 0));
    assert_eq!(
        std::fs::read(path.join("audio-settings.json")).unwrap(),
        b"audio sentinel must remain untouched"
    );
    assert!(!path.join("audio-settings.lock").exists());
}
#[test]
fn controls_app_relaunch_process_restores_saved_action_map() {
    const PATH: &str = "ORR_CONTROLS_TEST_CHILD_PATH";
    const MODE: &str = "ORR_CONTROLS_TEST_CHILD_MODE";
    if let Some(path) = std::env::var_os(PATH) {
        let options = Options {
            controls_directory: Some(path.into()),
            ..Default::default()
        };
        let mut c = prepare_controls(&options).unwrap().unwrap();
        if std::env::var(MODE).unwrap() == "write" {
            assert!(!options
                .controls_directory
                .as_ref()
                .unwrap()
                .join("controls-settings.json")
                .exists());
            rebind(&mut c.draft, "summon", InputKey::K);
            c.save_requested = true;
            crate::defense_controls::process_save(&mut c);
            assert!(c.status.contains("saved"));
        } else {
            assert_eq!(c.hint(0, false), "K");
        }
        return;
    }
    let (_root, path) = directory();
    sentinel(&path.join("audio-settings.json"));
    for mode in ["write", "read"] {
        let result=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","defense_app::controls_tests::controls_app_relaunch_process_restores_saved_action_map","--test-threads=1"])
            .env(PATH,&path).env(MODE,mode).output().unwrap();
        assert!(
            result.status.success(),
            "{mode}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(
        std::fs::read(path.join("audio-settings.json")).unwrap(),
        b"audio sentinel must remain untouched"
    );
}
#[test]
fn controls_app_cli_admission_refuses_headless_relative_duplicate_missing_and_source_directories() {
    for args in [
        vec!["--controls-directory"],
        vec![
            "--controls-directory",
            "/missing",
            "--controls-directory",
            "/missing",
        ],
        vec!["--headless", "--controls-directory", "/missing"],
    ] {
        assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
    }
    assert!(headless(Options {
        controls_directory: Some("/missing".into()),
        ..Default::default()
    })
    .unwrap_err()
    .contains("native interactive"));
    assert!(prepare_controls(&Options {
        controls_directory: Some("relative".into()),
        ..Default::default()
    })
    .is_err());
    assert!(prepare_controls(&Options {
        controls_directory: Some(env!("CARGO_MANIFEST_DIR").into()),
        ..Default::default()
    })
    .is_err());
    assert!(prepare_controls(&Options::default()).unwrap().is_none());
}
