//! Bounded view-event delivery recovers from slow consumers without stopping
//! simulation or reviving speculative effects that were lost at a reset.
#![allow(clippy::disallowed_types)]

use orr_bridge::{
    view_event_channel, Bridge, BridgeConfig, BridgeEvent, ControlOp, EventStatus, InProc,
    Lifecycle, LoopbackPair, Pacing, PlayConfig, PlayHost, PlaySession, PlayerSlot, SimControl,
    Snapshot, Threaded, ThreadedConfig,
};
use orr_session::SessionConfig;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, Hit, Score, SpawnBulletCmd};

fn loopback() -> LoopbackPair<Arena> {
    LoopbackPair::new(
        || ArenaConfig { player_count: 2 },
        SessionConfig::new(2, PlayerSlot(0), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        4,
        1,
        777,
        |_tick| (ArenaInput::default(), Vec::new()),
    )
}

fn firing_bridge_config(capacity: usize) -> BridgeConfig<Arena> {
    BridgeConfig::default()
        .with_view_event_capacity(capacity)
        .with_commands_from_input(|_| vec![SpawnBulletCmd { owner: 0 }])
}

fn paused_play_bridge(capacity: usize) -> InProc<Arena, PlayHost<Arena>> {
    let mut play = PlayConfig::new(2, 11, 50);
    play.keyframe_interval = 16;
    play.ring_capacity = 256;
    play.start_paused = true;
    let host = PlayHost::new(
        PlaySession::<Arena>::new(play, ArenaConfig { player_count: 2 }),
        PlayerSlot(0),
    );
    InProc::new(
        host,
        BridgeConfig::default().with_view_event_capacity(capacity),
    )
}

fn timeline_snapshots_across_seek() -> (Snapshot, Snapshot, Snapshot) {
    let mut bridge = paused_play_bridge(8);
    bridge.control(ControlOp::Step(120)).unwrap();
    bridge.control(ControlOp::Seek(100)).unwrap();
    let epoch_one_head = bridge.snapshot().unwrap();
    assert_eq!(
        (
            epoch_one_head.tick(),
            epoch_one_head.timeline().unwrap().epoch
        ),
        (100, 1)
    );

    bridge.control(ControlOp::Seek(50)).unwrap();
    let epoch_two_seek = bridge.snapshot().unwrap();
    assert_eq!(
        (
            epoch_two_seek.tick(),
            epoch_two_seek.timeline().unwrap().epoch
        ),
        (50, 2)
    );

    bridge.control(ControlOp::Step(50)).unwrap();
    let epoch_two_head = bridge.snapshot().unwrap();
    assert_eq!(
        (
            epoch_two_head.tick(),
            epoch_two_head.timeline().unwrap().epoch
        ),
        (100, 2)
    );
    (epoch_one_head, epoch_two_seek, epoch_two_head)
}

#[test]
fn slow_consumer_gets_bounded_resync_and_snapshot_contains_lost_hit_effects() {
    let mut bridge = InProc::new(loopback(), firing_bridge_config(2));
    let initial = bridge.poll_view();
    assert!(initial.resync.is_none());
    assert!(initial.snapshot.is_some());

    // Repeatedly let the producer get well ahead of the view. The simulation
    // and score continue, while each recovery returns one pinned state and a
    // fixed-size set of lifecycle summaries rather than a replay of old VFX.
    let mut generations = Vec::new();
    for _ in 0..5 {
        bridge.step(100);
        let update = bridge.poll_view();
        let snapshot = update
            .snapshot
            .expect("a view update always pins the latest snapshot");
        assert_eq!(snapshot.tick(), bridge.host().peer_a().head_tick());
        assert_eq!(snapshot.predicted().tick(), snapshot.tick());
        assert!(bridge.is_alive());
        if let Some(reset) = update.resync {
            assert_eq!(reset.head_tick, snapshot.tick());
            assert!(reset.discarded_events > 0);
            assert!(reset.lifecycle.len() <= 12);
            generations.push(reset.generation);
        }
    }

    assert!(
        generations.len() >= 3,
        "slow polling should trigger repeated recovery: {generations:?}"
    );
    assert!(generations.windows(2).all(|pair| pair[0] < pair[1]));
    let snapshot = bridge.snapshot().unwrap();
    let score = snapshot.predicted().singleton::<Score>();
    assert!(
        score.kills[0] > 0,
        "lost Hit notifications are represented in the recovered simulation state"
    );
}

#[test]
fn poll_view_pins_the_snapshot_that_covers_its_event_batch() {
    let (snapshot, _, _) = timeline_snapshots_across_seek();
    let event_tick = snapshot.tick();
    let event = BridgeEvent::Sim {
        key: orr_bridge::EventKey::new(event_tick, 2, 0),
        status: EventStatus::Predicted(Hit {
            victim_slot: 1,
            shooter_slot: 0,
        }),
    };
    let (mut sender, mut receiver) = view_event_channel::<Hit>(2);
    sender.publish(Some(snapshot), vec![event.clone()]);

    let update = receiver.poll();
    let pinned = update
        .snapshot
        .expect("the event batch and snapshot are returned together");
    assert!(update.resync.is_none());
    assert_eq!(update.events, vec![event]);
    assert!(pinned.tick() >= event_tick);
    assert_eq!(pinned.predicted().tick(), pinned.tick());
}

#[test]
fn paused_lifecycle_only_overflow_preserves_refused_control_diagnostics() {
    let mut bridge = paused_play_bridge(2);
    let start = bridge.poll_view();
    assert_eq!(start.snapshot.unwrap().tick(), 0);
    assert!(start.resync.is_none());

    const REFUSED: u64 = 20;
    for _ in 0..REFUSED {
        bridge.control(ControlOp::Seek(999)).unwrap();
    }
    let update = bridge.poll_view();
    let snapshot = update
        .snapshot
        .expect("paused updates still publish a baseline");
    assert_eq!(
        snapshot.tick(),
        0,
        "overflow recovery must not advance the paused simulation"
    );
    assert!(!snapshot.timeline().unwrap().playing);
    let reset = update
        .resync
        .expect("lifecycle-only overflow must also recover");
    assert_eq!(reset.head_tick, 0);
    let refused = reset
        .lifecycle
        .iter()
        .find(|summary| matches!(summary.last, Lifecycle::SeekRejected { target: 999 }));
    assert_eq!(refused.map(|summary| summary.count), Some(REFUSED));
}

#[test]
fn legacy_split_read_api_emits_an_explicit_resync_marker() {
    let mut bridge = InProc::new(loopback(), firing_bridge_config(1));
    assert!(bridge.drain_events().iter().any(|event| matches!(
        event,
        BridgeEvent::Lifecycle(Lifecycle::SessionStarted { .. })
    )));

    bridge.step(100);
    let events = bridge.drain_events();
    let reset = events.first().expect("overflow is never silently consumed");
    let BridgeEvent::ViewResynced(reset) = reset else {
        panic!("legacy drain_events must expose the recovery marker first")
    };
    assert_eq!(reset.head_tick, bridge.snapshot().unwrap().tick());
    assert!(reset.discarded_events > 0);
}

#[test]
fn resync_discards_late_old_epoch_ghosts_but_accepts_same_identity_after_seek() {
    let (head_100_e1, seek_50_e2, head_100_e2) = timeline_snapshots_across_seek();
    let (mut sender, mut receiver) = view_event_channel::<Hit>(3);
    let ghost = orr_bridge::EventKey::new(99, 2, 4);
    let payload = Hit {
        victim_slot: 1,
        shooter_slot: 0,
    };

    // The predicted occurrence is lost with the complete oversized batch.
    sender.publish(
        Some(head_100_e1.clone()),
        vec![
            BridgeEvent::Sim {
                key: ghost,
                status: EventStatus::Predicted(payload),
            },
            BridgeEvent::Sim {
                key: orr_bridge::EventKey::new(98, 2, 4),
                status: EventStatus::Predicted(payload),
            },
            BridgeEvent::Sim {
                key: orr_bridge::EventKey::new(97, 2, 4),
                status: EventStatus::Predicted(payload),
            },
            BridgeEvent::Sim {
                key: orr_bridge::EventKey::new(96, 2, 4),
                status: EventStatus::Predicted(payload),
            },
        ],
    );
    let recovered = receiver.poll();
    assert_eq!(recovered.snapshot.as_ref().unwrap().tick(), 100);
    assert_eq!(recovered.resync.as_ref().unwrap().head_tick, 100);
    assert!(
        recovered.events.is_empty(),
        "a reset communicates lost effects through its snapshot"
    );

    // An old status batch and the seek may both be queued before the view
    // polls. The seek's newer epoch must not erase the old batch's recovery
    // boundary before those statuses are filtered.
    sender.publish(
        Some(head_100_e1),
        vec![
            BridgeEvent::Sim {
                key: ghost,
                status: EventStatus::Verified(payload),
            },
            BridgeEvent::Sim {
                key: ghost,
                status: EventStatus::Canceled,
            },
        ],
    );
    sender.publish(
        Some(seek_50_e2),
        vec![BridgeEvent::Lifecycle(Lifecycle::Seeked {
            from: 100,
            to: 50,
        })],
    );
    let after_seek = receiver.poll();
    assert_eq!(after_seek.snapshot.as_ref().unwrap().tick(), 50);
    assert_eq!(
        after_seek
            .snapshot
            .as_ref()
            .unwrap()
            .timeline()
            .unwrap()
            .epoch,
        2
    );
    assert!(after_seek.events.iter().any(|event| matches!(
        event,
        BridgeEvent::Lifecycle(Lifecycle::Seeked { from: 100, to: 50 })
    )));
    assert!(
        after_seek.events.iter().all(|event| !matches!(event, BridgeEvent::Sim { key, .. } if *key == ghost)),
        "a late Verified/Canceled status must not resurrect a ghost whose Predicted status was lost"
    );

    // EventKey is deterministic across replay, but the new timeline epoch is a
    // fresh visual history: the same occurrence can legitimately be predicted
    // again after the seek.
    sender.publish(
        Some(head_100_e2),
        vec![BridgeEvent::Sim {
            key: ghost,
            status: EventStatus::Predicted(payload),
        }],
    );
    let replayed = receiver.poll();
    assert_eq!(
        replayed
            .snapshot
            .as_ref()
            .unwrap()
            .timeline()
            .unwrap()
            .epoch,
        2
    );
    assert!(replayed.events.contains(&BridgeEvent::Sim {
        key: ghost,
        status: EventStatus::Predicted(payload)
    }));
}

#[test]
fn large_manual_threaded_step_and_drop_complete_with_a_full_view_mailbox() {
    let config = firing_bridge_config(1);
    let mut bridge = Threaded::spawn(
        loopback,
        config,
        ThreadedConfig {
            pacing: Pacing::Manual,
            max_catchup: 8,
        },
    )
    .unwrap();
    let initial = bridge.poll_view();
    assert!(initial.snapshot.is_some());

    // No consumer reads while this batch runs. A full presentation mailbox is
    // lossy and nonblocking, so the manual producer still completes all ticks.
    bridge.step(2_000);
    let update = bridge.poll_view();
    let snapshot = update
        .snapshot
        .expect("thread publishes after the large step");
    assert_eq!(snapshot.tick(), 2_000);
    assert_eq!(snapshot.stats().ticks, 2_000);
    assert!(update.resync.is_some());
    assert!(bridge.is_alive());

    // Explicitly exercise shutdown/join while the consumer is behind.
    drop(bridge);
}
