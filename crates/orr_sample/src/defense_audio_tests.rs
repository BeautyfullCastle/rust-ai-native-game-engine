use super::*;
use orr_games::coop_defense_game as game;
use orr_sim::SimEvent;
use sha2::{Digest, Sha256};

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn bank() -> AdmittedBank {
    // Original synthetic test PCM, distributed under this repository's MIT license.
    let mut records = Vec::new();
    let mut objects = BTreeMap::new();
    for (index, cue) in orr_package::coop_audio_admission::CUES.iter().enumerate() {
        let mut pcm = [48_000u32.to_le_bytes(), 4800u32.to_le_bytes()].concat();
        for _ in 0..4800 {
            pcm.extend_from_slice(&8192i16.to_le_bytes());
        }
        records.push(serde_json::json!({"cue":cue,"file":format!("{cue}.pcm"),"assetId":format!("a_000000005444{:04x}",index+1),"frames":4800,"bytes":pcm.len(),"sha256":digest(&pcm)}));
        objects.insert(format!("{cue}.pcm"), pcm);
    }
    orr_package::coop_audio_admission::admit(
        &serde_json::to_vec(&serde_json::json!({"version":1,"sampleRate":48000,"cues":records}))
            .unwrap(),
        &objects,
    )
    .unwrap()
}
fn owner() -> Consumer<OfflineAudio> {
    Consumer::new(&bank()).unwrap()
}
pub(crate) fn render(owner: &mut Consumer<OfflineAudio>, frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.0; frames * 2];
    for chunk in pcm.chunks_mut(256) {
        owner.output.render(chunk).unwrap();
    }
    assert!(
        pcm.iter().all(|v| v.is_finite() && v.abs() <= 0.25001),
        "coherent mix exceeded admitted peak budget"
    );
    pcm
}
fn audible(pcm: &[f32]) -> bool {
    pcm.iter().any(|v| v.abs() > 0.00001)
}
pub(crate) fn silence(owner: &mut Consumer<OfflineAudio>) {
    render(owner, 2048);
    assert!(!audible(&render(owner, 1024)));
}
fn event(kind: Kind, generation: u32) -> DefenseEvent {
    DefenseEvent {
        kind: kind as u32,
        generation,
        ..Default::default()
    }
}
fn batch(events: impl IntoIterator<Item = (EventKey, DefenseEvent)>) -> EventBatch<DefenseEvent> {
    EventBatch::verified(
        events
            .into_iter()
            .map(|(key, payload)| SimEvent { key, payload })
            .collect(),
    )
}
fn events(result: orr_session::AdvanceResult<game::DefenseGame>) -> EventBatch<DefenseEvent> {
    match result {
        orr_session::AdvanceResult::Advanced { events, .. }
        | orr_session::AdvanceResult::Stalled { events } => events,
    }
}
#[test]
fn coherent_peak_gain_mute_and_no_history_replay() {
    let mut audio = owner();
    let many =
        batch((0..MAX_VOICES as u32).map(|seq| (EventKey::new(1, 0, seq), event(Kind::Summon, 0))));
    audio.consume(&many).unwrap();
    assert_eq!(audio.output.stats().started, 0);
    audio.controls(false, 1.0, false, true).unwrap();
    audio.consume(&many).unwrap();
    assert_eq!(audio.output.stats().started, 0); // suppressed history stays tombstoned
    for tick in 2..12 {
        audio
            .controls(false, if tick % 2 == 0 { 1.0 } else { 0.1 }, false, true)
            .unwrap();
        audio
            .consume(&batch((0..MAX_VOICES as u32).map(|seq| {
                (EventKey::new(tick, 0, seq), event(Kind::Summon, 0))
            })))
            .unwrap();
        let pcm = render(&mut audio, 512);
        assert!(audible(&pcm));
        audio.controls(true, 1.0, false, true).unwrap();
        silence(&mut audio);
    }
    let gain = audio.gain;
    for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(audio.controls(false, invalid, false, true).is_err());
        assert_eq!(audio.gain, gain);
    }
}
#[test]
fn invalid_batch_is_atomic_and_restart_confirmation_does_not_reset_twice() {
    let mut a = owner();
    a.controls(false, 1.0, false, true).unwrap();
    let before = a.output.stats();
    assert!(a
        .consume(&batch([
            (EventKey::new(1, 0, 0), event(Kind::Summon, 0)),
            (EventKey::new(1, 1, 1), event(Kind::Summon, 0))
        ]))
        .is_err());
    assert_eq!(a.output.stats(), before);
    assert_eq!(a.generation, 0);
    let mut session = crate::defense_app::session(game::DefenseConfig::default());
    a.consume(&events(session.advance(
        game::DefenseInput {
            action: game::SUMMON,
            sequence: 1,
            ..Default::default()
        },
        vec![],
    )))
    .unwrap();
    for _ in 0..2 {
        a.consume(&events(session.advance(Default::default(), vec![])))
            .unwrap();
    }
    assert!(audible(&render(&mut a, 256)));
    a.consume(&events(session.advance(
        game::DefenseInput {
            action: game::RESTART,
            sequence: 2,
            ..Default::default()
        },
        vec![],
    )))
    .unwrap();
    a.consume(&events(session.advance(Default::default(), vec![])))
        .unwrap();
    let restart = events(session.advance(Default::default(), vec![]));
    assert!(restart.iter().any(
        |(_, s)| matches!(s,EventStatus::Predicted(e) if e.event_kind()==Some(Kind::Restart))
    ));
    a.consume(&restart).unwrap();
    let resets = a.output.stats().resets;
    let next = events(session.advance(Default::default(), vec![]));
    assert!(next
        .iter()
        .any(|(_, s)| matches!(s,EventStatus::Verified(e) if e.event_kind()==Some(Kind::Restart))));
    a.consume(&next).unwrap();
    assert_eq!(a.output.stats().resets, resets);
    silence(&mut a);
    a.replace(100, 0).unwrap();
    a.controls(false, 1.0, false, true).unwrap();
    a.consume(&batch([(EventKey::new(100, 0, 0), event(Kind::Summon, 0))]))
        .unwrap();
    silence(&mut a);
    a.consume(&batch([(EventKey::new(101, 0, 0), event(Kind::Summon, 0))]))
        .unwrap();
    assert!(audible(&render(&mut a, 512)));
}
#[test]
fn real_session_audio_on_off_exact_events_and_every_tick_state() {
    let mut a = owner();
    a.controls(false, 1.0, false, true).unwrap();
    let mut with = crate::defense_app::session(game::DefenseConfig::default());
    let mut without = crate::defense_app::session(game::DefenseConfig::default());
    let mut heard = false;
    let mut kinds = std::collections::BTreeSet::new();
    for tick in 1..=1800 {
        let input = if tick % 45 == 1 {
            game::DefenseInput {
                action: game::SUMMON,
                sequence: tick,
                ..Default::default()
            }
        } else {
            Default::default()
        };
        let one = events(with.advance(input, vec![]));
        let two = events(without.advance(input, vec![]));
        assert_eq!(one.clone().into_vec(), two.into_vec());
        assert_eq!(
            with.predicted_frame().to_bytes(),
            without.predicted_frame().to_bytes()
        );
        assert_eq!(
            with.verified_frame().map(|f| f.to_bytes()),
            without.verified_frame().map(|f| f.to_bytes())
        );
        for (_, s) in one.iter() {
            if let EventStatus::Predicted(e) = s {
                kinds.insert(e.kind);
            }
        }
        a.consume(&one).unwrap();
        heard |= audible(&render(&mut a, 800));
    }
    assert!(heard);
    assert!(kinds.contains(&(Kind::Release as u32)));
    assert!(kinds.contains(&(Kind::Hit as u32)));
    assert!(a.output.stats().confirmed > 0);
}
#[test]
fn actual_session_rollback_cancels_audible_prediction() {
    use orr_session::{InputSource, PlayerSlot, RemoteInput, Session, SessionConfig};
    use std::{cell::RefCell, rc::Rc};
    struct Source(Rc<RefCell<Vec<RemoteInput<game::DefenseGame>>>>);
    impl InputSource<game::DefenseGame> for Source {
        fn send_local(
            &mut self,
            _: u64,
            _: PlayerSlot,
            _: game::DefenseInput,
            _: Vec<game::NoCommand>,
        ) {
        }
        fn poll_remote(&mut self) -> Vec<RemoteInput<game::DefenseGame>> {
            std::mem::take(&mut *self.0.borrow_mut())
        }
    }
    let queue = Rc::new(RefCell::new(Vec::new()));
    let mut cfg = SessionConfig::new(1, PlayerSlot(0), 1, 60);
    cfg.relay = true;
    cfg.input_delay = 0;
    let mut session =
        Session::<game::DefenseGame, _>::new(Default::default(), cfg, Source(queue.clone()));
    let mut audio = owner();
    audio.controls(false, 1.0, false, true).unwrap();
    let predicted = events(session.advance(
        game::DefenseInput {
            action: game::SUMMON,
            sequence: 1,
            ..Default::default()
        },
        vec![],
    ));
    audio.consume(&predicted).unwrap();
    assert!(audible(&render(&mut audio, 256)));
    queue.borrow_mut().push(RemoteInput {
        tick: 1,
        slot: PlayerSlot(0),
        input: Default::default(),
        commands: vec![],
        disconnected: false,
    });
    let corrected = events(session.step());
    assert!(corrected
        .iter()
        .any(|(_, s)| matches!(s, EventStatus::Canceled)));
    audio.consume(&corrected).unwrap();
    assert!(audio.output.stats().canceled > 0);
    silence(&mut audio);
}
#[test]
fn explicit_modes_never_require_a_device_for_off_or_missing_native() {
    assert!(!DefenseAudio::open(Some(&bank()), AudioMode::Off)
        .unwrap()
        .enabled());
    assert!(DefenseAudio::open(None, AudioMode::Required).is_err());
    #[cfg(not(feature = "audio-native"))]
    {
        assert!(!DefenseAudio::open(Some(&bank()), AudioMode::Auto)
            .unwrap()
            .enabled());
        assert!(DefenseAudio::open(Some(&bank()), AudioMode::Required).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "mandatory separate source-hidden CLI bank gate requires explicit fixture/evidence paths"]
fn cli_authored_bank_source_hidden() {
    use std::path::PathBuf;
    let env = |name| {
        PathBuf::from(
            std::env::var(name).unwrap_or_else(|_| panic!("mandatory environment {name} missing")),
        )
    };
    let path = env("ORR_STORMGLASS_AUDIO_BANK");
    let expected = env("ORR_STORMGLASS_AUDIO_EXPECTED");
    let evidence = env("ORR_STORMGLASS_AUDIO_EVIDENCE_DIR");
    let originals = env("ORR_STORMGLASS_AUDIO_ORIGINALS");
    assert!(
        !originals.exists(),
        "original authoring source must be hidden"
    );
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(expected).unwrap()).unwrap();
    let bank = orr_package::coop_audio_admission::load_bank(&path).unwrap();
    let mut decoded = Vec::new();
    for record in bank.records() {
        let wanted = expected["cues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["cue"] == record.cue())
            .unwrap();
        assert_eq!(digest(record.pcm_bytes()), wanted["pcm_sha256"]);
        let mut stereo = Vec::new();
        for sample in record.pcm_bytes()[8..].chunks_exact(2) {
            let v = f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0;
            stereo.extend_from_slice(&v.to_le_bytes());
            stereo.extend_from_slice(&v.to_le_bytes());
        }
        let hash = digest(&stereo);
        assert_eq!(hash, wanted["decoded_stereo_f32le_sha256"]);
        for probe in wanted["sample_probes"].as_array().unwrap() {
            let i = probe[0].as_u64().unwrap() as usize;
            assert_eq!(
                i64::from(i16::from_le_bytes(
                    record.pcm_bytes()[8 + i * 2..10 + i * 2]
                        .try_into()
                        .unwrap()
                )),
                probe[1].as_i64().unwrap()
            );
        }
        decoded.push(serde_json::json!({"cue":record.cue(),"decoded_stereo_f32le_sha256":hash,"sample_probes":wanted["sample_probes"]}));
    }
    let license_hash = digest(&std::fs::read(path.join("LICENSE.txt")).unwrap());
    assert_eq!(license_hash, expected["license_sha256"]);
    let mapping = exact_mapped_pcm(&bank);
    let (mixed, started) = crate::defense_app::audio_tests::authored_fixture_app_pcm(&bank);
    assert!(audible(&mixed));
    let peak = mixed.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
    let pcm: Vec<_> = mixed.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("native-authored-mixed.f32le"), &pcm).unwrap();
    let receipt = serde_json::json!({"version":1,"bank_manifest_sha256":digest(&std::fs::read(path.join("bank.json")).unwrap()),"decoded":decoded,"mixed_pcm_sha256":digest(&pcm),"mixed_peak":peak,"mixed_nonzero":true,"source_hidden":true,"device_playback_verified":false,"started":started,"actual_app_egui":true,"license_sha256":license_hash,"mapped_cues":mapping,"producer_sha256":digest(&std::fs::read(std::env::current_exe().unwrap()).unwrap())});
    std::fs::write(
        evidence.join("native-authored-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
fn exact_mapped_pcm(bank: &AdmittedBank) -> Vec<serde_json::Value> {
    let cases = [
        (Kind::Summon, 0, "summon"),
        (Kind::Merge, 0, "merge"),
        (Kind::Death, 0, "death"),
        (Kind::Release, 0, "attack-light"),
        (Kind::Release, 1, "attack-heavy"),
        (Kind::Release, 2, "attack-heavy"),
        (Kind::Release, 3, "attack-magic"),
        (Kind::Hit, 0, "impact-light"),
        (Kind::Hit, 1, "impact-heavy"),
        (Kind::Hit, 2, "impact-heavy"),
        (Kind::Hit, 3, "impact-light"),
    ];
    let mut evidence = Vec::new();
    for (kind, role, name) in cases {
        let record = bank.records().iter().find(|r| r.cue() == name).unwrap();
        let frames = record.pcm_bytes()[8..]
            .chunks_exact(2)
            .map(|v| {
                let v = i16::from_le_bytes(v.try_into().unwrap()) as f32 / 32768.0;
                [v, v]
            })
            .collect();
        let clip = Clip::from_stereo(48000, frames).unwrap();
        let mut reference = OfflineAudio::new_offline_with_gain(config(), 48000, 0.0).unwrap();
        reference.set_gain(1.0 / MAX_VOICES as f32).unwrap();
        reference.event(EventKey::new(1, 0, 0), &EventStatus::Verified(()), |_| {
            Some(clip.clone())
        });
        let mut actual = Consumer::<OfflineAudio>::new(bank).unwrap();
        actual.controls(false, 1.0, false, true).unwrap();
        let e = DefenseEvent {
            role,
            ..event(kind, 0)
        };
        actual
            .consume(&batch([(EventKey::new(1, 0, 0), e)]))
            .unwrap();
        let mixed = render(&mut actual, 6000);
        let mut expected = vec![0.0; 12000];
        for block in expected.chunks_mut(256) {
            reference.render(block).unwrap();
        }
        assert_eq!(
            mixed, expected,
            "wrong decoded clip or mapping for {kind:?} role{role}"
        );
        assert!(audible(&mixed));
        evidence.push(serde_json::json!({"event":kind.name(),"role":role,"cue":name,"mixed_pcm_sha256":digest(&mixed.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>())}));
    }
    let mut audio = Consumer::<OfflineAudio>::new(bank).unwrap();
    audio.controls(false, 1.0, false, true).unwrap();
    for (seq, kind) in [
        Kind::Release,
        Kind::Hit,
        Kind::Move,
        Kind::Wave,
        Kind::Finish,
    ]
    .into_iter()
    .enumerate()
    {
        audio
            .consume(&batch([(
                EventKey::new(1, 0, seq as u32),
                DefenseEvent {
                    role: 4,
                    ..event(kind, 0)
                },
            )]))
            .unwrap();
    }
    assert_eq!(audio.output.stats().started, 0);
    assert!(!audible(&render(&mut audio, 6000)));
    evidence
}
