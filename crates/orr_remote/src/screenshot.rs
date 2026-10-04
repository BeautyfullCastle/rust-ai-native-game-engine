//! Bounded, typed handoff between a local editor UI and its ERP host.
//!
//! The host owns the request/result slot. The editor owns the capture ticket
//! and the independent encoder permit, which remains held until a worker exits.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

const MAX_DIMENSION: u32 = 2048;
const MAX_PIXELS: u64 = 1_048_576;
const MAX_PNG_BYTES: usize = 4 << 20;
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1);

const TRACE_REQUESTS: usize = 2;
const TRACE_RECORDS_PER_REQUEST: u8 = 64;
const TRACE_MAX_OUTPUT_BYTES: usize = 32 << 10;
const TRACE_MARKER_RESERVE: usize = 256;
const TRACE_PREFIX: &str = "ORR_NATIVE_SCREENSHOT_TRACE";
static TRACE_ENABLED: OnceLock<bool> = OnceLock::new();
static TRACE_LEDGER: Mutex<TraceLedger> = Mutex::new(TraceLedger::new());

#[derive(Clone, Copy)]
struct TraceRequest {
    serial: u64,
    connection: u64,
    rpc_id: u64,
    started: Option<Instant>,
    records: u8,
    seen: [TraceFingerprint; TRACE_RECORDS_PER_REQUEST as usize],
    incomplete: bool,
}

impl TraceRequest {
    const EMPTY: Self = Self {
        serial: 0,
        connection: 0,
        rpc_id: 0,
        started: None,
        records: 0,
        seen: [TraceFingerprint::EMPTY; TRACE_RECORDS_PER_REQUEST as usize],
        incomplete: false,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TraceFingerprint {
    stage: &'static str,
    a: u64,
    b: u64,
}

impl TraceFingerprint {
    const EMPTY: Self = Self { stage: "", a: 0, b: 0 };
}

struct TraceLedger {
    requests: [TraceRequest; TRACE_REQUESTS],
    output_bytes: usize,
    stopped: bool,
}

impl TraceLedger {
    const fn new() -> Self {
        Self {
            requests: [TraceRequest::EMPTY; TRACE_REQUESTS],
            output_bytes: 0,
            stopped: false,
        }
    }
}

fn diagnostic_trace_enabled() -> bool {
    *TRACE_ENABLED.get_or_init(|| std::env::var_os("ORR_NATIVE_SCREENSHOT_TRACE").is_some_and(|value| value == "1"))
}

/// Begin a bounded scalar trace for one admitted native screenshot RPC.
/// The environment switch is sampled once per process. After a disabled
/// result, calls return before locking the ledger, formatting, or writing;
/// the one-time environment lookup may allocate. Numeric JSON-RPC ids are
/// preserved by the caller; string ids use a stable numeric fingerprint.
pub fn diagnostic_trace_begin(serial: u64, connection: u64, rpc_id: u64) {
    if serial == 0 || !diagnostic_trace_enabled() {
        return;
    }
    let mut ledger = lock(&TRACE_LEDGER);
    if ledger.stopped {
        return;
    }
    if let Some(index) = ledger.requests.iter().position(|request| request.serial == serial) {
        let request = &mut ledger.requests[index];
        if connection != 0 {
            request.connection = connection;
            request.rpc_id = rpc_id;
        }
    } else if let Some(index) = ledger.requests.iter().position(|request| request.serial == 0) {
        ledger.requests[index] = TraceRequest {
            serial,
            connection,
            rpc_id,
            started: Some(Instant::now()),
            ..TraceRequest::EMPTY
        };
    } else {
        emit_global_incomplete(&mut ledger, serial, "request_cap");
        return;
    }
}

/// Record a fixed-size scalar stage. `a` and `b` are stage-specific numeric
/// values documented at each call site. Repeated identical observations are
/// coalesced so a ready/poll loop cannot fill the trace by itself.
pub fn diagnostic_trace(serial: u64, stage: &'static str, a: u64, b: u64) {
    if serial == 0 || !diagnostic_trace_enabled() {
        return;
    }
    let mut ledger = lock(&TRACE_LEDGER);
    if ledger.stopped {
        return;
    }
    let Some(index) = ledger.requests.iter().position(|request| request.serial == serial) else { return };
    emit_trace(&mut ledger, index, stage, a, b);
}

fn emit_trace(ledger: &mut TraceLedger, index: usize, stage: &'static str, a: u64, b: u64) {
    let request = &ledger.requests[index];
    if request.incomplete || ledger.stopped {
        return;
    }
    let fingerprint = TraceFingerprint { stage, a, b };
    if request.seen[..usize::from(request.records)].contains(&fingerprint) {
        return;
    }
    if !valid_trace_stage(stage) {
        emit_request_incomplete(ledger, index, "invalid_stage");
        return;
    }
    if request.records >= TRACE_RECORDS_PER_REQUEST - 1 {
        emit_request_incomplete(ledger, index, "record_cap");
        return;
    }
    let elapsed_us = request.started.map_or(0, |started| started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
    let line = format!(
        "{TRACE_PREFIX} serial={} conn={} rpc={} elapsed_us={} stage={} a={} b={}\n",
        request.serial, request.connection, request.rpc_id, elapsed_us, stage, a, b
    );
    if ledger.output_bytes.saturating_add(line.len()) > TRACE_MAX_OUTPUT_BYTES - TRACE_MARKER_RESERVE {
        emit_request_incomplete(ledger, index, "output_cap");
        ledger.stopped = true;
        return;
    }
    if !write_trace_line(ledger, &line) {
        ledger.requests[index].incomplete = true;
        ledger.stopped = true;
        return;
    }
    let request = &mut ledger.requests[index];
    request.seen[usize::from(request.records)] = fingerprint;
    request.records += 1;
}

fn emit_request_incomplete(ledger: &mut TraceLedger, index: usize, reason: &'static str) {
    let (serial, connection, rpc_id) = {
        let request = &ledger.requests[index];
        (request.serial, request.connection, request.rpc_id)
    };
    ledger.requests[index].incomplete = true;
    let line = format!(
        "{TRACE_PREFIX} serial={} conn={} rpc={} stage=incomplete reason={}\n",
        serial, connection, rpc_id, reason
    );
    if ledger.output_bytes.saturating_add(line.len()) <= TRACE_MAX_OUTPUT_BYTES {
        if write_trace_line(ledger, &line) {
            ledger.requests[index].records = ledger.requests[index].records.saturating_add(1);
        } else {
            ledger.stopped = true;
        }
    } else {
        ledger.stopped = true;
    }
}

fn emit_global_incomplete(ledger: &mut TraceLedger, serial: u64, reason: &'static str) {
    if let Some(index) = ledger.requests.iter().position(|request| request.serial != 0 && request.records < TRACE_RECORDS_PER_REQUEST) {
        let (request_serial, connection, rpc_id) = {
            let request = &ledger.requests[index];
            (request.serial, request.connection, request.rpc_id)
        };
        ledger.requests[index].incomplete = true;
        let line = format!(
            "{TRACE_PREFIX} serial={} conn={} rpc={} stage=incomplete reason={} dropped_serial={}\n",
            request_serial, connection, rpc_id, reason, serial
        );
        if ledger.output_bytes.saturating_add(line.len()) <= TRACE_MAX_OUTPUT_BYTES {
            if write_trace_line(ledger, &line) {
                ledger.requests[index].records = ledger.requests[index].records.saturating_add(1);
            }
        }
    }
    ledger.stopped = true;
}

fn write_trace_line(ledger: &mut TraceLedger, line: &str) -> bool {
    use std::io::Write;
    if std::io::stderr().lock().write_all(line.as_bytes()).is_err() {
        ledger.stopped = true;
        return false;
    }
    ledger.output_bytes = ledger.output_bytes.saturating_add(line.len());
    true
}

fn valid_trace_stage(stage: &str) -> bool {
    !stage.is_empty()
        && stage.len() <= 40
        && stage.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

// Keep the checked allocator compatible with the workspace's older Rust
// versions and with Rust 1.99's atomic API rename. Never wrap or reuse IDs.
fn next_identity(counter: &AtomicU64) -> u64 {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1).expect("screenshot identity exhausted");
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return current,
            Err(observed) => current = observed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Edit,
    Play,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewState {
    pub mode: ViewMode,
    pub paused: bool,
    pub tick: u64,
    pub epoch: u64,
    pub checksum: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenshotOptions {
    pub max_width: u32,
    pub max_height: u32,
}

impl ScreenshotOptions {
    pub fn validate(self) -> Result<Self, CaptureError> {
        if self.max_width == 0 || self.max_height == 0 || self.max_width > MAX_DIMENSION || self.max_height > MAX_DIMENSION {
            return Err(CaptureError::Failed);
        }
        Ok(self)
    }

    fn accepts(self, width: u32, height: u32) -> bool {
        width > 0
            && height > 0
            && width <= self.max_width
            && height <= self.max_height
            && u64::from(width) * u64::from(height) <= MAX_PIXELS
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub endpoint_id: u64,
    pub serial: u64,
    pub options: ScreenshotOptions,
    pub requested: ViewState,
    pub deadline: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub captured: ViewState,
    pub frame_seq: u64,
    pub ui_frame: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unavailable,
    Stale,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotAdmissionError {
    Unavailable,
    Busy,
    Invalid,
}

#[derive(Default)]
struct SharedState {
    owner_alive: bool,
    service_alive: bool,
    pending: Option<Pending>,
    capture: Option<u64>,
    encoder: Option<u64>,
}

struct Pending {
    request: CaptureRequest,
    delivered: bool,
    result: Option<Result<CapturedImage, CaptureError>>,
}

struct Shared {
    endpoint_id: u64,
    state: Mutex<SharedState>,
}

/// Host-side endpoint. Clone this into the local host's `ServerConfig`.
pub struct ScreenshotService {
    shared: Arc<Shared>,
    handles: Arc<()>,
}

/// UI-side endpoint. Dropping it makes future calls unavailable.
pub struct ScreenshotOwner {
    shared: Arc<Shared>,
}

impl ScreenshotService {
    /// Create a paired host endpoint and the sole editor owner for one host lifetime.
    pub fn pair() -> (Self, ScreenshotOwner) {
        let endpoint_id = next_identity(&NEXT_ENDPOINT_ID);
        let shared = Arc::new(Shared {
            endpoint_id,
            state: Mutex::new(SharedState {
                owner_alive: true,
                service_alive: true,
                ..SharedState::default()
            }),
        });
        (Self { shared: shared.clone(), handles: Arc::new(()) }, ScreenshotOwner { shared })
    }

    /// Reserve the sole caller slot. Encoder capacity is checked independently,
    /// so cancellation cannot admit a second readback while an old worker runs.
    pub fn submit(
        &self,
        options: ScreenshotOptions,
        requested: ViewState,
        deadline: Instant,
    ) -> Result<CaptureRequest, ScreenshotAdmissionError> {
        let mut state = lock(&self.shared.state);
        if !state.service_alive || !state.owner_alive {
            return Err(ScreenshotAdmissionError::Unavailable);
        }
        if state.pending.is_some() || state.capture.is_some() || state.encoder.is_some() {
            return Err(ScreenshotAdmissionError::Busy);
        }
        options.validate().map_err(|_| ScreenshotAdmissionError::Invalid)?;
        let serial = next_identity(&NEXT_SERIAL);
        let request = CaptureRequest { endpoint_id: self.shared.endpoint_id, serial, options, requested, deadline };
        // Reserve the serial and trace clock before publishing so the editor's
        // first stage is retained; the server adds connection/RPC correlation.
        diagnostic_trace_begin(serial, 0, 0);
        state.pending = Some(Pending { request: request.clone(), delivered: false, result: None });
        drop(state);
        // Scalars: requested max width and max height, respectively.
        diagnostic_trace(serial, "service_admit", u64::from(options.max_width), u64::from(options.max_height));
        Ok(request)
    }

    /// Abandon a caller ticket. A live encoder permit is deliberately retained.
    pub fn cancel(&self, serial: u64) {
        let (removed, capture_held, encoder_held) = {
            let mut state = lock(&self.shared.state);
            let removed = state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial);
            if removed {
                state.pending = None;
            }
            (removed, state.capture == Some(serial), state.encoder == Some(serial))
        };
        // b is a bitset: bit 0 = encoder permit held, bit 1 = capture permit held.
        diagnostic_trace(serial, "service_cancel", removed as u64, ((capture_held as u64) << 1) | encoder_held as u64);
    }

    /// Take the terminal result for a live ticket, if the owner has completed it.
    pub fn poll_result(&self, serial: u64) -> Option<Result<CapturedImage, CaptureError>> {
        let result = {
            let mut state = lock(&self.shared.state);
            if !state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
                None
            } else {
                let result = state.pending.as_mut().and_then(|pending| pending.result.take());
                if result.is_some() {
                    state.pending = None;
                }
                result
            }
        };
        let status = match &result {
            None => 0,
            Some(Ok(_)) => 1,
            Some(Err(CaptureError::Unavailable)) => 2,
            Some(Err(CaptureError::Stale)) => 3,
            Some(Err(CaptureError::Failed)) => 4,
        };
        // a: 0 = absent/not ready, 1 = image, 2 = unavailable, 3 = stale, 4 = failed.
        diagnostic_trace(serial, "service_poll_result", status, 0);
        result
    }

    pub fn owner_available(&self) -> bool {
        let state = lock(&self.shared.state);
        state.service_alive && state.owner_alive
    }

    pub fn endpoint_id(&self) -> u64 {
        self.shared.endpoint_id
    }
}

impl Clone for ScreenshotService {
    fn clone(&self) -> Self {
        Self { shared: self.shared.clone(), handles: self.handles.clone() }
    }
}

impl core::fmt::Debug for ScreenshotService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScreenshotService").field("available", &self.owner_available()).finish()
    }
}

impl Drop for ScreenshotService {
    fn drop(&mut self) {
        if Arc::strong_count(&self.handles) == 1 {
            let (serial, capture_held, encoder_held) = {
                let mut state = lock(&self.shared.state);
                let serial = state.pending.as_ref().map(|pending| pending.request.serial).or(state.capture).or(state.encoder);
                state.service_alive = false;
                state.pending = None;
                (serial, state.capture.is_some(), state.encoder.is_some())
            };
            if let Some(serial) = serial {
                diagnostic_trace(serial, "service_drop", capture_held as u64, encoder_held as u64);
            }
        }
    }
}

impl ScreenshotOwner {
    pub fn endpoint_id(&self) -> u64 {
        self.shared.endpoint_id
    }

    /// Whether this endpoint pair still has both its editor owner and host service.
    pub fn available(&self) -> bool {
        let state = lock(&self.shared.state);
        state.service_alive && state.owner_alive
    }

    /// Nonblocking; returns the current ticket at most once to the UI caller.
    pub fn take_request(&self) -> Option<CaptureRequest> {
        let request = {
            let mut state = lock(&self.shared.state);
            if !state.service_alive || !state.owner_alive || state.encoder.is_some() {
                None
            } else {
                state.pending.as_mut().and_then(|pending| {
                    if pending.delivered || pending.result.is_some() {
                        None
                    } else {
                        pending.delivered = true;
                        Some(pending.request.clone())
                    }
                })
            }
        };
        if let Some(request) = &request {
            // a is the opaque endpoint identity; no image/frame data is logged.
            diagnostic_trace(request.serial, "owner_take", request.endpoint_id, 0);
        }
        request
    }

    pub fn is_active(&self, serial: u64) -> bool {
        let state = lock(&self.shared.state);
        state.service_alive && state.owner_alive && state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial)
    }

    /// Reserve the physical framebuffer/readback slot before issuing egui's
    /// screenshot command. This permit is independent of the caller ticket.
    pub fn begin_capture(&self, serial: u64) -> Result<(), CaptureError> {
        let result = {
            let mut state = lock(&self.shared.state);
            if !state.service_alive || !state.owner_alive {
                Err(CaptureError::Unavailable)
            } else if !state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
                Err(CaptureError::Stale)
            } else if state.capture.is_some() || state.encoder.is_some() {
                Err(CaptureError::Failed)
            } else {
                state.capture = Some(serial);
                Ok(())
            }
        };
        // a: 1 = reserved, 2 = unavailable, 3 = stale, 4 = permit conflict.
        diagnostic_trace(serial, "owner_begin_capture", capture_error_code(result), 0);
        result
    }

    /// Release a capture whose matching GPU event was consumed and which has
    /// no live encoder worker. Cancellation alone never calls this implicitly.
    pub fn end_capture(&self, serial: u64) {
        let (released, encoder_held) = {
            let mut state = lock(&self.shared.state);
            let encoder_held = state.encoder == Some(serial);
            let released = state.capture == Some(serial) && !encoder_held;
            if released {
                state.capture = None;
            }
            (released, encoder_held)
        };
        // a = capture permit released; b = encoder permit still held.
        diagnostic_trace(serial, "owner_end_capture", released as u64, encoder_held as u64);
    }

    /// Claim the single asynchronous encoder permit for this active request.
    pub fn begin_encoder(&self, serial: u64) -> Result<(), CaptureError> {
        let result = {
            let mut state = lock(&self.shared.state);
            if !state.service_alive || !state.owner_alive {
                Err(CaptureError::Unavailable)
            } else if !state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
                Err(CaptureError::Stale)
            } else if state.capture != Some(serial) {
                Err(CaptureError::Stale)
            } else if state.encoder.is_some() {
                Err(CaptureError::Failed)
            } else {
                state.encoder = Some(serial);
                Ok(())
            }
        };
        // a: 1 = reserved, 2 = unavailable, 3 = stale, 4 = permit conflict.
        diagnostic_trace(serial, "owner_begin_encoder", capture_error_code(result), 0);
        result
    }

    /// Release the encoder permit only after its worker has actually exited.
    pub fn end_encoder(&self, serial: u64) {
        let (released, capture_released) = {
            let mut state = lock(&self.shared.state);
            let released = state.encoder == Some(serial);
            let mut capture_released = false;
            if released {
                state.encoder = None;
                if state.capture == Some(serial) {
                    state.capture = None;
                    capture_released = true;
                }
            }
            (released, capture_released)
        };
        // Called after the worker exits. a = encoder permit released; b = capture permit released.
        diagnostic_trace(serial, "owner_end_encoder", released as u64, capture_released as u64);
    }

    /// Publish a terminal capture result. Canceled and late results are ignored.
    pub fn complete(&self, serial: u64, result: Result<CapturedImage, CaptureError>) {
        let (status, png_bytes) = {
            let mut state = lock(&self.shared.state);
            let capture_held = state.capture == Some(serial);
            if let Some(pending) = state.pending.as_mut().filter(|pending| pending.request.serial == serial) {
                let checked = result.and_then(|image| {
                    if !capture_held {
                        Err(CaptureError::Stale)
                    } else if !pending.request.options.accepts(image.width, image.height) || image.png.is_empty() || image.png.len() > MAX_PNG_BYTES {
                        Err(CaptureError::Failed)
                    } else {
                        Ok(image)
                    }
                });
                let status = match &checked {
                    Ok(_) => 1,
                    Err(CaptureError::Unavailable) => 2,
                    Err(CaptureError::Stale) => 3,
                    Err(CaptureError::Failed) => 4,
                };
                let png_bytes = checked.as_ref().ok().map_or(0, |image| image.png.len() as u64);
                pending.result = Some(checked);
                (status, png_bytes)
            } else {
                (0, 0)
            }
        };
        // a: 0 = canceled/late, 1 = success, 2 = unavailable, 3 = stale, 4 = failed; b = PNG bytes only.
        diagnostic_trace(serial, "owner_complete", status, png_bytes);
    }
}

fn capture_error_code(result: Result<(), CaptureError>) -> u64 {
    match result {
        Ok(()) => 1,
        Err(CaptureError::Unavailable) => 2,
        Err(CaptureError::Stale) => 3,
        Err(CaptureError::Failed) => 4,
    }
}

impl Drop for ScreenshotOwner {
    fn drop(&mut self) {
        let (serial, capture_held, encoder_held) = {
            let mut state = lock(&self.shared.state);
            let serial = state.pending.as_ref().map(|pending| pending.request.serial).or(state.capture).or(state.encoder);
            state.owner_alive = false;
            state.pending = None;
            (serial, state.capture.is_some(), state.encoder.is_some())
        };
        if let Some(serial) = serial {
            diagnostic_trace(serial, "owner_drop", capture_held as u64, encoder_held as u64);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_unique_under_contention_and_exhaustion_never_wraps() {
        let counter = Arc::new(AtomicU64::new(1));
        let workers: Vec<_> = (0..8).map(|_| {
            let counter = counter.clone();
            std::thread::spawn(move || (0..32).map(|_| next_identity(&counter)).collect::<Vec<_>>())
        }).collect();
        let mut identities: Vec<_> = workers.into_iter().flat_map(|worker| worker.join().unwrap()).collect();
        identities.sort_unstable();
        assert_eq!(identities, (1..=256).collect::<Vec<_>>());

        let exhausted = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_identity(&exhausted), u64::MAX - 1);
        assert!(std::panic::catch_unwind(|| next_identity(&exhausted)).is_err());
        assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
    }

    fn pair() -> (ScreenshotService, ScreenshotOwner, CaptureRequest) {
        let (service, owner) = ScreenshotService::pair();
        let request = service.submit(
            ScreenshotOptions { max_width: 2048, max_height: 2048 },
            ViewState { mode: ViewMode::Edit, paused: true, tick: 0, epoch: 0, checksum: 1 },
            Instant::now() + std::time::Duration::from_secs(5),
        ).unwrap();
        (service, owner, request)
    }

    #[test]
    fn cancellation_keeps_admission_busy_until_encoder_exits() {
        let (service, owner, request) = pair();
        assert_eq!(owner.take_request(), Some(request.clone()));
        owner.begin_capture(request.serial).unwrap();
        owner.begin_encoder(request.serial).unwrap();
        service.cancel(request.serial);
        assert!(!owner.is_active(request.serial));
        assert_eq!(service.submit(request.options, request.requested, request.deadline), Err(ScreenshotAdmissionError::Busy));
        owner.end_capture(request.serial);
        assert_eq!(service.submit(request.options, request.requested, request.deadline), Err(ScreenshotAdmissionError::Busy));
        owner.end_encoder(request.serial);
        assert!(service.submit(request.options, request.requested, request.deadline).is_ok());
    }

    #[test]
    fn cancellation_before_gpu_event_keeps_capture_permit_until_discard() {
        let (service, owner, request) = pair();
        owner.take_request().unwrap();
        owner.begin_capture(request.serial).unwrap();
        service.cancel(request.serial);
        assert_eq!(service.submit(request.options, request.requested, request.deadline), Err(ScreenshotAdmissionError::Busy));
        owner.complete(request.serial, Ok(CapturedImage { png: vec![1], width: 1, height: 1, captured: request.requested, frame_seq: 1, ui_frame: 1 }));
        assert_eq!(service.poll_result(request.serial), None, "late readback is discarded after caller cancellation");
        owner.end_capture(request.serial);
        assert!(service.submit(request.options, request.requested, request.deadline).is_ok());
    }

    #[test]
    fn late_completion_after_cancel_is_ignored_and_limits_are_checked() {
        let (service, owner, request) = pair();
        service.cancel(request.serial);
        owner.complete(request.serial, Ok(CapturedImage { png: vec![0; MAX_PNG_BYTES + 1], width: 1, height: 1, captured: request.requested, frame_seq: 1, ui_frame: 1 }));
        assert_eq!(service.poll_result(request.serial), None);
        let next = service.submit(request.options, request.requested, request.deadline).unwrap();
        owner.begin_capture(next.serial).unwrap();
        owner.complete(next.serial, Ok(CapturedImage { png: vec![0; MAX_PNG_BYTES + 1], width: 1, height: 1, captured: next.requested, frame_seq: 1, ui_frame: 1 }));
        assert_eq!(service.poll_result(next.serial), Some(Err(CaptureError::Failed)));
    }

    #[test]
    fn dimensions_pixel_budget_and_empty_png_are_rejected() {
        let (service, owner, request) = pair();
        owner.begin_capture(request.serial).unwrap();
        owner.complete(request.serial, Ok(CapturedImage { png: vec![1], width: 2048, height: 513, captured: request.requested, frame_seq: 1, ui_frame: 1 }));
        assert_eq!(service.poll_result(request.serial), Some(Err(CaptureError::Failed)));
        owner.end_capture(request.serial);

        let next = service.submit(request.options, request.requested, request.deadline).unwrap();
        owner.begin_capture(next.serial).unwrap();
        owner.complete(next.serial, Ok(CapturedImage { png: Vec::new(), width: 1, height: 1, captured: next.requested, frame_seq: 1, ui_frame: 1 }));
        assert_eq!(service.poll_result(next.serial), Some(Err(CaptureError::Failed)));
        owner.end_capture(next.serial);

        let capped = service.submit(request.options, request.requested, request.deadline).unwrap();
        owner.begin_capture(capped.serial).unwrap();
        owner.complete(capped.serial, Ok(CapturedImage { png: vec![1; MAX_PNG_BYTES], width: 1, height: 1, captured: capped.requested, frame_seq: 1, ui_frame: 1 }));
        assert!(service.poll_result(capped.serial).unwrap().is_ok(), "the inclusive 4 MiB output cap is accepted");
        owner.end_capture(capped.serial);
    }

    #[test]
    fn replacement_endpoint_never_reuses_old_worker_serial() {
        let (old_service, old_owner, old_request) = pair();
        old_owner.begin_capture(old_request.serial).unwrap();
        old_owner.begin_encoder(old_request.serial).unwrap();
        drop(old_service);

        let (new_service, new_owner) = ScreenshotService::pair();
        let new_request = new_service.submit(old_request.options, old_request.requested, old_request.deadline).unwrap();
        assert_ne!(old_request.endpoint_id, new_request.endpoint_id);
        assert_ne!(old_request.serial, new_request.serial);
        assert!(!old_owner.available());
        assert!(new_owner.available());
        new_owner.begin_capture(new_request.serial).unwrap();
        new_owner.begin_encoder(new_request.serial).unwrap();
        old_owner.end_encoder(old_request.serial);
        old_owner.complete(old_request.serial, Ok(CapturedImage { png: vec![1], width: 1, height: 1, captured: old_request.requested, frame_seq: 1, ui_frame: 1 }));
        assert!(new_owner.is_active(new_request.serial));
        new_owner.end_encoder(old_request.serial);
        new_service.cancel(new_request.serial);
        assert_eq!(
            new_service.submit(new_request.options, new_request.requested, new_request.deadline),
            Err(ScreenshotAdmissionError::Busy),
            "a stale worker cannot release the replacement endpoint's permit"
        );
        new_owner.end_encoder(new_request.serial);
        assert!(new_service.submit(new_request.options, new_request.requested, new_request.deadline).is_ok());
    }

}
