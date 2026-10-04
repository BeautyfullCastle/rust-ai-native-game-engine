//! Normal socket polling with deterministic I/O interruptions, without platform-specific signals.

use std::io::{self, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};

use tungstenite::protocol::Role;

use super::*;

struct ScriptedStream {
    reads: VecDeque<io::Result<Vec<u8>>>,
    read_calls: usize,
    writes: Vec<u8>,
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
        self.writes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn socket(reads: Vec<io::Result<Vec<u8>>>) -> WebSocket<ScriptedStream> {
    WebSocket::from_raw_socket(ScriptedStream { reads: reads.into(), read_calls: 0, writes: Vec::new() }, Role::Client, None)
}

/// A small, unmasked server-to-client WebSocket message.
fn message(opcode: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126);
    let mut bytes = vec![0x80 | opcode, payload.len() as u8];
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn interrupted_websocket_read_resumes_the_same_response() {
    let text = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
    let bytes = message(1, text.as_bytes());
    // Before a header, within the header, and within the payload.
    for split in [0, 1, 2, 10, bytes.len() - 1] {
        let mut reads = Vec::new();
        if split > 0 {
            reads.push(Ok(bytes[..split].to_vec()));
        }
        reads.extend([Err(ErrorKind::Interrupted.into()), Err(ErrorKind::Interrupted.into()), Ok(bytes[split..].to_vec())]);
        let mut ws = socket(reads);
        let mut queue = VecDeque::new();
        assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none(), "split {split}");
        let calls = ws.get_ref().read_calls;
        assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none());
        assert_eq!(ws.get_ref().read_calls, calls + 1, "yield on each interruption so the caller can check its deadline");
        assert_eq!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().as_deref(), Some(text));
        assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none(), "the response arrives only once");
        assert!(queue.is_empty());
        assert!(ws.get_ref().writes.is_empty(), "polling does not resend an RPC");
    }
}

#[test]
fn interrupted_websocket_frame_preserves_the_queue() {
    let frame = orr_viewstream::ViewFrame { flags: 0, tick: 1, verified_tick: 1, seq: 1, rollback: None, entities: Vec::new(), props: Vec::new() }.encode();
    let bytes = message(2, &frame);
    let mut ws = socket(vec![Ok(bytes[..5].to_vec()), Err(ErrorKind::Interrupted.into()), Ok(bytes[5..].to_vec())]);
    let mut queue = VecDeque::from([Incoming::Error("already queued".into())]);
    assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none());
    assert_eq!(queue.len(), 1);
    assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none());
    assert_eq!(queue.len(), 2);
    assert!(matches!(queue.pop_front(), Some(Incoming::Error(e)) if e == "already queued"));
    assert!(matches!(queue.pop_front(), Some(Incoming::Frame(f)) if f == frame));
    assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none());
    assert!(queue.is_empty(), "the completed frame is queued only once");
}

#[test]
fn websocket_classifies_a_v2_3d_frame_without_losing_bytes() {
    let frame = orr_viewstream::ViewFrame3 {
        flags: 0,
        tick: 7,
        verified_tick: 6,
        seq: 9,
        rollback: None,
        entities: Vec::new(),
        props: Vec::new(),
    }
    .encode();
    let bytes = message(2, &frame);
    let mut ws = socket(vec![Ok(bytes)]);
    let mut queue = VecDeque::new();
    assert!(read_ws_text_or_queue(&mut ws, &mut queue).unwrap().is_none());
    assert!(matches!(queue.pop_front(), Some(Incoming::Frame3(b)) if b == frame));
    assert!(queue.is_empty());
}

#[test]
fn tcp_hex_notification_classifies_a_v2_3d_frame() {
    let frame = orr_viewstream::ViewFrame3 {
        flags: 0,
        tick: 11,
        verified_tick: 10,
        seq: 12,
        rollback: None,
        entities: Vec::new(),
        props: Vec::new(),
    }
    .encode();
    let text = serde_json::json!({"method":"watch.viewstream","params":{"data":to_hex(&frame)}});
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let mut source = SocketSource {
        wire: Wire::Tcp { reader: BufReader::new(client), partial: Vec::new() },
        schema: String::new(),
        next_id: 1,
        queue: VecDeque::new(),
        target: "tcp://127.0.0.1:9".into(),
        client: None,
    };
    drop(listener.accept().unwrap());
    source.handle_text(&text);
    assert!(matches!(source.queue.pop_front(), Some(Incoming::Frame3(b)) if b == frame));
}

#[test]
fn websocket_read_keeps_timeouts_recoverable_and_real_failures_fatal() {
    for kind in [ErrorKind::WouldBlock, ErrorKind::TimedOut, ErrorKind::Interrupted] {
        let mut ws = socket(vec![Err(kind.into())]);
        assert!(read_ws_text_or_queue(&mut ws, &mut VecDeque::new()).unwrap().is_none(), "{kind:?}");
        assert_eq!(ws.get_ref().read_calls, 1);
    }
    let mut ws = socket(vec![Err(ErrorKind::ConnectionReset.into())]);
    let error = read_ws_text_or_queue(&mut ws, &mut VecDeque::new()).unwrap_err();
    assert!(error.starts_with("connection:"), "{error}");
    let mut ws = socket(vec![Ok(message(8, &[]))]);
    assert_eq!(read_ws_text_or_queue(&mut ws, &mut VecDeque::new()).unwrap_err(), "the host closed the connection");
}
