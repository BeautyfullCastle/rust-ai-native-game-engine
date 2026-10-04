#![allow(clippy::disallowed_types)]
//! Retained queue admission, reliable refusal and observable terminal loss.

mod common;

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use orr_bridge::{Bridge, BridgeError};
use orr_remote::link::{Incoming, PumpedWs, QueueLimits, Request, SendError, TransportQueueLimits};
use orr_remote::sample::spawn_phys_host;
use orr_remote::{
    Auth, Caps, ErpServer, Host, RemoteBridge, RemoteConfig, ServerConfig, Transport,
};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Value as J};
use tungstenite::Message;

struct RunningHost {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RunningHost {
    fn start(mut host: Host<PhysGame>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || host.run(&signal, Duration::from_millis(1)));
        Self {
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for RunningHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn wait_until(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn response(t: &mut dyn Transport, id: u64) -> J {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "no response for {id}");
        if let Some(Incoming::Text(text)) = t.recv(Duration::from_millis(10)).unwrap() {
            let answer: J = serde_json::from_str(&text).unwrap();
            if answer["id"] == id {
                assert!(answer.get("error").is_none(), "{answer}");
                return answer["result"].clone();
            }
        }
    }
}

#[test]
fn local_refusal_preserves_accepted_edit_order_and_reclaims_both_quotas() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    cfg.limits.game = phys_hooks();
    let server = ErpServer::start(cfg).unwrap();
    let mut t = server
        .connector()
        .connect_with_queue_limits(
            "user",
            Caps::ALL,
            TransportQueueLimits {
                outgoing: QueueLimits {
                    max_items: 2,
                    max_bytes: 4096,
                },
                incoming: QueueLimits::default(),
            },
        )
        .unwrap();
    let tx = t.sender().unwrap();
    let mut host = Host::<PhysGame>::new(demo_doc(), server);
    t.send(Request {
        id: Some(1),
        method: "world.query".into(),
        params: json!({"name":"body_05"}),
    })
    .unwrap();
    host.frame();
    let guid = response(&mut t, 1)["entities"][0]["guid"]
        .as_str()
        .unwrap()
        .to_owned();
    let patch = |id, x| Request {
        id: Some(id),
        method: "world.patch".into(),
        params: json!({
            "entity":guid,"component":"orr_physics::Body","path":"pos.x","value":x,
        }),
    };
    assert_eq!(tx.try_send(patch(2, "1.5")), Ok(()));
    assert_eq!(tx.try_send(patch(3, "2.5")), Ok(()));
    assert_eq!(tx.try_send(patch(4, "3.5")), Err(SendError::Backpressure));
    assert_eq!(tx.queue_stats().unwrap().outgoing.current_items, 2);
    assert_eq!(host.frame().requests, 2);
    response(&mut t, 2);
    response(&mut t, 3);
    let stats = tx.queue_stats().unwrap().outgoing;
    assert_eq!((stats.current_items, stats.current_bytes), (0, 0));
    assert_eq!((stats.peak_items, stats.saturations), (2, 1));
    // Re-enqueue only the previously refused request, once, after capacity returns.
    assert_eq!(tx.try_send(patch(4, "3.5")), Ok(()));
    host.frame();
    response(&mut t, 4);
    t.send(Request {
        id: Some(5),
        method: "history.list".into(),
        params: J::Null,
    })
    .unwrap();
    host.frame();
    let history = response(&mut t, 5);
    assert_eq!(
        history["entries"].as_array().unwrap().len(),
        3,
        "accepted edits execute once"
    );
    t.send(Request {
        id: Some(6),
        method: "world.get".into(),
        params: json!({
            "entity":guid,"component":"orr_physics::Body","path":"pos.x",
        }),
    })
    .unwrap();
    host.frame();
    assert_eq!(response(&mut t, 6)["value"].to_string(), "3.5");
    drop(t);
    assert_eq!(tx.try_send(patch(7, "4.5")), Err(SendError::Disconnected));
    host.frame();
    assert_eq!(host.server.connection_count(), 0);
}

#[test]
fn local_slow_receiver_is_terminal_and_reconnect_reads_latest_state() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    cfg.limits.game = phys_hooks();
    let server = ErpServer::start(cfg).unwrap();
    let connector = server.connector();
    let mut t = connector
        .connect_with_queue_limits(
            "user",
            Caps::ALL,
            TransportQueueLimits {
                outgoing: QueueLimits::default(),
                incoming: QueueLimits {
                    max_items: 1,
                    max_bytes: 4096,
                },
            },
        )
        .unwrap();
    let tx = t.sender().unwrap();
    let mut host = Host::<PhysGame>::new(demo_doc(), server);
    t.send(Request {
        id: Some(1),
        method: "world.query".into(),
        params: json!({"name":"body_05"}),
    })
    .unwrap();
    host.frame();
    let guid = response(&mut t, 1)["entities"][0]["guid"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = host.doc.checksum();
    t.send(Request {
        id: Some(2),
        method: "world.patch".into(),
        params: json!({
            "entity":guid,"component":"orr_physics::Body","path":"pos.x","value":"1.25",
        }),
    })
    .unwrap();
    t.send(Request {
        id: Some(3),
        method: "sim.state".into(),
        params: J::Null,
    })
    .unwrap();
    assert_eq!(host.frame().requests, 2);
    let latest = host.doc.checksum();
    assert_ne!(
        before, latest,
        "an accepted edit may execute before terminal RX loss"
    );
    let stats = tx.queue_stats().unwrap();
    assert!(stats.incoming.closed);
    assert_eq!(stats.incoming.peak_items, 1);
    assert_eq!(stats.incoming.saturations, 1);
    assert!(
        t.recv(Duration::ZERO).is_err(),
        "no incomplete healthy reply stream"
    );
    assert_eq!(tx.queue_stats().unwrap().incoming.current_items, 0);
    assert_eq!(
        tx.try_send(Request {
            id: Some(3),
            method: "sim.state".into(),
            params: J::Null
        }),
        Err(SendError::Disconnected)
    );
    drop(t);
    host.frame();
    let _running = RunningHost::start(host);
    let fresh = connector.connect("user", Caps::ALL).unwrap();
    let mut cfg = RemoteConfig::new("");
    cfg.source = "view".into();
    let bridge = RemoteBridge::<PhysGame>::connect_transport(Box::new(fresh), cfg).unwrap();
    assert_eq!(
        bridge.view_delivery(),
        orr_remote::RemoteViewDelivery::Fenced
    );
    wait_until("fresh fenced snapshot", || {
        bridge
            .snapshot()
            .is_some_and(|s| s.predicted().checksum() == latest)
    });
    assert!(bridge.is_alive());
    assert!(bridge.take_errors().is_empty());
}

#[test]
fn pumped_slow_receiver_closes_worker_even_with_a_surviving_sender() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (start, started) = channel();
    let peer = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        ws.send(Message::text("a".repeat(4096))).unwrap();
        ws.send(Message::text("b".repeat(4096))).unwrap();
        !matches!(ws.read(), Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
    });
    let mut t = PumpedWs::connect_with_queue_limits(
        &url,
        Duration::from_secs(5),
        TransportQueueLimits {
            outgoing: QueueLimits::default(),
            incoming: QueueLimits {
                max_items: 1,
                max_bytes: 4096,
            },
        },
    )
    .unwrap();
    let tx = t.sender().unwrap();
    start.send(()).unwrap();
    wait_until("RX overflow", || tx.queue_stats().unwrap().incoming.closed);
    assert!(t.recv(Duration::ZERO).is_err());
    assert_eq!(tx.queue_stats().unwrap().incoming.current_bytes, 0);
    drop(t);
    assert_eq!(
        tx.try_send(Request {
            id: Some(1),
            method: "scene.save".into(),
            params: J::Null
        }),
        Err(SendError::Disconnected)
    );
    wait_until("worker reclaimed TX", || {
        tx.queue_stats().unwrap().outgoing.current_items == 0
    });
    assert!(
        peer.join().unwrap(),
        "worker/socket must terminate before peer timeout"
    );
}

#[test]
fn pumped_large_payload_is_delivered_once_and_reclaimed_on_dequeue() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (release, released) = channel();
    let peer = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        let text = ws.read().unwrap().into_text().unwrap();
        let request: J = serde_json::from_str(&text).unwrap();
        assert_eq!(request["id"], 7);
        assert_eq!(request["params"]["text"].as_str().unwrap().len(), 1 << 20);
        ws.send(Message::text("r".repeat(1 << 20))).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let mut t = PumpedWs::connect_with_queue_limits(
        &url,
        Duration::from_secs(5),
        TransportQueueLimits {
            outgoing: QueueLimits {
                max_items: 2,
                max_bytes: 2 << 20,
            },
            incoming: QueueLimits {
                max_items: 2,
                max_bytes: 2 << 20,
            },
        },
    )
    .unwrap();
    let tx = t.sender().unwrap();
    t.send(Request {
        id: Some(7),
        method: "echo".into(),
        params: json!({"text":"q".repeat(1<<20)}),
    })
    .unwrap();
    wait_until("retained large reply", || {
        tx.queue_stats().unwrap().incoming.current_bytes == 1 << 20
    });
    assert!(
        matches!(t.recv(Duration::ZERO).unwrap(), Some(Incoming::Text(s)) if s.len() == 1 << 20)
    );
    let stats = tx.queue_stats().unwrap();
    assert_eq!(
        (stats.incoming.current_items, stats.incoming.current_bytes),
        (0, 0)
    );
    assert_eq!(stats.incoming.peak_bytes, 1 << 20);
    assert_eq!(stats.outgoing.current_items, 0);
    assert_eq!(stats.outgoing.saturations + stats.incoming.saturations, 0);
    release.send(()).unwrap();
    drop(t);
    peer.join().unwrap();
}

#[test]
fn remote_error_budget_reclaims_and_overflow_terminates_local_and_websocket() {
    for websocket in [false, true] {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = websocket;
        let h = spawn_phys_host(demo_text(), None, cfg).unwrap();
        let error_limits = QueueLimits {
            max_items: 1,
            max_bytes: 4096,
        };
        let bridge = if websocket {
            RemoteBridge::<PhysGame>::connect_with_queue_limits(
                RemoteConfig::new(h.url().unwrap()),
                TransportQueueLimits::default(),
                error_limits,
            )
            .unwrap()
        } else {
            let t = h.connector().connect("user", Caps::ALL).unwrap();
            RemoteBridge::<PhysGame>::connect_transport_with_error_limits(
                Box::new(t),
                RemoteConfig::new(""),
                error_limits,
            )
            .unwrap()
        };
        bridge.request("no.such.method", J::Null).unwrap();
        wait_until("first RPC error", || {
            bridge.error_queue_stats().current_items == 1
        });
        assert_eq!(bridge.take_errors().len(), 1);
        assert_eq!(bridge.error_queue_stats().current_bytes, 0);
        bridge.request("no.such.method", J::Null).unwrap();
        wait_until("second RPC error", || {
            bridge.error_queue_stats().current_items == 1
        });
        bridge.request("no.such.method", J::Null).unwrap();
        wait_until("error overflow terminal", || !bridge.is_alive());
        let stats = bridge.error_queue_stats();
        assert!(stats.closed);
        assert_eq!(
            (stats.current_items, stats.peak_items, stats.saturations),
            (1, 1, 1)
        );
        assert!(stats.peak_bytes <= error_limits.max_bytes);
        assert!(matches!(
            bridge.request("sim.state", J::Null),
            Err(BridgeError::Disconnected)
        ));
        let errors = bridge.take_errors();
        assert_eq!(
            errors.len(),
            2,
            "retained refusal plus separate terminal marker"
        );
        assert_eq!(errors[1].kind(), Some("remote_error_queue_overflow"));
        assert!(
            bridge.take_errors().is_empty(),
            "terminal marker is reported once"
        );
        assert_eq!(
            (
                bridge.error_queue_stats().current_items,
                bridge.error_queue_stats().current_bytes
            ),
            (0, 0)
        );
        assert!(bridge.transport_queue_stats().unwrap().incoming.closed);
    }
}

#[test]
fn zero_and_small_socket_ingress_limits_are_observable_terminal() {
    for limits in [
        QueueLimits {
            max_items: 0,
            max_bytes: 4096,
        },
        QueueLimits {
            max_items: 1,
            max_bytes: 0,
        },
        QueueLimits {
            max_items: 1,
            max_bytes: 3,
        },
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (start, started) = channel();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            ws.send(Message::text("four")).unwrap();
            !matches!(ws.read(), Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
        });
        let mut t = PumpedWs::connect_with_queue_limits(
            &url,
            Duration::from_secs(5),
            TransportQueueLimits {
                outgoing: QueueLimits::default(),
                incoming: limits,
            },
        )
        .unwrap();
        let tx = t.sender().unwrap();
        start.send(()).unwrap();
        wait_until("small ingress terminal", || {
            tx.queue_stats().unwrap().incoming.closed
        });
        assert!(t.recv(Duration::ZERO).is_err());
        let stats = tx.queue_stats().unwrap().incoming;
        assert_eq!(
            (
                stats.current_items,
                stats.current_bytes,
                stats.peak_items,
                stats.peak_bytes
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(stats.saturations, 1);
        drop(t);
        assert!(peer.join().unwrap());
    }
}

#[test]
fn zero_or_undersized_error_limits_reject_retention_and_keep_terminal_marker() {
    for limits in [
        QueueLimits {
            max_items: 0,
            max_bytes: 4096,
        },
        QueueLimits {
            max_items: 1,
            max_bytes: 0,
        },
        QueueLimits {
            max_items: 1,
            max_bytes: 1,
        },
    ] {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = false;
        let host = spawn_phys_host(demo_text(), None, cfg).unwrap();
        let t = host.connector().connect("user", Caps::ALL).unwrap();
        let bridge = RemoteBridge::<PhysGame>::connect_transport_with_error_limits(
            Box::new(t),
            RemoteConfig::new(""),
            limits,
        )
        .unwrap();
        bridge.request("no.such.method", J::Null).unwrap();
        wait_until("first error terminal", || !bridge.is_alive());
        let stats = bridge.error_queue_stats();
        assert_eq!(
            (
                stats.current_items,
                stats.current_bytes,
                stats.peak_items,
                stats.peak_bytes
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(stats.saturations, 1);
        assert!(stats.closed);
        let errors = bridge.take_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind(), Some("remote_error_queue_overflow"));
        assert!(bridge.take_errors().is_empty());
    }
}
