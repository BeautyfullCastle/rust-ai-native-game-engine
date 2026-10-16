#![allow(clippy::float_arithmetic)]

use orr_audio::{AudioConfig, Clip, OfflineAudio};
use orr_bridge::{
    BridgeEvent, EventKey, EventStatus, Lifecycle, PlayerSlot, ViewResync, ViewUpdate,
};
use std::time::Duration;

const RATE: u32 = 48_000;
fn clip() -> Clip {
    Clip::from_stereo(
        RATE,
        (0..4800)
            .map(|i| {
                let v = 0.2 * (i as f32 * std::f32::consts::TAU * 330.0 / RATE as f32).sin();
                [v, v]
            })
            .collect(),
    )
    .unwrap()
}
fn mixer() -> OfflineAudio {
    OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()
}
fn key(tick: u64) -> EventKey {
    EventKey::new(tick, 0, 0)
}
fn send(audio: &mut OfflineAudio, tick: u64, status: EventStatus<()>, sound: &Clip) {
    audio.event(key(tick), &status, |_| Some(sound.clone()));
}
fn render(audio: &mut OfflineAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    for block in samples.chunks_mut(256) {
        audio.render(block).unwrap();
    }
    samples
}
fn audible(samples: &[f32]) -> bool {
    samples.iter().any(|v| v.abs() > 0.001)
}
fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|v| v.abs() < 0.000001)
}
fn update(events: Vec<BridgeEvent<()>>) -> ViewUpdate<()> {
    ViewUpdate {
        snapshot: None,
        events,
        resync: None,
    }
}
fn event(tick: u64, status: EventStatus<()>) -> BridgeEvent<()> {
    BridgeEvent::Sim {
        key: key(tick),
        status,
    }
}
fn lifecycle(note: Lifecycle) -> BridgeEvent<()> {
    BridgeEvent::Lifecycle(note)
}

#[test]
fn real_mixer_outputs_owned_finite_pcm_then_silence() {
    let mut audio = mixer();
    let sound = clip();
    send(&mut audio, 1, EventStatus::Verified(()), &sound);
    drop(sound);
    let pcm = render(&mut audio, 6000);
    assert!(audible(&pcm[..4000]));
    assert!(pcm.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    assert!(silent(&pcm[10_000..]));
    assert_eq!(audio.stats().started, 1);
    assert_eq!(audio.voice_count(), 0);
    assert_eq!(audio.history_len(), 1);
}

#[test]
fn predicted_verified_and_duplicates_have_exactly_one_pcm_onset() {
    let sound = clip();
    let mut one = mixer();
    let mut reconciled = mixer();
    send(&mut one, 1, EventStatus::Predicted(()), &sound);
    send(&mut reconciled, 1, EventStatus::Predicted(()), &sound);
    assert_eq!(render(&mut one, 1000), render(&mut reconciled, 1000));
    send(&mut reconciled, 1, EventStatus::Predicted(()), &sound);
    send(&mut reconciled, 1, EventStatus::Verified(()), &sound);
    send(&mut reconciled, 1, EventStatus::Verified(()), &sound);
    assert_eq!(render(&mut one, 6000), render(&mut reconciled, 6000));
    assert_eq!(reconciled.stats().started, 1);
    assert_eq!(reconciled.stats().confirmed, 1);
    assert_eq!(reconciled.stats().duplicates, 2);
}

#[test]
fn completion_tombstone_prevents_late_verification_replay() {
    let mut audio = mixer();
    let sound = clip();
    send(&mut audio, 1, EventStatus::Predicted(()), &sound);
    render(&mut audio, 6000);
    assert_eq!(audio.voice_count(), 0);
    send(&mut audio, 1, EventStatus::Verified(()), &sound);
    send(&mut audio, 1, EventStatus::Predicted(()), &sound);
    assert!(silent(&render(&mut audio, 6000)));
    assert_eq!(audio.stats().started, 1);
}

#[test]
fn cancellation_really_fades_pcm_and_stale_confirmation_stays_silent() {
    let mut audio = mixer();
    let sound = clip();
    send(&mut audio, 1, EventStatus::Predicted(()), &sound);
    assert!(audible(&render(&mut audio, 1000)));
    send(&mut audio, 1, EventStatus::Canceled, &sound);
    let tail = render(&mut audio, 1600);
    assert!(
        audible(&tail[..256]),
        "cancellation is a fade, not an immediate cut"
    );
    assert!(
        silent(&tail[2048..]),
        "silent after 15 ms fade plus mixer blocks"
    );
    assert_eq!(audio.voice_count(), 0);
    send(&mut audio, 1, EventStatus::Verified(()), &sound);
    assert!(silent(&render(&mut audio, 1000)));
    assert_eq!(audio.stats().started, 1);
}

#[test]
fn canceled_then_predicted_is_a_new_occurrence_even_at_same_key() {
    let mut audio = mixer();
    let sound = clip();
    audio.update(
        1,
        &update(vec![
            event(1, EventStatus::Predicted(())),
            event(1, EventStatus::Canceled),
            event(1, EventStatus::Predicted(())),
            event(1, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 2);
    assert_eq!(audio.stats().confirmed, 1);
    assert!(audible(&render(&mut audio, 2000)));
    render(&mut audio, 6000);
    assert_eq!(audio.voice_count(), 0);
}

#[test]
fn seek_branch_resync_disconnect_and_new_source_own_their_lifetimes() {
    let mut audio = mixer();
    let sound = clip();
    audio.update(
        1,
        &update(vec![event(1, EventStatus::Predicted(()))]),
        |_| Some(sound.clone()),
    );
    render(&mut audio, 500);
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Seeked { from: 20, to: 10 }),
            event(1, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert!(silent(&render(&mut audio, 2000)[2048..]));
    audio.update(
        1,
        &update(vec![event(11, EventStatus::Verified(()))]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 2);
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Branched {
                tick: 11,
                dropped: 2,
            }),
            event(11, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    render(&mut audio, 2000);
    let mut resync = update(vec![event(15, EventStatus::Verified(()))]);
    resync.resync = Some(ViewResync {
        generation: 1,
        discarded_events: 50,
        head_tick: 20,
        verified_tick: 15,
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    });
    audio.update(1, &resync, |_| Some(sound.clone()));
    assert!(silent(&render(&mut audio, 2000)));
    audio.update(
        1,
        &update(vec![
            event(21, EventStatus::Verified(())),
            lifecycle(Lifecycle::Disconnected),
            event(22, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert!(silent(&render(&mut audio, 2000)[2048..]));
    assert_eq!(audio.stats().started, 3);
    audio.update(
        2,
        &update(vec![event(1, EventStatus::Verified(()))]),
        |_| Some(sound.clone()),
    );
    assert!(audible(&render(&mut audio, 1000)));
    assert_eq!(audio.stats().started, 4);
    audio.update(
        2,
        &update(vec![
            lifecycle(Lifecycle::SessionStarted {
                tick_rate: 60,
                local_slot: PlayerSlot(0),
                player_count: 2,
            }),
            event(1, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 5);
}

#[test]
fn pause_stops_transients_without_replay_on_resume() {
    let mut audio = mixer();
    let sound = clip();
    audio.update(
        1,
        &update(vec![event(1, EventStatus::Predicted(()))]),
        |_| Some(sound.clone()),
    );
    render(&mut audio, 500);
    audio.update(
        1,
        &update(vec![lifecycle(Lifecycle::Paused { tick: 2 })]),
        |_| Some(sound.clone()),
    );
    assert!(silent(&render(&mut audio, 2000)[2048..]));
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Resumed { tick: 2 }),
            event(1, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert!(silent(&render(&mut audio, 1000)));
    assert_eq!(audio.stats().started, 1);
}

#[test]
fn voices_drains_and_tombstones_are_bounded_and_retired_keys_never_replay() {
    let mut audio = OfflineAudio::new_offline(
        AudioConfig {
            max_voices: 2,
            history_capacity: 3,
            ..AudioConfig::default()
        },
        RATE,
    )
    .unwrap();
    let sound = clip();
    for tick in 1..10_000 {
        send(&mut audio, tick, EventStatus::Predicted(()), &sound);
        assert!(audio.voice_count() <= 2);
        assert!(audio.history_len() <= 3);
    }
    assert_eq!(audio.stats().started, 2);
    render(&mut audio, 6000);
    send(&mut audio, 1, EventStatus::Verified(()), &sound);
    send(&mut audio, 9_999, EventStatus::Verified(()), &sound);
    assert!(silent(&render(&mut audio, 1000)));
    assert_eq!(audio.stats().started, 2);
    audio.reset();
    send(&mut audio, 1, EventStatus::Verified(()), &sound);
    assert!(audible(&render(&mut audio, 1000)));
}

#[test]
fn reset_fades_cannot_bypass_the_voice_bound() {
    let mut audio = OfflineAudio::new_offline(
        AudioConfig {
            max_voices: 1,
            ..AudioConfig::default()
        },
        RATE,
    )
    .unwrap();
    let sound = clip();
    for _ in 0..1000 {
        send(&mut audio, 1, EventStatus::Predicted(()), &sound);
        audio.reset();
    }
    assert_eq!(audio.voice_count(), 1);
    assert_eq!(audio.stats().started, 1);
    render(&mut audio, 6000);
    send(&mut audio, 2, EventStatus::Verified(()), &sound);
    assert_eq!(audio.stats().started, 2);
}

#[test]
fn malformed_pcm_output_and_configuration_are_errors_not_panics() {
    assert!(Clip::from_stereo(0, vec![[0.0; 2]]).is_err());
    assert!(Clip::from_stereo(RATE, vec![]).is_err());
    assert!(Clip::from_stereo(RATE, vec![[f32::NAN, 0.0]]).is_err());
    assert!(Clip::from_stereo(RATE, vec![[1.1, 0.0]]).is_err());
    assert!(OfflineAudio::new_offline(AudioConfig::default(), 0).is_err());
    assert!(OfflineAudio::new_offline(
        AudioConfig {
            max_voices: 0,
            ..AudioConfig::default()
        },
        RATE
    )
    .is_err());
    assert!(OfflineAudio::new_offline(
        AudioConfig {
            cancel_fade: Duration::from_secs(2),
            ..AudioConfig::default()
        },
        RATE
    )
    .is_err());
    let mut audio = mixer();
    assert!(audio.render(&mut [0.0; 3]).is_err());
    assert!(audio.render(&mut []).is_ok());
}

#[test]
fn resync_preserves_pause_and_disconnect_recovery_without_replaying_history() {
    use orr_bridge::LifecycleRecovery;
    let mut audio = mixer();
    let sound = clip();
    let mut recovered = update(vec![event(11, EventStatus::Verified(()))]);
    recovered.resync = Some(ViewResync {
        generation: 2,
        discarded_events: 20,
        head_tick: 10,
        verified_tick: 10,
        disconnected: false,
        last_desync: None,
        lifecycle: vec![LifecycleRecovery {
            count: 1,
            last: Lifecycle::Paused { tick: 10 },
        }],
    });
    audio.update(1, &recovered, |_| Some(sound.clone()));
    assert_eq!(audio.stats().started, 0);
    assert!(silent(&render(&mut audio, 1000)));
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Resumed { tick: 11 }),
            event(11, EventStatus::Verified(())),
            event(12, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 1);
    recovered.resync.as_mut().unwrap().disconnected = true;
    recovered.resync.as_mut().unwrap().lifecycle = vec![LifecycleRecovery {
        count: 1,
        last: Lifecycle::Disconnected,
    }];
    recovered.events = vec![
        lifecycle(Lifecycle::Resumed { tick: 12 }),
        event(13, EventStatus::Verified(())),
    ];
    audio.update(1, &recovered, |_| Some(sound.clone()));
    assert!(silent(&render(&mut audio, 2000)[2048..]));
    assert_eq!(audio.stats().started, 1);
}

#[test]
fn pause_survives_seek_branch_and_incremental_resync_without_resume() {
    let mut audio = mixer();
    let sound = clip();
    audio.update(
        1,
        &update(vec![lifecycle(Lifecycle::Paused { tick: 1 })]),
        |_| Some(sound.clone()),
    );
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Seeked { from: 10, to: 2 }),
            event(3, EventStatus::Verified(())),
            lifecycle(Lifecycle::Branched {
                tick: 3,
                dropped: 5,
            }),
            event(4, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    let recovered = ViewUpdate {
        snapshot: None,
        events: vec![event(11, EventStatus::Verified(()))],
        resync: Some(ViewResync {
            generation: 3,
            discarded_events: 10,
            head_tick: 10,
            verified_tick: 10,
            disconnected: false,
            last_desync: None,
            lifecycle: vec![],
        }),
    };
    audio.update(1, &recovered, |_| Some(sound.clone()));
    assert_eq!(audio.stats().started, 0);
    assert!(silent(&render(&mut audio, 1000)));
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Resumed { tick: 11 }),
            event(12, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 1);
}

#[test]
fn disconnect_is_sticky_across_seek_but_old_diagnostic_cannot_undo_session_start() {
    use orr_bridge::LifecycleRecovery;
    let mut audio = mixer();
    let sound = clip();
    audio.update(
        1,
        &update(vec![
            lifecycle(Lifecycle::Disconnected),
            lifecycle(Lifecycle::Seeked { from: 10, to: 1 }),
            event(2, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 0);
    let started = Lifecycle::SessionStarted {
        tick_rate: 60,
        local_slot: PlayerSlot(0),
        player_count: 2,
    };
    audio.update(1, &update(vec![lifecycle(started)]), |_| {
        Some(sound.clone())
    });
    let mut recovered = ViewUpdate {
        snapshot: None,
        events: vec![event(4, EventStatus::Verified(()))],
        resync: Some(ViewResync {
            generation: 4,
            discarded_events: 10,
            head_tick: 3,
            verified_tick: 3,
            disconnected: true,
            last_desync: None,
            lifecycle: vec![],
        }),
    };
    audio.update(1, &recovered, |_| Some(sound.clone()));
    assert_eq!(
        audio.stats().started,
        1,
        "historical disconnect does not undo observed restart"
    );
    audio.update(
        1,
        &update(vec![lifecycle(Lifecycle::Paused { tick: 4 })]),
        |_| Some(sound.clone()),
    );
    recovered.resync.as_mut().unwrap().lifecycle = vec![LifecycleRecovery {
        count: 1,
        last: started,
    }];
    recovered.events = vec![event(5, EventStatus::Verified(()))];
    audio.update(1, &recovered, |_| Some(sound.clone()));
    assert_eq!(
        audio.stats().started,
        2,
        "recovered session start clears old pause"
    );
    let mut fresh = mixer();
    recovered.resync.as_mut().unwrap().lifecycle.clear();
    fresh.update(9, &recovered, |_| Some(sound.clone()));
    assert_eq!(
        fresh.stats().started,
        0,
        "unseen owner seeds sticky disconnect"
    );
}

#[test]
fn repeated_reset_does_not_restart_an_already_detached_fade() {
    let mut audio = mixer();
    let sound = clip();
    send(&mut audio, 1, EventStatus::Predicted(()), &sound);
    render(&mut audio, 500);
    for _ in 0..20 {
        audio.reset();
        render(&mut audio, 128);
    }
    assert_eq!(
        audio.voice_count(),
        0,
        "15 ms fade must finish despite repeated resets"
    );
}
