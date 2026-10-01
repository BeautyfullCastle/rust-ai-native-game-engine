//! The same stream through the C ABI (`crates/orr_ffi/include/orrery.h`), the way a C, C++ or C#
//! host gets it: the shared library is loaded at run time (`dlopen` through `libloading`) and
//! its C functions are called through `extern "C"` declarations that mirror the header. This
//! crate does not link `orr_ffi` as a Rust dependency, because that would link the game.

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use libloading::Library;

use crate::source::{Control, Incoming, NetState, NetStatus, Source};

// ---- orrery.h ----
/// The ABI this viewer speaks (2 added client sessions; a local host works with 1 as well).
const ABI_VERSION: u32 = 2;
const ORR_OK: c_int = 0;
const ORR_NO_FRAME: c_int = 1;
const ORR_ERR_BUFFER: c_int = -3;
const ORR_CTL_PLAY: c_int = 0;
const ORR_CTL_PAUSE: c_int = 1;
const ORR_CTL_STEP: c_int = 2;

#[repr(C)]
struct OrrHostConfig {
    struct_size: u32,
    flags: u32,
    listen_port: u32,
}

#[repr(C)]
struct OrrClientConfig {
    struct_size: u32,
    flags: u32,
    transport: u32,
    slot: i32,
    room: u64,
    sim_seed: u64,
    sim_latency_ms: u32,
    sim_jitter_ms: u32,
    sim_loss_permille: u32,
    connect_timeout_ms: u32,
    server: *const c_char,
    fingerprint: *const c_char,
    desync_dir: *const c_char,
}

#[repr(C)]
#[derive(Default)]
struct OrrSessionStatus {
    struct_size: u32,
    mode: u32,
    state: u32,
    flags: u32,
    slot: u32,
    player_count: u32,
    rtt_ms: u32,
    input_delay: u32,
    head_tick: u64,
    verified_tick: u64,
    rollbacks: u64,
    resim_ticks: u64,
    last_rollback_from: u64,
    last_rollback_to: u64,
    desyncs: u64,
    stall_episodes: u64,
    stalled_ms: u64,
    repeated_inputs: u64,
}

const ORR_CLIENT_INSECURE: u32 = 2;
const ORR_TRANSPORT_QUIC: u32 = 0;
const ORR_TRANSPORT_WS: u32 = 1;

#[repr(C)]
struct OrrHost {
    _opaque: [u8; 0],
}

type AbiVersion = unsafe extern "C" fn() -> u32;
type LastError = unsafe extern "C" fn() -> *const c_char;
type HostOpen = unsafe extern "C" fn(*const c_char, *const OrrHostConfig) -> *mut OrrHost;
type HostClose = unsafe extern "C" fn(*mut OrrHost);
type SchemaJson = unsafe extern "C" fn(*mut OrrHost, *mut c_char, usize) -> usize;
type Poll = unsafe extern "C" fn(*mut OrrHost, *mut u8, usize, *mut usize) -> c_int;
type SetInput = unsafe extern "C" fn(*mut OrrHost, u8, *const u8, usize) -> c_int;
type ControlFn = unsafe extern "C" fn(*mut OrrHost, c_int, i64) -> c_int;
type ClientOpen = unsafe extern "C" fn(*const OrrClientConfig) -> *mut OrrHost;
type SessionStatus = unsafe extern "C" fn(*mut OrrHost, *mut OrrSessionStatus) -> c_int;
type ConfirmedChecksum = unsafe extern "C" fn(*mut OrrHost, u64, *mut u64, *mut u64) -> c_int;

/// How to join a game on a server (`orr_client_open`).
#[derive(Clone, Debug, Default)]
pub struct ClientOpts {
    /// `host:port`.
    pub server: String,
    /// QUIC: the certificate's SHA-256 as hex (the server prints it).
    pub fingerprint: Option<String>,
    /// QUIC: accept any certificate (development only).
    pub insecure: bool,
    /// WebSocket instead of QUIC.
    pub ws: bool,
    pub room: u64,
    /// The slot to ask for (`None`: any).
    pub slot: Option<u8>,
    pub sim_latency_ms: u32,
    pub sim_jitter_ms: u32,
    /// Simulated loss in thousandths.
    pub sim_loss_permille: u32,
    pub sim_seed: u64,
    pub connect_timeout: Duration,
}

/// The library file name on this platform.
pub fn lib_file_name() -> &'static str {
    if cfg!(windows) {
        "orr_ffi.dll"
    } else if cfg!(target_os = "macos") {
        "liborr_ffi.dylib"
    } else {
        "liborr_ffi.so"
    }
}

/// Where to look for the library when no path is given: `ORR_FFI_LIB`, next to this executable
/// (`target/<profile>/`), else the bare name for the system loader.
pub fn default_lib_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ORR_FFI_LIB") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        for d in [Some(dir.as_path()), dir.parent()].into_iter().flatten() {
            let p = d.join(lib_file_name());
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from(lib_file_name())
}

pub struct FfiSource {
    // Keeps the library mapped for as long as the function pointers below are used.
    _lib: Library,
    host: *mut OrrHost,
    close: HostClose,
    last_error: LastError,
    view_poll: Poll,
    events_poll: Poll,
    set_input: SetInput,
    control: ControlFn,
    schema: String,
    path: PathBuf,
    buf: Vec<u8>,
    /// Present for a client session.
    client: Option<ClientFns>,
}

struct ClientFns {
    status: SessionStatus,
    checksum: ConfirmedChecksum,
}

impl FfiSource {
    /// Loads the library and opens a host (paused, on `scene` or the library's demo scene).
    pub fn open(lib_path: &Path, scene: Option<&Path>) -> Result<FfiSource, String> {
        Self::open_with(lib_path, Some(scene), None)
    }

    /// Loads the library and joins a game on a server as a client; returns when the room has
    /// started (or after `opts.connect_timeout` with an error).
    pub fn open_client(lib_path: &Path, opts: &ClientOpts) -> Result<FfiSource, String> {
        Self::open_with(lib_path, None, Some(opts))
    }

    fn open_with(lib_path: &Path, scene: Option<Option<&Path>>, client: Option<&ClientOpts>) -> Result<FfiSource, String> {
        unsafe {
            let lib = Library::new(lib_path).map_err(|e| format!("cannot load {}: {e} (build it with `cargo build -p orr_ffi`, or pass --lib)", lib_path.display()))?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    *lib.get::<$ty>(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: missing {}: {e}", lib_path.display(), $name))?
                };
            }
            let abi_version: AbiVersion = sym!("orr_abi_version", AbiVersion);
            let needs = if client.is_some() { ABI_VERSION } else { 1 };
            if abi_version() < needs {
                return Err(format!("{}: ABI version {} (this viewer needs at least {needs})", lib_path.display(), abi_version()));
            }
            let last_error: LastError = sym!("orr_last_error", LastError);
            let open: HostOpen = sym!("orr_host_open", HostOpen);
            let close: HostClose = sym!("orr_host_close", HostClose);
            let schema_json: SchemaJson = sym!("orr_schema_json", SchemaJson);
            let view_poll: Poll = sym!("orr_view_poll", Poll);
            let events_poll: Poll = sym!("orr_events_poll", Poll);
            let set_input: SetInput = sym!("orr_set_input", SetInput);
            let control: ControlFn = sym!("orr_control", ControlFn);

            let err = || CStr::from_ptr(last_error()).to_string_lossy().into_owned();
            let mut fns = None;
            let host = match client {
                None => {
                    let cfg = OrrHostConfig { struct_size: std::mem::size_of::<OrrHostConfig>() as u32, flags: 0, listen_port: 0 };
                    let scene_c = match scene.flatten() {
                        Some(p) => Some(CString::new(p.to_string_lossy().as_bytes()).map_err(|_| "scene path contains a NUL".to_string())?),
                        None => None,
                    };
                    let host = open(scene_c.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()), &cfg);
                    if host.is_null() {
                        return Err(format!("orr_host_open failed: {}", err()));
                    }
                    host
                }
                Some(o) => {
                    let client_open: ClientOpen = sym!("orr_client_open", ClientOpen);
                    let status: SessionStatus = sym!("orr_session_status", SessionStatus);
                    let checksum: ConfirmedChecksum = sym!("orr_confirmed_checksum", ConfirmedChecksum);
                    let server = CString::new(o.server.as_str()).map_err(|_| "server address contains a NUL".to_string())?;
                    let fingerprint = match &o.fingerprint {
                        Some(f) => Some(CString::new(f.as_str()).map_err(|_| "fingerprint contains a NUL".to_string())?),
                        None => None,
                    };
                    let cfg = OrrClientConfig {
                        struct_size: std::mem::size_of::<OrrClientConfig>() as u32,
                        flags: if o.insecure { ORR_CLIENT_INSECURE } else { 0 },
                        transport: if o.ws { ORR_TRANSPORT_WS } else { ORR_TRANSPORT_QUIC },
                        slot: o.slot.map_or(-1, i32::from),
                        room: o.room,
                        sim_seed: o.sim_seed,
                        sim_latency_ms: o.sim_latency_ms,
                        sim_jitter_ms: o.sim_jitter_ms,
                        sim_loss_permille: o.sim_loss_permille,
                        connect_timeout_ms: u32::try_from(o.connect_timeout.as_millis()).unwrap_or(u32::MAX),
                        server: server.as_ptr(),
                        fingerprint: fingerprint.as_ref().map_or(std::ptr::null(), |f| f.as_ptr()),
                        desync_dir: std::ptr::null(),
                    };
                    // Not waiting inside the library: the viewer sees the joining state itself.
                    let host = client_open(&cfg);
                    if host.is_null() {
                        return Err(format!("orr_client_open failed: {}", err()));
                    }
                    fns = Some(ClientFns { status, checksum });
                    host
                }
            };
            let mut src = FfiSource {
                _lib: lib,
                host,
                close,
                last_error,
                view_poll,
                events_poll,
                set_input,
                control,
                schema: String::new(),
                path: lib_path.to_path_buf(),
                buf: vec![0; 1 << 16],
                client: fns,
            };
            if let Some(o) = client {
                // Wait for the room to start: the schema is known only then.
                let end = Instant::now() + o.connect_timeout + Duration::from_secs(10);
                loop {
                    let st = src.net_status().ok_or("no session status")?;
                    match st.state {
                        NetState::Playing => break,
                        NetState::Failed | NetState::Disconnected => {
                            // The reason comes back from a view call (the status has no text).
                            let why = src.poll(false).err().unwrap_or_else(|| src.last_error_text());
                            return Err(format!("could not join {}: {why}", o.server));
                        }
                        NetState::Connecting => {}
                    }
                    if Instant::now() > end {
                        return Err(format!("timed out joining {} (is the room full or still waiting for players?)", o.server));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let need = schema_json(src.host, std::ptr::null_mut(), 0);
            let mut text = vec![0 as c_char; need.max(1)];
            schema_json(src.host, text.as_mut_ptr(), text.len());
            src.schema = CStr::from_ptr(text.as_ptr()).to_string_lossy().into_owned();
            Ok(src)
        }
    }

    fn last_error_text(&self) -> String {
        unsafe { CStr::from_ptr((self.last_error)()) }.to_string_lossy().into_owned()
    }

    fn error(&self, what: &str, rc: c_int) -> String {
        let text = unsafe { CStr::from_ptr((self.last_error)()) }.to_string_lossy().into_owned();
        format!("{what} returned {rc}: {text}")
    }

    /// Polls one of the two queues into `self.buf`; grows the buffer when it was too small.
    fn poll(&mut self, events: bool) -> Result<Option<Vec<u8>>, String> {
        let f = if events { self.events_poll } else { self.view_poll };
        for _ in 0..2 {
            let mut written = 0usize;
            let rc = unsafe { f(self.host, self.buf.as_mut_ptr(), self.buf.len(), &mut written) };
            match rc {
                ORR_OK => return Ok(Some(self.buf[..written].to_vec())),
                ORR_NO_FRAME => return Ok(None),
                ORR_ERR_BUFFER => self.buf.resize(written, 0), // nothing was consumed: retry with the size it asked for
                other => return Err(self.error(if events { "orr_events_poll" } else { "orr_view_poll" }, other)),
            }
        }
        Err("the poll buffer stayed too small".into())
    }
}

impl Drop for FfiSource {
    fn drop(&mut self) {
        unsafe { (self.close)(self.host) };
    }
}

impl Source for FfiSource {
    fn schema_text(&self) -> &str {
        &self.schema
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, String> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(f) = self.poll(false)? {
                return Ok(Some(Incoming::Frame(f)));
            }
            if let Some(e) = self.poll(true)? {
                return Ok(Some(Incoming::Events(e)));
            }
            if Instant::now() >= end {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), String> {
        match unsafe { (self.set_input)(self.host, player, bytes.as_ptr(), bytes.len()) } {
            ORR_OK => Ok(()),
            rc => Err(self.error("orr_set_input", rc)),
        }
    }

    fn control(&mut self, c: Control) -> Result<(), String> {
        if self.client.is_some() {
            return Err("timeline controls are not available in a network game".into());
        }
        let (op, arg) = match c {
            Control::Play => (ORR_CTL_PLAY, 0),
            Control::Pause => (ORR_CTL_PAUSE, 0),
            Control::Step(n) => (ORR_CTL_STEP, i64::from(n)),
        };
        match unsafe { (self.control)(self.host, op, arg) } {
            ORR_OK => Ok(()),
            rc => Err(self.error("orr_control", rc)),
        }
    }

    fn describe(&self) -> String {
        format!("C ABI {}{}", self.path.display(), if self.client.is_some() { " (network client)" } else { "" })
    }

    fn net_status(&mut self) -> Option<NetStatus> {
        let f = self.client.as_ref()?;
        let mut s = OrrSessionStatus { struct_size: std::mem::size_of::<OrrSessionStatus>() as u32, ..OrrSessionStatus::default() };
        if unsafe { (f.status)(self.host, &mut s) } != ORR_OK {
            return None;
        }
        Some(NetStatus {
            state: match s.state {
                0 => NetState::Connecting,
                1 => NetState::Playing,
                2 => NetState::Disconnected,
                _ => NetState::Failed,
            },
            slot: s.slot,
            players: s.player_count,
            rtt_ms: s.rtt_ms,
            input_delay: s.input_delay,
            head_tick: s.head_tick,
            verified_tick: s.verified_tick,
            rollbacks: s.rollbacks,
            resim_ticks: s.resim_ticks,
            last_rollback: (s.last_rollback_from, s.last_rollback_to),
            desyncs: s.desyncs,
            stall_episodes: s.stall_episodes,
        })
    }

    fn confirmed_checksum(&mut self, tick: u64) -> Option<u64> {
        let f = self.client.as_ref()?;
        let (mut found, mut sum) = (0u64, 0u64);
        (unsafe { (f.checksum)(self.host, tick, &mut found, &mut sum) } == ORR_OK && found == tick).then_some(sum)
    }
}
