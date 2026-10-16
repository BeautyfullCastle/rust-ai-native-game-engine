//! Bounded, test-binary-only observer for the opt-in real PCM preload path.
//!
//! The two end-to-end cases are deliberately ignored. They may only be run by
//! the reviewed decode-memory campaign, one exact case at a time. The ordinary
//! tests below exercise the fixed-capacity accounting model without enabling
//! the process allocator observer.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::io::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use orr_asset::{encode_manifest, manifest_encoded_len, AssetRef, Domain, ManifestEntry};
use orr_asset_fixture::audio::{DecodeProbeBank, ViewBundleBytes};
use orr_asset_fixture::{ArtifactBytes, BundleBytes, PreparedFixture, ReleaseBinding, IMPACT_ID};
use serde::Serialize;
use sha2::{Digest, Sha256};

const EVENT_CAPACITY: usize = 4096;
const LIVE_CAPACITY: usize = EVENT_CAPACITY;
const RESULT_CAP: usize = 1024 * 1024;
const PREFIX: &[u8] = b"ORR_DECODE_ALLOCATION_PROBE ";
const PCM_MAX_FRAMES: usize = 48_000;
const MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;
const STEREO_FRAME_BYTES: usize = 8;
const SIM_OBJECT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fixture_v1/cooked/objects/a02ae51509464de11084e34346a86574191b861de2cb5cf064661b459c9bc9e4.bin"
));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum EventKind {
    Empty = 0,
    Allocate = 1,
    Deallocate = 2,
    Reallocate = 3,
    ReallocateFailed = 4,
    ReallocateToZero = 5,
}

impl EventKind {
    fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Allocate => "allocate",
            Self::Deallocate => "deallocate",
            Self::Reallocate => "reallocate_old_to_new",
            Self::ReallocateFailed => "reallocate_failed_old_preserved",
            Self::ReallocateToZero => "reallocate_to_zero",
        }
    }
}

#[derive(Clone, Copy)]
struct Event {
    kind: EventKind,
    id: u64,
    old_bytes: usize,
    new_bytes: usize,
}

impl Event {
    const EMPTY: Self = Self {
        kind: EventKind::Empty,
        id: 0,
        old_bytes: 0,
        new_bytes: 0,
    };
}

#[derive(Clone, Copy)]
struct LiveAllocation {
    pointer: usize,
    id: u64,
    bytes: usize,
    active: bool,
}

impl LiveAllocation {
    const EMPTY: Self = Self {
        pointer: 0,
        id: 0,
        bytes: 0,
        active: false,
    };
}

const FLAG_EVENT_OVERFLOW: u32 = 1 << 0;
const FLAG_LIVE_OVERFLOW: u32 = 1 << 1;
const FLAG_UNKNOWN_POINTER: u32 = 1 << 2;
const FLAG_DUPLICATE_POINTER: u32 = 1 << 3;
const FLAG_ARITHMETIC: u32 = 1 << 4;
const FLAG_ID_OVERFLOW: u32 = 1 << 5;
const FLAG_ALLOC_FAILURE: u32 = 1 << 6;

/// All mutable fields are touched by the single tagged owner thread only,
/// while CAPTURE_CLAIM is held. Foreign-thread hooks never write here.
struct Ledger {
    events: [Event; EVENT_CAPACITY],
    event_count: usize,
    live: [LiveAllocation; LIVE_CAPACITY],
    next_id: u64,
    flags: u32,
    allocation_calls: u64,
    deallocation_calls: u64,
    reallocation_calls: u64,
    failed_reallocations: u64,
}

impl Ledger {
    const fn new() -> Self {
        Self {
            events: [Event::EMPTY; EVENT_CAPACITY],
            event_count: 0,
            live: [LiveAllocation::EMPTY; LIVE_CAPACITY],
            next_id: 1,
            flags: 0,
            allocation_calls: 0,
            deallocation_calls: 0,
            reallocation_calls: 0,
            failed_reallocations: 0,
        }
    }

    fn reset(&mut self) {
        self.event_count = 0;
        self.next_id = 1;
        self.flags = 0;
        self.allocation_calls = 0;
        self.deallocation_calls = 0;
        self.reallocation_calls = 0;
        self.failed_reallocations = 0;
        self.live.fill(LiveAllocation::EMPTY);
    }

    fn append(&mut self, event: Event) {
        if self.event_count == EVENT_CAPACITY {
            self.flags |= FLAG_EVENT_OVERFLOW;
            return;
        }
        self.events[self.event_count] = event;
        self.event_count += 1;
    }

    fn live_slot(&self, pointer: usize) -> Option<usize> {
        self.live
            .iter()
            .position(|entry| entry.active && entry.pointer == pointer)
    }

    fn free_slot(&self) -> Option<usize> {
        self.live.iter().position(|entry| !entry.active)
    }

    fn increment(counter: &mut u64, flags: &mut u32) {
        match counter.checked_add(1) {
            Some(next) => *counter = next,
            None => *flags |= FLAG_ARITHMETIC,
        }
    }

    fn allocate(&mut self, pointer: usize, bytes: usize) {
        if bytes == 0 {
            return;
        }
        Self::increment(&mut self.allocation_calls, &mut self.flags);
        if pointer == 0 {
            self.flags |= FLAG_UNKNOWN_POINTER;
            return;
        }
        if self.live_slot(pointer).is_some() {
            self.flags |= FLAG_DUPLICATE_POINTER;
            return;
        }
        let Some(slot) = self.free_slot() else {
            self.flags |= FLAG_LIVE_OVERFLOW;
            return;
        };
        let id = self.next_id;
        let Some(next_id) = self.next_id.checked_add(1) else {
            self.flags |= FLAG_ID_OVERFLOW;
            return;
        };
        self.next_id = next_id;
        self.live[slot] = LiveAllocation {
            pointer,
            id,
            bytes,
            active: true,
        };
        self.append(Event {
            kind: EventKind::Allocate,
            id,
            old_bytes: 0,
            new_bytes: bytes,
        });
    }

    fn deallocate(&mut self, pointer: usize) {
        Self::increment(&mut self.deallocation_calls, &mut self.flags);
        let Some(slot) = self.live_slot(pointer) else {
            self.flags |= FLAG_UNKNOWN_POINTER;
            return;
        };
        let allocation = self.live[slot];
        self.live[slot] = LiveAllocation::EMPTY;
        self.append(Event {
            kind: EventKind::Deallocate,
            id: allocation.id,
            old_bytes: allocation.bytes,
            new_bytes: 0,
        });
    }

    fn reallocate(&mut self, old_pointer: usize, new_pointer: Option<usize>, new_bytes: usize) {
        Self::increment(&mut self.reallocation_calls, &mut self.flags);
        let Some(slot) = self.live_slot(old_pointer) else {
            self.flags |= FLAG_UNKNOWN_POINTER;
            return;
        };
        let old = self.live[slot];
        if new_bytes == 0 {
            self.live[slot] = LiveAllocation::EMPTY;
            self.append(Event {
                kind: EventKind::ReallocateToZero,
                id: old.id,
                old_bytes: old.bytes,
                new_bytes: 0,
            });
            return;
        }
        match new_pointer {
            None => {
                Self::increment(&mut self.failed_reallocations, &mut self.flags);
                self.append(Event {
                    kind: EventKind::ReallocateFailed,
                    id: old.id,
                    old_bytes: old.bytes,
                    new_bytes,
                });
            }
            Some(new_ptr) => {
                if self
                    .live
                    .iter()
                    .enumerate()
                    .any(|(index, entry)| index != slot && entry.active && entry.pointer == new_ptr)
                {
                    self.flags |= FLAG_DUPLICATE_POINTER;
                    return;
                }
                self.live[slot] = LiveAllocation {
                    pointer: new_ptr,
                    id: old.id,
                    bytes: new_bytes,
                    active: true,
                };
                self.append(Event {
                    kind: EventKind::Reallocate,
                    id: old.id,
                    old_bytes: old.bytes,
                    new_bytes,
                });
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        let mut snapshot = Snapshot::EMPTY;
        snapshot.event_index = self.event_count;
        for allocation in self.live.iter().filter(|allocation| allocation.active) {
            if snapshot.retained_count == LIVE_CAPACITY {
                snapshot.flags |= FLAG_LIVE_OVERFLOW;
                break;
            }
            let Some(next_bytes) = snapshot.returned_live_bytes.checked_add(allocation.bytes)
            else {
                snapshot.flags |= FLAG_ARITHMETIC;
                break;
            };
            snapshot.returned_live_bytes = next_bytes;
            snapshot.retained_ids[snapshot.retained_count] = allocation.id;
            snapshot.retained_count += 1;
        }
        snapshot
    }

    fn summary(&self, snapshot: &Snapshot, foreign_calls: u64) -> Summary {
        let mut result = Summary {
            event_count: self.event_count,
            allocation_calls: self.allocation_calls,
            deallocation_calls: self.deallocation_calls,
            reallocation_calls: self.reallocation_calls,
            failed_reallocations: self.failed_reallocations,
            flags: self.flags | snapshot.flags,
            foreign_thread_calls: foreign_calls,
            returned_live_bytes: snapshot.returned_live_bytes,
            ..Summary::ZERO
        };

        let mut total = 0usize;
        let mut transient = 0usize;
        let mut retained = 0usize;
        for (event_index, event) in self.events.iter().take(self.event_count).enumerate() {
            let before_return = event_index < snapshot.event_index;
            let is_retained = snapshot.contains(event.id);
            match event.kind {
                EventKind::Allocate => {
                    if !add_bytes(&mut total, event.new_bytes, &mut result.flags) {
                        continue;
                    }
                    let category = if is_retained {
                        &mut retained
                    } else {
                        &mut transient
                    };
                    if add_bytes(category, event.new_bytes, &mut result.flags) && before_return {
                        result.transient_peak_bytes = result.transient_peak_bytes.max(transient);
                        result.retained_peak_bytes = result.retained_peak_bytes.max(retained);
                        result.combined_peak_bytes = result.combined_peak_bytes.max(total);
                    }
                }
                EventKind::Deallocate | EventKind::ReallocateToZero => {
                    if sub_bytes(&mut total, event.old_bytes, &mut result.flags) {
                        let category = if is_retained {
                            &mut retained
                        } else {
                            &mut transient
                        };
                        sub_bytes(category, event.old_bytes, &mut result.flags);
                    }
                }
                EventKind::Reallocate => {
                    if event.new_bytes >= event.old_bytes {
                        let growth = event.new_bytes - event.old_bytes;
                        let category = if is_retained {
                            &mut retained
                        } else {
                            &mut transient
                        };
                        if add_bytes(&mut total, growth, &mut result.flags)
                            && add_bytes(category, growth, &mut result.flags)
                            && before_return
                        {
                            result.combined_peak_bytes = result.combined_peak_bytes.max(total);
                            result.transient_peak_bytes =
                                result.transient_peak_bytes.max(transient);
                            result.retained_peak_bytes = result.retained_peak_bytes.max(retained);
                        }
                    } else {
                        let shrink = event.old_bytes - event.new_bytes;
                        if sub_bytes(&mut total, shrink, &mut result.flags) {
                            let category = if is_retained {
                                &mut retained
                            } else {
                                &mut transient
                            };
                            sub_bytes(category, shrink, &mut result.flags);
                        }
                    }
                }
                EventKind::ReallocateFailed => {}
                EventKind::Empty => result.flags |= FLAG_UNKNOWN_POINTER,
            }
            if event_index + 1 == snapshot.event_index && total != snapshot.returned_live_bytes {
                result.flags |= FLAG_ARITHMETIC;
            }
        }
        if snapshot.event_index == 0 && snapshot.returned_live_bytes != 0 {
            result.flags |= FLAG_ARITHMETIC;
        }
        result.dropped_live_bytes = total;
        result.transient_final_bytes = transient;
        result.retained_final_bytes = retained;
        result.complete = result.flags == 0 && foreign_calls == 0;
        result
    }

    fn sanitized_events(&self) -> Vec<EventReport> {
        self.events
            .iter()
            .take(self.event_count)
            .map(|event| EventReport {
                lifetime_id: event.id,
                kind: event.kind.label(),
                old_bytes: event.old_bytes,
                new_bytes: event.new_bytes,
            })
            .collect()
    }
}

fn add_bytes(value: &mut usize, extra: usize, flags: &mut u32) -> bool {
    match value.checked_add(extra) {
        Some(next) => {
            *value = next;
            true
        }
        None => {
            *flags |= FLAG_ARITHMETIC;
            false
        }
    }
}

fn sub_bytes(value: &mut usize, amount: usize, flags: &mut u32) -> bool {
    match value.checked_sub(amount) {
        Some(next) => {
            *value = next;
            true
        }
        None => {
            *flags |= FLAG_ARITHMETIC;
            false
        }
    }
}

#[derive(Clone, Copy)]
struct Snapshot {
    retained_ids: [u64; LIVE_CAPACITY],
    retained_count: usize,
    returned_live_bytes: usize,
    event_index: usize,
    flags: u32,
}

impl Snapshot {
    const EMPTY: Self = Self {
        retained_ids: [0; LIVE_CAPACITY],
        retained_count: 0,
        returned_live_bytes: 0,
        event_index: 0,
        flags: 0,
    };

    fn contains(&self, id: u64) -> bool {
        self.retained_ids[..self.retained_count].contains(&id)
    }
}

#[derive(Clone, Copy)]
struct Summary {
    complete: bool,
    flags: u32,
    event_count: usize,
    allocation_calls: u64,
    deallocation_calls: u64,
    reallocation_calls: u64,
    failed_reallocations: u64,
    foreign_thread_calls: u64,
    returned_live_bytes: usize,
    dropped_live_bytes: usize,
    combined_peak_bytes: usize,
    retained_peak_bytes: usize,
    transient_peak_bytes: usize,
    retained_final_bytes: usize,
    transient_final_bytes: usize,
}

#[derive(Serialize)]
struct SummaryReport {
    complete: bool,
    flags: u32,
    foreign_thread_calls: u64,
    event_count: Option<usize>,
    allocation_calls: Option<u64>,
    deallocation_calls: Option<u64>,
    reallocation_calls: Option<u64>,
    failed_reallocations: Option<u64>,
    returned_live_bytes: Option<usize>,
    dropped_live_bytes: Option<usize>,
    combined_peak_bytes: Option<usize>,
    retained_peak_bytes: Option<usize>,
    transient_peak_bytes: Option<usize>,
    retained_final_bytes: Option<usize>,
    transient_final_bytes: Option<usize>,
    metric_boundary: &'static str,
}

impl From<Summary> for SummaryReport {
    fn from(summary: Summary) -> Self {
        let valid = summary.complete;
        Self {
            complete: valid,
            flags: summary.flags,
            foreign_thread_calls: summary.foreign_thread_calls,
            event_count: valid.then_some(summary.event_count),
            allocation_calls: valid.then_some(summary.allocation_calls),
            deallocation_calls: valid.then_some(summary.deallocation_calls),
            reallocation_calls: valid.then_some(summary.reallocation_calls),
            failed_reallocations: valid.then_some(summary.failed_reallocations),
            returned_live_bytes: valid.then_some(summary.returned_live_bytes),
            dropped_live_bytes: valid.then_some(summary.dropped_live_bytes),
            combined_peak_bytes: valid.then_some(summary.combined_peak_bytes),
            retained_peak_bytes: valid.then_some(summary.retained_peak_bytes),
            transient_peak_bytes: valid.then_some(summary.transient_peak_bytes),
            retained_final_bytes: valid.then_some(summary.retained_final_bytes),
            transient_final_bytes: valid.then_some(summary.transient_final_bytes),
            metric_boundary: "requested Rust layout bytes at allocator-call boundaries; realloc is old-to-new and excludes allocator-internal overlap, RSS and private buffers",
        }
    }
}

#[derive(Serialize)]
struct EventReport {
    lifetime_id: u64,
    kind: &'static str,
    old_bytes: usize,
    new_bytes: usize,
}

impl Summary {
    const ZERO: Self = Self {
        complete: false,
        flags: 0,
        event_count: 0,
        allocation_calls: 0,
        deallocation_calls: 0,
        reallocation_calls: 0,
        failed_reallocations: 0,
        foreign_thread_calls: 0,
        returned_live_bytes: 0,
        dropped_live_bytes: 0,
        combined_peak_bytes: 0,
        retained_peak_bytes: 0,
        transient_peak_bytes: 0,
        retained_final_bytes: 0,
        transient_final_bytes: 0,
    };
}

// SAFETY: all ledger access is serialized by CAPTURE_CLAIM. During a capture,
// only the tagged owner thread mutates it; foreign hooks touch atomics only.
// finish/drop disable capture and drain in-flight allocator hooks before
// releasing the claim, so the next capture cannot alias a ledger access.
struct SharedLedger(UnsafeCell<Ledger>);
unsafe impl Sync for SharedLedger {}

static LEDGER: SharedLedger = SharedLedger(UnsafeCell::new(Ledger::new()));
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CAPTURE_CLAIM: AtomicBool = AtomicBool::new(false);
static ACTIVE_HOOKS: AtomicUsize = AtomicUsize::new(0);
static OWNER_TAG: AtomicUsize = AtomicUsize::new(0);
static FOREIGN_CALLS: AtomicU64 = AtomicU64::new(0);
static INCOMPLETE_FLAGS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static THREAD_TAG: Cell<u8> = const { Cell::new(0) };
}

fn current_thread_tag() -> Option<usize> {
    THREAD_TAG
        .try_with(|tag| tag as *const Cell<u8> as usize)
        .ok()
}

struct ProbeAllocator;

#[global_allocator]
static GLOBAL: ProbeAllocator = ProbeAllocator;

unsafe impl GlobalAlloc for ProbeAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let hook = AllocatorHook::enter();
        let pointer = unsafe { System.alloc(layout) };
        if hook.active {
            observe_allocation(pointer, layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let hook = AllocatorHook::enter();
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if hook.active {
            observe_allocation(pointer, layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let hook = AllocatorHook::enter();
        if hook.active {
            observe_deallocation(pointer);
        }
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let hook = AllocatorHook::enter();
        let next = unsafe { System.realloc(pointer, layout, new_size) };
        if hook.active {
            observe_reallocation(pointer, next, new_size);
        }
        next
    }
}

struct AllocatorHook {
    active: bool,
    counted: bool,
}

impl AllocatorHook {
    fn enter() -> Self {
        let counted = ACTIVE_HOOKS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_add(1)
            })
            .is_ok();
        if !counted {
            INCOMPLETE_FLAGS.fetch_or(u64::from(FLAG_ARITHMETIC), Ordering::Relaxed);
        }
        Self {
            active: counted && ACTIVE.load(Ordering::SeqCst),
            counted,
        }
    }
}

impl Drop for AllocatorHook {
    fn drop(&mut self) {
        if self.counted {
            ACTIVE_HOOKS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn observe_owner_thread() -> bool {
    let tag = current_thread_tag();
    if tag.is_some() && tag == Some(OWNER_TAG.load(Ordering::Relaxed)) {
        true
    } else {
        if FOREIGN_CALLS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_add(1)
            })
            .is_err()
        {
            INCOMPLETE_FLAGS.fetch_or(u64::from(FLAG_ARITHMETIC), Ordering::Relaxed);
        }
        if tag.is_none() {
            INCOMPLETE_FLAGS.fetch_or(u64::from(FLAG_UNKNOWN_POINTER), Ordering::Relaxed);
        }
        false
    }
}

fn observe_allocation(pointer: *mut u8, bytes: usize) {
    if !observe_owner_thread() {
        return;
    }
    if pointer.is_null() {
        INCOMPLETE_FLAGS.fetch_or(u64::from(FLAG_ALLOC_FAILURE), Ordering::Relaxed);
        return;
    }
    if bytes == 0 {
        return;
    }
    unsafe { (&mut *LEDGER.0.get()).allocate(pointer as usize, bytes) };
}

fn observe_deallocation(pointer: *mut u8) {
    if !observe_owner_thread() {
        return;
    }
    unsafe { (&mut *LEDGER.0.get()).deallocate(pointer as usize) };
}

fn observe_reallocation(old_pointer: *mut u8, new_pointer: *mut u8, new_bytes: usize) {
    if !observe_owner_thread() {
        return;
    }
    let new_pointer = if new_pointer.is_null() {
        None
    } else {
        Some(new_pointer as usize)
    };
    unsafe { (&mut *LEDGER.0.get()).reallocate(old_pointer as usize, new_pointer, new_bytes) };
}

struct Capture {
    active: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl Capture {
    fn begin() -> Result<Self, &'static str> {
        if CAPTURE_CLAIM
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("another capture owns the fixed ledger");
        }
        let Some(tag) = current_thread_tag() else {
            CAPTURE_CLAIM.store(false, Ordering::Release);
            return Err("thread-local tag unavailable before capture");
        };
        ACTIVE.store(false, Ordering::SeqCst);
        OWNER_TAG.store(0, Ordering::Relaxed);
        unsafe { (&mut *LEDGER.0.get()).reset() };
        FOREIGN_CALLS.store(0, Ordering::Relaxed);
        INCOMPLETE_FLAGS.store(0, Ordering::Relaxed);
        OWNER_TAG.store(tag, Ordering::Relaxed);
        ACTIVE.store(true, Ordering::SeqCst);
        Ok(Self {
            active: true,
            _not_send: PhantomData,
        })
    }

    fn returned_boundary(&self) -> Snapshot {
        unsafe { (&*LEDGER.0.get()).snapshot() }
    }

    fn finish(mut self, returned: &Snapshot) -> (Summary, Vec<EventReport>) {
        ACTIVE.store(false, Ordering::SeqCst);
        while ACTIVE_HOOKS.load(Ordering::SeqCst) != 0 {
            std::hint::spin_loop();
        }
        let foreign = FOREIGN_CALLS.load(Ordering::Relaxed);
        let flags = INCOMPLETE_FLAGS.load(Ordering::Relaxed) as u32;
        let (mut summary, events) = {
            let ledger = unsafe { &*LEDGER.0.get() };
            (ledger.summary(returned, foreign), ledger.sanitized_events())
        };
        summary.flags |= flags;
        summary.complete = summary.flags == 0 && foreign == 0;
        OWNER_TAG.store(0, Ordering::Relaxed);
        self.active = false;
        CAPTURE_CLAIM.store(false, Ordering::Release);
        (summary, events)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.active {
            ACTIVE.store(false, Ordering::SeqCst);
            while ACTIVE_HOOKS.load(Ordering::SeqCst) != 0 {
                std::hint::spin_loop();
            }
            OWNER_TAG.store(0, Ordering::Relaxed);
            CAPTURE_CLAIM.store(false, Ordering::Release);
        }
    }
}

#[derive(Serialize)]
struct FixtureReport {
    game: String,
    game_code_id: String,
    build_id: String,
    frame_format_version: u32,
    records: usize,
    manifest_bytes: usize,
    cooked_bytes: usize,
    decoded_bytes: usize,
    frame_count: usize,
    serialized_oram_bytes: usize,
    result_limit_bytes: usize,
    scope: &'static str,
    input_capacities: InputCapacities,
    sha256: FixtureHashes,
}

#[derive(Serialize)]
struct InputCapacities {
    manifest_capacity_bytes: usize,
    record_capacity_elements: usize,
    pcm_object_capacity_elements: usize,
    pcm_payload_capacity_bytes: usize,
}

#[derive(Serialize)]
struct FixtureHashes {
    sim_manifest: String,
    sim_object: String,
    view_manifest: String,
    pcm_objects: Vec<String>,
}

#[derive(Serialize)]
struct ProbeRecord {
    protocol: &'static str,
    case: &'static str,
    status: &'static str,
    result: &'static str,
    expected_result_matched: bool,
    conversion_callbacks: usize,
    fixture: FixtureReport,
    started: BoundaryReport,
    returned: BoundaryReport,
    dropped: BoundaryReport,
    allocations: SummaryReport,
    ordered_events: Vec<EventReport>,
}

#[derive(Serialize)]
struct BoundaryReport {
    event_index: Option<usize>,
    live_bytes: Option<usize>,
    live_allocations: Option<usize>,
    live_lifetime_ids: Option<Vec<u64>>,
}

fn run_probe_case(case: &'static str, frames: &[usize], expect_success: bool) {
    with_prepared_package(frames, |fixture, bundle, fixture_report| {
        let mut conversion_callbacks = 0usize;
        let capture = Capture::begin().expect("exclusive allocator observer");
        let preload = DecodeProbeBank::preload(fixture, bundle, || {
            conversion_callbacks = conversion_callbacks.saturating_add(1);
        });
        let (preload_succeeded, budget_exceeded, stats, returned, dropped) = match preload {
            Ok(bank) => {
                let stats = bank.stats();
                let returned = capture.returned_boundary();
                drop(bank);
                let dropped = capture.returned_boundary();
                (true, false, Some(stats), returned, dropped)
            }
            Err(error) => {
                let is_budget_exceeded = matches!(&error, orr_asset_fixture::Error::BudgetExceeded);
                drop(error);
                let returned = capture.returned_boundary();
                let dropped = capture.returned_boundary();
                (false, is_budget_exceeded, None, returned, dropped)
            }
        };
        let (allocations, ordered_events) = capture.finish(&returned);
        let (result, records, manifest_bytes, cooked_bytes, decoded_bytes) = match stats {
            Some(stats) => (
                "ok",
                stats.records,
                stats.manifest_bytes,
                stats.cooked_bytes,
                stats.decoded_bytes,
            ),
            None => (
                if budget_exceeded {
                    "BudgetExceeded"
                } else {
                    "unexpected_error"
                },
                fixture_report.records,
                fixture_report.manifest_bytes,
                fixture_report.cooked_bytes,
                fixture_report.decoded_bytes,
            ),
        };
        let fixture_report = FixtureReport {
            records,
            manifest_bytes,
            cooked_bytes,
            decoded_bytes,
            ..fixture_report
        };
        let expected_result_matched =
            preload_succeeded == expect_success && (expect_success || budget_exceeded);
        let exact_metrics = if expect_success {
            conversion_callbacks == 11
                && records == 11
                && manifest_bytes == 632
                && cooked_bytes == 1_048_664
                && decoded_bytes == MAX_DECODED_BYTES
                && allocations.returned_live_bytes > 0
                && allocations.dropped_live_bytes == 0
                && dropped.returned_live_bytes == 0
                && dropped.retained_count == 0
        } else {
            conversion_callbacks == 0
                && records == 11
                && manifest_bytes == 632
                && cooked_bytes == 1_048_666
                && decoded_bytes == MAX_DECODED_BYTES + STEREO_FRAME_BYTES
                && allocations.returned_live_bytes == 0
                && allocations.dropped_live_bytes == 0
                && dropped.returned_live_bytes == 0
                && dropped.retained_count == 0
        };
        let boundaries_match = returned.event_index <= dropped.event_index
            && dropped.event_index == allocations.event_count
            && dropped.returned_live_bytes == allocations.dropped_live_bytes;
        let status =
            if allocations.complete && expected_result_matched && exact_metrics && boundaries_match
            {
                "passed"
            } else {
                "incomplete_or_failed"
            };
        let record = ProbeRecord {
            protocol: "orr.decode-allocation/1",
            case,
            status,
            result,
            expected_result_matched,
            conversion_callbacks,
            fixture: fixture_report,
            started: BoundaryReport {
                event_index: allocations.complete.then_some(0),
                live_bytes: allocations.complete.then_some(0),
                live_allocations: allocations.complete.then_some(0),
                live_lifetime_ids: allocations.complete.then(Vec::new),
            },
            returned: BoundaryReport {
                event_index: allocations.complete.then_some(returned.event_index),
                live_bytes: allocations.complete.then_some(returned.returned_live_bytes),
                live_allocations: allocations.complete.then_some(returned.retained_count),
                live_lifetime_ids: allocations
                    .complete
                    .then(|| returned.retained_ids[..returned.retained_count].to_vec()),
            },
            dropped: BoundaryReport {
                event_index: allocations.complete.then_some(dropped.event_index),
                live_bytes: allocations.complete.then_some(dropped.returned_live_bytes),
                live_allocations: allocations.complete.then_some(dropped.retained_count),
                live_lifetime_ids: allocations
                    .complete
                    .then(|| dropped.retained_ids[..dropped.retained_count].to_vec()),
            },
            allocations: allocations.into(),
            ordered_events,
        };
        let output_within_cap = emit_record(&record).expect("write probe result to stdout");
        assert!(output_within_cap, "probe JSON exceeded the output cap");

        assert!(allocations.complete, "allocator accounting was incomplete");
        assert!(expected_result_matched, "unexpected preload result");
        assert!(
            exact_metrics && boundaries_match,
            "preload stats or lifetime bounds changed"
        );
        assert_eq!(
            allocations.dropped_live_bytes, 0,
            "tracked allocations survived bank drop"
        );
        assert_eq!(
            allocations.retained_final_bytes, 0,
            "retained allocations survived bank drop"
        );
        assert_eq!(
            allocations.transient_final_bytes, 0,
            "transient allocations survived preload"
        );
        if expect_success {
            assert_eq!(
                conversion_callbacks, 11,
                "all admitted records must convert"
            );
            assert_eq!(records, 11);
            assert_eq!(manifest_bytes, 632);
            assert_eq!(cooked_bytes, 1_048_664);
            assert_eq!(decoded_bytes, MAX_DECODED_BYTES);
            assert_eq!(
                allocations.returned_live_bytes,
                returned.returned_live_bytes
            );
            assert!(allocations.retained_peak_bytes > 0);
        } else {
            assert_eq!(
                conversion_callbacks, 0,
                "over-budget input must reject before conversion"
            );
            assert_eq!(records, 11);
            assert_eq!(manifest_bytes, 632);
            assert_eq!(cooked_bytes, 1_048_666);
            assert_eq!(decoded_bytes, MAX_DECODED_BYTES + STEREO_FRAME_BYTES);
            assert_eq!(allocations.returned_live_bytes, 0);
        }
    });
}

fn emit_record(record: &ProbeRecord) -> std::io::Result<bool> {
    let mut json = serde_json::to_vec(record).expect("bounded probe JSON serialization");
    let full_len = PREFIX
        .len()
        .checked_add(json.len())
        .and_then(|length| length.checked_add(1));
    if full_len.is_none_or(|length| length > RESULT_CAP) {
        json = b"{\"protocol\":\"orr.decode-allocation/1\",\"status\":\"incomplete_or_failed\",\"flags\":\"result_over_1MiB\",\"numeric_metrics\":null}".to_vec();
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(PREFIX)?;
        stdout.write_all(&json)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        return Ok(false);
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(PREFIX)?;
    stdout.write_all(&json)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(true)
}

fn with_prepared_package(
    frames: &[usize],
    run: impl FnOnce(&PreparedFixture, ViewBundleBytes<'_>, FixtureReport),
) {
    assert!(!frames.is_empty());
    let baseline = PreparedFixture::embedded().expect("checked-in release fixture");
    let mut pcm_payloads = Vec::<Vec<u8>>::with_capacity(frames.len());
    for (record_index, &frame_count) in frames.iter().enumerate() {
        assert!((1..=PCM_MAX_FRAMES).contains(&frame_count));
        pcm_payloads.push(make_pcm(
            frame_count,
            i16::try_from(record_index + 1).expect("eleven records fit i16"),
        ));
    }

    let entries: Vec<_> = frames
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let payload = &pcm_payloads[index];
            ManifestEntry {
                id: AssetRef::from_raw(
                    IMPACT_ID
                        .get()
                        .checked_add(u64::try_from(index).expect("bounded record index"))
                        .expect("bounded asset ID"),
                ),
                type_id: orr_asset::IMPACT_PCM16_TYPE_ID,
                schema_version: 1,
                payload_len: u64::try_from(payload.len()).expect("bounded PCM object"),
                payload_sha256: sha256(payload),
            }
        })
        .collect();
    let mut view_manifest =
        vec![0; manifest_encoded_len(Domain::View, entries.len()).expect("bounded records")];
    encode_manifest(Domain::View, &entries, &mut view_manifest)
        .expect("canonical fixture manifest");
    let view_objects: Vec<_> = pcm_payloads
        .iter()
        .map(|bytes| ArtifactBytes {
            sha256: sha256(bytes),
            bytes,
        })
        .collect();
    let mut full_objects = Vec::with_capacity(view_objects.len() + 1);
    full_objects.push(ArtifactBytes {
        sha256: sha256(SIM_OBJECT),
        bytes: SIM_OBJECT,
    });
    full_objects.extend(view_objects.iter().copied());

    let release_json = baseline.release().to_json().expect("release JSON");
    let mut release_value: serde_json::Value =
        serde_json::from_slice(&release_json).expect("release schema");
    release_value["view_manifest_sha256"] = serde_json::Value::String(hex(&sha256(&view_manifest)));
    let release_json = serde_json::to_vec(&release_value).expect("custom release JSON");
    let binding = ReleaseBinding::parse(&release_json).expect("custom view binding");
    let sim_manifest = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fixture_v1/cooked/sim.manifest.bin"
    ));
    let fixture = PreparedFixture::prepare(
        BundleBytes {
            sim_manifest,
            view_manifest: &view_manifest,
            objects: &full_objects,
        },
        &binding,
    )
    .expect("strict fixture preparation before allocation observation");
    let stats = FixtureReport {
        game: fixture.release().game().to_owned(),
        game_code_id: fixture.release().game_code_id().to_string(),
        build_id: fixture.release().build_id().to_string(),
        frame_format_version: orr_ecs::FRAME_FORMAT_VERSION,
        records: frames.len(),
        manifest_bytes: view_manifest.len(),
        cooked_bytes: entries
            .iter()
            .map(|entry| usize::try_from(entry.payload_len).expect("bounded payload"))
            .try_fold(0usize, usize::checked_add)
            .expect("bounded cooked bytes"),
        decoded_bytes: frames
            .iter()
            .try_fold(0usize, |total, frames| total.checked_add(*frames))
            .and_then(|frames| frames.checked_mul(STEREO_FRAME_BYTES))
            .expect("bounded decoded bytes"),
        frame_count: frames
            .iter()
            .try_fold(0usize, |total, frames| total.checked_add(*frames))
            .expect("bounded frame count"),
        serialized_oram_bytes: view_manifest.len(),
        result_limit_bytes: RESULT_CAP,
        scope: "single-thread real clip-bank preload through returned-bank boundary and explicit bank drop; no mixer/device",
        input_capacities: InputCapacities {
            manifest_capacity_bytes: view_manifest.capacity(),
            record_capacity_elements: entries.capacity(),
            pcm_object_capacity_elements: view_objects.capacity(),
            pcm_payload_capacity_bytes: pcm_payloads
                .iter()
                .map(Vec::capacity)
                .try_fold(0usize, usize::checked_add)
                .expect("bounded input capacity"),
        },
        sha256: FixtureHashes {
            sim_manifest: hex(&sha256(sim_manifest)),
            sim_object: hex(&sha256(SIM_OBJECT)),
            view_manifest: hex(&sha256(&view_manifest)),
            pcm_objects: pcm_payloads.iter().map(|payload| hex(&sha256(payload))).collect(),
        },
    };
    drop(baseline);
    run(
        &fixture,
        ViewBundleBytes {
            manifest: &view_manifest,
            objects: &view_objects,
        },
        stats,
    );
}

fn make_pcm(frames: usize, sample_value: i16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + frames * 2);
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(frames)
            .expect("PCM frame limit fits u32")
            .to_le_bytes(),
    );
    for _ in 0..frames {
        bytes.extend_from_slice(&sample_value.to_le_bytes());
    }
    bytes
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        let byte = *byte;
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[test]
#[ignore = "run only as the approved exact maximum decode-memory probe"]
fn max_decode_memory_case() {
    let mut frames = vec![PCM_MAX_FRAMES; 10];
    frames.push(MAX_DECODED_BYTES / STEREO_FRAME_BYTES - 480_000);
    run_probe_case("max_decode_memory_case", &frames, true);
}

#[test]
#[ignore = "run only as the approved exact over-budget decode-memory probe"]
fn over_budget_decode_memory_case() {
    let mut frames = vec![PCM_MAX_FRAMES; 10];
    frames.push(MAX_DECODED_BYTES / STEREO_FRAME_BYTES - 480_000 + 1);
    run_probe_case("over_budget_decode_memory_case", &frames, false);
}

#[test]
fn synthetic_ledger_separates_retained_transient_and_combined_peaks() {
    let mut ledger = Ledger::new();
    ledger.allocate(0x10, 100);
    ledger.allocate(0x20, 80);
    ledger.deallocate(0x20);
    let returned = ledger.snapshot();
    ledger.reallocate(0x10, Some(0x30), 150);
    ledger.allocate(0x40, 80);
    ledger.deallocate(0x40);
    ledger.deallocate(0x30);
    let summary = ledger.summary(&returned, 0);

    assert_eq!(summary.retained_peak_bytes, 100);
    assert_eq!(summary.transient_peak_bytes, 80);
    assert_eq!(summary.combined_peak_bytes, 180);
    assert_eq!(summary.returned_live_bytes, 100);
    assert_eq!(summary.dropped_live_bytes, 0);
    assert!(summary.complete);
}

#[test]
fn synthetic_realloc_failure_and_pointer_reuse_keep_lifetimes_distinct() {
    let mut ledger = Ledger::new();
    ledger.allocate(0x10, 64);
    let original_id = ledger.live[ledger.live_slot(0x10).unwrap()].id;
    ledger.reallocate(0x10, None, 128);
    ledger.reallocate(0x10, Some(0x20), 128);
    let moved_id = ledger.live[ledger.live_slot(0x20).unwrap()].id;
    assert_eq!(original_id, moved_id);
    ledger.deallocate(0x20);
    ledger.allocate(0x20, 32);
    ledger.reallocate(0x20, Some(0x20), 40);
    let reused_id = ledger.live[ledger.live_slot(0x20).unwrap()].id;
    assert_ne!(original_id, reused_id);
    let returned = ledger.snapshot();
    ledger.deallocate(0x20);
    ledger.allocate(0x30, 16);
    ledger.reallocate(0x30, None, 0);
    let summary = ledger.summary(&returned, 0);

    assert_eq!(summary.failed_reallocations, 1);
    assert_eq!(summary.reallocation_calls, 4);
    assert_eq!(summary.returned_live_bytes, 40);
    assert_eq!(summary.dropped_live_bytes, 0);
    assert!(summary.complete);
}

#[test]
fn synthetic_unknown_pointer_and_capacity_overflow_are_incomplete() {
    let mut unknown = Ledger::new();
    unknown.deallocate(0xdead);
    assert!(!unknown.summary(&unknown.snapshot(), 0).complete);

    let mut foreign = Ledger::new();
    foreign.allocate(0x101, 16);
    assert!(!foreign.summary(&foreign.snapshot(), 1).complete);

    let mut id_overflow = Ledger::new();
    id_overflow.next_id = u64::MAX;
    id_overflow.allocate(0x102, 1);
    assert!(!id_overflow.summary(&id_overflow.snapshot(), 0).complete);

    let mut overflow = Ledger::new();
    for index in 0..=EVENT_CAPACITY {
        overflow.allocate(0x1000 + index, 1);
    }
    assert!(!overflow.summary(&overflow.snapshot(), 0).complete);

    let mut arithmetic = Ledger::new();
    arithmetic.allocate(0x2000, usize::MAX);
    arithmetic.allocate(0x2001, 1);
    assert!(!arithmetic.summary(&arithmetic.snapshot(), 0).complete);
}
