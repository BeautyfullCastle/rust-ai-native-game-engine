#![allow(clippy::disallowed_types)] // a tool crate: the remote integration test uses a wall-clock timeout

mod common;

use std::time::{Duration, Instant};

use common::TestHost;
use orr_bridge::{Bridge, BridgeEvent, ControlOp, Lifecycle, SimControl};
use orr_remote::{RemoteBridge, RemoteConfig};
use orr_sample::physics_game::PhysGame;
use serde_json::Value as J;

#[test]
fn remote_poll_view_is_best_effort_and_does_not_claim_resync() {
    let host = TestHost::standard();
    let mut control = host.client("tok-all");
    control.call("sim.start", J::Null).unwrap();

    let mut cfg = RemoteConfig::new(&host.url);
    cfg.token = Some("tok-all".to_string());
    let mut bridge = RemoteBridge::<PhysGame>::connect(cfg).expect("connect remote bridge");

    // Keep draining while the first frame arrives. SessionStarted is emitted
    // during the handshake, independently of the remote frame stream.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    let first_snapshot = loop {
        let update = bridge.poll_view();
        assert!(
            update.resync.is_none(),
            "remote ERP has no cursor to report a resync"
        );
        seen.extend(update.events);
        if let Some(snapshot) = update.snapshot {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the first remote frame"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(first_snapshot.tick(), 0);
    assert!(seen.iter().any(|event| matches!(
        event,
        BridgeEvent::Lifecycle(Lifecycle::SessionStarted { .. })
    )));

    bridge.control(ControlOp::Step(2)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let update = bridge.poll_view();
        assert!(update.resync.is_none());
        if update.snapshot.is_some_and(|snapshot| snapshot.tick() == 2) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the stepped remote frame"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
