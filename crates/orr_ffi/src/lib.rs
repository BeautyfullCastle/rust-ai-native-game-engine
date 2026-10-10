//! `orr_ffi`: Orrery's C ABI. A view written in C, C++, C#, GDScript (through
//! GDExtension) or any language that loads a C library can host a simulation,
//! read what to draw as a stream of bytes and feed it inputs, without linking
//! a Rust type or reading a Rust `Frame` (design doc decision 12).
//!
//! The header is `include/orrery.h` (hand-written; keep it in step with this
//! file). The byte formats are in `docs/view-stream.md`.
//!
//! # Explicit built-in games
//!
//! A Rust game is generic code. This build provides the physics demo through
//! [`orr_host_open`] and a built-in Yard3D scene through
//! [`orr_yard3d_host_open_v1`]. Existing PhysGame and relay-client calls keep
//! their meaning. To host
//! another game, copy this crate, and in [`open_host`] replace
//! `orr_remote::sample::spawn_phys_host` by a `LocalHost::spawn::<YourGame>`
//! with your scene loader, `HostLimits` (including `view_stream`) and
//! extractor. Nothing else changes: the header, the schema, the stream and
//! the control calls are the same for every game.
//!
//! # How it works
//!
//! [`orr_host_open`] starts a host thread (the same `LocalHost` the editor
//! uses: scene document, play session, ERP server) and connects to it
//! in process. The host publishes the view stream (`viewstream` topic) for
//! every tick; the newest frame waits in the handle until polled. Inputs,
//! commands and controls are ERP calls (`sim.input`, `sim.command`,
//! `sim.step`, ...), so anything the editor can do is also available through
//! [`orr_erp_call`].
//!
//! # Client sessions
//!
//! [`orr_client_open`] opens a handle that plays on an `orr_server` room instead of hosting a
//! simulation: it joins as a relay client (QUIC or WebSocket, with the optional simulated network
//! conditions of the sample programs), predicts, rolls back, and publishes the same view stream
//! with the rollback flag and range and with events as predicted, verified or canceled
//! (`client.rs`). `orr_view_poll`, `orr_events_poll`, `orr_set_input` and `orr_schema_json` work
//! as before (inputs for the joined slot only); timeline control, commands from the timeline and
//! ERP calls belong to a local host and are refused. [`orr_session_status`] reports the
//! connection (joining, slot, round trip time, input delay, rollbacks, desync, disconnect).
//!
//! # Rules of the ABI
//!
//! - Every function returns a code (`ORR_OK` = 0, `ORR_NO_FRAME` = 1, negative
//!   = error). The text of the last error on the calling thread is
//!   [`orr_last_error`].
//! - Every pointer argument is checked for null. Nothing else can be checked:
//!   pointers must be valid for the lengths given.
//! - No panic crosses the boundary: each call is wrapped in `catch_unwind` and
//!   turns into `ORR_ERR_PANIC`.
//! - Output buffers: a call that writes into a caller buffer reports the size
//!   needed when the buffer is too small, and then writes nothing (and
//!   consumes nothing).
//! - Threads: a handle is internally locked, so calls from different threads
//!   are safe (they run one after another). `orr_host_close` must not run
//!   concurrently with any other call on the same handle. A pointer from
//!   [`orr_view_poll_ptr`] is valid until the next `orr_view_poll*` call on
//!   that handle or until close.
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::disallowed_types)]
// A view-boundary crate (like `orr_view`): the simulated loss of a client is a fraction.
#![allow(clippy::float_arithmetic)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CStr, CString};
use std::net::SocketAddr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

mod client;

use orr_remote::{Auth, Caps, ClientError, ErpClient, LocalHost, ServerConfig};
use orr_viewstream::{
    message_type, reset_frame_events, EventBatch, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET,
    MSG_EVENTS, MSG_FRAME, MSG_FRAME3D, VERSION_3D,
};
use serde_json::{json, Value as J};

/// Success.
pub const ORR_OK: c_int = 0;
/// Nothing new to read (not an error).
pub const ORR_NO_FRAME: c_int = 1;
/// A required pointer argument was null.
pub const ORR_ERR_NULL: c_int = -1;
/// An argument was invalid (wrong size, unknown op, bad JSON).
pub const ORR_ERR_ARG: c_int = -2;
/// The caller's buffer is too small; the size needed was reported.
pub const ORR_ERR_BUFFER: c_int = -3;
/// The host is gone (it stopped or crashed).
pub const ORR_ERR_HOST: c_int = -4;
/// The host refused the request (an ERP error; the text is in `orr_last_error`).
pub const ORR_ERR_RPC: c_int = -5;
/// A panic was caught inside the library (a bug); the handle may be unusable.
pub const ORR_ERR_PANIC: c_int = -6;
/// A client handle is still joining the room (poll `orr_session_status` until it is playing).
pub const ORR_ERR_NOT_READY: c_int = -7;

/// `orr_control` ops.
pub const ORR_CTL_PLAY: c_int = 0;
/// Stop running by the wall clock.
pub const ORR_CTL_PAUSE: c_int = 1;
/// Run `arg` ticks now.
pub const ORR_CTL_STEP: c_int = 2;
/// Go to recorded tick `arg` and pause.
pub const ORR_CTL_SEEK: c_int = 3;
/// Speed `arg` / 1000 (1000 = 1x).
pub const ORR_CTL_SPEED: c_int = 4;
/// Cut the recorded future after the head.
pub const ORR_CTL_BRANCH: c_int = 5;
/// Start a fresh play session from the scene (tick 0, paused).
pub const ORR_CTL_RESTART: c_int = 6;

/// `OrrHostConfig::flags`: also listen on `127.0.0.1:listen_port` (ERP over WebSocket and TCP).
pub const ORR_HOST_LISTEN: u32 = 1;
/// `OrrHostConfig::flags`: start running by the wall clock (default: paused, step by calls).
pub const ORR_HOST_RUN: u32 = 2;

/// `OrrClientConfig::flags`: `orr_client_open` returns only when the room has started (or failed).
pub const ORR_CLIENT_WAIT: u32 = 1;
/// `OrrClientConfig::flags`: QUIC accepts any server certificate (development only).
pub const ORR_CLIENT_INSECURE: u32 = 2;

/// `OrrClientConfig::transport`: QUIC (needs `fingerprint` or `ORR_CLIENT_INSECURE`).
pub const ORR_TRANSPORT_QUIC: u32 = 0;
/// `OrrClientConfig::transport`: WebSocket (plain; `fingerprint` is unused).
pub const ORR_TRANSPORT_WS: u32 = 1;

/// `OrrSessionStatus::mode`: a local host (`orr_host_open` or `orr_yard3d_host_open_v1`).
pub const ORR_MODE_LOCAL: u32 = 0;
/// `OrrSessionStatus::mode`: a client of a relay server (`orr_client_open`).
pub const ORR_MODE_CLIENT: u32 = 1;

/// `OrrSessionStatus::state`: joining (connecting, handshake, waiting for the other players).
pub const ORR_STATE_CONNECTING: u32 = 0;
/// `OrrSessionStatus::state`: playing.
pub const ORR_STATE_PLAYING: u32 = 1;
/// `OrrSessionStatus::state`: the connection to the server is gone.
pub const ORR_STATE_DISCONNECTED: u32 = 2;
/// `OrrSessionStatus::state`: joining failed (the server refused, or it was not reachable).
pub const ORR_STATE_FAILED: u32 = 3;

/// `OrrSessionStatus::flags`: a desync was detected (the server's room found different checksums).
pub const ORR_STATUS_DESYNC: u32 = 1;

/// Version of this C ABI (bumped when a function changes incompatibly).
/// 2: client sessions (`orr_client_open`, `orr_session_status`, `ORR_ERR_NOT_READY`).
pub const ORR_ABI_VERSION: u32 = 2;

/// Settings of [`orr_host_open`] and [`orr_yard3d_host_open_v1`]. Zero the struct, then set `struct_size`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrrHostConfig {
    /// `sizeof(OrrHostConfig)` as the caller knows it (room to grow).
    pub struct_size: u32,
    /// `ORR_HOST_*` bits.
    pub flags: u32,
    /// With `ORR_HOST_LISTEN`: the port (0 = any free port; see `orr_host_url`).
    pub listen_port: u32,
}

/// Settings of [`orr_client_open`]. Zero the struct, then set `struct_size` and `server`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrrClientConfig {
    /// `sizeof(OrrClientConfig)` as the caller knows it.
    pub struct_size: u32,
    /// `ORR_CLIENT_*` bits.
    pub flags: u32,
    /// `ORR_TRANSPORT_*`.
    pub transport: u32,
    /// The slot to ask for; negative = any free slot.
    pub slot: i32,
    /// Room id (0 = room 1, the default room of `orr_server`).
    pub room: u64,
    /// Seed of the simulated loss and jitter (0 = a fresh seed per client).
    pub sim_seed: u64,
    /// Simulated one-way delay added in each direction, milliseconds (0 = none).
    pub sim_latency_ms: u32,
    /// Simulated random extra delay, up to this many milliseconds.
    pub sim_jitter_ms: u32,
    /// Simulated loss of unreliable messages in each direction, in thousandths (20 = 2 %).
    pub sim_loss_permille: u32,
    /// How long to wait for the handshake and for the room to start (0 = 60 s).
    pub connect_timeout_ms: u32,
    /// `host:port` of the server (UTF-8, required).
    pub server: *const c_char,
    /// QUIC: the server certificate's SHA-256 as hex (the server prints it); null = none.
    pub fingerprint: *const c_char,
    /// Where desync dumps (`.orrd`) go; null = a folder in the system temp directory.
    pub desync_dir: *const c_char,
}

/// What [`orr_session_status`] fills. Zero the struct and set `struct_size`; fields the
/// library does not know (an older library) stay zero, fields the caller does not know are not written.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OrrSessionStatus {
    /// `sizeof(OrrSessionStatus)` as the caller knows it. On return: the bytes written.
    pub struct_size: u32,
    /// `ORR_MODE_*`.
    pub mode: u32,
    /// `ORR_STATE_*`.
    pub state: u32,
    /// `ORR_STATUS_*` bits.
    pub flags: u32,
    /// The joined slot (client), else 0.
    pub slot: u32,
    /// Players of the session.
    pub player_count: u32,
    /// Smoothed round trip time to the server, milliseconds (client).
    pub rtt_ms: u32,
    /// Input delay in ticks the client currently uses (client).
    pub input_delay: u32,
    /// Predicted head tick.
    pub head_tick: u64,
    /// Newest fully confirmed tick.
    pub verified_tick: u64,
    /// Rollbacks so far.
    pub rollbacks: u64,
    /// Ticks resimulated by all rollbacks together.
    pub resim_ticks: u64,
    /// The latest rollback resimulated `last_rollback_from..=last_rollback_to` (0, 0 = none yet).
    pub last_rollback_from: u64,
    pub last_rollback_to: u64,
    /// Desyncs the server's room reported.
    pub desyncs: u64,
    /// Episodes of waiting because the prediction limit was reached, and their total time in ms.
    pub stall_episodes: u64,
    pub stalled_ms: u64,
    /// Ticks the server confirmed with a repeated input of this client (it was late).
    pub repeated_inputs: u64,
}

const DEMO_SCENE: &str = include_str!("../../../scenes/physics_demo.scene.yaml");
const CLIENT_NAME: &str = "ffi";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Event batches kept for a caller that does not poll; the oldest are dropped above this.
/// This FFI-side cap does not synthesize `FLAG_EVENTS_RESET`; only a reset delivered by the
/// upstream view producer can establish a fresh baseline. In particular, the local-host ERP path
/// does not gain a new bounded-queue recovery guarantee from this FFI mailbox change.
const MAX_QUEUED_EVENT_BATCHES: usize = 4096;

#[derive(Debug)]
struct Fail {
    code: c_int,
    msg: String,
}

fn fail<T>(code: c_int, msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail { code, msg: msg.into() })
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: &str) {
    let text = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = text);
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Runs `f`, turning errors and panics into a code and the thread's last error.
fn guard(f: impl FnOnce() -> Result<c_int, Fail>) -> c_int {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            set_error(&e.msg);
            e.code
        }
        Err(p) => {
            set_error(&format!("panic inside orr_ffi: {}", panic_text(p.as_ref())));
            ORR_ERR_PANIC
        }
    }
}

/// A host of our own and the connection to it.
struct Local {
    // Field order is drop order: the connection goes before the host thread.
    client: ErpClient,
    host: LocalHost,
    url: Option<String>,
    run_on_start: bool,
}

enum Backend {
    Local(Box<Local>),
    Client(Box<client::Client>),
}

struct Inner {
    backend: Backend,
    schema_json: String,
    input_size: usize,
    player_count: u8,
    /// The newest frame not read yet, with event-reset baselines latched until acknowledged.
    view: FrameMailbox,
    /// The frame handed out by `orr_view_poll_ptr`.
    held: Vec<u8>,
    events: VecDeque<Vec<u8>>,
}

/// The newest unread view frame. An event reset remains sticky until the caller
/// successfully takes this baseline; a too-small copy buffer does not acknowledge it.
#[derive(Default)]
struct FrameMailbox {
    latest: Option<Vec<u8>>,
    reset_pending: bool,
    /// After a reset baseline is acknowledged, ignore late old-timeline event records.
    reset_floor_tick: Option<u64>,
}

impl FrameMailbox {
    fn push(&mut self, mut frame: Vec<u8>, events: &mut VecDeque<Vec<u8>>) -> Result<(), Fail> {
        // Both 2D and 3D view-frame headers put the flags byte at offset 7. The
        // producer's helper handles the versioned body layout when we need to
        // coalesce a newer snapshot into the still-unread baseline.
        let flags = frame.get(7).copied().unwrap_or_default();
        let reset = flags & FLAG_EVENTS_RESET != 0;
        if reset {
            self.reset_pending = true;
            events.clear();
        } else if !self.reset_pending && flags & FLAG_DISCONTINUITY != 0 {
            // A seek/branch starts a new timeline; an older reset's tick cutoff
            // must not hide valid events in that new timeline.
            self.reset_floor_tick = None;
        }
        if self.reset_pending {
            frame = reset_frame_events(&frame).map_err(|e| Fail {
                code: ORR_ERR_HOST,
                msg: format!("cannot preserve the view event-reset baseline: {e}"),
            })?;
        }
        self.latest = Some(frame);
        Ok(())
    }

    fn take(&mut self) -> Option<Vec<u8>> {
        let frame = self.latest.take()?;
        if self.reset_pending {
            self.reset_floor_tick = frame_tick(&frame);
        }
        self.reset_pending = false;
        Some(frame)
    }

    fn acknowledge_copy(&mut self) {
        if self.reset_pending {
            self.reset_floor_tick = self.latest.as_deref().and_then(frame_tick);
        }
        self.latest = None;
        self.reset_pending = false;
    }

    fn clear(&mut self) {
        self.latest = None;
        self.reset_pending = false;
        self.reset_floor_tick = None;
    }
}

fn frame_tick(frame: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(frame.get(8..16)?.try_into().ok()?))
}

fn queue_event_batch(view: &FrameMailbox, events: &mut VecDeque<Vec<u8>>, mut bytes: Vec<u8>) -> Result<(), Fail> {
    // Do not let events from beyond the reset cut race ahead of the baseline.
    // Once the consumer has acknowledged that baseline, newly pumped batches resume.
    if view.reset_pending {
        return Ok(());
    }
    if let Some(floor) = view.reset_floor_tick {
        let mut batch = EventBatch::decode(&bytes).map_err(|e| Fail {
            code: ORR_ERR_HOST,
            msg: format!("cannot filter events against the view reset baseline: {e}"),
        })?;
        batch.events.retain(|event| event.tick > floor);
        if batch.events.is_empty() {
            return Ok(());
        }
        bytes = batch.encode();
    }
    if events.len() >= MAX_QUEUED_EVENT_BATCHES {
        events.pop_front();
    }
    events.push_back(bytes);
    Ok(())
}

/// An opaque host handle (see the header).
pub struct OrrHost {
    inner: Mutex<Inner>,
}

impl Inner {
    /// Moves everything that arrived into the frame slot and the event queue.
    fn pump(&mut self) -> Result<(), Fail> {
        match &mut self.backend {
            Backend::Local(l) => {
                if !l.host.is_running() {
                    let why = l.host.stopped_reason(Duration::from_millis(50)).unwrap_or_default();
                    return fail(ORR_ERR_HOST, format!("the host thread has stopped: {why}"));
                }
                let alive = l.client.poll();
                while let Some(bytes) = l.client.frames.pop_front() {
                    match message_type(&bytes) {
                        Ok(MSG_FRAME | MSG_FRAME3D) => self.view.push(bytes, &mut self.events)?,
                        Ok(MSG_EVENTS) => queue_event_batch(&self.view, &mut self.events, bytes)?,
                        _ => {}
                    }
                }
                // Notifications (watch.*) are not needed here.
                l.client.notifications.clear();
                l.client.local_frames.clear();
                match alive {
                    Ok(_) => Ok(()),
                    Err(e) => fail(ORR_ERR_HOST, format!("the host is gone: {e}")),
                }
            }
            Backend::Client(c) => {
                if let Some(ready) = c.poll_join() {
                    self.schema_json = ready.schema_json;
                    self.input_size = ready.input_size;
                    self.player_count = ready.player_count;
                }
                if let Some(why) = c.failure() {
                    return fail(ORR_ERR_HOST, format!("joining the server failed: {why}"));
                }
                let out = c.pump();
                // RelayView returns a frame and its events separately, so process
                // the frame first: a reset frame must clear/suppress any batches
                // associated with the same pump before they reach the FFI queue.
                if let Some(frame) = out.frame {
                    self.view.push(frame, &mut self.events)?;
                }
                if let Some(batch) = out.events {
                    queue_event_batch(&self.view, &mut self.events, batch)?;
                }
                Ok(())
            }
        }
    }

    /// The local host, or an error for a client session.
    fn local(&mut self) -> Result<&mut Local, Fail> {
        match &mut self.backend {
            Backend::Local(l) => Ok(l),
            Backend::Client(_) => fail(ORR_ERR_ARG, "not available in a client session (it plays on a server; see orr_client_open)"),
        }
    }

    fn call(&mut self, method: &str, params: J) -> Result<J, Fail> {
        self.pump()?;
        let r = self.local()?.client.call(method, params);
        let out = match r {
            Ok(v) => Ok(v),
            Err(ClientError::Rpc(e)) => fail(ORR_ERR_RPC, format!("{method}: {e}")),
            Err(e) => fail(ORR_ERR_HOST, format!("{method}: {e}")),
        };
        // Frames that arrived while waiting for the response.
        let _ = self.pump();
        out
    }

    fn start_session(&mut self) -> Result<(), Fail> {
        let params = json!({"run": self.local()?.run_on_start});
        self.call("sim.start", params)?;
        Ok(())
    }
}

fn lock(h: &OrrHost) -> MutexGuard<'_, Inner> {
    h.inner.lock().unwrap_or_else(PoisonError::into_inner)
}

unsafe fn host_ref<'a>(h: *mut OrrHost) -> Result<&'a OrrHost, Fail> {
    match h.as_ref() {
        Some(r) => Ok(r),
        None => fail(ORR_ERR_NULL, "host handle is null"),
    }
}

/// Copies `data` into the caller's buffer if it fits. Returns whether it did.
unsafe fn write_out(buf: *mut u8, cap: usize, data: &[u8]) -> Result<bool, Fail> {
    if buf.is_null() {
        return if cap == 0 { Ok(false) } else { fail(ORR_ERR_NULL, "output buffer is null but its capacity is not 0") };
    }
    if cap < data.len() {
        return Ok(false);
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
    Ok(true)
}

/// Text output: NUL-terminated; reports the size needed including the NUL.
unsafe fn write_text(buf: *mut c_char, cap: usize, text: &str) -> Result<usize, Fail> {
    let needed = text.len() + 1;
    let mut bytes = Vec::with_capacity(needed);
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    if !write_out(buf.cast(), cap, &bytes)? && !buf.is_null() && cap > 0 {
        *buf = 0;
    }
    Ok(needed)
}

fn host_settings(cfg: *const OrrHostConfig) -> Result<(u32, u16), Fail> {
    let (flags, port) = match unsafe { cfg.as_ref() } {
        None => (0, 0),
        Some(c) => {
            if (c.struct_size as usize) < 3 * std::mem::size_of::<u32>() {
                return fail(
                    ORR_ERR_ARG,
                    "OrrHostConfig.struct_size is too small (set it to sizeof(OrrHostConfig))",
                );
            }
            (c.flags, c.listen_port)
        }
    };
    if port > u32::from(u16::MAX) {
        return fail(ORR_ERR_ARG, "listen_port must be 0..=65535");
    }
    Ok((flags, port as u16))
}

fn host_server(flags: u32, port: u16) -> ServerConfig {
    let mut server = ServerConfig::new(Auth::DevNoAuth);
    server.listen = flags & ORR_HOST_LISTEN != 0;
    server.bind = SocketAddr::from(([127, 0, 0, 1], port));
    server
}

fn open_host(scene_path: *const c_char, cfg: *const OrrHostConfig) -> Result<OrrHost, Fail> {
    let (flags, port) = host_settings(cfg)?;
    let (text, path) = if scene_path.is_null() {
        (DEMO_SCENE.to_string(), None)
    } else {
        let p = unsafe { CStr::from_ptr(scene_path) }
            .to_str()
            .map_err(|_| Fail {
                code: ORR_ERR_ARG,
                msg: "scene path is not UTF-8".into(),
            })?;
        let text = std::fs::read_to_string(p).map_err(|e| Fail {
            code: ORR_ERR_ARG,
            msg: format!("cannot read scene {p}: {e}"),
        })?;
        (text, Some(PathBuf::from(p)))
    };
    let host =
        orr_remote::sample::spawn_phys_host(text, path, host_server(flags, port)).map_err(|e| {
            Fail {
                code: ORR_ERR_ARG,
                msg: e,
            }
        })?;
    connect_local(host, flags)
}

fn open_yard3d_host(max_view_version: u32, cfg: *const OrrHostConfig) -> Result<OrrHost, Fail> {
    // Refuse before starting a thread or binding a socket. The API suffix is
    // this entry's contract version; the negotiated view format is version 2.
    if max_view_version < u32::from(VERSION_3D) {
        return fail(
            ORR_ERR_ARG,
            "Yard3D requires view-stream version 2 (max_view_version is too low)",
        );
    }
    let (flags, port) = host_settings(cfg)?;
    let host = orr_remote::yard3d::spawn_yard3d_host(
        orr_sample::yard3d_game::YardConfig::new(24),
        host_server(flags, port),
    )
    .map_err(|e| Fail {
        code: ORR_ERR_ARG,
        msg: e,
    })?;
    connect_local(host, flags)
}

fn connect_local(host: LocalHost, flags: u32) -> Result<OrrHost, Fail> {
    let url = host.url().map(str::to_string);
    let transport = host
        .connector()
        .connect(CLIENT_NAME, Caps::ALL)
        .map_err(|e| Fail {
            code: ORR_ERR_HOST,
            msg: e.to_string(),
        })?;
    let mut client = ErpClient::with_transport(Box::new(transport));
    client.call_timeout = CALL_TIMEOUT;
    let mut inner = Inner {
        backend: Backend::Local(Box::new(Local {
            client,
            host,
            url,
            run_on_start: flags & ORR_HOST_RUN != 0,
        })),
        schema_json: String::new(),
        input_size: 0,
        player_count: 0,
        view: FrameMailbox::default(),
        held: Vec::new(),
        events: VecDeque::new(),
    };
    inner.start_session()?;
    // Up to 1000 frames a second: the caller paces itself by polling.
    // (Not through `Inner::call`: it would discard the schema notification that comes with the response.)
    let local = inner.local()?;
    match local.client.call(
        "watch.subscribe",
        json!({"topics": ["viewstream"], "max_fps": 1000, "source": "sim"}),
    ) {
        Ok(_) => {}
        Err(ClientError::Rpc(e)) => return fail(ORR_ERR_RPC, format!("watch.subscribe: {e}")),
        Err(e) => return fail(ORR_ERR_HOST, format!("watch.subscribe: {e}")),
    }
    let schema = local
        .client
        .wait_notification("watch.viewstream.schema", Duration::from_secs(10))
        .map_err(|e| Fail {
            code: ORR_ERR_HOST,
            msg: e.to_string(),
        })?
        .ok_or_else(|| Fail {
            code: ORR_ERR_HOST,
            msg: "the host sent no schema".into(),
        })?;
    let schema = schema.get("params").cloned().unwrap_or(J::Null);
    inner.input_size = schema
        .pointer("/input/size")
        .and_then(J::as_u64)
        .unwrap_or(0) as usize;
    inner.player_count = schema.get("player_count").and_then(J::as_u64).unwrap_or(0) as u8;
    inner.schema_json = schema.to_string();
    inner.pump()?;
    Ok(OrrHost {
        inner: Mutex::new(inner),
    })
}

fn text_arg(p: *const c_char, what: &str) -> Result<Option<String>, Fail> {
    if p.is_null() {
        return Ok(None);
    }
    let s = unsafe { CStr::from_ptr(p) }.to_str().map_err(|_| Fail { code: ORR_ERR_ARG, msg: format!("{what} is not UTF-8") })?;
    Ok(Some(s.to_string()))
}

fn open_client(cfg: *const OrrClientConfig) -> Result<OrrHost, Fail> {
    let c = unsafe { cfg.as_ref() }.ok_or(Fail { code: ORR_ERR_NULL, msg: "client config is null".into() })?;
    if (c.struct_size as usize) < std::mem::size_of::<OrrClientConfig>() {
        return fail(ORR_ERR_ARG, "OrrClientConfig.struct_size is too small (set it to sizeof(OrrClientConfig))");
    }
    let server = text_arg(c.server, "server")?.ok_or(Fail { code: ORR_ERR_ARG, msg: "OrrClientConfig.server is required (host:port)".into() })?;
    let kind = match c.transport {
        ORR_TRANSPORT_QUIC => orr_relay_net::TransportKind::Quic,
        ORR_TRANSPORT_WS => orr_relay_net::TransportKind::Ws,
        other => return fail(ORR_ERR_ARG, format!("unknown transport {other}")),
    };
    if c.sim_loss_permille > 1000 {
        return fail(ORR_ERR_ARG, "sim_loss_permille must be 0..=1000");
    }
    let mut args = orr_sample::net_client::NetArgs {
        connect: Some(server),
        kind,
        insecure_dev: c.flags & ORR_CLIENT_INSECURE != 0,
        room: if c.room == 0 { 1 } else { c.room },
        slot: u8::try_from(c.slot).ok(),
        name: CLIENT_NAME.to_string(),
        sim_seed: (c.sim_seed != 0).then_some(c.sim_seed),
        quiet: true,
        ..orr_sample::net_client::NetArgs::default()
    };
    if c.slot > i32::from(u8::MAX) {
        return fail(ORR_ERR_ARG, "slot must be below 256 (or negative for any)");
    }
    if let Some(fp) = text_arg(c.fingerprint, "fingerprint")? {
        args.fingerprint = Some(orr_relay_net::parse_fingerprint(&fp).map_err(|e| Fail { code: ORR_ERR_ARG, msg: format!("fingerprint: {e}") })?);
    }
    args.sim = orr_relay_net::SimConditions {
        latency_ms: u64::from(c.sim_latency_ms),
        jitter_ms: u64::from(c.sim_jitter_ms),
        loss: c.sim_loss_permille as f32 / 1000.0,
        seed: 0,
    };
    args.desync_dir = match text_arg(c.desync_dir, "desync_dir")? {
        Some(d) => PathBuf::from(d),
        None => std::env::temp_dir().join("orr_desync"),
    };
    if c.connect_timeout_ms != 0 {
        args.connect_timeout = Duration::from_millis(u64::from(c.connect_timeout_ms));
    }
    let wait = c.flags & ORR_CLIENT_WAIT != 0;
    let timeout = args.connect_timeout + client::WAIT_SLACK;
    let client = client::Client::start(args).map_err(|e| Fail { code: ORR_ERR_HOST, msg: e })?;
    let mut inner = Inner {
        backend: Backend::Client(Box::new(client)),
        schema_json: String::new(),
        input_size: 0,
        player_count: 0,
        view: FrameMailbox::default(),
        held: Vec::new(),
        events: VecDeque::new(),
    };
    if wait {
        let end = std::time::Instant::now() + timeout;
        loop {
            inner.pump()?;
            let Backend::Client(c) = &inner.backend else { unreachable!() };
            if !c.is_joining() {
                break;
            }
            if std::time::Instant::now() > end {
                return fail(ORR_ERR_HOST, "timed out waiting for the room to start");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(OrrHost { inner: Mutex::new(inner) })
}

// ---- the C functions ----

/// The ABI version ([`ORR_ABI_VERSION`]), to check a loaded library against the header.
#[no_mangle]
pub extern "C" fn orr_abi_version() -> u32 {
    ORR_ABI_VERSION
}

/// The text of the last error of the calling thread, valid until the next
/// call of this library on that thread. Never null.
#[no_mangle]
pub extern "C" fn orr_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// Opens the PhysGame host on the scene at `scene_path` (a UTF-8
/// path to a scene YAML; null = the built-in demo scene) and starts its
/// thread. `cfg` may be null. Returns null on failure (see `orr_last_error`).
#[no_mangle]
pub unsafe extern "C" fn orr_host_open(scene_path: *const c_char, cfg: *const OrrHostConfig) -> *mut OrrHost {
    let mut out: *mut OrrHost = std::ptr::null_mut();
    guard(|| {
        out = Box::into_raw(Box::new(open_host(scene_path, cfg)?));
        Ok(ORR_OK)
    });
    out
}

/// Opens the deterministic built-in Yard3D scene, paused unless `ORR_HOST_RUN` is set.
/// `max_view_version` is the highest view-stream format the caller understands:
/// values below 2 are refused before host startup, with a null result and an
/// explanation in [`orr_last_error`]. A higher maximum still receives version 2.
/// `cfg` may be null and has the same meaning as for [`orr_host_open`].
///
/// This additive entry's `_v1` suffix versions its C contract, independently of
/// view-stream version 2. Existing ABI 2 symbols/layouts and the PhysGame entry
/// are unchanged. There is no scene-path or relay-client variant of this entry.
#[no_mangle]
pub unsafe extern "C" fn orr_yard3d_host_open_v1(
    max_view_version: u32,
    cfg: *const OrrHostConfig,
) -> *mut OrrHost {
    let mut out: *mut OrrHost = std::ptr::null_mut();
    guard(|| {
        out = Box::into_raw(Box::new(open_yard3d_host(max_view_version, cfg)?));
        Ok(ORR_OK)
    });
    out
}

/// Opens a handle that plays on a relay server (`orr_server --game physics`) as a client:
/// predicts, rolls back, and publishes the same view stream and events as a local host, with the
/// rollback flag and range and events as predicted, verified or canceled. Returns at once (the
/// handle is `ORR_STATE_CONNECTING`; see `orr_session_status`) unless `ORR_CLIENT_WAIT` is set,
/// which waits until the room has started. Returns null on failure (see `orr_last_error`).
#[no_mangle]
pub unsafe extern "C" fn orr_client_open(cfg: *const OrrClientConfig) -> *mut OrrHost {
    let mut out: *mut OrrHost = std::ptr::null_mut();
    guard(|| {
        out = Box::into_raw(Box::new(open_client(cfg)?));
        Ok(ORR_OK)
    });
    out
}

/// Fills `*out` with the state of the session: mode, joining state, slot, round trip time,
/// input delay, rollbacks and their depth, desyncs, stalls (see `OrrSessionStatus`). Works on
/// both kinds of handle (a local host reports `ORR_MODE_LOCAL`, playing, and the newest frame's ticks).
#[no_mangle]
pub unsafe extern "C" fn orr_session_status(host: *mut OrrHost, out: *mut OrrSessionStatus) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let out = out.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "out is null".into() })?;
        let known = out.struct_size as usize;
        if known < std::mem::size_of::<u32>() {
            return fail(ORR_ERR_ARG, "OrrSessionStatus.struct_size is not set");
        }
        let mut inner = lock(h);
        let _ = inner.pump();
        let mut s = OrrSessionStatus { struct_size: std::mem::size_of::<OrrSessionStatus>() as u32, ..OrrSessionStatus::default() };
        match &inner.backend {
            Backend::Local(l) => {
                s.mode = ORR_MODE_LOCAL;
                s.state = if l.host.is_running() { ORR_STATE_PLAYING } else { ORR_STATE_DISCONNECTED };
                s.player_count = u32::from(inner.player_count);
            }
            Backend::Client(c) => {
                let st = c.status();
                s.mode = ORR_MODE_CLIENT;
                s.state = st.state;
                s.flags = if st.desyncs > 0 { ORR_STATUS_DESYNC } else { 0 };
                s.slot = st.slot;
                s.player_count = st.player_count;
                s.rtt_ms = st.rtt_ms;
                s.input_delay = st.input_delay;
                s.head_tick = st.head_tick;
                s.verified_tick = st.verified_tick;
                s.rollbacks = st.rollbacks;
                s.resim_ticks = st.resim_ticks;
                s.last_rollback_from = st.last_rollback_from;
                s.last_rollback_to = st.last_rollback_to;
                s.desyncs = st.desyncs;
                s.stall_episodes = st.stall_episodes;
                s.stalled_ms = st.stalled_ms;
                s.repeated_inputs = st.repeats;
            }
        }
        let n = known.min(std::mem::size_of::<OrrSessionStatus>());
        // Copy the part both sides know; the caller's struct_size reports how much that was.
        std::ptr::copy_nonoverlapping(std::ptr::addr_of!(s).cast::<u8>(), (out as *mut OrrSessionStatus).cast::<u8>(), n);
        out.struct_size = n as u32;
        Ok(ORR_OK)
    })
}

/// The checksum of the confirmed (verified) state of a client session at `tick`, the one the
/// client also reports to the server (every `checksum_interval` ticks, 30 by default: ticks that
/// are multiples of it). Two peers that agree on a tick have the same state there: a view, a test or
/// a replay uploader can compare them. `tick` 0 asks for the newest checkpoint. On `ORR_OK`,
/// `*found_tick` and `*checksum` are set; `ORR_NO_FRAME` if that tick is not confirmed yet (or
/// is no checkpoint tick); `ORR_ERR_ARG` on a local host.
#[no_mangle]
pub unsafe extern "C" fn orr_confirmed_checksum(host: *mut OrrHost, tick: u64, found_tick: *mut u64, checksum: *mut u64) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let found_tick = found_tick.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "found_tick is null".into() })?;
        let checksum = checksum.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "checksum is null".into() })?;
        let inner = lock(h);
        match &inner.backend {
            Backend::Client(c) => match c.confirmed_checksum(tick) {
                Some((t, sum)) => {
                    *found_tick = t;
                    *checksum = sum;
                    Ok(ORR_OK)
                }
                None => Ok(ORR_NO_FRAME),
            },
            Backend::Local(_) => fail(ORR_ERR_ARG, "orr_confirmed_checksum is for client sessions (a local host has no confirmation to wait for)"),
        }
    })
}

/// Stops the host thread and frees the handle. Null is ignored.
#[no_mangle]
pub unsafe extern "C" fn orr_host_close(host: *mut OrrHost) {
    if host.is_null() {
        return;
    }
    guard(|| {
        drop(Box::from_raw(host));
        Ok(ORR_OK)
    });
}

/// Writes the schema JSON (NUL-terminated) into `buf` if it fits and returns
/// the size needed including the NUL (0 on error). Call with `buf` null and
/// `cap` 0 to ask for the size.
#[no_mangle]
pub unsafe extern "C" fn orr_schema_json(host: *mut OrrHost, buf: *mut c_char, cap: usize) -> usize {
    let mut needed = 0;
    guard(|| {
        let h = host_ref(host)?;
        let mut inner = lock(h);
        let _ = inner.pump();
        if inner.schema_json.is_empty() {
            return fail(ORR_ERR_NOT_READY, "the schema is not known yet (the client is still joining the room)");
        }
        needed = write_text(buf, cap, &inner.schema_json)?;
        Ok(ORR_OK)
    });
    needed
}

/// Writes the `ws://` URL of the host's ERP socket (NUL-terminated) and
/// returns the size needed including the NUL. Returns 0 if the host does not
/// listen (`ORR_HOST_LISTEN` was not set) or on error.
#[no_mangle]
pub unsafe extern "C" fn orr_host_url(host: *mut OrrHost, buf: *mut c_char, cap: usize) -> usize {
    let mut needed = 0;
    guard(|| {
        let h = host_ref(host)?;
        let inner = lock(h);
        if let Backend::Local(l) = &inner.backend {
            if let Some(url) = &l.url {
                needed = write_text(buf, cap, url)?;
            }
        }
        Ok(ORR_OK)
    });
    needed
}

/// Copies the newest view frame that was not read yet into `buf` and sets
/// `*written` to its size. Returns `ORR_OK`; `ORR_NO_FRAME` if nothing new
/// (`*written` = 0); `ORR_ERR_BUFFER` if `cap` is too small (`*written` = the
/// size needed, the frame stays for the next poll).
/// A null `buf` with nonzero `cap` returns `ORR_ERR_NULL` before polling,
/// even when there is no new frame. Null with zero capacity remains a size probe.
#[no_mangle]
pub unsafe extern "C" fn orr_view_poll(host: *mut OrrHost, buf: *mut u8, cap: usize, written: *mut usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let written = written.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "written is null".into() })?;
        *written = 0;
        if buf.is_null() && cap != 0 {
            return fail(ORR_ERR_NULL, "output buffer is null but its capacity is not 0");
        }
        let mut inner = lock(h);
        inner.pump()?;
        let Some(frame) = inner.view.latest.as_ref() else { return Ok(ORR_NO_FRAME) };
        *written = frame.len();
        if !write_out(buf, cap, frame)? {
            return fail(ORR_ERR_BUFFER, format!("buffer too small: {} bytes needed, {cap} given", frame.len()));
        }
        // Only a complete copy acknowledges a reset baseline. ORR_ERR_BUFFER
        // above deliberately leaves both the frame and reset latch untouched.
        inner.view.acknowledge_copy();
        Ok(ORR_OK)
    })
}

/// Zero-copy variant of `orr_view_poll`: sets `*data` and `*len` to the newest
/// unread frame, valid until the next `orr_view_poll` or `orr_view_poll_ptr`
/// on this handle or until close. `ORR_NO_FRAME` if nothing new (`*data` null).
#[no_mangle]
pub unsafe extern "C" fn orr_view_poll_ptr(host: *mut OrrHost, data: *mut *const u8, len: *mut usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let data = data.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "data is null".into() })?;
        let len = len.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "len is null".into() })?;
        *data = std::ptr::null();
        *len = 0;
        let mut inner = lock(h);
        inner.pump()?;
        let Some(frame) = inner.view.take() else { return Ok(ORR_NO_FRAME) };
        inner.held = frame;
        *data = inner.held.as_ptr();
        *len = inner.held.len();
        Ok(ORR_OK)
    })
}

/// Takes the oldest queued event batch message (see `docs/view-stream.md`).
/// Same buffer rules as `orr_view_poll`; events are never dropped by a newer
/// one (a batch is only dropped after 4096 unread ones).
#[no_mangle]
pub unsafe extern "C" fn orr_events_poll(host: *mut OrrHost, buf: *mut u8, cap: usize, written: *mut usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let written = written.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "written is null".into() })?;
        *written = 0;
        if buf.is_null() && cap != 0 {
            return fail(ORR_ERR_NULL, "output buffer is null but its capacity is not 0");
        }
        let mut inner = lock(h);
        inner.pump()?;
        let Some(batch) = inner.events.front() else { return Ok(ORR_NO_FRAME) };
        *written = batch.len();
        if !write_out(buf, cap, batch)? {
            return fail(ORR_ERR_BUFFER, format!("buffer too small: {} bytes needed, {cap} given", batch.len()));
        }
        inner.events.pop_front();
        Ok(ORR_OK)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sets the held input of `player` (the bytes of the game's input type, laid
/// out as the schema's `input` says; exactly `input.size` bytes).
#[no_mangle]
pub unsafe extern "C" fn orr_set_input(host: *mut OrrHost, player: u8, bytes: *const u8, len: usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        if bytes.is_null() {
            return fail(ORR_ERR_NULL, "input bytes are null");
        }
        let mut inner = lock(h);
        if let Backend::Client(_) = inner.backend {
            let _ = inner.pump();
            let Backend::Client(c) = &mut inner.backend else { unreachable!() };
            c.set_input(player, std::slice::from_raw_parts(bytes, len)).map_err(|(code, msg)| Fail { code, msg })?;
            return Ok(ORR_OK);
        }
        if len != inner.input_size {
            return fail(ORR_ERR_ARG, format!("input must be {} bytes (the schema's input.size), got {len}", inner.input_size));
        }
        if player >= inner.player_count {
            return fail(ORR_ERR_ARG, format!("player {player} does not exist (the session has {})", inner.player_count));
        }
        let input = hex(std::slice::from_raw_parts(bytes, len));
        inner.call("sim.input", json!({"player": player, "input": input}))?;
        Ok(ORR_OK)
    })
}

/// Queues a command of the game's command type for the next tick.
#[no_mangle]
pub unsafe extern "C" fn orr_send_command(host: *mut OrrHost, player: u8, bytes: *const u8, len: usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        if bytes.is_null() {
            return fail(ORR_ERR_NULL, "command bytes are null");
        }
        let mut inner = lock(h);
        if let Backend::Client(_) = inner.backend {
            let _ = inner.pump();
            let Backend::Client(c) = &mut inner.backend else { unreachable!() };
            c.send_command(player, std::slice::from_raw_parts(bytes, len)).map_err(|(code, msg)| Fail { code, msg })?;
            return Ok(ORR_OK);
        }
        if player >= inner.player_count {
            return fail(ORR_ERR_ARG, format!("player {player} does not exist (the session has {})", inner.player_count));
        }
        let command = hex(std::slice::from_raw_parts(bytes, len));
        inner.call("sim.command", json!({"player": player, "command": command}))?;
        Ok(ORR_OK)
    })
}

/// Timeline control: `op` is one of `ORR_CTL_*`, `arg` its argument (see the header).
#[no_mangle]
pub unsafe extern "C" fn orr_control(host: *mut OrrHost, op: c_int, arg: i64) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let mut inner = lock(h);
        match op {
            ORR_CTL_PLAY => inner.call("sim.play", J::Null)?,
            ORR_CTL_PAUSE => inner.call("sim.pause", J::Null)?,
            ORR_CTL_STEP => {
                if arg < 1 {
                    return fail(ORR_ERR_ARG, "ORR_CTL_STEP needs arg >= 1 (ticks to run)");
                }
                inner.call("sim.step", json!({"n": arg}))?
            }
            ORR_CTL_SEEK => {
                if arg < 0 {
                    return fail(ORR_ERR_ARG, "ORR_CTL_SEEK needs arg >= 0 (a tick)");
                }
                inner.call("sim.seek", json!({"tick": arg}))?
            }
            ORR_CTL_SPEED => {
                if arg < 1 {
                    return fail(ORR_ERR_ARG, "ORR_CTL_SPEED needs arg >= 1 (permille, 1000 = 1x)");
                }
                inner.call("sim.speed", json!({"permille": arg}))?
            }
            ORR_CTL_BRANCH => inner.call("sim.branch", J::Null)?,
            ORR_CTL_RESTART => {
                inner.call("sim.stop", J::Null)?;
                inner.view.clear();
                inner.start_session()?;
                J::Null
            }
            other => return fail(ORR_ERR_ARG, format!("unknown control op {other}")),
        };
        Ok(ORR_OK)
    })
}

/// Calls any ERP method in process. `request_json` is
/// `{"method":"world.query","params":{...}}` (params optional). The answer
/// is written to `out` as NUL-terminated JSON: `{"result":...}` on success,
/// `{"error":{"code":...,"message":...}}` if the host refused (return value
/// `ORR_ERR_RPC`). `*needed` is the size of the answer including the NUL;
/// if `cap` is smaller, nothing is written and `ORR_ERR_BUFFER` is returned
/// (the call was made, so do not repeat a call that changes things).
#[no_mangle]
pub unsafe extern "C" fn orr_erp_call(host: *mut OrrHost, request_json: *const c_char, out: *mut c_char, cap: usize, needed: *mut usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let needed = needed.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "needed is null".into() })?;
        *needed = 0;
        if request_json.is_null() {
            return fail(ORR_ERR_NULL, "request_json is null");
        }
        let text = CStr::from_ptr(request_json).to_str().map_err(|_| Fail { code: ORR_ERR_ARG, msg: "request_json is not UTF-8".into() })?;
        let req: J = serde_json::from_str(text).map_err(|e| Fail { code: ORR_ERR_ARG, msg: format!("request_json is not JSON: {e}") })?;
        let method = req
            .get("method")
            .and_then(J::as_str)
            .ok_or(Fail { code: ORR_ERR_ARG, msg: "request_json needs a \"method\" text".into() })?
            .to_string();
        if method.starts_with("watch.") || method == "auth" {
            return fail(ORR_ERR_ARG, "watch.* and auth are not available through orr_erp_call (the view stream has its own calls)");
        }
        let params = req.get("params").cloned().unwrap_or(J::Null);
        let mut inner = lock(h);
        inner.pump()?;
        let (answer, code) = match inner.local()?.client.call(&method, params) {
            Ok(v) => (json!({"result": v}), ORR_OK),
            Err(ClientError::Rpc(e)) => (json!({"error": e.to_json()}), ORR_ERR_RPC),
            Err(e) => return fail(ORR_ERR_HOST, format!("{method}: {e}")),
        };
        let _ = inner.pump();
        let answer = answer.to_string();
        *needed = answer.len() + 1;
        if code == ORR_ERR_RPC {
            set_error(&format!("{method}: {answer}"));
        }
        let mut bytes = answer.into_bytes();
        bytes.push(0);
        if !write_out(out.cast(), cap, &bytes)? {
            if !out.is_null() && cap > 0 {
                *out = 0;
            }
            return fail(ORR_ERR_BUFFER, format!("buffer too small: {} bytes needed, {cap} given", *needed));
        }
        Ok(code)
    })
}

#[cfg(test)]
mod frame_mailbox_tests {
    use super::*;
    use orr_viewstream::{
        EntityRecord, EntityRecord3, EventRecord, Pose3, ViewFrame, ViewFrame3, STATE_CANCELED,
        STATE_PREDICTED, STATE_VERIFIED,
    };

    /// Keep the real host alive, but make incoming presentation data entirely
    /// fixture-owned: empty polls cannot race the host's initial publication.
    struct EmptyTransport;

    #[test]
    fn frame3_reset_survives_replacement_and_copy_probes_until_pointer_take() {
        let frame3 = |tick, flags| {
            ViewFrame3 {
                tick,
                flags,
                verified_tick: tick,
                seq: tick,
                rollback: None,
                entities: vec![EntityRecord3 {
                    id: 7,
                    kind: 1,
                    shape: orr_viewstream::SHAPE3_SPHERE,
                    mode: orr_viewstream::MODE_PREDICTION,
                    size: [1.0, 0.0, 0.0],
                    rgba: [1, 2, 3, 255],
                    roughness: 200,
                    metallic: 0,
                    style_flags: 0,
                    prev: Pose3::IDENTITY,
                    cur: Pose3 {
                        pos: [1.0, 2.0, 3.0],
                        rot: [0.0, 0.0, 0.0, 1.0],
                    },
                }],
                props: vec![0; 4],
            }
            .encode()
        };
        let mut host = open_yard3d_host(2, std::ptr::null()).unwrap();
        {
            let mut inner = lock(&host);
            inner.local().unwrap().client = ErpClient::with_transport(Box::new(EmptyTransport));
            inner.view = FrameMailbox::default();
            inner.events.clear();
            inner.events.push_back(
                EventBatch {
                    events: vec![event(1, STATE_PREDICTED)],
                }
                .encode(),
            );
            let Inner { view, events, .. } = &mut *inner;
            view.push(frame3(2, FLAG_EVENTS_RESET), events).unwrap();
            assert!(events.is_empty());
            view.push(frame3(3, 0), events).unwrap();
        }
        let mut written = 0;
        assert_eq!(
            unsafe { orr_view_poll(&mut host, std::ptr::null_mut(), 0, &mut written) },
            ORR_ERR_BUFFER
        );
        let mut short = [99u8; 1];
        assert_eq!(
            unsafe { orr_view_poll(&mut host, short.as_mut_ptr(), short.len(), &mut written) },
            ORR_ERR_BUFFER
        );
        assert_eq!(short, [99]);
        assert!(lock(&host).view.reset_pending);
        let mut data = std::ptr::null();
        let mut len = 0;
        assert_eq!(
            unsafe { orr_view_poll_ptr(&mut host, &mut data, &mut len) },
            ORR_OK
        );
        let decoded = ViewFrame3::decode(unsafe { std::slice::from_raw_parts(data, len) }).unwrap();
        assert_eq!(decoded.tick, 3);
        assert!(decoded.has(FLAG_EVENTS_RESET));
        assert!(decoded.has(FLAG_DISCONTINUITY));
        assert_eq!(decoded.entities[0].prev, decoded.entities[0].cur);
        let inner = lock(&host);
        assert!(!inner.view.reset_pending);
        assert_eq!(inner.view.reset_floor_tick, Some(3));
    }

    impl orr_remote::Transport for EmptyTransport {
        fn send(&mut self, _req: orr_remote::Request) -> Result<(), ClientError> {
            panic!("the polling fixture does not send requests");
        }

        fn recv(&mut self, _timeout: Duration) -> Result<Option<orr_remote::Incoming>, ClientError> {
            Ok(None)
        }
    }

    #[test]
    fn poll_buffer_validation_precedes_data_availability() {
        let mut host = open_host(std::ptr::null(), std::ptr::null()).unwrap();
        {
            let mut inner = lock(&host);
            inner.local().unwrap().client = ErpClient::with_transport(Box::new(EmptyTransport));
            inner.view = FrameMailbox::default();
            inner.events.clear();
        }
        type Poll = unsafe extern "C" fn(*mut OrrHost, *mut u8, usize, *mut usize) -> c_int;
        let polls: [Poll; 2] = [orr_view_poll, orr_events_poll];
        for poll in polls {
            let mut written = 99;
            assert_eq!(unsafe { poll(&mut host, std::ptr::null_mut(), 5, &mut written) }, ORR_ERR_NULL);
            assert_eq!(written, 0);
            assert_eq!(unsafe { poll(&mut host, std::ptr::null_mut(), 0, &mut written) }, ORR_NO_FRAME);
            assert_eq!(written, 0);
            let mut untouched = [17u8; 1];
            assert_eq!(unsafe { poll(&mut host, untouched.as_mut_ptr(), 1, &mut written) }, ORR_NO_FRAME);
            assert_eq!(untouched, [17]);
        }

        let frame = frame(10, FLAG_EVENTS_RESET | FLAG_DISCONTINUITY, [1.0; 3], [1.0; 3]);
        {
            let mut inner = lock(&host);
            let Inner { view, events, .. } = &mut *inner;
            view.push(frame.clone(), events).unwrap();
        }
        let mut written = 99;
        assert_eq!(unsafe { orr_view_poll(&mut host, std::ptr::null_mut(), 5, &mut written) }, ORR_ERR_NULL);
        assert_eq!(written, 0);
        assert_eq!(unsafe { orr_view_poll(&mut host, std::ptr::null_mut(), 0, &mut written) }, ORR_ERR_BUFFER);
        assert_eq!(written, frame.len());
        assert!(lock(&host).view.reset_pending, "invalid buffers and size probes must retain the reset");
        let mut copied = vec![0; written];
        assert_eq!(unsafe { orr_view_poll(&mut host, copied.as_mut_ptr(), copied.len(), &mut written) }, ORR_OK);
        assert_eq!(copied, frame);
        assert!(!lock(&host).view.reset_pending);

        let batch = EventBatch { events: vec![event(11, STATE_VERIFIED)] }.encode();
        lock(&host).events.push_back(batch.clone());
        assert_eq!(unsafe { orr_events_poll(&mut host, std::ptr::null_mut(), 5, &mut written) }, ORR_ERR_NULL);
        assert_eq!(written, 0);
        assert_eq!(unsafe { orr_events_poll(&mut host, std::ptr::null_mut(), 0, &mut written) }, ORR_ERR_BUFFER);
        assert_eq!(written, batch.len());
        assert_eq!(lock(&host).events.front(), Some(&batch));
        copied.resize(written, 0);
        assert_eq!(unsafe { orr_events_poll(&mut host, copied.as_mut_ptr(), copied.len(), &mut written) }, ORR_OK);
        assert_eq!(copied, batch);
        assert!(lock(&host).events.is_empty());
    }

    fn frame(tick: u64, flags: u8, prev: [f32; 3], cur: [f32; 3]) -> Vec<u8> {
        ViewFrame {
            flags,
            tick,
            verified_tick: tick,
            seq: tick,
            rollback: None,
            entities: vec![EntityRecord {
                id: 7,
                kind: 1,
                shape: 0,
                mode: 0,
                size: 1.0,
                half_y: 0.0,
                rgba: [255; 4],
                prev,
                cur,
            }],
            props: Vec::new(),
        }
        .encode()
    }

    fn event(tick: u64, state: u8) -> EventRecord {
        EventRecord { tick, system: 1, seq: 1, state, event_type: 1, payload: vec![1, 2, 3] }
    }

    #[test]
    fn unread_reset_survives_newer_frames_and_small_copy_buffers() {
        let mut mailbox = FrameMailbox::default();
        let mut events = VecDeque::new();
        queue_event_batch(&mailbox, &mut events, vec![1]).unwrap();
        assert_eq!(events.len(), 1);

        // A reset clears pre-cut batches. A subsequent status/events poll pumps a
        // newer frame before the caller takes the baseline; that frame must still
        // be an events-reset discontinuity with prev == cur.
        mailbox
            .push(frame(10, FLAG_EVENTS_RESET, [0.0, 0.0, 0.0], [10.0, 0.0, 1.0]), &mut events)
            .unwrap();
        assert!(events.is_empty());
        assert!(mailbox.reset_pending);
        assert_eq!(mailbox.reset_floor_tick, None, "the late-event cutoff starts only after acknowledgement");
        queue_event_batch(&mailbox, &mut events, vec![2]).unwrap();
        assert!(events.is_empty(), "post-cut events wait until the baseline is acknowledged");

        mailbox.push(frame(11, 0, [10.0, 0.0, 1.0], [11.0, 0.0, 2.0]), &mut events).unwrap();
        let mut too_small = [0u8; 1];
        let latest = mailbox.latest.as_ref().unwrap();
        assert!(!unsafe { write_out(too_small.as_mut_ptr(), too_small.len(), latest) }.unwrap());
        assert!(mailbox.reset_pending, "a short-buffer poll must not acknowledge the baseline");
        assert_eq!(mailbox.reset_floor_tick, None);
        queue_event_batch(&mailbox, &mut events, vec![3]).unwrap();
        assert!(events.is_empty());

        mailbox.push(frame(12, 0, [11.0, 0.0, 2.0], [12.0, 0.0, 3.0]), &mut events).unwrap();
        let coalesced = ViewFrame::decode(mailbox.latest.as_ref().unwrap()).unwrap();
        assert_eq!(coalesced.tick, 12);
        assert!(coalesced.has(FLAG_EVENTS_RESET));
        assert!(coalesced.has(FLAG_DISCONTINUITY));
        assert_eq!(coalesced.entities[0].prev, coalesced.entities[0].cur);

        let mut exact = vec![0u8; mailbox.latest.as_ref().unwrap().len()];
        assert!(unsafe { write_out(exact.as_mut_ptr(), exact.len(), mailbox.latest.as_ref().unwrap()) }.unwrap());
        mailbox.acknowledge_copy();
        assert!(!mailbox.reset_pending);
        assert_eq!(mailbox.reset_floor_tick, Some(12));
        mailbox.push(frame(13, 0, [12.0, 0.0, 3.0], [13.0, 0.0, 4.0]), &mut events).unwrap();
        let following = ViewFrame::decode(mailbox.latest.as_ref().unwrap()).unwrap();
        assert!(!following.has(FLAG_EVENTS_RESET | FLAG_DISCONTINUITY));
        let post_reset = EventBatch { events: vec![event(13, 1)] }.encode();
        queue_event_batch(&mailbox, &mut events, post_reset).unwrap();
        assert_eq!(EventBatch::decode(events.front().unwrap()).unwrap().events[0].tick, 13);
    }

    #[test]
    fn pointer_take_acknowledges_the_reset_baseline() {
        let mut mailbox = FrameMailbox::default();
        let mut events = VecDeque::new();
        mailbox
            .push(frame(2, FLAG_EVENTS_RESET, [0.0; 3], [2.0, 0.0, 0.0]), &mut events)
            .unwrap();
        let taken = mailbox.take().expect("reset baseline");
        assert!(ViewFrame::decode(&taken).unwrap().has(FLAG_EVENTS_RESET));
        assert!(!mailbox.reset_pending);
        assert_eq!(mailbox.reset_floor_tick, Some(2));
    }

    #[test]
    fn late_old_tick_events_are_filtered_until_a_new_discontinuity() {
        let mut mailbox = FrameMailbox::default();
        let mut events = VecDeque::new();
        mailbox
            .push(frame(10, FLAG_EVENTS_RESET, [0.0; 3], [10.0, 0.0, 0.0]), &mut events)
            .unwrap();
        mailbox.acknowledge_copy();
        assert_eq!(mailbox.reset_floor_tick, Some(10));

        let batch = EventBatch {
            events: vec![
                event(9, STATE_PREDICTED),
                event(10, STATE_PREDICTED),
                event(10, STATE_CANCELED),
                event(11, STATE_VERIFIED),
            ],
        }
        .encode();
        queue_event_batch(&mailbox, &mut events, batch).unwrap();
        let accepted = EventBatch::decode(events.front().unwrap()).unwrap();
        assert_eq!(accepted.events.len(), 1);
        assert_eq!(accepted.events[0].tick, 11);

        mailbox
            .push(frame(3, FLAG_DISCONTINUITY, [10.0, 0.0, 0.0], [3.0, 0.0, 0.0]), &mut events)
            .unwrap();
        assert_eq!(mailbox.reset_floor_tick, None);
        let new_timeline = EventBatch { events: vec![event(1, STATE_PREDICTED)] }.encode();
        queue_event_batch(&mailbox, &mut events, new_timeline).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(EventBatch::decode(events.back().unwrap()).unwrap().events[0].tick, 1);
    }
}
