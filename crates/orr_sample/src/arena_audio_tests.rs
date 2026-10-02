#![allow(clippy::float_arithmetic)]

use crate::arena_audio::{ArenaAudio, AudioMode};

#[test]
fn audio_modes_are_explicit_and_off_never_needs_a_device() {
    assert_eq!("off".parse::<AudioMode>().unwrap(), AudioMode::Off);
    assert_eq!("auto".parse::<AudioMode>().unwrap(), AudioMode::Auto);
    assert_eq!(
        "required".parse::<AudioMode>().unwrap(),
        AudioMode::Required
    );
    assert!("silent-success".parse::<AudioMode>().is_err());
    assert_eq!(ArenaAudio::open(AudioMode::Off).unwrap().status(), "off");
}

#[cfg(not(feature = "audio-native"))]
#[test]
fn missing_native_feature_is_reported_and_required_fails() {
    assert!(ArenaAudio::open(AudioMode::Auto)
        .unwrap()
        .status()
        .contains("unavailable"));
    assert!(ArenaAudio::open(AudioMode::Required).is_err());
}

#[cfg(feature = "audio")]
#[test]
fn actual_arena_hits_feed_real_pcm_without_changing_the_simulation() {
    use crate::arena_view::{arena_bridge_config, loopback_pair, Loopback};
    use orr_audio::{AudioConfig, OfflineAudio};
    use orr_bridge::{Bridge, InProc};
    use std::time::Duration;
    let mut with_audio = InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
    let mut without_audio = InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
    let mut audio = OfflineAudio::new_offline(AudioConfig::default(), 48_000).unwrap();
    let clip = crate::arena_audio::hit_clip();
    let mut audible = false;
    for _ in 0..600 {
        with_audio.update(Duration::from_nanos(16_666_667));
        without_audio.update(Duration::from_nanos(16_666_667));
        let update = with_audio.poll_view();
        let baseline = without_audio.poll_view();
        assert_eq!(
            update.snapshot.as_ref().map(|s| s.tick()),
            baseline.snapshot.as_ref().map(|s| s.tick())
        );
        assert_eq!(update.events, baseline.events);
        assert_eq!(
            update.snapshot.as_ref().map(|s| s.predicted().checksum()),
            baseline.snapshot.as_ref().map(|s| s.predicted().checksum())
        );
        audio.update(1, &update, |_| Some(clip.clone()));
        let mut pcm = [0.0; 1600];
        audio.render(&mut pcm).unwrap();
        assert!(pcm.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        audible |= pcm.iter().any(|v| v.abs() > 0.001);
    }
    assert!(
        audio.stats().started > 0,
        "scripted arena must really emit Hit events"
    );
    assert!(audible, "actual arena notifications must produce mixed PCM");
}

#[cfg(feature = "audio")]
#[test]
fn newer_snapshot_epoch_cannot_rebind_an_older_event_tail() {
    use orr_audio::{AudioConfig, OfflineAudio};
    use orr_bridge::{
        BridgeEvent, BridgeStats, EventKey, EventStatus, PlayConfig, PlaySession, Snapshot,
        SnapshotParts, ViewUpdate,
    };
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_testgame::{Arena, ArenaConfig};
    use std::sync::Arc;
    let session =
        PlaySession::<Arena>::new(PlayConfig::new(2, 42, 60), ArenaConfig { player_count: 2 });
    let mut timeline = session.timeline();
    let snapshot = |timeline| {
        Snapshot::from_parts(SnapshotParts {
            seq: 10,
            tick: 10,
            verified_tick: 10,
            tick_rate: 60,
            predicted: Arc::new(Frame::new(ComponentRegistryBuilder::new().build())),
            predicted_prev: None,
            verified: None,
            stats: BridgeStats::default(),
            last_rollback: None,
            timeline: Some(timeline),
        })
    };
    let key = EventKey::new(1, 0, 0);
    let clip = crate::arena_audio::hit_clip();
    let mut audio = OfflineAudio::new_offline(AudioConfig::default(), 48_000).unwrap();
    let first = ViewUpdate {
        snapshot: Some(snapshot(timeline.clone())),
        resync: None,
        events: vec![BridgeEvent::Sim {
            key,
            status: EventStatus::Predicted(()),
        }],
    };
    audio.update(1, &first, |_| Some(clip.clone()));
    let mut pcm = [0.0; 20_000];
    audio.render(&mut pcm).unwrap();
    timeline.epoch += 1;
    let later = ViewUpdate {
        snapshot: Some(snapshot(timeline)),
        resync: None,
        events: vec![BridgeEvent::Sim {
            key,
            status: EventStatus::Verified(()),
        }],
    };
    audio.update(1, &later, |_| Some(clip.clone()));
    audio.render(&mut pcm).unwrap();
    assert_eq!(audio.stats().started, 1);
    assert!(pcm.iter().all(|v| v.abs() < 0.000001));
}
