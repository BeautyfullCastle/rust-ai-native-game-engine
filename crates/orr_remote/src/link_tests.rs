//! Blocking ERP polling survives ordinary I/O interruption without sending the RPC again.

use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::sync::atomic::AtomicUsize;

use tungstenite::protocol::Role;

use super::*;
use crate::ErpClient;

struct ScriptedStream {
    reads: VecDeque<io::Result<Vec<u8>>>,
    read_calls: usize,
}

impl Read for ScriptedStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.read_calls += 1;
        let bytes = self
            .reads
            .pop_front()
            .unwrap_or_else(|| Err(ErrorKind::WouldBlock.into()))?;
        let n = out.len().min(bytes.len());
        out[..n].copy_from_slice(&bytes[..n]);
        if n < bytes.len() {
            self.reads.push_front(Ok(bytes[n..].to_vec()));
        }
        Ok(n)
    }
}

impl Write for ScriptedStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn socket(reads: Vec<io::Result<Vec<u8>>>) -> WebSocket<ScriptedStream> {
    WebSocket::from_raw_socket(
        ScriptedStream {
            reads: reads.into(),
            read_calls: 0,
        },
        Role::Client,
        None,
    )
}

fn message(opcode: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126);
    let mut bytes = vec![0x80 | opcode, payload.len() as u8];
    bytes.extend_from_slice(payload);
    bytes
}

struct TestTransport {
    ws: WebSocket<ScriptedStream>,
    requests: Arc<AtomicUsize>,
}

impl Transport for TestTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.requests.fetch_add(1, Relaxed);
        self.ws
            .send(Message::text(req.to_text()))
            .map_err(transport)
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        recv_ws(&mut self.ws)
    }
}

#[test]
fn interrupted_rpc_read_keeps_partial_messages_and_sends_only_once() {
    let note = message(
        1,
        br#"{"jsonrpc":"2.0","method":"watch.activity","params":{}}"#,
    );
    let response = message(
        1,
        br#"{"jsonrpc":"2.0","id":1,"result":{"text":"saved scene"}}"#,
    );
    let ws = socket(vec![
        Ok(note[..1].to_vec()),
        Err(ErrorKind::Interrupted.into()),
        Ok(note[1..].to_vec()),
        Ok(message(2, b"frame")),
        Ok(response[..10].to_vec()),
        Err(ErrorKind::Interrupted.into()),
        Err(ErrorKind::Interrupted.into()),
        Ok(response[10..].to_vec()),
    ]);
    let requests = Arc::new(AtomicUsize::new(0));
    let mut client = ErpClient::with_transport(Box::new(TestTransport {
        ws,
        requests: requests.clone(),
    }));
    assert_eq!(
        client.call("scene.save", json!({})).unwrap(),
        json!({"text": "saved scene"})
    );
    assert_eq!(requests.load(Relaxed), 1);
    assert_eq!(client.notifications.len(), 1);
    assert_eq!(
        client.notifications.pop_front().unwrap()["method"],
        "watch.activity"
    );
    assert_eq!(client.frames.pop_front().unwrap(), b"frame");
    assert!(client.frames.is_empty());
    assert_eq!(
        client.poll().unwrap(),
        0,
        "each response and frame was consumed once"
    );
}

#[test]
fn websocket_poll_yields_on_each_recoverable_read() {
    let mut ws = socket(vec![
        Err(ErrorKind::Interrupted.into()),
        Err(ErrorKind::Interrupted.into()),
        Err(ErrorKind::WouldBlock.into()),
        Err(ErrorKind::TimedOut.into()),
    ]);
    for calls in 1..=4 {
        assert!(recv_ws(&mut ws).unwrap().is_none());
        assert_eq!(
            ws.get_ref().read_calls,
            calls,
            "yield so the caller checks its existing deadline"
        );
    }
}

#[test]
fn websocket_poll_keeps_real_failures_fatal() {
    let mut ws = socket(vec![Err(ErrorKind::ConnectionReset.into())]);
    assert!(matches!(recv_ws(&mut ws), Err(ClientError::Transport(_))));
    let mut ws = socket(vec![Ok(message(8, &[]))]);
    assert!(
        matches!(recv_ws(&mut ws), Err(ClientError::Transport(e)) if e == "closed by the server")
    );
}

#[test]
fn queue_reservation_is_atomic_at_both_limits_and_reclaims_owned_payloads() {
    let budget = QueueBudget::new(QueueLimits {
        max_items: 5,
        max_bytes: 12,
    });
    let start = Arc::new(std::sync::Barrier::new(17));
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let budget = budget.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                budget.reserve(3).ok()
            })
        })
        .collect();
    start.wait();
    let held: Vec<_> = workers
        .into_iter()
        .filter_map(|w| w.join().unwrap())
        .collect();
    assert_eq!(held.len(), 4);
    let stats = budget.stats();
    assert_eq!((stats.current_items, stats.current_bytes), (4, 12));
    assert_eq!(
        (stats.peak_items, stats.peak_bytes, stats.saturations),
        (4, 12, 12)
    );
    drop(held);
    assert_eq!(
        (budget.stats().current_items, budget.stats().current_bytes),
        (0, 0)
    );
    let exact = budget.reserve(12).unwrap();
    assert!(matches!(budget.reserve(1), Err(SendError::Backpressure)));
    drop(exact);
    assert!(matches!(budget.reserve(13), Err(SendError::Backpressure)));
}

#[test]
fn item_zero_byte_overflow_and_closed_admission_are_explicit() {
    for limits in [
        QueueLimits {
            max_items: 0,
            max_bytes: 1,
        },
        QueueLimits {
            max_items: 1,
            max_bytes: 0,
        },
    ] {
        let budget = QueueBudget::new(limits);
        assert!(matches!(budget.reserve(0), Err(SendError::Backpressure)));
        assert_eq!(
            (budget.stats().current_items, budget.stats().current_bytes),
            (0, 0)
        );
    }
    let budget = QueueBudget::new(QueueLimits {
        max_items: 1,
        max_bytes: usize::MAX,
    });
    let full = budget.reserve(usize::MAX).unwrap();
    assert!(matches!(budget.reserve(1), Err(SendError::Backpressure)));
    budget.close("test closed");
    assert!(matches!(budget.reserve(0), Err(SendError::Disconnected)));
    assert_eq!(
        budget.stats().current_bytes,
        usize::MAX,
        "close does not fake reclamation"
    );
    drop(full);
    assert_eq!(
        (budget.stats().current_items, budget.stats().current_bytes),
        (0, 0)
    );
}

#[test]
fn incoming_dequeue_reclaims_and_overflow_is_observable_terminal() {
    let (tx, rx) = incoming_channel(QueueLimits {
        max_items: 1,
        max_bytes: 4,
    });
    assert!(tx.send(Incoming::Text("four".into()), 4));
    assert_eq!(tx.pending(), 4);
    assert!(matches!(rx.recv(Duration::ZERO), Ok(Some(Incoming::Text(s))) if s == "four"));
    assert_eq!(tx.pending(), 0);
    assert!(tx.send(Incoming::Wire(vec![1, 2, 3, 4]), 4));
    assert!(!tx.send(Incoming::Text("x".into()), 1));
    assert!(
        matches!(rx.recv(Duration::ZERO), Err(ClientError::Transport(e)) if e.contains("full"))
    );
    assert_eq!((tx.budget.stats().current_items, tx.pending()), (0, 0));
    assert!(tx.budget.stats().closed);
    assert!(!tx.send(Incoming::Text("x".into()), 1));
}

#[test]
fn incoming_receiver_drop_and_failed_delivery_release_permits() {
    let (tx, rx) = incoming_channel(QueueLimits {
        max_items: 2,
        max_bytes: 8,
    });
    assert!(tx.send(Incoming::Text("four".into()), 4));
    drop(rx);
    assert_eq!((tx.budget.stats().current_items, tx.pending()), (0, 0));
    assert!(!tx.send(Incoming::Text("x".into()), 1));
    // A physical channel failure after quota reservation also rolls back.
    let (sender, receiver) = sync_channel(1);
    let budget = QueueBudget::new(QueueLimits {
        max_items: 2,
        max_bytes: 8,
    });
    let tx = IncomingSender {
        tx: sender,
        budget: budget.clone(),
    };
    drop(receiver);
    assert!(!tx.send(Incoming::Text("four".into()), 4));
    assert_eq!(
        (budget.stats().current_items, budget.stats().current_bytes),
        (0, 0)
    );
}

#[test]
fn websocket_enqueue_refuses_without_waiting_or_retrying_and_rolls_back() {
    let req = Request {
        id: Some(1),
        method: "scene.save".into(),
        params: J::Null,
    };
    let bytes = req.to_text().len();
    let (tx, mut rx) = async_channel(1);
    let outgoing = QueueBudget::new(QueueLimits {
        max_items: 1,
        max_bytes: bytes,
    });
    let handle = WsTx {
        tx,
        outgoing: outgoing.clone(),
        incoming: QueueBudget::new(QueueLimits::default()),
    };
    assert_eq!(handle.try_send(req.clone()), Ok(()));
    assert_eq!(handle.try_send(req.clone()), Err(SendError::Backpressure));
    assert_eq!(outgoing.stats().current_items, 1);
    assert_eq!(rx.try_recv().unwrap().into_inner(), req.to_text());
    assert!(
        rx.try_recv().is_err(),
        "refused call was never queued or retried"
    );
    assert_eq!(outgoing.stats().current_bytes, 0);
    assert_eq!(handle.try_send(req.clone()), Ok(()));
    drop(rx);
    assert_eq!(outgoing.stats().current_items, 0);
    assert_eq!(handle.try_send(req), Err(SendError::Disconnected));
    assert_eq!(outgoing.stats().current_bytes, 0);
}

#[test]
fn local_waiting_permit_reclaims_global_and_client_quota_on_dequeue_and_send_failure() {
    let (inbox, rx) = channel();
    let shared = Arc::new(NetShared {
        auth: crate::Auth::DevNoAuth,
        max_message_bytes: 4096,
        allowed_origins: Vec::new(),
        max_connections: 1,
        max_queued: 2,
        max_queued_bytes: 1024,
        inbox,
        next_id: std::sync::atomic::AtomicU64::new(1),
        conns: AtomicUsize::new(0),
        queued: Arc::new(AtomicUsize::new(0)),
        queued_bytes: Arc::new(AtomicUsize::new(0)),
    });
    let budget = QueueBudget::new(QueueLimits {
        max_items: 1,
        max_bytes: 1024,
    });
    let tx = LocalTx {
        conn: 1,
        shared: shared.clone(),
        outgoing: budget.clone(),
        incoming: QueueBudget::new(QueueLimits::default()),
    };
    let req = Request {
        id: Some(1),
        method: "scene.save".into(),
        params: J::Null,
    };
    assert_eq!(tx.try_send(req.clone()), Ok(()));
    assert_eq!(tx.try_send(req.clone()), Err(SendError::Backpressure));
    assert_eq!(shared.queued.load(Relaxed), 1);
    assert_eq!(
        shared.queued_bytes.load(Relaxed),
        budget.stats().current_bytes
    );
    drop(rx.recv().unwrap());
    assert_eq!(
        (
            shared.queued.load(Relaxed),
            shared.queued_bytes.load(Relaxed)
        ),
        (0, 0)
    );
    assert_eq!(budget.stats().current_items, 0);
    drop(rx);
    assert_eq!(tx.try_send(req), Err(SendError::Disconnected));
    assert_eq!(
        (
            shared.queued.load(Relaxed),
            shared.queued_bytes.load(Relaxed)
        ),
        (0, 0)
    );
    assert_eq!(
        (budget.stats().current_items, budget.stats().current_bytes),
        (0, 0)
    );
}
