//! The same stream through the C ABI (`crates/orr_ffi/include/orrery.h`), the way a C, C++ or C#
//! host gets it: the shared library is loaded at run time (`dlopen` through `libloading`) and
//! its C functions are called through `extern "C"` declarations that mirror the header. This
//! crate does not link `orr_ffi` as a Rust dependency, because that would link the game.

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use libloading::Library;

use crate::source::{Control, Incoming, Source};

// ---- orrery.h ----
const ABI_VERSION: u32 = 1;
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
}

impl FfiSource {
    /// Loads the library and opens a host (paused, on `scene` or the library's demo scene).
    pub fn open(lib_path: &Path, scene: Option<&Path>) -> Result<FfiSource, String> {
        unsafe {
            let lib = Library::new(lib_path).map_err(|e| format!("cannot load {}: {e} (build it with `cargo build -p orr_ffi`, or pass --lib)", lib_path.display()))?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    *lib.get::<$ty>(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: missing {}: {e}", lib_path.display(), $name))?
                };
            }
            let abi_version: AbiVersion = sym!("orr_abi_version", AbiVersion);
            if abi_version() != ABI_VERSION {
                return Err(format!("{}: ABI version {} (this viewer speaks {ABI_VERSION})", lib_path.display(), abi_version()));
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
            let cfg = OrrHostConfig { struct_size: std::mem::size_of::<OrrHostConfig>() as u32, flags: 0, listen_port: 0 };
            let scene_c = match scene {
                Some(p) => Some(CString::new(p.to_string_lossy().as_bytes()).map_err(|_| "scene path contains a NUL".to_string())?),
                None => None,
            };
            let host = open(scene_c.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()), &cfg);
            if host.is_null() {
                return Err(format!("orr_host_open failed: {}", err()));
            }
            let need = schema_json(host, std::ptr::null_mut(), 0);
            let mut text = vec![0 as c_char; need.max(1)];
            schema_json(host, text.as_mut_ptr(), text.len());
            let schema = CStr::from_ptr(text.as_ptr()).to_string_lossy().into_owned();
            Ok(FfiSource { _lib: lib, host, close, last_error, view_poll, events_poll, set_input, control, schema, path: lib_path.to_path_buf(), buf: vec![0; 1 << 16] })
        }
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
        format!("C ABI {}", self.path.display())
    }
}
