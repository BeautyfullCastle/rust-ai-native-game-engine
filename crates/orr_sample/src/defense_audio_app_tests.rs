//! Actual CPU egui/App admission and offline Kira, never a window/device.
use super::*;
use crate::{
    arena_audio::AudioMode,
    defense_audio::{
        tests::{bank, render, silence},
        DefenseAudio,
    },
};
fn app() -> App {
    App {
        audio: DefenseAudio::offline(&bank(), AudioMode::Required),
        #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
        saves: None,
        session: session(Default::default()),
        ui: DefenseUi::new(),
        gfx: None,
        last: Instant::now(),
        accumulator: 0.0,
        commands: VecDeque::new(),
        error: None,
        focused: true,
        focus_lost_pending: false,
        board: PendingBoard::default(),
    }
}

#[cfg(all(feature = "coop-defense-audio-settings", any(target_os = "linux", windows)))]
#[test]
fn audio_settings_explicit_click_restores_without_starting_or_advancing_game() {
    use crate::defense_audio_settings::{self as settings, Preferences, Store, Outcome};
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(target_os = "linux")]
    let dir = tempfile::tempdir().unwrap();
    #[cfg(target_os = "linux")]
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    #[cfg(target_os = "linux")]
    let path = dir.path().to_owned();
    #[cfg(windows)]
    let (_dir, path) = settings::windows_test_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    assert!(matches!(store.save(&Preferences::new(40, false).unwrap()), Outcome::Saved));
    let mut a = app();
    settings::install(&mut a.ui, Store::open(&path, &[]).unwrap());
    let before = a.session.predicted_frame().to_bytes();
    frame(&mut a, vec![], 0.0);
    silence(a.audio.offline_owner());
    assert!(!a.ui.started);
    assert_eq!((a.ui.audio.gain_percent, a.ui.audio.muted), (40, false));
    click(&mut a, 690.0, 30.0);
    assert_eq!(a.ui.audio.gain_percent, 30);
    assert_eq!(Store::open(&path, &[]).unwrap().preferences.gain_percent, 40);
    click(&mut a, 840.0, 30.0);
    assert_eq!(Store::open(&path, &[]).unwrap().preferences.gain_percent, 30);
    assert!(a.ui.audio_settings.status.contains("saved"));
    assert!(!a.ui.started);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    start(&mut a);
    summon(&mut a);
    assert!(render(a.audio.offline_owner(), 256).iter().any(|v| v.abs() > 0.00001));
}

#[cfg(all(feature = "coop-defense-audio-settings", any(target_os = "linux", windows)))]
#[test]
fn audio_settings_cli_and_direct_headless_api_refuse_without_filesystem_effects() {
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for input in [vec!["--audio-settings-directory"],
        vec!["--audio-settings-directory", "/missing", "--audio-settings-directory", "/missing"],
        vec!["--headless", "--audio-settings-directory", "/missing"]] {
        assert!(Options::parse(args(&input)).is_err());
    }
    let options = Options { audio_settings_directory: Some("/missing".into()), ..Default::default() };
    assert!(headless(options).unwrap_err().contains("native interactive"));
    let options = Options { audio_settings_directory: Some("relative".into()), ..Default::default() };
    assert!(prepare_audio_settings(&options).is_err());
    assert!(prepare_audio_settings(&Options::default()).unwrap().is_none());
}
fn frame(a: &mut App, events: Vec<egui::Event>, delta: f64) {
    let mut output = a.process_input(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 940.0),
            )),
            events,
            ..Default::default()
        },
        delta,
    );
    output.textures_delta.clear(); // CPU-only harness has no renderer to consume uploads.
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
    frame(
        a,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            })
            .collect(),
        0.0,
    )
}
fn start(a: &mut App) {
    frame(a, key(egui::Key::Enter), 0.0);
    assert!(a.ui.started);
}
fn summon(a: &mut App) {
    frame(a, key(egui::Key::Space), 1.0 / 60.0);
    for _ in 0..2 {
        frame(a, vec![], 1.0 / 60.0);
    }
}
#[test]
fn app_start_mute_pause_focus_resume_restart_and_no_backlog() {
    let mut a = app();
    start(&mut a);
    summon(&mut a);
    assert_eq!(a.audio.offline_owner().output.stats().started, 0);
    a.ui.audio.muted = false;
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(a.audio.offline_owner().output.stats().started, 0);
    summon(&mut a);
    assert!(render(a.audio.offline_owner(), 256)
        .iter()
        .any(|v| v.abs() > 0.00001));
    frame(&mut a, key(egui::Key::P), 0.0);
    silence(a.audio.offline_owner());
    let stopped = a.session.predicted_frame().to_bytes();
    let count = a.audio.offline_owner().output.stats().started;
    frame(&mut a, key(egui::Key::Space), 10.0);
    assert_eq!(a.session.predicted_frame().to_bytes(), stopped);
    frame(&mut a, key(egui::Key::Enter), 10.0);
    assert_eq!(a.session.predicted_frame().to_bytes(), stopped);
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(a.audio.offline_owner().output.stats().started, count);
    summon(&mut a);
    a.set_focused(false);
    silence(a.audio.offline_owner());
    assert!(a.ui.paused);
    a.set_focused(true);
    frame(&mut a, vec![], 0.0);
    assert!(a.ui.paused);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::R), 1.0 / 60.0);
    for _ in 0..2 {
        frame(&mut a, vec![], 1.0 / 60.0);
    }
    assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
    let resets = a.audio.offline_owner().output.stats().resets;
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(a.audio.offline_owner().output.stats().resets, resets);
    silence(a.audio.offline_owner());
    assert!(a.error.is_none());
}
#[test]
fn output_failure_polled_while_paused_and_required_stops_admission() {
    for mode in [AudioMode::Auto, AudioMode::Required] {
        let mut a = app();
        a.audio = DefenseAudio::offline(&bank(), mode);
        start(&mut a);
        frame(&mut a, key(egui::Key::P), 0.0);
        let before = a.session.predicted_frame().to_bytes();
        a.audio.injected_error = Some("stream failure\n".repeat(100));
        frame(&mut a, key(egui::Key::Enter), 1.0);
        assert!(!a.audio.enabled());
        assert!(a.audio.status().chars().count() <= 160);
        assert!(!a.audio.status().contains('\n'));
        assert_eq!(a.error.is_some(), mode == AudioMode::Required);
        assert_eq!(a.session.predicted_frame().to_bytes(), before);
        frame(&mut a, vec![], 1.0 / 60.0);
        if mode == AudioMode::Required {
            assert_eq!(a.session.predicted_frame().to_bytes(), before);
        } else {
            assert_ne!(a.session.predicted_frame().to_bytes(), before);
        }
    }
}
#[test]
fn cli_options_reject_unsupported_combinations_before_output() {
    for args in [
        vec!["--audio", "required"],
        vec![
            "--headless",
            "--audio",
            "required",
            "--audio-bank",
            "/missing",
        ],
        vec!["--audio", "bogus"],
        vec!["--audio", "off", "--audio", "auto"],
    ] {
        assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
    }
    let options = Options {
        audio_bank: Some("/definitely-not-a-bank".into()),
        audio: AudioMode::Off,
        ..Default::default()
    };
    assert!(
        prepare_audio(&options).is_err(),
        "off still validates an explicitly supplied bank"
    );
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
fn saved_app() -> (tempfile::TempDir, App) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut a = app();
    a.saves = Some(RunSaves {
        store: crate::defense_run_save_store::RunStore::open(dir.path(), &[]).unwrap(),
        identity: "a".repeat(64),
        journal: crate::defense_run_save::Journal::new(
            crate::defense_run_save::Origin { scene: None },
            &a.session,
        ),
        candidate: None,
    });
    (dir, a)
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn actual_save_load_success_cancel_failure_are_silent_and_binding_atomic() {
    use crate::defense_ui::RunSaveMode;
    let (_dir, mut a) = saved_app();
    start(&mut a);
    a.ui.audio.muted = false;
    summon(&mut a);
    for _ in 0..500 {
        frame(&mut a, vec![], 1.0 / 60.0);
        render(a.audio.offline_owner(), 800);
    }
    frame(&mut a, key(egui::Key::P), 0.0);
    click(&mut a, 450.0, 582.0);
    assert_eq!(a.saves.as_ref().unwrap().store.list().unwrap().len(), 1);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::R), 1.0 / 60.0);
    for _ in 0..2 {
        frame(&mut a, vec![], 1.0 / 60.0);
    }
    assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
    frame(&mut a, key(egui::Key::P), 0.0);
    let before = a.session.predicted_frame().to_bytes();
    let binding = a.audio.offline_owner().bindings;
    click(&mut a, 750.0, 582.0);
    click(&mut a, 600.0, 321.0);
    click(&mut a, 450.0, 642.0);
    assert!(matches!(a.ui.run_save.mode, RunSaveMode::Loading));
    silence(a.audio.offline_owner());
    assert_eq!(a.audio.offline_owner().bindings, binding);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    click(&mut a, 800.0, 642.0);
    assert!(a.saves.as_ref().unwrap().candidate.is_none());
    assert_eq!(a.audio.offline_owner().bindings, binding);
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    click(&mut a, 750.0, 582.0);
    click(&mut a, 600.0, 321.0);
    click(&mut a, 450.0, 642.0);
    let started = a.audio.offline_owner().output.stats().started;
    for _ in 0..20 {
        frame(&mut a, vec![], 0.1);
        render(a.audio.offline_owner(), 800);
        if a.saves.as_ref().unwrap().candidate.is_none() {
            break;
        }
    }
    assert!(a.saves.as_ref().unwrap().candidate.is_none());
    assert!(a.ui.paused);
    assert_eq!(a.audio.offline_owner().bindings, binding + 1);
    assert_eq!(game::run(a.session.predicted_frame()).generation, 0);
    assert_eq!(a.audio.offline_owner().output.stats().started, started);
    silence(a.audio.offline_owner());
    // The same saved generation/tick is a fresh binding too, not a stale epoch.
    click(&mut a, 750.0, 582.0);
    click(&mut a, 600.0, 321.0);
    click(&mut a, 450.0, 642.0);
    for _ in 0..20 {
        frame(&mut a, vec![], 0.1);
        if a.saves.as_ref().unwrap().candidate.is_none() {
            break;
        }
    }
    assert_eq!(a.audio.offline_owner().bindings, binding + 2);
    silence(a.audio.offline_owner());
    frame(&mut a, key(egui::Key::Enter), 0.0);
    summon(&mut a);
    assert!(render(a.audio.offline_owner(), 800)
        .iter()
        .any(|v| v.abs() > 0.00001));
    assert!(a.audio.offline_owner().output.stats().started > started);
    frame(&mut a, key(egui::Key::P), 0.0);
    // Real store corruption is rejected before replacing the admitted live owner.
    let name = a.saves.as_ref().unwrap().store.list().unwrap()[0].clone();
    // A malformed candidate through the same real App modal route.
    let dir_path = _dir.path().join(name);
    std::fs::write(dir_path, b"invalid").unwrap();
    let before = a.session.predicted_frame().to_bytes();
    click(&mut a, 750.0, 582.0);
    click(&mut a, 600.0, 321.0);
    click(&mut a, 450.0, 642.0);
    assert!(a.saves.as_ref().unwrap().candidate.is_none());
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    assert_eq!(a.audio.offline_owner().bindings, binding + 2);
    silence(a.audio.offline_owner());
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn audio_controls_leave_admission_journal_and_saved_identity_exact() {
    let (_one, mut with) = saved_app();
    let (_two, mut without) = saved_app();
    without.audio = DefenseAudio::default();
    start(&mut with);
    start(&mut without);
    for tick in 0..180 {
        with.ui.audio.muted = tick % 3 == 0;
        with.ui.audio.gain_percent = (tick % 101) as u8;
        let events = if tick % 45 == 0 {
            key(egui::Key::Space)
        } else {
            vec![]
        };
        frame(&mut with, events.clone(), 1.0 / 60.0);
        frame(&mut without, events, 1.0 / 60.0);
        render(with.audio.offline_owner(), 800);
        assert_eq!(
            with.session.predicted_frame().to_bytes(),
            without.session.predicted_frame().to_bytes()
        );
        assert_eq!(with.ui.high_water(), without.ui.high_water());
    }
    let encode = |a: &mut App| {
        let saves = a.saves.as_mut().unwrap();
        saves
            .journal
            .encode(&a.session, a.ui.high_water(), &saves.identity)
            .unwrap()
    };
    assert_eq!(encode(&mut with), encode(&mut without));
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn failed_digest_and_detached_checksum_reconstruction_preserve_audio_binding() {
    for resign in [false, true] {
        let (_dir, mut a) = saved_app();
        start(&mut a);
        a.ui.audio.muted = false;
        for _ in 0..300 {
            frame(&mut a, vec![], 1.0 / 60.0);
            render(a.audio.offline_owner(), 800);
        }
        frame(&mut a, key(egui::Key::P), 0.0);
        let saves = a.saves.as_mut().unwrap();
        let bytes = saves
            .journal
            .encode(&a.session, a.ui.high_water(), &saves.identity)
            .unwrap();
        let mut damaged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        damaged["final_checksum"] =
            serde_json::json!(damaged["final_checksum"].as_u64().unwrap() ^ 1);
        let mut bytes = serde_json::to_vec(&damaged).unwrap();
        if resign {
            bytes = crate::defense_run_save::resign_test_payload(&bytes);
        }
        saves.store.save(&bytes).unwrap();
        let before = a.session.predicted_frame().to_bytes();
        let started = a.audio.offline_owner().output.stats().started;
        click(&mut a, 750.0, 582.0);
        click(&mut a, 600.0, 321.0);
        click(&mut a, 450.0, 642.0);
        for _ in 0..20 {
            frame(&mut a, vec![], 0.1);
            if a.saves.as_ref().unwrap().candidate.is_none() {
                break;
            }
        }
        assert!(a.saves.as_ref().unwrap().candidate.is_none());
        assert_eq!(a.audio.offline_owner().bindings, 0);
        assert_eq!(a.audio.offline_owner().output.stats().started, started);
        assert_eq!(a.session.predicted_frame().to_bytes(), before);
        silence(a.audio.offline_owner());
        assert!(
            a.ui.run_save
                .status
                .contains(if resign { "checksum" } else { "integrity" }),
            "{}",
            a.ui.run_save.status
        );
    }
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn required_failure_during_save_modal_does_not_reconstruct_or_advance() {
    let (_dir, mut a) = saved_app();
    start(&mut a);
    frame(&mut a, key(egui::Key::P), 0.0);
    click(&mut a, 750.0, 582.0);
    let before = a.session.predicted_frame().to_bytes();
    a.audio.injected_error = Some("stream disconnected".into());
    frame(&mut a, vec![], 10.0);
    assert!(a.error.is_some());
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    assert!(!a.audio.enabled());
}

#[cfg(target_os = "linux")]
pub(crate) fn authored_fixture_app_pcm(
    bank: &orr_package::coop_audio_admission::AdmittedBank,
) -> (Vec<f32>, u64) {
    #[cfg(feature = "coop-defense-run-save")]
    let (_with_dir, mut with) = saved_app();
    #[cfg(feature = "coop-defense-run-save")]
    let (_without_dir, mut without) = saved_app();
    #[cfg(not(feature = "coop-defense-run-save"))]
    let mut with = app();
    #[cfg(not(feature = "coop-defense-run-save"))]
    let mut without = app();
    with.audio = DefenseAudio::offline(bank, AudioMode::Required);
    without.audio = DefenseAudio::default();
    start(&mut with);
    start(&mut without);
    click(&mut with, 580.0, 30.0);
    frame(&mut without, vec![], 0.0);
    assert!(!with.ui.audio.muted);
    let mut mixed = Vec::new();
    for tick in 0..900 {
        if tick == 300 {
            frame(&mut with, key(egui::Key::P), 0.0);
            frame(&mut without, key(egui::Key::P), 0.0);
            silence(with.audio.offline_owner());
            frame(&mut with, key(egui::Key::Enter), 0.0);
            frame(&mut without, key(egui::Key::Enter), 0.0);
        }
        if tick == 600 {
            with.set_focused(false);
            without.set_focused(false);
            silence(with.audio.offline_owner());
            with.set_focused(true);
            without.set_focused(true);
            frame(&mut with, vec![], 0.0);
            frame(&mut without, vec![], 0.0);
            frame(&mut with, key(egui::Key::Enter), 0.0);
            frame(&mut without, key(egui::Key::Enter), 0.0);
        }
        let input = if tick % 45 == 0 {
            key(egui::Key::Space)
        } else {
            vec![]
        };
        frame(&mut with, input.clone(), 1.0 / 60.0);
        frame(&mut without, input, 1.0 / 60.0);
        assert_eq!(
            with.session.predicted_frame().to_bytes(),
            without.session.predicted_frame().to_bytes()
        );
        assert_eq!(
            with.session.verified_frame().map(|f| f.to_bytes()),
            without.session.verified_frame().map(|f| f.to_bytes())
        );
        #[cfg(feature = "coop-defense-run-save")]
        assert_eq!(with.ui.high_water(), without.ui.high_water());
        assert!(with.error.is_none());
        mixed.extend(render(with.audio.offline_owner(), 800));
    }
    #[cfg(feature = "coop-defense-run-save")]
    {
        let encode = |a: &mut App| {
            let s = a.saves.as_mut().unwrap();
            s.journal
                .encode(&a.session, a.ui.high_water(), &s.identity)
                .unwrap()
        };
        assert_eq!(encode(&mut with), encode(&mut without));
    }
    let started = with.audio.offline_owner().output.stats().started;
    assert!(started > 0);
    (mixed, started)
}

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn replacement_output_failure_cannot_partially_commit_loaded_session() {
    for mode in [AudioMode::Auto, AudioMode::Required] {
        let (_dir, mut a) = saved_app();
        a.audio = DefenseAudio::offline(&bank(), mode);
        start(&mut a);
        summon(&mut a);
        frame(&mut a, key(egui::Key::P), 0.0);
        click(&mut a, 450.0, 582.0);
        let saved = a.session.predicted_frame().to_bytes();
        let high_water = a.ui.high_water();
        frame(&mut a, key(egui::Key::Enter), 0.0);
        summon(&mut a);
        frame(&mut a, key(egui::Key::P), 0.0);
        assert_ne!(a.session.predicted_frame().to_bytes(), saved);
        click(&mut a, 750.0, 582.0);
        click(&mut a, 600.0, 321.0);
        a.audio.injected_replace_error = Some("replacement output failed".into());
        click(&mut a, 450.0, 642.0);
        assert_eq!(a.session.predicted_frame().to_bytes(), saved);
        assert_eq!(a.ui.high_water(), high_water);
        assert!(a.ui.paused);
        assert!(!a.audio.enabled());
        assert_eq!(a.error.is_some(), mode == AudioMode::Required);
        assert!(a.ui.run_save.status.starts_with("Loaded tick"));
        let saves = a.saves.as_mut().unwrap();
        assert!(saves
            .journal
            .encode(&a.session, a.ui.high_water(), &saves.identity)
            .is_ok());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn public_headless_api_admits_bank_even_off_and_rejects_required_without_opening_output() {
    use crate::defense_audio::tests::digest;
    let dir = tempfile::tempdir().unwrap();
    let admitted = bank();
    let mut records = Vec::new();
    for (i, record) in admitted.records().iter().enumerate() {
        let file = format!("{}.pcm", record.cue());
        std::fs::write(dir.path().join(&file), record.pcm_bytes()).unwrap();
        records.push(serde_json::json!({"cue":record.cue(),"file":file,"assetId":format!("a_000000005444{:04x}",i+1),"frames":record.frames(),"bytes":record.pcm_bytes().len(),"sha256":digest(record.pcm_bytes())}));
    }
    std::fs::write(
        dir.path().join("bank.json"),
        serde_json::to_vec(&serde_json::json!({"version":1,"sampleRate":48000,"cues":records}))
            .unwrap(),
    )
    .unwrap();
    for mode in [AudioMode::Off, AudioMode::Auto] {
        headless(Options {
            audio: mode,
            audio_bank: Some(dir.path().into()),
            ticks: 3,
            ..Default::default()
        })
        .unwrap();
    }
    assert!(headless(Options {
        audio: AudioMode::Required,
        audio_bank: Some(dir.path().into()),
        ..Default::default()
    })
    .unwrap_err()
    .contains("native interactive"));
    std::fs::write(dir.path().join("death.pcm"), b"malformed").unwrap();
    for mode in [AudioMode::Off, AudioMode::Auto] {
        assert!(headless(Options {
            audio: mode,
            audio_bank: Some(dir.path().into()),
            ..Default::default()
        })
        .is_err());
    }
}
