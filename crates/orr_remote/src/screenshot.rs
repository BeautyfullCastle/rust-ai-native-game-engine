//! Bounded, typed handoff between a local editor UI and its ERP host.
//!
//! The host owns the request/result slot. The editor owns the capture ticket
//! and the independent encoder permit, which remains held until a worker exits.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_DIMENSION: u32 = 2048;
const MAX_PIXELS: u64 = 1_048_576;
const MAX_PNG_BYTES: usize = 4 << 20;
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1);

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
        let endpoint_id = NEXT_ENDPOINT_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("screenshot endpoint id exhausted");
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
        let serial = NEXT_SERIAL.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |serial| serial.checked_add(1))
            .expect("screenshot serial exhausted");
        let request = CaptureRequest { endpoint_id: self.shared.endpoint_id, serial, options, requested, deadline };
        state.pending = Some(Pending { request: request.clone(), delivered: false, result: None });
        Ok(request)
    }

    /// Abandon a caller ticket. A live encoder permit is deliberately retained.
    pub fn cancel(&self, serial: u64) {
        let mut state = lock(&self.shared.state);
        if state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
            state.pending = None;
        }
    }

    /// Take the terminal result for a live ticket, if the owner has completed it.
    pub fn poll_result(&self, serial: u64) -> Option<Result<CapturedImage, CaptureError>> {
        let mut state = lock(&self.shared.state);
        let pending = state.pending.as_mut().filter(|pending| pending.request.serial == serial)?;
        let result = pending.result.take()?;
        state.pending = None;
        Some(result)
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
            let mut state = lock(&self.shared.state);
            state.service_alive = false;
            state.pending = None;
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
        let mut state = lock(&self.shared.state);
        if !state.service_alive || !state.owner_alive || state.encoder.is_some() {
            return None;
        }
        let pending = state.pending.as_mut()?;
        if pending.delivered || pending.result.is_some() { return None; }
        pending.delivered = true;
        Some(pending.request.clone())
    }

    pub fn is_active(&self, serial: u64) -> bool {
        let state = lock(&self.shared.state);
        state.service_alive && state.owner_alive && state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial)
    }

    /// Reserve the physical framebuffer/readback slot before issuing egui's
    /// screenshot command. This permit is independent of the caller ticket.
    pub fn begin_capture(&self, serial: u64) -> Result<(), CaptureError> {
        let mut state = lock(&self.shared.state);
        if !state.service_alive || !state.owner_alive {
            return Err(CaptureError::Unavailable);
        }
        if !state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
            return Err(CaptureError::Stale);
        }
        if state.capture.is_some() || state.encoder.is_some() {
            return Err(CaptureError::Failed);
        }
        state.capture = Some(serial);
        Ok(())
    }

    /// Release a capture whose matching GPU event was consumed and which has
    /// no live encoder worker. Cancellation alone never calls this implicitly.
    pub fn end_capture(&self, serial: u64) {
        let mut state = lock(&self.shared.state);
        if state.capture == Some(serial) && state.encoder != Some(serial) {
            state.capture = None;
        }
    }

    /// Claim the single asynchronous encoder permit for this active request.
    pub fn begin_encoder(&self, serial: u64) -> Result<(), CaptureError> {
        let mut state = lock(&self.shared.state);
        if !state.service_alive || !state.owner_alive {
            return Err(CaptureError::Unavailable);
        }
        if !state.pending.as_ref().is_some_and(|pending| pending.request.serial == serial) {
            return Err(CaptureError::Stale);
        }
        if state.capture != Some(serial) {
            return Err(CaptureError::Stale);
        }
        if state.encoder.is_some() {
            return Err(CaptureError::Failed);
        }
        state.encoder = Some(serial);
        Ok(())
    }

    /// Release the encoder permit only after its worker has actually exited.
    pub fn end_encoder(&self, serial: u64) {
        let mut state = lock(&self.shared.state);
        if state.encoder == Some(serial) {
            state.encoder = None;
            if state.capture == Some(serial) {
                state.capture = None;
            }
        }
    }

    /// Publish a terminal capture result. Canceled and late results are ignored.
    pub fn complete(&self, serial: u64, result: Result<CapturedImage, CaptureError>) {
        let mut state = lock(&self.shared.state);
        let capture_held = state.capture == Some(serial);
        let Some(pending) = state.pending.as_mut().filter(|pending| pending.request.serial == serial) else { return };
        let checked = result.and_then(|image| {
            if !capture_held {
                Err(CaptureError::Stale)
            } else if !pending.request.options.accepts(image.width, image.height) || image.png.is_empty() || image.png.len() > MAX_PNG_BYTES {
                Err(CaptureError::Failed)
            } else {
                Ok(image)
            }
        });
        pending.result = Some(checked);
    }
}

impl Drop for ScreenshotOwner {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        state.owner_alive = false;
        state.pending = None;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

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
