//! Real LocalHost and WebSocket coverage for the negotiated RemoteBridge path.
#![allow(clippy::disallowed_types)]
mod common;

use common::{demo_doc, phys_hooks};
use orr_bridge::{Bridge, BridgeEvent};
use orr_remote::{
    Auth, Caps, ErpClient, LocalHost, RemoteBridge, RemoteConfig, RemoteViewDelivery, ServerConfig,
    ViewDeliveryMode,
};
use orr_sample::physics_game::{PhysGame, PhysInput};
use serde_json::{json, Value as J};
use std::time::{Duration, Instant};

fn wait<T>(mut read: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "remote view did not reach expected publication"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn exercise(websocket: bool) {
    let host = LocalHost::spawn::<PhysGame>(move || {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = websocket;
        cfg.limits.game = phys_hooks();
        Ok((demo_doc(), cfg))
    })
    .unwrap();
    let mut control = ErpClient::with_transport(Box::new(
        host.connector().connect("driver", Caps::ALL).unwrap(),
    ));
    control.call("sim.start", J::Null).unwrap();
    let input = PhysInput::new(0, 0, 0, true);
    control
        .call(
            "sim.input",
            json!({"player":0,"input":orr_remote::codec::hex_encode(bytemuck::bytes_of(&input))}),
        )
        .unwrap();
    let mut cfg = RemoteConfig::new(host.url().unwrap_or(""));
    cfg.max_fps = 1;
    cfg.view_event_capacity = 1;
    cfg.view_delivery = ViewDeliveryMode::RequireFenced;
    let mut bridge = if websocket {
        RemoteBridge::<PhysGame>::connect(cfg).unwrap()
    } else {
        RemoteBridge::<PhysGame>::connect_transport(
            Box::new(host.connector().connect("view", Caps::parse("read").unwrap()).unwrap()),
            cfg,
        )
        .unwrap()
    };
    assert_eq!(bridge.view_delivery(), RemoteViewDelivery::Fenced);
    wait(|| bridge.snapshot().filter(|s| s.tick() == 0));
    assert!(bridge.poll_view().resync.is_some());

    // Shooting produces actual simulation events. One request alone exceeds
    // the capacity-one staging budget, while the initial frame is FPS-limited.
    control.call("sim.step", json!({"n":100})).unwrap();
    let early = bridge.poll_view();
    let early_tick = early.snapshot.as_ref().map_or(0, |s| s.tick());
    for event in &early.events {
        if let BridgeEvent::Sim { key, .. } = event {
            assert!(key.tick <= early_tick);
        }
    }
    control.call("sim.step", json!({"n":4})).unwrap();
    wait(|| bridge.snapshot().filter(|s| s.tick() == 104));
    let baseline = bridge.poll_view();
    assert_eq!(baseline.snapshot.as_ref().unwrap().tick(), 104);
    assert!(baseline
        .resync
        .as_ref()
        .is_some_and(|r| r.head_tick == 104 && r.discarded_events > 0));
    assert!(!baseline
        .events
        .iter()
        .any(|event| matches!(event, BridgeEvent::Sim { .. })));
    let checksum = control.call("sim.checksum", J::Null).unwrap();
    assert_eq!(
        Some(baseline.snapshot.as_ref().unwrap().predicted().checksum()),
        orr_remote::wire::parse_checksum(&checksum["checksum"])
    );

    // Never consume presentation while real-time simulation runs. Authoritative
    // controls and ticks remain live, independent of a permanently slow view.
    control.call("sim.play", J::Null).unwrap();
    wait(|| {
        let state = control.call("sim.state", J::Null).unwrap();
        (state["head_tick"].as_u64().unwrap_or(0) >= 124).then_some(())
    });
    control.call("sim.pause", J::Null).unwrap();
    let paused = control.call("sim.state", J::Null).unwrap()["head_tick"]
        .as_u64()
        .unwrap();
    assert!(paused >= 124);
    wait(|| {
        bridge
            .snapshot()
            .filter(|s| s.tick() == paused && s.timeline().is_some_and(|t| !t.playing))
    });
    let recovered = bridge.poll_view();
    assert!(recovered.resync.is_some());
    assert_eq!(recovered.snapshot.unwrap().tick(), paused);
    assert!(bridge.is_alive());

    // A new tail is delivered only with a covering snapshot, and the old
    // recovery floor does not suppress genuine later effects.
    control.call("sim.step", json!({"n":3})).unwrap();
    wait(|| bridge.snapshot().filter(|s| s.tick() == paused + 3));
    let tail = bridge.poll_view();
    assert!(tail.resync.is_none());
    let sim: Vec<_> = tail
        .events
        .iter()
        .filter_map(|event| match event {
            BridgeEvent::Sim { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(sim.len(), 1);
    assert!(sim[0].tick > paused && sim[0].tick <= tail.snapshot.unwrap().tick());
}

#[test]
fn local_host_capacity_one_fps_one_recovers_without_stalling_simulation() {
    exercise(false);
}
#[test]
fn websocket_capacity_one_fps_one_recovers_without_stalling_simulation() {
    exercise(true);
}
