//! Consumer regressions: the real host and transport drive the capped view
//! mailbox; synthetic updates exercise the editor's pinned-baseline boundary.

use super::*;
use orr_bridge::{Bridge, DebugError, LifecycleRecovery};
use orr_remote::{RemoteBridge, Auth, Caps, LocalHost, RemoteConfig, ServerConfig, ViewDeliveryMode, USER_CLIENT};

fn reset(snapshot: Option<&Snapshot>) -> ViewResync {
    ViewResync {
        generation: 7,
        discarded_events: 19,
        head_tick: snapshot.map_or(0, Snapshot::tick),
        verified_tick: snapshot.map_or(0, Snapshot::verified_tick),
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    }
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn capped_editor(capacity: usize, websocket: bool) -> (Editor, Option<LocalHost>) {
    let mut cfg = RemoteConfig::new("");
    cfg.source = "view".to_string();
    cfg.max_fps = 240;
    cfg.view_event_capacity = capacity;
    cfg.view_delivery = ViewDeliveryMode::RequireFenced;
    let (mut backend, host) = if websocket {
        let host = orr_remote::sample::spawn_phys_host(
            std::fs::read_to_string(default_scene_path()).unwrap(),
            Some(default_scene_path()),
            ServerConfig::new(Auth::DevNoAuth),
        )
        .unwrap();
        let url = host.url().unwrap().to_string();
        cfg.url = url.clone();
        (Backend::connect(&HostSpec::remote(&url, None)).unwrap(), Some(host))
    } else {
        (Backend::connect(&HostSpec::local(default_scene_path())).unwrap(), None)
    };
    backend.bridge = EditorStream::Phys(if let Some(host) = &backend.host {
        let transport = host.connector().connect(USER_CLIENT, Caps::ALL).unwrap();
        RemoteBridge::connect_transport(Box::new(transport), cfg).unwrap()
    } else {
        RemoteBridge::connect(cfg).unwrap()
    });
    assert_eq!(backend.bridge.view_delivery(), RemoteViewDelivery::Fenced);
    let mut ed = Editor::on_backend(backend).unwrap();
    ed.sync();
    (ed, host)
}

fn assert_current_pose(ed: &Editor) {
    let snapshot = ed.snapshot().expect("current baseline");
    let expected = ed.game().drawables(snapshot.predicted());
    assert_eq!(ed.checksum(), snapshot.predicted().checksum());
    assert_eq!(ed.bodies().len(), expected.len());
    for (shown, current) in ed.bodies().iter().zip(expected) {
        assert_eq!(shown.entity, current.entity);
        assert_eq!(shown.pos, current.pos, "recovery displays the exact head pose");
        assert_eq!(shown.angle, current.angle);
    }
}

#[test]
fn a_pinned_reset_rebuilds_the_current_pose_even_at_the_same_sequence() {
    let (mut ed, _host) = capped_editor(64, false);
    ed.step(9);
    ed.sync();
    let snapshot = ed.snapshot().unwrap().clone();
    let seq = snapshot.seq();
    // A stale persistent visual must be rebuilt, even if this paused frame
    // was already seen. The consumer must not take a second bridge snapshot.
    ed.bodies[0].pos = [999.0, -999.0];
    ed.checksum = 0;
    ed.apply_view_update(ViewUpdate::<()> { resync: Some(reset(Some(&snapshot))), snapshot: Some(snapshot), events: Vec::new() });
    assert_eq!(ed.snapshot().unwrap().seq(), seq);
    assert_current_pose(&ed);
    assert!(ed.dirty.state && ed.dirty.rows && ed.dirty.inspect);
}

#[test]
fn an_inactive_reset_clears_the_old_scene_but_an_empty_normal_poll_does_not() {
    let (mut ed, _host) = capped_editor(64, false);
    let before = ed.checksum();
    ed.apply_view_update(ViewUpdate::<()> { snapshot: None, events: Vec::new(), resync: None });
    assert_eq!(ed.checksum(), before);
    assert!(!ed.bodies().is_empty());

    ed.apply_view_update(ViewUpdate::<()> { snapshot: None, events: Vec::new(), resync: Some(reset(None)) });
    assert!(ed.snapshot().is_none());
    assert!(ed.bodies().is_empty());
    assert_eq!(ed.checksum(), 0);
}

#[test]
fn coalesced_diagnostics_do_not_acknowledge_requests_or_replay_old_transitions() {
    let (mut ed, _host) = capped_editor(64, false);
    ed.step(3);
    ed.sync();
    let snapshot = ed.snapshot().unwrap().clone();
    let mut resync = reset(Some(&snapshot));
    resync.last_desync = Some(2);
    resync.lifecycle = vec![
        LifecycleRecovery { count: 5, last: Lifecycle::Resumed { tick: 1 } },
        LifecycleRecovery { count: 8, last: Lifecycle::DebugRejected(DebugError::EntityNotAlive) },
        LifecycleRecovery { count: 13, last: Lifecycle::SeekRejected { target: 999 } },
    ];
    ed.pending.insert(u64::MAX, Pending { kind: Pend::State, posted_at: Instant::now() });
    ed.log.clear();
    ed.apply_view_update(ViewUpdate::<()> { snapshot: Some(snapshot), events: Vec::new(), resync: Some(resync) });

    assert!(ed.pending.contains_key(&u64::MAX), "recovery cannot complete an ERP request");
    assert!(!ed.sim.playing, "old Resumed diagnostics cannot replay a state transition");
    assert!(!ed.timeline().unwrap().playing);
    assert_eq!(ed.log.len(), 5, "one reset, three category summaries, one sticky desync");
    assert!(ed.log.iter().any(|m| m.error && m.text.contains("8 coalesced") && m.text.contains("DebugRejected")));
    assert!(ed.log.iter().any(|m| m.error && m.text.contains("13 coalesced") && m.text.contains("SeekRejected")));
    assert!(ed.log.iter().any(|m| m.error && m.text.contains("last desync at tick 2")));
    assert!(ed.log.iter().all(|m| !m.text.starts_with("debug edit refused:")));
    assert_current_pose(&ed);
}

fn slow_editor_recovers_and_crosses_timeline_boundaries(websocket: bool) {
    let (mut ed, _host) = capped_editor(2, websocket);
    let scene_checksum = ed.checksum();
    ed.step(8);
    ed.sync();
    ed.log.clear();
    let started_at = ed.sim.head_tick;

    // Keep the background receiver current while deliberately never polling
    // the presentation mailbox. The real host continues accepting controls,
    // producing lifecycle records, and advancing its simulation.
    for _ in 0..8 {
        ed.play();
        ed.pause();
        ed.step(2);
        let want = ed.sim.head_tick;
        wait_until("the unpolled bridge to receive the advanced frame", || {
            ed.backend.bridge.snapshot().is_some_and(|s| s.tick() == want && s.timeline().is_some_and(|t| !t.playing))
        });
    }
    assert!(ed.sim.head_tick >= started_at + 16);
    assert!(ed.backend.bridge.is_alive());
    ed.refresh_view();
    assert!(ed.log.iter().any(|m| m.text.starts_with("view recovered at tick ")), "actual tiny-capacity overflow must recover");
    assert_eq!(ed.snapshot().unwrap().tick(), ed.sim.head_tick);
    assert_current_pose(&ed);

    ed.seek(1);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 1);
    assert_current_pose(&ed);
    ed.stop();
    ed.sync();
    assert_eq!(ed.mode(), Mode::Edit);
    assert!(ed.timeline().is_none());
    assert_eq!(ed.checksum(), scene_checksum);
    assert_current_pose(&ed);
    ed.step(2);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 2);
    assert_current_pose(&ed);

    if !websocket {
        assert!(ed.restart());
        ed.sync();
        assert_eq!(ed.backend.bridge.view_delivery(), RemoteViewDelivery::Fenced);
        assert_eq!(ed.mode(), Mode::Edit);
        assert!(ed.timeline().is_none());
        assert_eq!(ed.checksum(), scene_checksum);
        assert_current_pose(&ed);
    }
}

#[test]
fn a_slow_local_editor_recovers_then_seeks_stops_restarts_play_and_restarts_the_host() {
    slow_editor_recovers_and_crosses_timeline_boundaries(false);
}

#[test]
fn a_slow_websocket_editor_recovers_then_seeks_stops_and_restarts_play() {
    slow_editor_recovers_and_crosses_timeline_boundaries(true);
}

#[test]
fn proposal_previews_drain_events_even_when_the_snapshot_sequence_is_unchanged() {
    let (mut ed, _host) = capped_editor(64, false);
    let id = ed.call("proposal.begin", json!({"label": "preview drain regression"})).unwrap()["id"].as_str().unwrap().to_string();
    ed.read_proposals().unwrap();
    assert!(ed.set_preview(Some(id.clone())));

    for mode in [ViewDeliveryMode::RequireFenced, ViewDeliveryMode::Legacy] {
        // A fresh connection always has a SessionStarted diagnostic, even for
        // an unchanged paused proposal. Pretend its snapshot was already seen
        // to exercise the early-return path that used to leave events unread.
        let mut cfg = RemoteConfig::new("");
        cfg.source = format!("proposal:{id}");
        cfg.view_delivery = mode;
        cfg.view_event_capacity = 2;
        let transport = ed.backend.host.as_ref().unwrap().connector().connect(USER_CLIENT, Caps::ALL).unwrap();
        let stream = RemoteBridge::connect_transport(Box::new(transport), cfg).unwrap();
        wait_until("the proposal snapshot", || stream.snapshot().is_some());
        let seq = stream.snapshot().unwrap().seq();
        {
            let p = ed.preview.as_mut().unwrap();
            p.stream = EditorStream::Phys(stream);
            p.seq = seq;
        }
        ed.refresh_preview();
        let p = ed.preview.as_mut().unwrap();
        let remaining = p.stream.poll_view();
        assert!(remaining.events.is_empty(), "preview must drain lifecycle events even for an unchanged frame");
        assert!(remaining.resync.is_none(), "preview must consume its negotiated recovery baseline");
    }
}

#[test]
fn main_and_preview_stream_errors_are_visible_without_losing_the_healthy_connection() {
    let (mut ed, _host) = capped_editor(64, false);
    ed.backend.bridge.request("test.unknown_main_method", J::Null).unwrap();
    wait_until("the main stream refusal", || {
        ed.refresh_view();
        ed.log.iter().any(|m| m.error && m.text.starts_with("frame stream:"))
    });
    assert!(ed.down().is_none());

    let id = ed.call("proposal.begin", json!({"label": "preview error regression"})).unwrap()["id"].as_str().unwrap().to_string();
    ed.read_proposals().unwrap();
    assert!(ed.set_preview(Some(id)));
    ed.preview.as_ref().unwrap().stream.request("test.unknown_preview_method", J::Null).unwrap();
    wait_until("the preview stream refusal", || {
        ed.refresh_preview();
        ed.log.iter().any(|m| m.error && m.text.starts_with("preview frame stream:"))
    });
    assert!(ed.preview.is_some());
    assert!(ed.down().is_none());
}

#[test]
fn preview_tombstones_clear_rows_and_reject_responses_from_the_previous_baseline() {
    let (mut ed, _host) = capped_editor(64, false);
    let id = ed.call("proposal.begin", json!({"label": "preview stale response regression"})).unwrap()["id"].as_str().unwrap().to_string();
    ed.read_proposals().unwrap();
    assert!(ed.set_preview(Some(id.clone())));
    ed.sync();
    let old_generation = ed.preview.as_ref().unwrap().generation;
    assert!(!ed.preview.as_ref().unwrap().rows.is_empty());
    let old_response = ed.call("proposal.preview", json!({"id": id, "values": false, "limit": ROWS_LIMIT})).unwrap();
    ed.inflight.preview_rows = Some(old_generation);

    assert!(ed.start_play());
    wait_until("the inactive proposal tombstone", || ed.preview.as_ref().unwrap().stream.snapshot().is_none());
    ed.refresh_preview();
    let p = ed.preview.as_ref().unwrap();
    assert_eq!(p.seq, 0);
    assert!(p.bodies.is_empty() && p.rows.is_empty());
    assert!(p.generation > old_generation);
    ed.on_answer(Pend::PreviewRows(id, old_generation), Ok(old_response));
    assert!(ed.preview.as_ref().unwrap().rows.is_empty(), "an old ERP read cannot repopulate the discarded presentation baseline");

    ed.stop();
    ed.sync();
    let p = ed.preview.as_ref().unwrap();
    assert!(p.seq > 0 && !p.bodies.is_empty() && !p.rows.is_empty(), "the new baseline schedules a fresh row query");
}
