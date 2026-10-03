//! Blocking ERP polling survives ordinary I/O interruption without sending the RPC again.

use std::collections::VecDeque;
use std::io::{self, ErrorKind};

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
        let bytes = self.reads.pop_front().unwrap_or_else(|| Err(ErrorKind::WouldBlock.into()))?;
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
    WebSocket::from_raw_socket(ScriptedStream { reads: reads.into(), read_calls: 0 }, Role::Client, None)
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
        self.ws.send(Message::text(req.to_text())).map_err(transport)
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        recv_ws(&mut self.ws)
    }
}

#[test]
fn interrupted_rpc_read_keeps_partial_messages_and_sends_only_once() {
    let note = message(1, br#"{"jsonrpc":"2.0","method":"watch.activity","params":{}}"#);
    let response = message(1, br#"{"jsonrpc":"2.0","id":1,"result":{"text":"saved scene"}}"#);
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
    let mut client = ErpClient::with_transport(Box::new(TestTransport { ws, requests: requests.clone() }));
    assert_eq!(client.call("scene.save", json!({})).unwrap(), json!({"text": "saved scene"}));
    assert_eq!(requests.load(Relaxed), 1);
    assert_eq!(client.notifications.len(), 1);
    assert_eq!(client.notifications.pop_front().unwrap()["method"], "watch.activity");
    assert_eq!(client.frames.pop_front().unwrap(), b"frame");
    assert!(client.frames.is_empty());
    assert_eq!(client.poll().unwrap(), 0, "each response and frame was consumed once");
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
        assert_eq!(ws.get_ref().read_calls, calls, "yield so the caller checks its existing deadline");
    }
}

#[test]
fn websocket_poll_keeps_real_failures_fatal() {
    let mut ws = socket(vec![Err(ErrorKind::ConnectionReset.into())]);
    assert!(matches!(recv_ws(&mut ws), Err(ClientError::Transport(_))));
    let mut ws = socket(vec![Ok(message(8, &[]))]);
    assert!(matches!(recv_ws(&mut ws), Err(ClientError::Transport(e)) if e == "closed by the server"));
}
