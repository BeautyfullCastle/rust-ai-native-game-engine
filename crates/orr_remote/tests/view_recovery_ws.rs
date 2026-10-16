//! End-to-end recovery through the client-mode ERP host and a real WebSocket.
//!
//! A tiny-capacity bridge generates an actual view-event overflow. Its
//! `ViewStreamSource` turns that cut into an events-reset frame; the ERP host
//! then rate-skips that frame while caching newer frames for its subscriber.

#![allow(clippy::disallowed_types)] // the host and the WebSocket client run on wall-clock threads

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::demo_doc;
use orr_bridge::{Bridge, BridgeConfig, ControlOp, Pacing, PlayConfig, PlayHost, PlaySession, SimControl, Threaded, ThreadedConfig};
use orr_remote::{Auth, ClientPump, ClientSession, ClientSessionHook, ErpClient, ErpServer, Host, ServerConfig, SessionError};
use orr_sample::physics_game::{PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_sample::physics_stream::{phys_client_stream_source, PhysKinds};
use orr_sample::physics_view::PhysExtractor;
use orr_sim::PlayerSlot;
use orr_viewstream::{message_type, EventBatch, EventRecord, Schema, ViewFrame, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET, MSG_EVENTS, MSG_FRAME, STATE_PREDICTED, STATE_VERIFIED};
use serde_json::{json, Value as J};

const WAIT: Duration = Duration::from_secs(10);

enum Action {
    Step(u32),
    Emit(Vec<EventRecord>),
}

struct Work {
    id: u64,
    action: Action,
}

type PhysicsBridge = Threaded<PhysGame>;
type PhysicsSource = orr_viewstream::ViewStreamSource<PhysExtractor, PhysKinds>;

struct OverflowSession {
    bridge: PhysicsBridge,
    source: PhysicsSource,
    schema: Schema,
    work: Receiver<Work>,
    completed: Arc<AtomicU64>,
}

impl OverflowSession {
    fn new(work: Receiver<Work>, completed: Arc<AtomicU64>) -> Self {
        let mut play = PlayConfig::new(2, 11, 60);
        play.game_id = "PhysGame".into();
        play.keyframe_interval = 16;
        play.ring_capacity = 256;
        play.start_paused = true;
        // Shooting emits a deterministic event every third tick. Capacity two
        // ensures Step(100) overruns the bridge's presentation mailbox.
        let mut bridge = Threaded::spawn(
            move || PlayHost::new(PlaySession::<PhysGame>::new(play, PhysConfig::new(0, SceneMode::Rain)), PlayerSlot(0)),
            BridgeConfig::default().with_view_event_capacity(2),
            ThreadedConfig { pacing: Pacing::Manual, ..ThreadedConfig::default() },
        ).expect("start the manually paced simulation thread");
        bridge.set_input(PlayerSlot(0), PhysInput::new(0, 0, 0, true)).unwrap();
        let source = phys_client_stream_source(0, 2, 60, 0);
        let schema = source.schema().clone();
        Self { bridge, source, schema, work, completed }
    }
}

impl ClientSession for OverflowSession {
    fn pump(&mut self) -> ClientPump {
        let work = self.work.try_recv().ok();
        let (work_id, injected_events) = match work {
            Some(Work { id, action: Action::Step(n) }) => {
                self.bridge.control(ControlOp::Step(n)).expect("step the tiny local session");
                (Some(id), None)
            }
            Some(Work { id, action: Action::Emit(events) }) => (Some(id), Some(EventBatch { events }.encode())),
            None => (None, None),
        };

        let output = self.source.pump(&mut self.bridge);
        if let Some(id) = work_id {
            self.completed.store(id, Ordering::Release);
        }
        ClientPump {
            frame: output.frame.map(|frame| frame.encode()),
            events: injected_events.or_else(|| (!output.events.is_empty()).then(|| EventBatch { events: output.events }.encode())),
        }
    }

    fn schema(&self) -> Option<Schema> {
        Some(self.schema.clone())
    }

    fn status(&self) -> J {
        let tick = self.bridge.snapshot().map_or(0, |snapshot| snapshot.tick());
        json!({"mode": "client", "state": "playing", "playing": true, "slot": 0, "player_count": 2, "head_tick": tick, "last_tick": tick, "verified_tick": tick})
    }

    fn confirmed_checksum(&self, _tick: u64) -> Option<(u64, u64)> {
        None
    }

    fn set_input(&mut self, _player: u8, _bytes: &[u8]) -> Result<(), SessionError> {
        Ok(())
    }

    fn send_command(&mut self, _player: u8, _bytes: &[u8]) -> Result<(), SessionError> {
        Ok(())
    }
}

struct LiveHost {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LiveHost {
    fn start(work: Receiver<Work>, completed: Arc<AtomicU64>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let (url_tx, url_rx) = channel();
        let thread = thread::spawn(move || {
            let mut cfg = ServerConfig::new(Auth::DevNoAuth);
            cfg.limits.client_session = Some(ClientSessionHook::new(OverflowSession::new(work, completed)));
            let server = ErpServer::start(cfg).expect("start WebSocket ERP server");
            url_tx.send(server.url()).unwrap();
            let mut host = Host::<PhysGame>::new(demo_doc(), server);
            host.run(&thread_stop, Duration::from_micros(500));
        });
        let url = url_rx.recv_timeout(WAIT).expect("the ERP host starts");
        Self { url, stop, thread: Some(thread) }
    }
}

impl Drop for LiveHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn submit(tx: &Sender<Work>, completed: &AtomicU64, id: u64, action: Action) {
    tx.send(Work { id, action }).expect("the host is still running");
    let deadline = Instant::now() + WAIT;
    while completed.load(Ordering::Acquire) < id {
        assert!(Instant::now() < deadline, "host did not process client-session action {id}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_binary(c: &mut ErpClient, expected_type: u8) -> Vec<u8> {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out waiting for viewstream type {expected_type}");
        let bytes = c.wait_frame(left).unwrap().expect("binary stream message");
        if message_type(&bytes).unwrap() == expected_type {
            return bytes;
        }
    }
}

fn wait_next_binary(c: &mut ErpClient) -> Vec<u8> {
    c.wait_frame(WAIT).unwrap().expect("binary stream message")
}

fn event(tick: u64, seq: u32, state: u8) -> EventRecord {
    EventRecord { tick, system: 0, seq, state, event_type: 0, payload: vec![] }
}

#[test]
fn websocket_subscriber_gets_latest_overflow_baseline_and_no_old_event_revival() {
    let (work_tx, work_rx) = channel();
    let completed = Arc::new(AtomicU64::new(0));
    let host = LiveHost::start(work_rx, completed.clone());
    let mut client = ErpClient::connect(&host.url, None).unwrap();
    let subscribed = client.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1})).unwrap();
    assert_eq!(subscribed["topics"], json!(["viewstream"]));
    let schema = client.wait_notification("watch.viewstream.schema", WAIT).unwrap().expect("schema first");
    assert_eq!(schema["params"]["game"], "PhysGame");

    let first = ViewFrame::decode(&wait_binary(&mut client, MSG_FRAME)).unwrap();
    assert_eq!(first.tick, 0);
    assert!(!first.has(FLAG_EVENTS_RESET));

    // This predicted key is really delivered before the overflow cut. Its late
    // verification below must not resurrect an effect discarded by reset.
    submit(&work_tx, &completed, 1, Action::Emit(vec![event(0, 77, STATE_PREDICTED)]));
    let before_cut = EventBatch::decode(&wait_binary(&mut client, MSG_EVENTS)).unwrap();
    assert_eq!(before_cut.events.iter().map(|e| (e.tick, e.seq, e.state)).collect::<Vec<_>>(), [(0, 77, STATE_PREDICTED)]);

    // The bounded Threaded bridge and ViewStreamSource create the reset frame by
    // overflowing the real notification queue, not by scripting server bytes.
    submit(&work_tx, &completed, 2, Action::Step(100));
    submit(&work_tx, &completed, 3, Action::Step(4));

    // At 1 fps, the tick-100 reset frame is rate-skipped. Tick 104 supersedes it
    // while the interval is active; the server must keep the reset sticky.
    let baseline_bytes = wait_next_binary(&mut client);
    assert_eq!(message_type(&baseline_bytes).unwrap(), MSG_FRAME, "events must stay behind the recovery baseline");
    let baseline = ViewFrame::decode(&baseline_bytes).unwrap();
    assert_eq!(baseline.tick, 104, "the delivered recovery frame must be the newest cached head");
    assert!(baseline.has(FLAG_DISCONTINUITY), "the recovery frame is discontinuous: {baseline:?}");
    assert!(baseline.has(FLAG_EVENTS_RESET), "reset survives rate skipping: {baseline:?}");
    assert!(baseline.entities.iter().all(|entity| entity.prev == entity.cur), "the recovery baseline must not interpolate from an older pose");

    // After the reset baseline is on the wire, a delayed terminal status for
    // the old key is filtered while a post-baseline event remains deliverable.
    submit(
        &work_tx,
        &completed,
        4,
        Action::Emit(vec![event(0, 77, STATE_VERIFIED), event(baseline.tick + 1, 78, STATE_VERIFIED)]),
    );
    let after_cut = EventBatch::decode(&wait_binary(&mut client, MSG_EVENTS)).unwrap();
    assert_eq!(after_cut.events.iter().map(|e| (e.tick, e.seq)).collect::<Vec<_>>(), [(baseline.tick + 1, 78)]);
}
