//! `orr_ffi`: Orrery's C ABI. A view written in C, C++, C#, GDScript (through
//! GDExtension) or any language that loads a C library can host a simulation,
//! read what to draw as a stream of bytes and feed it inputs, without linking
//! a Rust type or reading a Rust `Frame` (design doc decision 12).
//!
//! The header is `include/orrery.h` (hand-written; keep it in step with this
//! file). The byte formats are in `docs/view-stream.md`.
//!
//! # The game is chosen at compile time
//!
//! A Rust game is generic code, so one build of this library hosts one game.
//! This crate ships the physics demo (`PhysGame` of `orr_sample`). To host
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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CStr, CString};
use std::net::SocketAddr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use orr_remote::{Auth, Caps, ClientError, ErpClient, LocalHost, ServerConfig};
use orr_viewstream::{message_type, MSG_EVENTS, MSG_FRAME};
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

/// Version of this C ABI (bumped when a function changes incompatibly).
pub const ORR_ABI_VERSION: u32 = 1;

/// Settings of [`orr_host_open`]. Zero the struct, then set `struct_size`.
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

const DEMO_SCENE: &str = include_str!("../../../scenes/physics_demo.scene.yaml");
const CLIENT_NAME: &str = "ffi";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Event batches kept for a caller that does not poll; the oldest are dropped above this.
const MAX_QUEUED_EVENT_BATCHES: usize = 4096;

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

struct Inner {
    // Field order is drop order: the connection goes before the host thread.
    client: ErpClient,
    host: LocalHost,
    schema_json: String,
    url: Option<String>,
    input_size: usize,
    player_count: u8,
    run_on_start: bool,
    /// The newest frame not read yet (a newer one replaces it).
    latest: Option<Vec<u8>>,
    /// The frame handed out by `orr_view_poll_ptr`.
    held: Vec<u8>,
    events: VecDeque<Vec<u8>>,
}

/// An opaque host handle (see the header).
pub struct OrrHost {
    inner: Mutex<Inner>,
}

impl Inner {
    /// Moves everything that arrived into the frame slot and the event queue.
    fn pump(&mut self) -> Result<(), Fail> {
        if !self.host.is_running() {
            let why = self.host.stopped_reason(Duration::from_millis(50)).unwrap_or_default();
            return fail(ORR_ERR_HOST, format!("the host thread has stopped: {why}"));
        }
        let alive = self.client.poll();
        while let Some(bytes) = self.client.frames.pop_front() {
            match message_type(&bytes) {
                Ok(MSG_FRAME) => self.latest = Some(bytes),
                Ok(MSG_EVENTS) => {
                    if self.events.len() >= MAX_QUEUED_EVENT_BATCHES {
                        self.events.pop_front();
                    }
                    self.events.push_back(bytes);
                }
                _ => {}
            }
        }
        // Notifications (watch.*) are not needed here.
        self.client.notifications.clear();
        self.client.local_frames.clear();
        match alive {
            Ok(_) => Ok(()),
            Err(e) => fail(ORR_ERR_HOST, format!("the host is gone: {e}")),
        }
    }

    fn call(&mut self, method: &str, params: J) -> Result<J, Fail> {
        self.pump()?;
        let r = self.client.call(method, params);
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
        let params = json!({"run": self.run_on_start});
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

fn open_host(scene_path: *const c_char, cfg: *const OrrHostConfig) -> Result<OrrHost, Fail> {
    let (flags, port) = match unsafe { cfg.as_ref() } {
        None => (0, 0),
        Some(c) => {
            if (c.struct_size as usize) < 3 * std::mem::size_of::<u32>() {
                return fail(ORR_ERR_ARG, "OrrHostConfig.struct_size is too small (set it to sizeof(OrrHostConfig))");
            }
            (c.flags, c.listen_port)
        }
    };
    if port > u32::from(u16::MAX) {
        return fail(ORR_ERR_ARG, "listen_port must be 0..=65535");
    }
    let (text, path) = if scene_path.is_null() {
        (DEMO_SCENE.to_string(), None)
    } else {
        let p = unsafe { CStr::from_ptr(scene_path) }.to_str().map_err(|_| Fail { code: ORR_ERR_ARG, msg: "scene path is not UTF-8".into() })?;
        let text = std::fs::read_to_string(p).map_err(|e| Fail { code: ORR_ERR_ARG, msg: format!("cannot read scene {p}: {e}") })?;
        (text, Some(PathBuf::from(p)))
    };
    let mut server = ServerConfig::new(Auth::DevNoAuth);
    server.listen = flags & ORR_HOST_LISTEN != 0;
    server.bind = SocketAddr::from(([127, 0, 0, 1], port as u16));
    // The game is chosen here, at compile time (see the crate docs).
    let host = orr_remote::sample::spawn_phys_host(text, path, server).map_err(|e| Fail { code: ORR_ERR_ARG, msg: e })?;
    let url = host.url().map(str::to_string);
    let transport = host.connector().connect(CLIENT_NAME, Caps::ALL).map_err(|e| Fail { code: ORR_ERR_HOST, msg: e.to_string() })?;
    let mut client = ErpClient::with_transport(Box::new(transport));
    client.call_timeout = CALL_TIMEOUT;
    let mut inner = Inner {
        client,
        host,
        schema_json: String::new(),
        url,
        input_size: 0,
        player_count: 0,
        run_on_start: flags & ORR_HOST_RUN != 0,
        latest: None,
        held: Vec::new(),
        events: VecDeque::new(),
    };
    inner.start_session()?;
    // Up to 1000 frames a second: the caller paces itself by polling.
    // (Not through `Inner::call`: it would discard the schema notification that comes with the response.)
    match inner.client.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1000, "source": "sim"})) {
        Ok(_) => {}
        Err(ClientError::Rpc(e)) => return fail(ORR_ERR_RPC, format!("watch.subscribe: {e}")),
        Err(e) => return fail(ORR_ERR_HOST, format!("watch.subscribe: {e}")),
    }
    let schema = inner
        .client
        .wait_notification("watch.viewstream.schema", Duration::from_secs(10))
        .map_err(|e| Fail { code: ORR_ERR_HOST, msg: e.to_string() })?
        .ok_or_else(|| Fail { code: ORR_ERR_HOST, msg: "the host sent no schema".into() })?;
    let schema = schema.get("params").cloned().unwrap_or(J::Null);
    inner.input_size = schema.pointer("/input/size").and_then(J::as_u64).unwrap_or(0) as usize;
    inner.player_count = schema.get("player_count").and_then(J::as_u64).unwrap_or(0) as u8;
    inner.schema_json = schema.to_string();
    inner.pump()?;
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

/// Opens a host of the compiled-in game on the scene at `scene_path` (a UTF-8
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
        let inner = lock(h);
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
        if let Some(url) = &inner.url {
            needed = write_text(buf, cap, url)?;
        }
        Ok(ORR_OK)
    });
    needed
}

/// Copies the newest view frame that was not read yet into `buf` and sets
/// `*written` to its size. Returns `ORR_OK`; `ORR_NO_FRAME` if nothing new
/// (`*written` = 0); `ORR_ERR_BUFFER` if `cap` is too small (`*written` = the
/// size needed, the frame stays for the next poll).
#[no_mangle]
pub unsafe extern "C" fn orr_view_poll(host: *mut OrrHost, buf: *mut u8, cap: usize, written: *mut usize) -> c_int {
    guard(|| {
        let h = host_ref(host)?;
        let written = written.as_mut().ok_or(Fail { code: ORR_ERR_NULL, msg: "written is null".into() })?;
        *written = 0;
        let mut inner = lock(h);
        inner.pump()?;
        let Some(frame) = inner.latest.as_ref() else { return Ok(ORR_NO_FRAME) };
        *written = frame.len();
        if !write_out(buf, cap, frame)? {
            return fail(ORR_ERR_BUFFER, format!("buffer too small: {} bytes needed, {cap} given", frame.len()));
        }
        inner.latest = None;
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
        let Some(frame) = inner.latest.take() else { return Ok(ORR_NO_FRAME) };
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
                inner.latest = None;
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
        let (answer, code) = match inner.client.call(&method, params) {
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
