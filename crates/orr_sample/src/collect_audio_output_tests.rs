#![allow(clippy::float_arithmetic)]
use crate::collect_audio::{
    tests::{fixture, prepared},
    Document, PreparedAudio, ALTERNATE_ASSET,
};
use crate::collect_audio_output::{AudioMode, Playback};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, Lifecycle, PlayConfig,
    PlayHost, PlaySession, PlayerSlot, SimControl, SimHost, ViewUpdate,
};
use orr_fp::{FPVec2, FP};
use orr_games::collect_dodge_game::{
    self as game, CollectDodgeV1, CollectEvent, CollectInput, CollectLevel,
};
use std::collections::BTreeSet;

type CollectBridge = InProc<CollectDodgeV1, PlayHost<CollectDodgeV1>>;
fn bridge() -> CollectBridge {
    let level = CollectLevel::new(
        FPVec2::ZERO,
        vec![FPVec2::ZERO, FPVec2::new(FP::from_int(12), FP::ZERO)],
        vec![],
        300,
    )
    .unwrap();
    let mut config = PlayConfig::new(1, 42, 60);
    config.start_paused = false;
    InProc::new(
        PlayHost::new(PlaySession::new(config, level), PlayerSlot(0)),
        BridgeConfig::default(),
    )
}
fn pickup(event: &CollectEvent) -> bool {
    event.kind == game::EVENT_COLLECTED
}
fn event(kind: u32) -> CollectEvent {
    CollectEvent {
        kind,
        ordinal: 0,
        value: 1,
    }
}
fn update(events: Vec<BridgeEvent<CollectEvent>>) -> ViewUpdate<CollectEvent> {
    ViewUpdate {
        snapshot: None,
        events,
        resync: None,
    }
}
fn sim(tick: u64, status: EventStatus<CollectEvent>) -> BridgeEvent<CollectEvent> {
    BridgeEvent::Sim {
        key: EventKey::new(tick, 0, 0),
        status,
    }
}
fn render(owner: &mut Playback, frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.0; frames * 2];
    owner.render(&mut pcm).unwrap();
    assert!(pcm
        .iter()
        .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
    pcm
}
fn peak(pcm: &[f32]) -> f32 {
    pcm.iter().copied().map(f32::abs).fold(0.0, f32::max)
}
fn one_shot(admitted: &PreparedAudio, document: Document) -> Vec<f32> {
    let mut owner = Playback::offline(admitted.with_document(document).unwrap()).unwrap();
    owner
        .update(
            1,
            &update(vec![sim(
                1,
                EventStatus::Verified(event(game::EVENT_COLLECTED)),
            )]),
            pickup,
        )
        .unwrap();
    render(&mut owner, 12_000)
}

#[test]
fn actual_installed_clips_assignment_gain_and_mute_change_real_pcm() {
    let root = fixture();
    let admitted = prepared(root.path()).unwrap();
    let mut full = admitted.document.clone();
    full.gain = 1000;
    let loud = one_shot(&admitted, full.clone());
    assert!(peak(&loud) > 0.1);
    let mut quiet = full.clone();
    quiet.gain = 250;
    let softer = one_shot(&admitted, quiet);
    let ratio = peak(&softer) / peak(&loud);
    assert!((0.245..=0.255).contains(&ratio), "gain ratio {ratio}");
    let mut tiny = full.clone();
    tiny.gain = 1;
    let tiny_pcm = one_shot(&admitted, tiny);
    assert!(peak(&tiny_pcm) > 0.0);
    let mut other = full.clone();
    other.pickup.asset = ALTERNATE_ASSET.into();
    let alternate = one_shot(&admitted, other);
    assert_ne!(loud, alternate);
    assert!(peak(&alternate) > 0.1);
    let mut muted = full.clone();
    muted.mute = true;
    assert!(one_shot(&admitted, muted)
        .iter()
        .all(|sample| *sample == 0.0));
    let mut zero = full;
    zero.gain = 0;
    assert!(one_shot(&admitted, zero)
        .iter()
        .all(|sample| *sample == 0.0));
    std::fs::remove_dir_all(root.path().join(".orr")).unwrap();
    std::fs::remove_file(&admitted.path).unwrap();
    assert!(peak(&one_shot(&admitted, admitted.document.clone())) > 0.0);
}

#[test]
fn authored_off_auto_and_required_are_explicit_device_policies() {
    let root = fixture();
    let admitted = prepared(root.path()).unwrap();
    let mut off = Playback::open(admitted.clone(), AudioMode::Off).unwrap();
    assert!(off.status().contains("off"));
    off.update(
        1,
        &update(vec![sim(
            1,
            EventStatus::Verified(event(game::EVENT_COLLECTED)),
        )]),
        pickup,
    )
    .unwrap();
    assert_eq!(peak(&render(&mut off, 1600)), 0.0);
    assert_eq!(off.stats().started, 0);
    #[cfg(not(feature = "collect-audio-native"))]
    {
        assert!(Playback::open(admitted.clone(), AudioMode::Auto)
            .unwrap()
            .status()
            .contains("unavailable"));
        assert!(Playback::open(admitted, AudioMode::Required).is_err());
    }
}

#[test]
fn authoritative_collect_events_restart_and_every_frame_are_identical_with_audio() {
    let root = fixture();
    let admitted = prepared(root.path()).unwrap();
    let mut audio = Playback::offline(admitted).unwrap();
    let mut actual = bridge();
    let mut baseline = bridge();
    let initial = actual.poll_view();
    let initial_baseline = baseline.poll_view();
    assert_eq!(initial.events, initial_baseline.events);
    audio.update(1, &initial, pickup).unwrap();
    let mut collected = BTreeSet::new();
    let mut restarts = 0;
    let mut audible = false;
    for tick in 1..=100 {
        let input = if tick == 30 || tick == 60 {
            CollectInput {
                buttons: game::RESTART,
                ..Default::default()
            }
        } else {
            CollectInput {
                x: FP::ONE,
                ..Default::default()
            }
        };
        actual.set_input(PlayerSlot(0), input).unwrap();
        baseline.set_input(PlayerSlot(0), input).unwrap();
        actual.step(1);
        baseline.step(1);
        let view = actual.poll_view();
        let expected = baseline.poll_view();
        assert_eq!(view.events, expected.events, "events at {tick}");
        assert_eq!(view.resync, expected.resync, "recovery at {tick}");
        assert_eq!(
            view.snapshot.as_ref().unwrap().predicted().checksum(),
            expected.snapshot.as_ref().unwrap().predicted().checksum()
        );
        let bytes = actual.host().predicted_frame().to_bytes();
        assert_eq!(
            bytes,
            baseline.host().predicted_frame().to_bytes(),
            "exact Frame at {tick}"
        );
        let started = audio.stats().started;
        let mut has_collection = false;
        for notification in &view.events {
            if let BridgeEvent::Sim {
                key,
                status: EventStatus::Verified(event) | EventStatus::Predicted(event),
            } = notification
            {
                if event.kind == game::EVENT_COLLECTED {
                    collected.insert(*key);
                    has_collection = true;
                }
                if event.kind == game::EVENT_RESTARTED {
                    restarts += 1;
                }
            }
        }
        audio.update(1, &view, pickup).unwrap();
        assert_eq!(
            actual.host().predicted_frame().to_bytes(),
            bytes,
            "audio cannot mutate observed Frame"
        );
        if !has_collection {
            assert_eq!(audio.stats().started, started);
        }
        audible |= peak(&render(&mut audio, 800)) > 0.001;
        let repeated = actual.poll_view();
        assert!(repeated.events.is_empty());
        let before = audio.stats().started;
        audio.update(1, &repeated, pickup).unwrap();
        assert_eq!(
            audio.stats().started,
            before,
            "snapshot repetition must not play audio"
        );
    }
    assert!(restarts >= 2);
    assert_eq!(collected.len(), 6);
    assert!(audible);
    assert_eq!(audio.stats().started, collected.len() as u64);
    let before = audio.stats().started;
    actual.control(orr_session::ControlOp::Seek(0)).unwrap();
    baseline.control(orr_session::ControlOp::Seek(0)).unwrap();
    let seek = actual.poll_view();
    let other = baseline.poll_view();
    assert_eq!(seek.events, other.events);
    assert_eq!(
        actual.host().predicted_frame().to_bytes(),
        baseline.host().predicted_frame().to_bytes()
    );
    audio.update(1, &seek, pickup).unwrap();
    assert_eq!(
        audio.stats().started,
        before,
        "seek cannot reconstruct past one-shots"
    );
    let pcm = render(&mut audio, 4000);
    assert!(pcm[2000..].iter().all(|sample| sample.abs() < 0.000001));
}

#[test]
fn late_duplicates_and_muted_updates_keep_event_identity_after_authored_edits() {
    let root = fixture();
    let mut owner = Playback::offline(prepared(root.path()).unwrap()).unwrap();
    let cue = event(game::EVENT_COLLECTED);
    owner
        .update(
            1,
            &update(vec![sim(1, EventStatus::Predicted(cue))]),
            pickup,
        )
        .unwrap();
    assert!(peak(&render(&mut owner, 12_000)) > 0.0);
    owner
        .update(
            1,
            &update(vec![
                sim(1, EventStatus::Verified(cue)),
                sim(1, EventStatus::Verified(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 1);
    assert_eq!(owner.stats().confirmed, 1);
    assert_eq!(peak(&render(&mut owner, 2000)), 0.0);
    let mut muted = owner.document().clone();
    muted.mute = true;
    owner.set_document(muted).unwrap();
    owner
        .update(
            1,
            &update(vec![sim(2, EventStatus::Predicted(cue))]),
            pickup,
        )
        .unwrap();
    let mut unmuted = owner.document().clone();
    unmuted.mute = false;
    unmuted.pickup.asset = ALTERNATE_ASSET.into();
    unmuted.gain = 300;
    owner.set_document(unmuted).unwrap();
    owner
        .update(
            1,
            &update(vec![
                sim(2, EventStatus::Verified(cue)),
                sim(1, EventStatus::Verified(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(
        owner.stats().started,
        1,
        "unmute/assignment cannot replay tombstoned cues"
    );
    owner
        .update(1, &update(vec![sim(3, EventStatus::Verified(cue))]), pickup)
        .unwrap();
    assert_eq!(owner.stats().started, 2);
    assert!(peak(&render(&mut owner, 12_000)) > 0.01);
    let before = owner.document().clone();
    let mut invalid = before.clone();
    invalid.pickup.asset = "a_000000000000ffff".into();
    assert!(owner.set_document(invalid).is_err());
    assert_eq!(owner.document(), &before);
}

#[test]
fn cancellation_pause_seek_and_source_replacement_use_owned_audio_lifetimes() {
    let root = fixture();
    let mut owner = Playback::offline(prepared(root.path()).unwrap()).unwrap();
    let cue = event(game::EVENT_COLLECTED);
    owner
        .update(
            7,
            &update(vec![sim(1, EventStatus::Predicted(cue))]),
            pickup,
        )
        .unwrap();
    render(&mut owner, 100);
    owner
        .update(7, &update(vec![sim(1, EventStatus::Canceled)]), pickup)
        .unwrap();
    let canceled = render(&mut owner, 2000);
    assert!(canceled[2000..].iter().all(|v| v.abs() < 0.000001));
    owner
        .update(7, &update(vec![sim(1, EventStatus::Verified(cue))]), pickup)
        .unwrap();
    assert_eq!(owner.stats().started, 1);
    owner
        .update(
            7,
            &update(vec![sim(1, EventStatus::Predicted(cue))]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 2);
    owner
        .update(
            7,
            &update(vec![
                BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 2 }),
                sim(3, EventStatus::Verified(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 2);
    owner
        .update(
            7,
            &update(vec![
                BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 3 }),
                sim(3, EventStatus::Verified(cue)),
                sim(4, EventStatus::Verified(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 3);
    owner
        .update(
            7,
            &update(vec![
                BridgeEvent::Lifecycle(Lifecycle::Seeked { from: 4, to: 2 }),
                sim(1, EventStatus::Predicted(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 3);
    owner
        .update(8, &update(vec![sim(1, EventStatus::Verified(cue))]), pickup)
        .unwrap();
    assert_eq!(
        owner.stats().started,
        4,
        "new session source owns fresh event identities"
    );
    owner
        .update(
            8,
            &update(vec![
                BridgeEvent::Lifecycle(Lifecycle::Disconnected),
                sim(20, EventStatus::Verified(cue)),
            ]),
            pickup,
        )
        .unwrap();
    assert_eq!(owner.stats().started, 4);
    let pcm = render(&mut owner, 4000);
    assert!(pcm[2000..].iter().all(|sample| sample.abs() < 0.000001));
}
