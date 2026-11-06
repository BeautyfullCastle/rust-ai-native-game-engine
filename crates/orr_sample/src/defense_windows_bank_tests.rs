//! Actual Windows bank IO and CPU App/offline PCM; never a native window/device.
use super::*;
use crate::{
    arena_audio::AudioMode,
    defense_audio::{
        tests::{bank, digest, render, silence},
        DefenseAudio,
    },
};
fn disk_bank(path: &std::path::Path) {
    let bank = bank();
    let mut records = Vec::new();
    for (index, cue) in bank.records().iter().enumerate() {
        let file = format!("{}.pcm", cue.cue());
        std::fs::write(path.join(&file), cue.pcm_bytes()).unwrap();
        records.push(serde_json::json!({"cue":cue.cue(),"file":file,"assetId":format!("a_000000005444{:04x}",index+1),
            "frames":cue.frames(),"bytes":cue.pcm_bytes().len(),"sha256":digest(cue.pcm_bytes())}));
    }
    std::fs::write(
        path.join("bank.json"),
        serde_json::to_vec(&serde_json::json!({"version":1,"sampleRate":48000,"cues":records}))
            .unwrap(),
    )
    .unwrap();
}
#[test]
fn windows_bank_off_auto_and_required_admission_share_exact_pcm_without_output() {
    let dir = tempfile::tempdir().unwrap();
    disk_bank(dir.path());
    let expected = bank();
    for mode in [AudioMode::Off, AudioMode::Auto, AudioMode::Required] {
        let loaded = prepare_audio(&Options {
            audio_bank: Some(dir.path().into()),
            audio: mode,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        for (actual, expected) in loaded.records().iter().zip(expected.records()) {
            assert_eq!(
                (actual.cue(), actual.frames(), actual.pcm_bytes()),
                (expected.cue(), expected.frames(), expected.pcm_bytes())
            );
        }
        // Off is explicitly device-free even in an audio-native build.
        assert!(!DefenseAudio::open(Some(&loaded), AudioMode::Off)
            .unwrap()
            .enabled());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 11);
}
#[test]
fn windows_bank_public_headless_off_and_auto_validate_supplied_files() {
    let dir = tempfile::tempdir().unwrap();
    disk_bank(dir.path());
    for mode in [AudioMode::Off, AudioMode::Auto] {
        headless(Options {
            audio_bank: Some(dir.path().into()),
            audio: mode,
            ticks: 3,
            ..Default::default()
        })
        .unwrap();
    }
    assert!(headless(Options {
        audio_bank: Some(dir.path().into()),
        audio: AudioMode::Required,
        ..Default::default()
    })
    .unwrap_err()
    .contains("native interactive"));
    let target = dir.path().join("death.pcm");
    std::fs::write(&target, b"invalid").unwrap();
    for mode in [AudioMode::Off, AudioMode::Auto, AudioMode::Required] {
        assert!(prepare_audio(&Options {
            audio_bank: Some(dir.path().into()),
            audio: mode,
            ..Default::default()
        })
        .is_err());
    }
    for mode in [AudioMode::Off, AudioMode::Auto] {
        assert!(headless(Options {
            audio_bank: Some(dir.path().into()),
            audio: mode,
            ..Default::default()
        })
        .is_err());
    }
    assert_eq!(std::fs::read(target).unwrap(), b"invalid");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 11);
}
#[cfg(feature = "coop-defense-audio-settings")]
#[test]
fn windows_bank_loaded_pcm_and_saved_unmuted_gain_stay_silent_until_start() {
    use crate::defense_audio_settings::{self as settings, Outcome, Preferences, Store};
    let bank_dir = tempfile::tempdir().unwrap();
    disk_bank(bank_dir.path());
    let (_settings_root, settings_path) = settings::windows_test_directory();
    let mut store = Store::open(&settings_path, &[]).unwrap().store;
    assert!(matches!(
        store.save(&Preferences::new(70, false).unwrap()),
        Outcome::Saved
    ));
    let options = Options {
        audio_bank: Some(bank_dir.path().into()),
        audio: AudioMode::Required,
        audio_settings_directory: Some(settings_path),
        ..Default::default()
    };
    let loaded_settings = prepare_audio_settings(&options).unwrap().unwrap();
    let loaded_bank = prepare_audio(&options).unwrap().unwrap();
    let mut a = audio_tests::app();
    a.audio = DefenseAudio::offline(&loaded_bank, AudioMode::Required);
    settings::install(&mut a.ui, loaded_settings);
    let before = a.session.predicted_frame().to_bytes();
    audio_tests::frame(&mut a, vec![], 0.0);
    silence(a.audio.offline_owner());
    assert!(!a.ui.started);
    assert_eq!((a.ui.audio.gain_percent, a.ui.audio.muted), (70, false));
    assert_eq!(a.session.predicted_frame().to_bytes(), before);
    audio_tests::start(&mut a);
    audio_tests::summon(&mut a);
    let pcm = render(a.audio.offline_owner(), 256);
    assert!(pcm.iter().any(|v| v.abs() > 0.00001));
    assert!(pcm.iter().all(|v| v.abs() <= 0.17501));
    a.ui.audio.muted = true;
    audio_tests::frame(&mut a, vec![], 0.0);
    silence(a.audio.offline_owner());
    let options = Options {
        audio_bank: Some(bank_dir.path().into()),
        audio_settings_directory: Some(bank_dir.path().into()),
        ..Default::default()
    };
    assert!(
        prepare_audio_settings(&options).is_err(),
        "settings must not write inside the admitted bank"
    );
}
