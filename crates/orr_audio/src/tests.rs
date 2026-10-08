use super::*;
use orr_bridge::PlayerSlot;

fn clip() -> Clip {
    let mut bytes = Vec::with_capacity(8 + 4096 * 2);
    bytes.extend_from_slice(&pcm16::SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    for _ in 0..4096 {
        bytes.extend_from_slice(&pcm16::MAX_PEAK.to_le_bytes());
    }
    pcm16::decode(&bytes).unwrap()
}

fn mixer(config: AudioConfig) -> OfflineAudio {
    OfflineAudio::new_offline(config, pcm16::SAMPLE_RATE).unwrap()
}

fn render(audio: &mut OfflineAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    for block in samples.chunks_mut(256) {
        audio.render(block).unwrap();
    }
    samples
}

fn batch(events: Vec<BridgeEvent<()>>) -> ViewUpdate<()> {
    ViewUpdate {
        snapshot: None,
        events,
        resync: None,
    }
}

fn event(tick: u64, status: EventStatus<()>) -> BridgeEvent<()> {
    BridgeEvent::Sim {
        key: EventKey::new(tick, 0, 0),
        status,
    }
}

fn gain_output(sound: Clip) -> Vec<f32> {
    let mut audio = mixer(AudioConfig::default());
    audio.update(7, &batch(vec![event(1, EventStatus::Verified(()))]), |_| {
        Some(sound.clone())
    });
    assert_eq!(audio.stats().started, 1);
    render(&mut audio, 1024)
}

#[test]
fn bounded_gain_is_applied_by_the_real_mixer_without_changing_shared_pcm() {
    let sound = clip();
    let unity = gain_output(sound.clone());
    assert!(unity.iter().any(|sample| sample.abs() > 0.01));
    for gain in [0, 1, 100, 500, 999, 1000] {
        let adjusted = sound.clone().with_gain_milli(gain).unwrap();
        assert!(Arc::ptr_eq(&sound.0.frames, &adjusted.0.frames));
        let output = gain_output(adjusted);
        assert!(output
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 0.25));
        for (sample, original) in output.iter().zip(&unity) {
            assert!((sample - original * f32::from(gain) / 1000.0).abs() < 0.000001);
        }
        if gain == 0 {
            assert!(output.iter().all(|sample| *sample == 0.0));
        } else {
            assert!(output.iter().any(|sample| sample.abs() > 0.00001));
        }
    }
    let restored = sound
        .clone()
        .with_gain_milli(500)
        .unwrap()
        .with_gain_milli(1000)
        .unwrap();
    assert_eq!(gain_output(restored), unity);
    for gain in [1001, u16::MAX] {
        assert!(sound.clone().with_gain_milli(gain).is_err());
    }
}

#[test]
fn stop_transients_keeps_identity_and_consumed_muted_events_until_real_restart() {
    let sound = clip();
    let mut audio = mixer(AudioConfig::default());
    audio.update(
        7,
        &batch(vec![event(1, EventStatus::Predicted(()))]),
        |_| Some(sound.clone()),
    );
    assert!(render(&mut audio, 256).iter().any(|sample| *sample != 0.0));
    for _ in 0..20 {
        audio.stop_transients();
        render(&mut audio, 128);
    }
    assert_eq!(
        audio.voice_count(),
        0,
        "repeated stops cannot restart fades"
    );
    assert_eq!(audio.source, Some(7));
    assert_eq!(audio.history_len(), 1);
    assert_eq!(audio.stats().resets, 0);
    assert!(render(&mut audio, 256).iter().all(|sample| *sample == 0.0));

    // Muted playback still consumes both the old confirmation and new events.
    audio.update(
        7,
        &batch(vec![
            event(1, EventStatus::Verified(())),
            event(2, EventStatus::Predicted(())),
            event(3, EventStatus::Verified(())),
        ]),
        |_| None,
    );
    assert_eq!(audio.history_len(), 3);
    audio.update(
        7,
        &batch(vec![
            event(1, EventStatus::Verified(())),
            event(2, EventStatus::Verified(())),
            event(3, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 1);
    assert!(render(&mut audio, 256).iter().all(|sample| *sample == 0.0));
    audio.update(7, &batch(vec![event(4, EventStatus::Verified(()))]), |_| {
        Some(sound.clone())
    });
    assert_eq!(audio.stats().started, 2);
    assert!(render(&mut audio, 256).iter().any(|sample| *sample != 0.0));

    // A real source/session replacement still owns a fresh event lifetime.
    audio.update(8, &batch(vec![event(1, EventStatus::Verified(()))]), |_| {
        Some(sound.clone())
    });
    assert_eq!(audio.stats().started, 3);
    audio.update(
        8,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::SessionStarted {
                tick_rate: 60,
                local_slot: PlayerSlot(0),
                player_count: 1,
            }),
            event(1, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert_eq!(audio.stats().started, 4);
}

#[test]
fn stopping_transients_preserves_lifecycle_retirement_and_voice_bounds() {
    let sound = clip();
    let mut audio = mixer(AudioConfig {
        max_voices: 1,
        history_capacity: 1,
        ..AudioConfig::default()
    });
    audio.update(
        7,
        &batch(vec![event(1, EventStatus::Predicted(()))]),
        |_| Some(sound.clone()),
    );
    render(&mut audio, 256);
    for tick in 2..=20 {
        audio.stop_transients();
        audio.update(
            7,
            &batch(vec![event(tick, EventStatus::Predicted(()))]),
            |_| Some(sound.clone()),
        );
        assert_eq!(audio.voice_count(), 1);
        assert_eq!(audio.history_len(), 1);
    }
    assert_eq!(
        audio.stats().started,
        1,
        "fades keep using the voice budget"
    );
    assert_eq!(audio.retired_through, Some(19));
    audio.stop_transients();
    assert_eq!(audio.retired_through, Some(19));
    render(&mut audio, 2048);
    audio.update(
        7,
        &batch(vec![
            event(1, EventStatus::Verified(())),
            event(20, EventStatus::Verified(())),
        ]),
        |_| Some(sound.clone()),
    );
    assert!(render(&mut audio, 256).iter().all(|sample| *sample == 0.0));
    assert_eq!(audio.stats().started, 1);

    audio.update(
        7,
        &batch(vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 20 })]),
        |_| Some(sound.clone()),
    );
    audio.stop_transients();
    assert!(audio.paused);
    assert_eq!(audio.source, Some(7));
    assert_eq!(audio.history_len(), 1);
    audio.update(
        7,
        &batch(vec![BridgeEvent::Lifecycle(Lifecycle::Disconnected)]),
        |_| Some(sound.clone()),
    );
    audio.stop_transients();
    assert!(audio.disconnected);
    assert!(audio.paused);
    assert_eq!(audio.source, Some(7));
    assert_eq!(audio.retired_through, Some(u64::MAX));
}
