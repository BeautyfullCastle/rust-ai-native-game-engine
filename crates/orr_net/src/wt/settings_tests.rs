//! Poll-controlled HTTP/3 delivery: CONNECT is already buffered while the
//! separate peer control stream is unavailable. No socket scheduling or sleeps.

use bytes::{Buf, Bytes};
use futures_util::FutureExt;
use h3::error::Code;
use h3::quic::{self, ConnectionErrorIncoming, StreamErrorIncoming, StreamId, WriteBuf};
use h3_datagram::quic_traits::{DatagramConnectionExt, RecvDatagram, SendDatagram, SendDatagramErrorIncoming};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use wtransport::proto::{frame::Frame, headers::Headers, session::{SessionRequest, SessionResponse}, settings::Settings};

#[derive(Default)]
struct Delivery {
    control_open: AtomicBool,
    control_polls: AtomicUsize,
    control_waker: Mutex<Option<Waker>>,
    request_delivered: AtomicBool,
    closed: Mutex<Option<(u64, Vec<u8>)>>,
    response: Mutex<Vec<u8>>,
}

impl Delivery {
    fn release_control(&self) {
        self.control_open.store(true, SeqCst);
        self.control_waker.lock().unwrap().take().expect("server registered a control-stream waker").wake();
    }
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, SeqCst);
    }
}

struct Stream {
    id: StreamId,
    data: Option<Bytes>,
    delivery: Arc<Delivery>,
}

impl Stream {
    fn new(id: u64, data: Option<Vec<u8>>, delivery: Arc<Delivery>) -> Self {
        Self { id: StreamId::try_from(id).unwrap(), data: data.map(Bytes::from), delivery }
    }
}

impl quic::RecvStream for Stream {
    type Buf = Bytes;

    fn poll_data(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Bytes>, StreamErrorIncoming>> {
        // The peer keeps both the CONNECT and its critical control stream open.
        match self.data.take() {
            Some(bytes) => Poll::Ready(Ok(Some(bytes))),
            None => Poll::Pending,
        }
    }

    fn stop_sending(&mut self, _: u64) {}
    fn recv_id(&self) -> StreamId { self.id }
}

impl quic::SendStream<Bytes> for Stream {
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> { Poll::Ready(Ok(())) }

    fn send_data<T: Into<WriteBuf<Bytes>>>(&mut self, data: T) -> Result<(), StreamErrorIncoming> {
        let mut data = data.into();
        if self.id == StreamId::try_from(0).unwrap() {
            self.delivery.response.lock().unwrap().extend_from_slice(&data.copy_to_bytes(data.remaining()));
        }
        Ok(())
    }

    fn poll_finish(&mut self, _: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> { Poll::Ready(Ok(())) }
    fn reset(&mut self, _: u64) {}
    fn send_id(&self) -> StreamId { self.id }
}

impl quic::BidiStream<Bytes> for Stream {
    type SendStream = Stream;
    type RecvStream = Stream;

    fn split(self) -> (Stream, Stream) {
        let send = Stream { id: self.id, data: None, delivery: self.delivery.clone() };
        (send, self)
    }
}

#[derive(Clone)]
struct Opener {
    next_uni: Arc<AtomicU64>,
    delivery: Arc<Delivery>,
}

impl quic::OpenStreams<Bytes> for Opener {
    type BidiStream = Stream;
    type SendStream = Stream;

    fn poll_open_bidi(&mut self, _: &mut Context<'_>) -> Poll<Result<Stream, StreamErrorIncoming>> {
        panic!("session acceptance must not open a bidirectional stream")
    }

    fn poll_open_send(&mut self, _: &mut Context<'_>) -> Poll<Result<Stream, StreamErrorIncoming>> {
        Poll::Ready(Ok(Stream::new(self.next_uni.fetch_add(4, SeqCst), None, self.delivery.clone())))
    }

    fn close(&mut self, code: Code, reason: &[u8]) {
        // Preserve the first protocol decision. h3's Connection destructor
        // subsequently calls close(H3_NO_ERROR), which must not replace it.
        let mut closed = self.delivery.closed.lock().unwrap();
        if closed.is_none() {
            *closed = Some((code.value(), reason.to_vec()));
        }
    }
}

struct GatedConnection {
    opener: Opener,
    request: Option<Stream>,
    control: Option<Stream>,
}

impl quic::OpenStreams<Bytes> for GatedConnection {
    type BidiStream = Stream;
    type SendStream = Stream;

    fn poll_open_bidi(&mut self, cx: &mut Context<'_>) -> Poll<Result<Stream, StreamErrorIncoming>> {
        self.opener.poll_open_bidi(cx)
    }

    fn poll_open_send(&mut self, cx: &mut Context<'_>) -> Poll<Result<Stream, StreamErrorIncoming>> {
        self.opener.poll_open_send(cx)
    }

    fn close(&mut self, code: Code, reason: &[u8]) { self.opener.close(code, reason); }
}

impl quic::Connection<Bytes> for GatedConnection {
    type RecvStream = Stream;
    type OpenStreams = Opener;

    fn poll_accept_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<Stream, ConnectionErrorIncoming>> {
        let delivery = &self.opener.delivery;
        delivery.control_polls.fetch_add(1, SeqCst);
        if !delivery.control_open.load(SeqCst) {
            *delivery.control_waker.lock().unwrap() = Some(cx.waker().clone());
            return Poll::Pending;
        }
        match self.control.take() {
            Some(stream) => Poll::Ready(Ok(stream)),
            None => Poll::Pending,
        }
    }

    fn poll_accept_bidi(&mut self, _: &mut Context<'_>) -> Poll<Result<Stream, ConnectionErrorIncoming>> {
        match self.request.take() {
            Some(stream) => {
                self.opener.delivery.request_delivered.store(true, SeqCst);
                Poll::Ready(Ok(stream))
            }
            None => Poll::Pending,
        }
    }

    fn opener(&self) -> Opener { self.opener.clone() }
}

struct NoDatagrams;

impl SendDatagram<Bytes> for NoDatagrams {
    fn send_datagram<T: Into<h3_datagram::datagram::EncodedDatagram<Bytes>>>(&mut self, _: T) -> Result<(), SendDatagramErrorIncoming> {
        panic!("session acceptance must not send a datagram")
    }
}

impl RecvDatagram for NoDatagrams {
    type Buffer = Bytes;

    fn poll_incoming_datagram(&mut self, _: &mut Context<'_>) -> Poll<Result<Bytes, ConnectionErrorIncoming>> {
        panic!("session acceptance must not receive a datagram")
    }
}

impl DatagramConnectionExt<Bytes> for GatedConnection {
    type SendDatagramHandler = NoDatagrams;
    type RecvDatagramHandler = NoDatagrams;

    fn send_datagram_handler(&self) -> NoDatagrams { NoDatagrams }
    fn recv_datagram_handler(&self) -> NoDatagrams { NoDatagrams }
}

fn settings_frame(webtransport: bool) -> Vec<u8> {
    let mut builder = Settings::builder().enable_connect_protocol().enable_h3_datagrams();
    if webtransport { builder = builder.enable_webtransport(); }
    let mut bytes = vec![0]; // HTTP/3 control stream type.
    builder.build().generate_frame().write(&mut bytes).unwrap();
    bytes
}

fn fixture(control: Vec<u8>, open: bool) -> (h3::server::Connection<GatedConnection, Bytes>, Arc<Delivery>) {
    let delivery = Arc::new(Delivery::default());
    delivery.control_open.store(open, SeqCst);
    let mut request = Vec::new();
    SessionRequest::new("https://localhost/relay").unwrap().headers().generate_frame().write(&mut request).unwrap();
    let backend = GatedConnection {
        opener: Opener { next_uni: Arc::new(AtomicU64::new(3)), delivery: delivery.clone() },
        request: Some(Stream::new(0, Some(request), delivery.clone())),
        control: Some(Stream::new(2, Some(control), delivery.clone())),
    };
    let connection = h3::server::builder()
        .enable_webtransport(true).enable_extended_connect(true).enable_datagram(true)
        .max_webtransport_sessions(1).send_grease(false).build(backend)
        .now_or_never().expect("all server control writes are immediately ready").unwrap();
    (connection, delivery)
}

#[test]
fn connect_before_peer_settings_waits_then_accepts() {
    let (connection, delivery) = fixture(settings_frame(true), false);
    let wakes = Arc::new(WakeCount::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    let mut accepting = pin!(super::accept_session(connection));
    let initial = accepting.as_mut().poll(&mut cx);
    if !initial.is_pending() {
        // Record the exact pre-fix outcome, rather than mistaking any error for
        // reproduction of the SETTINGS-order race.
        assert!(matches!(initial, Poll::Ready(None)));
        assert!(delivery.request_delivered.load(SeqCst));
        assert_eq!(*delivery.closed.lock().unwrap(), Some((Code::H3_SETTINGS_ERROR.value(), b"webtransport is not supported by client".to_vec())));
        panic!("valid buffered CONNECT was rejected before the delayed peer SETTINGS were delivered");
    }
    assert!(delivery.control_polls.load(SeqCst) > 0);
    assert!(delivery.closed.lock().unwrap().is_none());
    assert!(delivery.response.lock().unwrap().is_empty(), "no successful response before SETTINGS");

    let before = wakes.0.load(SeqCst);
    delivery.release_control();
    assert!(wakes.0.load(SeqCst) > before, "releasing SETTINGS must wake acceptance");
    let session = match accepting.as_mut().poll(&mut cx) {
        Poll::Ready(Some(session)) => session,
        _ => panic!("compatible released SETTINGS and buffered CONNECT must establish the session"),
    };
    assert_eq!(session.session_id(), h3::webtransport::SessionId::try_from(0u64).unwrap());
    assert!(delivery.request_delivered.load(SeqCst));
    assert!(delivery.closed.lock().unwrap().is_none());
    let response = delivery.response.lock().unwrap();
    let frame = Frame::read(&mut response.as_slice()).unwrap().expect("complete response frame");
    let headers = Headers::with_frame(&frame).unwrap();
    assert!(SessionResponse::try_from(headers).unwrap().code().is_successful());
}

#[test]
fn explicit_disabled_webtransport_settings_are_rejected() {
    let (connection, delivery) = fixture(settings_frame(false), true);
    assert!(super::accept_session(connection).now_or_never().expect("ready incompatible settings").is_none());
    assert_eq!(*delivery.closed.lock().unwrap(), Some((Code::H3_SETTINGS_ERROR.value(), b"webtransport is not supported by client".to_vec())));
    assert!(delivery.response.lock().unwrap().is_empty());
}

#[test]
fn non_settings_first_control_frame_is_rejected() {
    // GOAWAY(stream 0), before SETTINGS: a valid frame in the wrong position.
    let (connection, delivery) = fixture(vec![0, 7, 1, 0], true);
    assert!(super::accept_session(connection).now_or_never().expect("ready malformed control stream").is_none());
    let closed = delivery.closed.lock().unwrap();
    assert_eq!(closed.as_ref().map(|(code, _)| *code), Some(Code::H3_MISSING_SETTINGS.value()));
    assert!(!delivery.request_delivered.load(SeqCst));
    assert!(delivery.response.lock().unwrap().is_empty());
}
