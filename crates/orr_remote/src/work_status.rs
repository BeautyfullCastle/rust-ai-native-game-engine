//! Bounded, read-only observations of one host-paired main editor.
//!
//! This is not a close permit, a save receipt, or a coherent cross-thread fence.
//! The sole editor owner replaces a fixed-size mailbox after completed frames;
//! reads never request work from the UI. Binding IDs are process-local and must
//! be discarded on ERP reconnect or MCP reinitialization.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;

use serde_json::{json, Value as J};

const DEFAULT_MAX_AGE_MS: u64 = 1_000;
const MAX_AGE_MS: u64 = 5_000;
const MAX_DOMAIN_RECORDS: usize = 32;
const MAX_RESPONSE_BYTES: usize = 16 * 1_024;
const MAX_BLOCKERS: usize = 18;
const MAX_REASONS: usize = 7;
const SCHEMA_VERSION: u64 = 1;
const SCOPE: &str = "known_main_editor_authoring_domains_v1";
const TARGET: &str = "host_paired_local_editor";
const CONSISTENCY: &str = "host_at_request_local_at_published_frame";
const BINDING_SCOPE: &str = "process_local_live_host_connection";
const RECONNECT_POLICY: &str = "discard_prior_observations_on_reconnect_or_mcp_reinitialize";
const PREVIEW_REVISION_SCOPE: &str = "preview_only_non_persistent_non_monotonic_save_identity";
static NEXT_BINDING_ID: AtomicU64 = AtomicU64::new(1);

/// The complete, fixed v1 inventory. Other windows and authoring apps are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainId {
    SpriteBindings,
    Tilemap,
    CollectUiDocument,
    CollectUiPreview,
    ModelBindings,
    AnimationBindings,
    Terrain,
    IrradianceBindings,
    RoomCamera,
    MeshGeometryArt,
    MeshCamera,
    MeshLighting,
    MeshHud,
    MeshMission,
    MeshHazards,
    MeshAnimation,
}

impl DomainId {
    pub const ALL: [Self; 16] = [
        Self::SpriteBindings,
        Self::Tilemap,
        Self::CollectUiDocument,
        Self::CollectUiPreview,
        Self::ModelBindings,
        Self::AnimationBindings,
        Self::Terrain,
        Self::IrradianceBindings,
        Self::RoomCamera,
        Self::MeshGeometryArt,
        Self::MeshCamera,
        Self::MeshLighting,
        Self::MeshHud,
        Self::MeshMission,
        Self::MeshHazards,
        Self::MeshAnimation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpriteBindings => "sprite_bindings",
            Self::Tilemap => "tilemap",
            Self::CollectUiDocument => "collect_ui_document",
            Self::CollectUiPreview => "collect_ui_preview",
            Self::ModelBindings => "model_bindings",
            Self::AnimationBindings => "animation_bindings",
            Self::Terrain => "terrain",
            Self::IrradianceBindings => "irradiance_bindings",
            Self::RoomCamera => "room_camera",
            Self::MeshGeometryArt => "mesh_geometry_art",
            Self::MeshCamera => "mesh_camera",
            Self::MeshLighting => "mesh_lighting",
            Self::MeshHud => "mesh_hud",
            Self::MeshMission => "mesh_mission",
            Self::MeshHazards => "mesh_hazards",
            Self::MeshAnimation => "mesh_animation",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    const fn blocked_state(self) -> WorkState {
        match self {
            Self::CollectUiPreview => WorkState::StagedOrUnfinished,
            Self::RoomCamera
            | Self::MeshCamera
            | Self::MeshLighting
            | Self::MeshHud
            | Self::MeshAnimation => WorkState::UnsavedOrUnapplied,
            Self::MeshGeometryArt => WorkState::UnsavedOrUnappliedOrUnfinished,
            Self::MeshMission | Self::MeshHazards => WorkState::UnappliedOrUnfinished,
            _ => WorkState::Unsaved,
        }
    }

    const fn persistence_scope(self) -> &'static str {
        match self {
            Self::CollectUiDocument | Self::Terrain => "independent_document",
            Self::CollectUiPreview => "applies_to_collect_ui_document",
            Self::MeshGeometryArt => "mixed_scene_and_sidecars",
            Self::MeshMission | Self::MeshHazards => "applies_to_scene",
            _ => "independent_sidecar",
        }
    }
}

/// Compiled support and retained in-memory presence, not a claim about disk files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    FeatureDisabled,
    NotLoaded,
    Loaded,
    Unknown,
}

impl Presence {
    const ALL: [Self; 4] = [
        Self::FeatureDisabled,
        Self::NotLoaded,
        Self::Loaded,
        Self::Unknown,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::FeatureDisabled => "feature_disabled",
            Self::NotLoaded => "not_loaded",
            Self::Loaded => "loaded",
            Self::Unknown => "unknown",
        }
    }
}

/// The semantics of the reviewed predicate, without inventing finer distinctions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    NotApplicable,
    Clean,
    Unsaved,
    StagedOrUnfinished,
    UnsavedOrUnapplied,
    UnsavedOrUnappliedOrUnfinished,
    UnappliedOrUnfinished,
    Unknown,
}

impl WorkState {
    const ALL: [Self; 8] = [
        Self::NotApplicable,
        Self::Clean,
        Self::Unsaved,
        Self::StagedOrUnfinished,
        Self::UnsavedOrUnapplied,
        Self::UnsavedOrUnappliedOrUnfinished,
        Self::UnappliedOrUnfinished,
        Self::Unknown,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::Clean => "clean",
            Self::Unsaved => "unsaved",
            Self::StagedOrUnfinished => "staged_or_unfinished",
            Self::UnsavedOrUnapplied => "unsaved_or_unapplied",
            Self::UnsavedOrUnappliedOrUnfinished => "unsaved_or_unapplied_or_unfinished",
            Self::UnappliedOrUnfinished => "unapplied_or_unfinished",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainRecord {
    pub domain_id: DomainId,
    pub presence: Presence,
    pub work_state: WorkState,
}

impl DomainRecord {
    pub const fn disabled(domain_id: DomainId) -> Self {
        Self {
            domain_id,
            presence: Presence::FeatureDisabled,
            work_state: WorkState::NotApplicable,
        }
    }

    pub const fn not_loaded(domain_id: DomainId) -> Self {
        Self {
            domain_id,
            presence: Presence::NotLoaded,
            work_state: WorkState::NotApplicable,
        }
    }

    pub const fn observed(domain_id: DomainId, blocked: bool) -> Self {
        Self {
            domain_id,
            presence: Presence::Loaded,
            work_state: if blocked {
                domain_id.blocked_state()
            } else {
                WorkState::Clean
            },
        }
    }

    pub const fn unknown(domain_id: DomainId) -> Self {
        Self {
            domain_id,
            presence: Presence::Unknown,
            work_state: WorkState::Unknown,
        }
    }

    pub fn is_blocker(self) -> bool {
        self.presence == Presence::Loaded && self.work_state == self.domain_id.blocked_state()
    }

    fn valid(self) -> bool {
        match self.presence {
            Presence::FeatureDisabled | Presence::NotLoaded => {
                self.work_state == WorkState::NotApplicable
            }
            Presence::Loaded => {
                self.work_state == WorkState::Clean
                    || self.work_state == self.domain_id.blocked_state()
                    || self.work_state == WorkState::Unknown
            }
            Presence::Unknown => self.work_state == WorkState::Unknown,
        }
    }

    fn known(self) -> bool {
        self.presence != Presence::Unknown && self.work_state != WorkState::Unknown
    }

    fn to_json(self) -> J {
        json!({
            "domain_id": self.domain_id.as_str(),
            "owner": "editor_local",
            "presence": self.presence.as_str(),
            "work_state": self.work_state.as_str(),
            "persistence_scope": self.domain_id.persistence_scope(),
            "domain_revision": null,
        })
    }
}

/// Only authoring mutation uncertainty belongs here; pending reads are not work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operations {
    Idle,
    PendingMutation,
    OutcomeUncertain,
    Unknown,
}

impl Operations {
    const ALL: [Self; 4] = [
        Self::Idle,
        Self::PendingMutation,
        Self::OutcomeUncertain,
        Self::Unknown,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::PendingMutation => "pending_mutation",
            Self::OutcomeUncertain => "outcome_uncertain",
            Self::Unknown => "unknown",
        }
    }

    const fn reason(self) -> Option<Reason> {
        match self {
            Self::Idle => None,
            Self::PendingMutation => Some(Reason::PendingMutation),
            Self::OutcomeUncertain => Some(Reason::OutcomeUncertain),
            Self::Unknown => Some(Reason::OperationsUnknown),
        }
    }
}

#[derive(Debug)]
pub struct EditorSnapshot {
    pub domains: Vec<DomainRecord>,
    pub operations: Operations,
}

/// Read together from the authoritative host document at request handling time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostScene {
    pub dirty: bool,
    pub in_tx: bool,
    pub preview_revision: u64,
    pub doc_checksum: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    NotPaired,
    NotReady,
    OwnerAbsent,
    Busy,
    Stale,
    Incomplete,
    MissingDomain,
    DuplicateDomain,
    UnsupportedDomain,
    InvalidDomainRecord,
    DomainLimitExceeded,
    CounterExhausted,
    MailboxPoisoned,
    PendingMutation,
    OutcomeUncertain,
    OperationsUnknown,
    ResponseLimitExceeded,
}

impl Reason {
    const ALL: [Self; 17] = [
        Self::NotPaired,
        Self::NotReady,
        Self::OwnerAbsent,
        Self::Busy,
        Self::Stale,
        Self::Incomplete,
        Self::MissingDomain,
        Self::DuplicateDomain,
        Self::UnsupportedDomain,
        Self::InvalidDomainRecord,
        Self::DomainLimitExceeded,
        Self::CounterExhausted,
        Self::MailboxPoisoned,
        Self::PendingMutation,
        Self::OutcomeUncertain,
        Self::OperationsUnknown,
        Self::ResponseLimitExceeded,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::NotPaired => "local_not_paired",
            Self::NotReady => "local_not_ready",
            Self::OwnerAbsent => "local_owner_absent",
            Self::Busy => "local_busy",
            Self::Stale => "local_stale",
            Self::Incomplete => "local_incomplete",
            Self::MissingDomain => "missing_domain",
            Self::DuplicateDomain => "duplicate_domain",
            Self::UnsupportedDomain => "unsupported_domain",
            Self::InvalidDomainRecord => "invalid_domain_record",
            Self::DomainLimitExceeded => "domain_limit_exceeded",
            Self::CounterExhausted => "counter_exhausted",
            Self::MailboxPoisoned => "mailbox_poisoned",
            Self::PendingMutation => "pending_mutation",
            Self::OutcomeUncertain => "outcome_uncertain",
            Self::OperationsUnknown => "operations_unknown",
            Self::ResponseLimitExceeded => "response_limit_exceeded",
        }
    }
}

// Typed, allocation-free coverage defects retained with the one latest frame.
#[derive(Clone, Copy, Default)]
struct Defects {
    missing: bool,
    duplicate: bool,
    unsupported: bool,
    invalid: bool,
    overflow: bool,
}

impl Defects {
    fn complete(self) -> bool {
        !(self.missing || self.duplicate || self.unsupported || self.invalid || self.overflow)
    }

    fn reasons(self) -> impl Iterator<Item = Reason> {
        [
            (self.missing, Reason::MissingDomain),
            (self.duplicate, Reason::DuplicateDomain),
            (self.unsupported, Reason::UnsupportedDomain),
            (self.invalid, Reason::InvalidDomainRecord),
            (self.overflow, Reason::DomainLimitExceeded),
        ]
        .into_iter()
        .filter_map(|(present, reason)| present.then_some(reason))
    }
}

#[derive(Clone, Copy)]
struct Snapshot {
    domains: [DomainRecord; 16],
    operations: Operations,
    defects: Defects,
    published: Instant,
    snapshot_seq: u64,
    ui_frame: u64,
}

impl Snapshot {
    // Validate/copy before acquiring the mailbox. No input-controlled strings,
    // heap objects or more than the fixed inventory survive publication.
    fn bounded(
        input: EditorSnapshot,
        published: Instant,
        snapshot_seq: u64,
        ui_frame: u64,
    ) -> Self {
        let mut snapshot = Self {
            domains: DomainId::ALL.map(DomainRecord::unknown),
            operations: input.operations,
            defects: Defects::default(),
            published,
            snapshot_seq,
            ui_frame,
        };
        if input.domains.len() > MAX_DOMAIN_RECORDS {
            snapshot.defects.overflow = true;
            snapshot.operations = Operations::Unknown;
            return snapshot;
        }
        let mut seen = [false; 16];
        for record in input.domains {
            let index = record.domain_id.index();
            if seen[index] {
                snapshot.domains[index] = DomainRecord::unknown(record.domain_id);
                snapshot.defects.duplicate = true;
            } else {
                seen[index] = true;
                if record.valid() {
                    snapshot.domains[index] = record;
                    snapshot.defects.unsupported |= !record.known();
                } else {
                    snapshot.defects.invalid = true;
                }
            }
        }
        snapshot.defects.missing = seen.contains(&false);
        snapshot
    }
}

struct Shared {
    binding_id: Option<u64>,
    owner_alive: AtomicBool,
    service_alive: AtomicBool,
    exhausted: AtomicBool,
    snapshot: Mutex<Option<Snapshot>>,
}

/// Non-clone host endpoint: attach it to exactly one local ERP host.
pub struct WorkStatusService {
    shared: Arc<Shared>,
}

/// Non-clone sole publisher. Drop invalidates it without acquiring a lock.
pub struct WorkStatusOwner {
    shared: Arc<Shared>,
    snapshot_seq: u64,
    ui_frame: u64,
}

fn allocate_binding(counter: &AtomicU64) -> Option<u64> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return None;
        }
        let next = current.checked_add(1)?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Some(current),
            Err(observed) => current = observed,
        }
    }
}

impl WorkStatusService {
    pub fn pair() -> (Self, WorkStatusOwner) {
        Self::pair_with_binding(allocate_binding(&NEXT_BINDING_ID))
    }

    fn pair_with_binding(binding_id: Option<u64>) -> (Self, WorkStatusOwner) {
        let shared = Arc::new(Shared {
            binding_id,
            owner_alive: AtomicBool::new(true),
            service_alive: AtomicBool::new(true),
            exhausted: AtomicBool::new(binding_id.is_none()),
            snapshot: Mutex::new(None),
        });
        (
            Self {
                shared: Arc::clone(&shared),
            },
            WorkStatusOwner {
                shared,
                snapshot_seq: 0,
                ui_frame: 0,
            },
        )
    }

    /// Current readiness only, with no binding identity or domain data.
    /// Permission filtering is the ERP server's responsibility before calling.
    pub fn discovery(&self) -> J {
        self.discovery_at(Instant::now())
    }

    fn discovery_at(&self, now: Instant) -> J {
        let observation = self.observe(now, DEFAULT_MAX_AGE_MS);
        json!({
            "supported": true,
            "availability": observation.availability.as_str(),
            "default_max_age_ms": DEFAULT_MAX_AGE_MS,
            "max_age_ms_limit": MAX_AGE_MS,
        })
    }

    fn observe(&self, now: Instant, max_age_ms: u64) -> Observation {
        if !self.shared.owner_alive.load(Ordering::Acquire) {
            return Observation::absent(Availability::OwnerAbsent, Reason::OwnerAbsent);
        }
        if self.shared.exhausted.load(Ordering::Acquire) {
            return Observation::absent(Availability::Incomplete, Reason::CounterExhausted);
        }
        // Copy only fixed-size typed data. The guard is dropped before any JSON
        // allocation, serialization, or elapsed-time calculation.
        let snapshot = match self.shared.snapshot.try_lock() {
            Ok(guard) => *guard,
            Err(TryLockError::WouldBlock) => {
                return Observation::absent(Availability::Busy, Reason::Busy)
            }
            Err(TryLockError::Poisoned(_)) => {
                return Observation::absent(Availability::Incomplete, Reason::MailboxPoisoned);
            }
        };
        // A drop concurrent with the copy invalidates even a previously clean
        // frame. This read linearizes at this liveness check, not at serialization.
        if !self.shared.owner_alive.load(Ordering::Acquire) {
            return Observation::absent(Availability::OwnerAbsent, Reason::OwnerAbsent);
        }
        if self.shared.exhausted.load(Ordering::Acquire) {
            return Observation::absent(Availability::Incomplete, Reason::CounterExhausted);
        }
        let Some(snapshot) = snapshot else {
            return Observation::absent(Availability::NotReady, Reason::NotReady);
        };
        // A caller's test clock going backwards cannot turn an observation fresh.
        let elapsed = now.checked_duration_since(snapshot.published);
        let age_ms = elapsed
            .map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        let fresh = elapsed.is_some_and(|age| age <= std::time::Duration::from_millis(max_age_ms));
        let availability = if !fresh {
            Availability::Stale
        } else if snapshot.defects.complete() {
            Availability::Ready
        } else {
            Availability::Incomplete
        };
        Observation {
            availability,
            reason: None,
            binding_id: self.shared.binding_id,
            snapshot: Some(snapshot),
            age_ms: Some(age_ms),
            fresh,
        }
    }
}

impl WorkStatusOwner {
    /// Replace the latest frame without waiting. A failed attempt does not
    /// refresh the previous frame's time or sequence. Incomplete input is
    /// published as an explicitly incomplete observation, never truncated clean.
    pub fn publish(&mut self, snapshot: EditorSnapshot) -> bool {
        self.publish_at(snapshot, Instant::now())
    }

    fn publish_at(&mut self, input: EditorSnapshot, now: Instant) -> bool {
        if !self.shared.service_alive.load(Ordering::Acquire)
            || self.shared.exhausted.load(Ordering::Acquire)
        {
            return false;
        }
        let (Some(snapshot_seq), Some(ui_frame)) = (
            self.snapshot_seq.checked_add(1),
            self.ui_frame.checked_add(1),
        ) else {
            self.shared.exhausted.store(true, Ordering::Release);
            return false;
        };
        self.ui_frame = ui_frame;
        let snapshot = Snapshot::bounded(input, now, snapshot_seq, ui_frame);
        let Ok(mut slot) = self.shared.snapshot.try_lock() else {
            return false;
        };
        *slot = Some(snapshot);
        self.snapshot_seq = snapshot_seq;
        true
    }
}

impl Drop for WorkStatusOwner {
    fn drop(&mut self) {
        // Do not contend with a host read, and never leave old payloads live.
        self.shared.owner_alive.store(false, Ordering::Release);
    }
}

impl Drop for WorkStatusService {
    fn drop(&mut self) {
        self.shared.service_alive.store(false, Ordering::Release);
    }
}

impl core::fmt::Debug for WorkStatusService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WorkStatusService").finish_non_exhaustive()
    }
}

impl core::fmt::Debug for WorkStatusOwner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WorkStatusOwner").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Availability {
    Ready,
    NotReady,
    Stale,
    OwnerAbsent,
    NotPaired,
    Busy,
    Incomplete,
}

impl Availability {
    const ALL: [Self; 7] = [
        Self::Ready,
        Self::NotReady,
        Self::Stale,
        Self::OwnerAbsent,
        Self::NotPaired,
        Self::Busy,
        Self::Incomplete,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotReady => "not_ready",
            Self::Stale => "stale",
            Self::OwnerAbsent => "owner_absent",
            Self::NotPaired => "not_paired",
            Self::Busy => "busy",
            Self::Incomplete => "incomplete",
        }
    }
}

struct Observation {
    availability: Availability,
    reason: Option<Reason>,
    binding_id: Option<u64>,
    snapshot: Option<Snapshot>,
    age_ms: Option<u64>,
    fresh: bool,
}

impl Observation {
    fn absent(availability: Availability, reason: Reason) -> Self {
        Self {
            availability,
            reason: Some(reason),
            binding_id: None,
            snapshot: None,
            age_ms: None,
            fresh: false,
        }
    }
}

/// Only the fixed optional age bound is accepted. Errors never echo arguments.
pub fn parse_max_age(params: &J) -> Result<u64, crate::RpcError> {
    const INVALID: &str =
        "editor.work_status expects only optional max_age_ms, an integer from 1 to 5000";
    let object = params
        .as_object()
        .ok_or_else(|| crate::RpcError::params(INVALID))?;
    if object.keys().any(|key| key != "max_age_ms") {
        return Err(crate::RpcError::params(INVALID));
    }
    match object.get("max_age_ms") {
        None => Ok(DEFAULT_MAX_AGE_MS),
        Some(value) => value
            .as_u64()
            .filter(|age| (1..=MAX_AGE_MS).contains(age))
            .ok_or_else(|| crate::RpcError::params(INVALID)),
    }
}

/// The caller must enforce Read capability before exposing any owner state.
/// Local data and the current host scene are explicitly separate observations.
pub fn response(service: Option<&WorkStatusService>, host: HostScene, max_age_ms: u64) -> J {
    response_at(service, host, max_age_ms, Instant::now())
}

fn response_at(
    service: Option<&WorkStatusService>,
    host: HostScene,
    max_age_ms: u64,
    now: Instant,
) -> J {
    // The RPC parser rejects invalid values; direct typed callers cannot expand
    // the wire's freshness window beyond its fixed bounds either.
    let max_age_ms = max_age_ms.clamp(1, MAX_AGE_MS);
    let observation = service.map_or_else(
        || Observation::absent(Availability::NotPaired, Reason::NotPaired),
        |service| service.observe(now, max_age_ms),
    );
    let result = render(observation, host, service.is_some(), max_age_ms);
    if serde_json::to_writer(SizeLimit(0), &result).is_ok() {
        result
    } else {
        // This cannot be reached by the fixed v1 inventory; keep the hard limit
        // fail-closed if a future schema extension grows beyond the budget.
        render(
            Observation::absent(Availability::Incomplete, Reason::ResponseLimitExceeded),
            host,
            service.is_some(),
            max_age_ms,
        )
    }
}

struct SizeLimit(usize);

impl Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_RESPONSE_BYTES)
            .ok_or_else(|| io::Error::other("work status response limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn render(observation: Observation, host: HostScene, paired: bool, max_age_ms: u64) -> J {
    let mut blockers = Vec::with_capacity(MAX_BLOCKERS);
    if host.dirty {
        blockers.push("host_scene_unsaved");
    }
    if host.in_tx {
        blockers.push("host_scene_transaction");
    }
    let mut reasons = Vec::with_capacity(MAX_REASONS);
    if let Some(reason) = observation.reason {
        reasons.push(reason.as_str());
    }
    let (coverage, data_age, operations, domains) = if let Some(snapshot) = observation.snapshot {
        if !observation.fresh {
            reasons.push(Reason::Stale.as_str());
        }
        if !snapshot.defects.complete() {
            reasons.push(Reason::Incomplete.as_str());
            reasons.extend(snapshot.defects.reasons().map(Reason::as_str));
        }
        if let Some(reason) = snapshot.operations.reason() {
            reasons.push(reason.as_str());
        }
        if observation.fresh {
            blockers.extend(
                snapshot
                    .domains
                    .iter()
                    .filter(|record| record.is_blocker())
                    .map(|record| record.domain_id.as_str()),
            );
        }
        (
            if snapshot.defects.complete() {
                "complete"
            } else {
                "incomplete"
            },
            if observation.fresh {
                "fresh"
            } else {
                "last_observed"
            },
            snapshot.operations,
            snapshot
                .domains
                .into_iter()
                .map(DomainRecord::to_json)
                .collect::<Vec<_>>(),
        )
    } else {
        (
            if observation.availability == Availability::Incomplete {
                "incomplete"
            } else {
                "unavailable"
            },
            "unavailable",
            Operations::Unknown,
            Vec::new(),
        )
    };
    let assessment = assessment_state(
        !blockers.is_empty(),
        observation.fresh,
        coverage == "complete",
        operations,
    );
    json!({
        "schema_version": SCHEMA_VERSION,
        "scope": SCOPE,
        "target": TARGET,
        "availability": observation.availability.as_str(),
        "observation": {
            "owner_binding_id": observation.binding_id.map(|id| id.to_string()),
            "snapshot_seq": observation.snapshot.map(|snapshot| snapshot.snapshot_seq.to_string()),
            "ui_frame": observation.snapshot.map(|snapshot| snapshot.ui_frame.to_string()),
            "age_ms": observation.age_ms,
            "max_age_ms": observation.snapshot.map(|_| max_age_ms),
            "binding_scope": BINDING_SCOPE,
            "reconnect_policy": RECONNECT_POLICY,
        },
        "consistency": CONSISTENCY,
        "host_scene": {
            "authority": "erp_host",
            "owner": if paired { "paired_host" } else { "host" },
            "dirty": host.dirty,
            "in_tx": host.in_tx,
            "preview_revision": host.preview_revision.to_string(),
            "preview_revision_scope": PREVIEW_REVISION_SCOPE,
            "doc_checksum": format!("0x{:016x}", host.doc_checksum),
        },
        "editor_local": {
            "owner": "editor_local",
            "coverage": coverage,
            "observation": data_age,
            "domains": domains,
        },
        "editor_operations": {
            "state": operations.as_str(),
            "reason": operations.reason().map(Reason::as_str),
            "observation": data_age,
        },
        "assessment": {
            "state": assessment,
            "blockers": blockers,
            "reasons": reasons,
        },
    })
}

fn assessment_state(
    blocked: bool,
    fresh: bool,
    complete: bool,
    operations: Operations,
) -> &'static str {
    if blocked {
        "known_blockers"
    } else if fresh && complete && operations == Operations::Idle {
        "none_observed"
    } else {
        "indeterminate"
    }
}

/// Validate an untrusted ERP success before forwarding it through another
/// transport. This accepts only the bounded v1 schema, not arbitrary metadata.
/// It does not establish that the remote host's reported facts are truthful.
///
/// Shape is checked before serialization so attacker-controlled nested objects
/// or oversized arrays are never traversed by the output-size check.
pub fn validate_response(value: &J) -> bool {
    let Some(root) = exact_keys(
        value,
        &[
            "schema_version",
            "scope",
            "target",
            "availability",
            "observation",
            "consistency",
            "host_scene",
            "editor_local",
            "editor_operations",
            "assessment",
        ],
    ) else {
        return false;
    };
    if root["schema_version"].as_u64() != Some(SCHEMA_VERSION)
        || root["scope"] != SCOPE
        || root["target"] != TARGET
        || root["consistency"] != CONSISTENCY
    {
        return false;
    }
    let Some(availability) = Availability::ALL
        .into_iter()
        .find(|candidate| root["availability"].as_str() == Some(candidate.as_str()))
    else {
        return false;
    };
    let Some(observed) = validate_observation(&root["observation"]) else {
        return false;
    };
    let Some(host) = exact_keys(
        &root["host_scene"],
        &[
            "authority",
            "owner",
            "dirty",
            "in_tx",
            "preview_revision",
            "preview_revision_scope",
            "doc_checksum",
        ],
    ) else {
        return false;
    };
    if host["authority"] != "erp_host"
        || host["owner"]
            != if availability == Availability::NotPaired {
                "host"
            } else {
                "paired_host"
            }
        || !host["dirty"].is_boolean()
        || !host["in_tx"].is_boolean()
        || !decimal_u64(&host["preview_revision"], false)
        || host["preview_revision_scope"] != PREVIEW_REVISION_SCOPE
        || !checksum(&host["doc_checksum"])
    {
        return false;
    }
    let Some(local) = exact_keys(
        &root["editor_local"],
        &["owner", "coverage", "observation", "domains"],
    ) else {
        return false;
    };
    if local["owner"] != "editor_local" {
        return false;
    }
    let Some(domains) = local["domains"].as_array() else {
        return false;
    };
    let Some(operations) = exact_keys(
        &root["editor_operations"],
        &["state", "reason", "observation"],
    ) else {
        return false;
    };
    let Some(operation) = Operations::ALL
        .into_iter()
        .find(|candidate| operations["state"].as_str() == Some(candidate.as_str()))
    else {
        return false;
    };
    if operations["observation"] != local["observation"]
        || match operation.reason() {
            Some(reason) => operations["reason"].as_str() != Some(reason.as_str()),
            None => !operations["reason"].is_null(),
        }
    {
        return false;
    }
    let mut expected_blockers = [""; MAX_BLOCKERS];
    let mut blocker_count = 0;
    if host["dirty"] == true {
        expected_blockers[blocker_count] = "host_scene_unsaved";
        blocker_count += 1;
    }
    if host["in_tx"] == true {
        expected_blockers[blocker_count] = "host_scene_transaction";
        blocker_count += 1;
    }
    let fresh;
    let complete;
    if observed {
        if domains.len() != DomainId::ALL.len() {
            return false;
        }
        fresh = availability != Availability::Stale;
        // Millisecond truncation permits stale age == max_age, but never a
        // fresh claim beyond the advertised bound. Shape validation above
        // guarantees both fields are u64 for a present observation.
        if fresh
            && root["observation"]["age_ms"].as_u64() > root["observation"]["max_age_ms"].as_u64()
        {
            return false;
        }
        let mut all_known = true;
        for (row, id) in domains.iter().zip(DomainId::ALL) {
            let Some(record) = validate_domain(row, id) else {
                return false;
            };
            all_known &= record.known();
            if fresh && record.is_blocker() {
                expected_blockers[blocker_count] = id.as_str();
                blocker_count += 1;
            }
        }
        complete = all_known;
        if local["coverage"] != if complete { "complete" } else { "incomplete" }
            || local["observation"] != if fresh { "fresh" } else { "last_observed" }
            || !matches!(
                (availability, complete),
                (Availability::Ready, true)
                    | (Availability::Incomplete, false)
                    | (Availability::Stale, _)
            )
        {
            return false;
        }
    } else {
        fresh = false;
        complete = false;
        if !domains.is_empty()
            || operation != Operations::Unknown
            || local["observation"] != "unavailable"
            || local["coverage"]
                != if availability == Availability::Incomplete {
                    "incomplete"
                } else {
                    "unavailable"
                }
            || matches!(availability, Availability::Ready | Availability::Stale)
        {
            return false;
        }
    }
    let Some(assessment) = exact_keys(&root["assessment"], &["state", "blockers", "reasons"])
    else {
        return false;
    };
    let Some(blockers) = assessment["blockers"].as_array() else {
        return false;
    };
    if blockers.len() != blocker_count
        || !blockers
            .iter()
            .zip(expected_blockers)
            .all(|(actual, expected)| actual.as_str() == Some(expected))
        || assessment["state"] != assessment_state(blocker_count != 0, fresh, complete, operation)
    {
        return false;
    }
    if !validate_reasons(
        &assessment["reasons"],
        availability,
        observed,
        complete,
        operation,
    ) {
        return false;
    }
    serde_json::to_writer(SizeLimit(0), value).is_ok()
}

fn validate_reasons(
    value: &J,
    availability: Availability,
    observed: bool,
    complete: bool,
    operation: Operations,
) -> bool {
    let allowed_reasons = Reason::ALL.map(Reason::as_str);
    if !unique_allowed_strings(value, &allowed_reasons, MAX_REASONS) {
        return false;
    }
    let Some(reasons) = value.as_array() else {
        return false;
    };
    if !observed {
        let [reason] = reasons.as_slice() else {
            return false;
        };
        return match availability {
            Availability::NotPaired => reason == Reason::NotPaired.as_str(),
            Availability::NotReady => reason == Reason::NotReady.as_str(),
            Availability::OwnerAbsent => reason == Reason::OwnerAbsent.as_str(),
            Availability::Busy => reason == Reason::Busy.as_str(),
            Availability::Incomplete => [
                Reason::CounterExhausted,
                Reason::MailboxPoisoned,
                Reason::ResponseLimitExceeded,
            ]
            .into_iter()
            .any(|expected| reason == expected.as_str()),
            Availability::Ready | Availability::Stale => false,
        };
    }
    let has = |reason: Reason| reasons.iter().any(|value| value == reason.as_str());
    // Normalized unknown rows do not retain individual defect provenance, but
    // defects must explain incomplete coverage and cannot qualify complete data.
    let defect_count = [
        Reason::MissingDomain,
        Reason::DuplicateDomain,
        Reason::UnsupportedDomain,
        Reason::InvalidDomainRecord,
        Reason::DomainLimitExceeded,
    ]
    .into_iter()
    .filter(|reason| has(*reason))
    .count();
    let stale = availability == Availability::Stale;
    has(Reason::Stale) == stale
        && has(Reason::Incomplete) != complete
        && (defect_count != 0) != complete
        && operation.reason().is_none_or(has)
        // Counting the required reasons excludes unrelated lifecycle/operation
        // reasons, and requires none_observed to have no limiting reasons.
        && reasons.len()
            == usize::from(stale)
                + usize::from(!complete)
                + usize::from(operation.reason().is_some())
                + defect_count
}

fn exact_keys<'a>(value: &'a J, keys: &[&str]) -> Option<&'a serde_json::Map<String, J>> {
    let object = value.as_object()?;
    (object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)))
        .then_some(object)
}

fn decimal_u64(value: &J, nonzero: bool) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    !text.is_empty()
        && text.len() <= 20
        && (text.len() == 1 || !text.starts_with('0'))
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && text
            .parse::<u64>()
            .is_ok_and(|number| !nonzero || number != 0)
}

fn checksum(value: &J) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 18
            && text.starts_with("0x")
            && text.as_bytes()[2..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    })
}

fn validate_observation(value: &J) -> Option<bool> {
    let object = exact_keys(
        value,
        &[
            "owner_binding_id",
            "snapshot_seq",
            "ui_frame",
            "age_ms",
            "max_age_ms",
            "binding_scope",
            "reconnect_policy",
        ],
    )?;
    if object["binding_scope"] != BINDING_SCOPE || object["reconnect_policy"] != RECONNECT_POLICY {
        return None;
    }
    let numeric_fields = [
        "owner_binding_id",
        "snapshot_seq",
        "ui_frame",
        "age_ms",
        "max_age_ms",
    ];
    if numeric_fields.iter().all(|key| object[*key].is_null()) {
        return Some(false);
    }
    (decimal_u64(&object["owner_binding_id"], true)
        && decimal_u64(&object["snapshot_seq"], true)
        && decimal_u64(&object["ui_frame"], true)
        && object["age_ms"].as_u64().is_some()
        && object["max_age_ms"]
            .as_u64()
            .is_some_and(|age| (1..=MAX_AGE_MS).contains(&age)))
    .then_some(true)
}

fn validate_domain(value: &J, domain_id: DomainId) -> Option<DomainRecord> {
    let object = exact_keys(
        value,
        &[
            "domain_id",
            "owner",
            "presence",
            "work_state",
            "persistence_scope",
            "domain_revision",
        ],
    )?;
    if object["domain_id"].as_str() != Some(domain_id.as_str())
        || object["owner"] != "editor_local"
        || object["persistence_scope"].as_str() != Some(domain_id.persistence_scope())
        || !object["domain_revision"].is_null()
    {
        return None;
    }
    let presence = Presence::ALL
        .into_iter()
        .find(|candidate| object["presence"].as_str() == Some(candidate.as_str()))?;
    let work_state = WorkState::ALL
        .into_iter()
        .find(|candidate| object["work_state"].as_str() == Some(candidate.as_str()))?;
    let record = DomainRecord {
        domain_id,
        presence,
        work_state,
    };
    record.valid().then_some(record)
}

fn unique_allowed_strings(value: &J, allowed: &[&str], maximum: usize) -> bool {
    let Some(values) = value.as_array().filter(|values| values.len() <= maximum) else {
        return false;
    };
    values.iter().enumerate().all(|(index, value)| {
        value.as_str().is_some_and(|text| allowed.contains(&text))
            && !values[..index].contains(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn clean() -> EditorSnapshot {
        EditorSnapshot {
            domains: DomainId::ALL
                .map(|id| DomainRecord::observed(id, false))
                .to_vec(),
            operations: Operations::Idle,
        }
    }

    fn host() -> HostScene {
        HostScene {
            dirty: false,
            in_tx: false,
            preview_revision: u64::MAX,
            doc_checksum: u64::MAX,
        }
    }

    fn later(now: Instant, millis: u64) -> Instant {
        now.checked_add(Duration::from_millis(millis)).unwrap()
    }

    fn at(service: Option<&WorkStatusService>, now: Instant) -> J {
        let value = response_at(service, host(), DEFAULT_MAX_AGE_MS, now);
        assert!(validate_response(&value));
        value
    }

    fn reason(value: &J, expected: &str) -> bool {
        value["assessment"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == expected)
    }

    #[test]
    fn absence_before_publication_and_drop_never_mean_clean() {
        let now = Instant::now();
        let absent = at(None, now);
        assert_eq!(absent["availability"], "not_paired");
        assert_eq!(absent["host_scene"]["owner"], "host");
        let (service, mut owner) = WorkStatusService::pair();
        let before = at(Some(&service), now);
        assert_eq!(before["availability"], "not_ready");
        assert_eq!(before["assessment"]["state"], "indeterminate");
        for key in [
            "owner_binding_id",
            "snapshot_seq",
            "ui_frame",
            "age_ms",
            "max_age_ms",
        ] {
            assert!(before["observation"][key].is_null());
        }
        assert!(owner.publish_at(clean(), now));
        assert_eq!(
            at(Some(&service), now)["assessment"]["state"],
            "none_observed"
        );
        drop(owner);
        let after = at(Some(&service), now);
        assert_eq!(after["availability"], "owner_absent");
        assert_eq!(after["assessment"]["state"], "indeterminate");
        assert!(after["editor_local"]["domains"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fixed_schema_and_full_typed_inventory_preserve_domain_semantics() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let input = EditorSnapshot {
            domains: DomainId::ALL
                .map(|id| DomainRecord::observed(id, true))
                .to_vec(),
            operations: Operations::Idle,
        };
        assert!(owner.publish_at(input, now));
        let value = at(Some(&service), now);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["scope"], "known_main_editor_authoring_domains_v1");
        assert_eq!(value["target"], "host_paired_local_editor");
        assert_eq!(
            value["consistency"],
            "host_at_request_local_at_published_frame"
        );
        assert_eq!(value["availability"], "ready");
        assert_eq!(value["assessment"]["state"], "known_blockers");
        assert_eq!(
            value["assessment"]["blockers"].as_array().unwrap().len(),
            16
        );
        let rows = value["editor_local"]["domains"].as_array().unwrap();
        assert_eq!(rows.len(), DomainId::ALL.len());
        for (row, id) in rows.iter().zip(DomainId::ALL) {
            assert_eq!(row["domain_id"], id.as_str());
            assert_eq!(row["owner"], "editor_local");
            assert_eq!(row["presence"], "loaded");
            assert_eq!(row["work_state"], id.blocked_state().as_str());
            assert_eq!(row["persistence_scope"], id.persistence_scope());
            assert!(row["domain_revision"].is_null());
        }
        assert_eq!(value["observation"]["snapshot_seq"], "1");
        assert_eq!(value["observation"]["ui_frame"], "1");
        assert_eq!(
            value["observation"]["binding_scope"],
            "process_local_live_host_connection"
        );
        assert_eq!(
            value["host_scene"]["preview_revision"],
            u64::MAX.to_string()
        );
        assert_eq!(value["host_scene"]["doc_checksum"], "0xffffffffffffffff");
        assert!(serde_json::to_vec(&value).unwrap().len() <= MAX_RESPONSE_BYTES);
    }

    #[test]
    fn disabled_and_absent_domains_are_not_labeled_clean() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut input = clean();
        input.domains[0] = DomainRecord::disabled(DomainId::SpriteBindings);
        input.domains[1] = DomainRecord::not_loaded(DomainId::Tilemap);
        assert!(owner.publish_at(input, now));
        let value = at(Some(&service), now);
        assert_eq!(value["assessment"]["state"], "none_observed");
        assert_eq!(
            value["editor_local"]["domains"][0]["presence"],
            "feature_disabled"
        );
        assert_eq!(
            value["editor_local"]["domains"][1]["presence"],
            "not_loaded"
        );
        for index in [0, 1] {
            assert_eq!(
                value["editor_local"]["domains"][index]["work_state"],
                "not_applicable"
            );
        }
    }

    #[test]
    fn reads_do_not_refresh_and_stale_blockers_do_not_establish_current_work() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut input = clean();
        input.domains[0] = DomainRecord::observed(DomainId::SpriteBindings, true);
        assert!(owner.publish_at(input, now));
        assert_eq!(
            at(Some(&service), later(now, 1_000))["availability"],
            "ready"
        );
        let stale = at(Some(&service), later(now, 1_001));
        assert_eq!(stale["availability"], "stale");
        assert_eq!(stale["assessment"]["state"], "indeterminate");
        assert_eq!(stale["editor_local"]["observation"], "last_observed");
        assert_eq!(stale["editor_local"]["domains"][0]["work_state"], "unsaved");
        assert!(stale["assessment"]["blockers"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(stale["observation"]["snapshot_seq"], "1");
        assert_eq!(
            at(Some(&service), later(now, 2_000))["observation"]["age_ms"],
            2_000
        );
        assert!(owner.publish_at(clean(), later(now, 2_001)));
        let fresh = at(Some(&service), later(now, 2_001));
        assert_eq!(fresh["observation"]["snapshot_seq"], "2");
        assert_eq!(fresh["observation"]["ui_frame"], "2");
        assert_eq!(fresh["assessment"]["state"], "none_observed");
    }

    #[test]
    fn submillisecond_age_never_rounds_stale_to_fresh_and_time_cannot_go_backwards() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let just_stale = now.checked_add(Duration::from_micros(1_000_001)).unwrap();
        assert_eq!(at(Some(&service), just_stale)["availability"], "stale");
        assert!(owner.publish_at(clean(), later(now, 1)));
        assert_eq!(at(Some(&service), now)["availability"], "stale");
    }

    #[test]
    fn fresh_positive_blocker_survives_incomplete_other_coverage() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let input = EditorSnapshot {
            domains: vec![DomainRecord::observed(DomainId::Tilemap, true)],
            operations: Operations::Unknown,
        };
        assert!(owner.publish_at(input, now));
        let value = at(Some(&service), now);
        assert_eq!(value["availability"], "incomplete");
        assert_eq!(value["editor_local"]["coverage"], "incomplete");
        assert_eq!(value["assessment"]["state"], "known_blockers");
        assert_eq!(value["assessment"]["blockers"], json!(["tilemap"]));
        assert!(reason(&value, "missing_domain"));
        assert!(reason(&value, "operations_unknown"));
        let stale = at(Some(&service), later(now, 1_001));
        assert_eq!(stale["assessment"]["state"], "indeterminate");
        assert!(reason(&stale, "local_incomplete"));
        assert!(reason(&stale, "local_stale"));
    }

    #[test]
    fn authoritative_host_blocker_is_current_even_without_usable_local_data() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        for local in [None, Some(&service)] {
            let value = response_at(
                local,
                HostScene {
                    dirty: true,
                    in_tx: true,
                    ..host()
                },
                1_000,
                later(now, 1_001),
            );
            assert_eq!(value["assessment"]["state"], "known_blockers");
            assert_eq!(
                value["assessment"]["blockers"],
                json!(["host_scene_unsaved", "host_scene_transaction"])
            );
            assert!(!value["assessment"]["reasons"]
                .as_array()
                .unwrap()
                .is_empty());
            assert_eq!(value["host_scene"]["dirty"], true);
            assert_eq!(value["host_scene"]["in_tx"], true);
        }
        let mut incomplete = clean();
        incomplete.domains.pop();
        assert!(owner.publish_at(incomplete, now));
        let value = response_at(
            Some(&service),
            HostScene {
                dirty: true,
                ..host()
            },
            1_000,
            now,
        );
        assert_eq!(value["assessment"]["state"], "known_blockers");
        assert_eq!(value["editor_local"]["coverage"], "incomplete");
        assert!(reason(&value, "missing_domain"));
    }

    #[test]
    fn unknown_and_pending_operations_never_infer_idle() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        for operations in [
            Operations::PendingMutation,
            Operations::OutcomeUncertain,
            Operations::Unknown,
        ] {
            let mut input = clean();
            input.operations = operations;
            assert!(owner.publish_at(input, now));
            let value = at(Some(&service), now);
            assert_eq!(value["availability"], "ready");
            assert_eq!(value["editor_operations"]["state"], operations.as_str());
            assert_eq!(value["assessment"]["state"], "indeterminate");
            assert!(reason(&value, operations.reason().unwrap().as_str()));
        }
    }

    #[test]
    fn invalid_duplicate_missing_and_unsupported_rows_fail_closed() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut input = clean();
        input
            .domains
            .push(DomainRecord::observed(DomainId::SpriteBindings, true));
        input
            .domains
            .push(DomainRecord::observed(DomainId::SpriteBindings, false));
        assert!(owner.publish_at(input, now));
        let value = at(Some(&service), now);
        assert!(reason(&value, "duplicate_domain"));
        assert_eq!(value["editor_local"]["domains"][0]["work_state"], "unknown");
        assert_eq!(value["assessment"]["state"], "indeterminate");
        for record in [
            DomainRecord {
                domain_id: DomainId::SpriteBindings,
                presence: Presence::FeatureDisabled,
                work_state: WorkState::Clean,
            },
            DomainRecord {
                domain_id: DomainId::SpriteBindings,
                presence: Presence::Loaded,
                work_state: WorkState::StagedOrUnfinished,
            },
            DomainRecord {
                domain_id: DomainId::SpriteBindings,
                presence: Presence::Loaded,
                work_state: WorkState::Unknown,
            },
            DomainRecord::unknown(DomainId::SpriteBindings),
        ] {
            let mut input = clean();
            input.domains[0] = record;
            assert!(owner.publish_at(input, now));
            let value = at(Some(&service), now);
            assert_eq!(value["assessment"]["state"], "indeterminate");
            assert_eq!(value["availability"], "incomplete");
            assert!(!record.is_blocker());
        }
    }

    #[test]
    fn record_overflow_is_not_truncated_into_a_clean_inventory() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let input = EditorSnapshot {
            domains: vec![DomainRecord::observed(DomainId::Tilemap, true); MAX_DOMAIN_RECORDS + 1],
            operations: Operations::Idle,
        };
        assert!(owner.publish_at(input, now));
        let value = at(Some(&service), now);
        assert_eq!(value["availability"], "incomplete");
        assert_eq!(value["assessment"]["state"], "indeterminate");
        assert!(reason(&value, "domain_limit_exceeded"));
        assert!(value["editor_local"]["domains"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["work_state"] == "unknown"));
        assert!(serde_json::to_vec(&value).unwrap().len() <= MAX_RESPONSE_BYTES);
    }

    #[test]
    fn contention_is_nonblocking_and_failed_publication_preserves_age_and_sequence() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let guard = service.shared.snapshot.lock().unwrap();
        let busy = at(Some(&service), now);
        assert_eq!(busy["availability"], "busy");
        assert_eq!(busy["assessment"]["state"], "indeterminate");
        assert_eq!(service.discovery()["availability"], "busy");
        assert!(!owner.publish_at(clean(), later(now, 500)));
        drop(guard);
        let stale = at(Some(&service), later(now, 1_001));
        assert_eq!(stale["availability"], "stale");
        assert_eq!(stale["observation"]["snapshot_seq"], "1");
        assert_eq!(stale["observation"]["ui_frame"], "1");
        assert!(owner.publish_at(clean(), later(now, 1_002)));
        let next = at(Some(&service), later(now, 1_002));
        assert_eq!(next["observation"]["snapshot_seq"], "2");
        assert_eq!(next["observation"]["ui_frame"], "3");
    }

    #[test]
    fn owner_drop_invalidates_even_while_mailbox_is_locked() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let guard = service.shared.snapshot.lock().unwrap();
        drop(owner);
        assert_eq!(at(Some(&service), now)["availability"], "owner_absent");
        drop(guard);
        assert_eq!(at(Some(&service), now)["availability"], "owner_absent");
    }

    #[test]
    fn service_drop_disables_publisher_and_new_pair_is_independent() {
        let now = Instant::now();
        let (old_service, mut old_owner) = WorkStatusService::pair();
        assert!(old_owner.publish_at(clean(), now));
        let old_id = at(Some(&old_service), now)["observation"]["owner_binding_id"].clone();
        drop(old_service);
        assert!(!old_owner.publish_at(clean(), now));
        let (new_service, mut new_owner) = WorkStatusService::pair();
        assert!(new_owner.publish_at(clean(), now));
        assert_ne!(
            at(Some(&new_service), now)["observation"]["owner_binding_id"],
            old_id
        );
        drop(old_owner);
        assert_eq!(at(Some(&new_service), now)["availability"], "ready");
    }

    #[test]
    fn identity_and_both_observation_counters_fail_closed_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_binding(&counter), Some(u64::MAX - 1));
        assert_eq!(allocate_binding(&counter), None);
        assert_eq!(allocate_binding(&counter), None);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert_eq!(allocate_binding(&AtomicU64::new(0)), None);
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair_with_binding(None);
        assert!(!owner.publish_at(clean(), now));
        assert!(reason(&at(Some(&service), now), "counter_exhausted"));
        for frame_exhausted in [false, true] {
            let (service, mut owner) = WorkStatusService::pair();
            assert!(owner.publish_at(clean(), now));
            if frame_exhausted {
                owner.ui_frame = u64::MAX;
            } else {
                owner.snapshot_seq = u64::MAX;
            }
            assert!(!owner.publish_at(clean(), now));
            let value = at(Some(&service), now);
            assert_eq!(value["availability"], "incomplete");
            assert_eq!(value["assessment"]["state"], "indeterminate");
            assert!(value["observation"]["snapshot_seq"].is_null());
            assert!(reason(&value, "counter_exhausted"));
            assert!(!owner.publish_at(clean(), now));
        }
    }

    #[test]
    fn poisoned_mailbox_fails_closed() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let shared = Arc::clone(&service.shared);
        let _ = std::panic::catch_unwind(|| {
            let _guard = shared.snapshot.lock().unwrap();
            panic!("poison test");
        });
        let value = at(Some(&service), now);
        assert_eq!(value["availability"], "incomplete");
        assert_eq!(value["assessment"]["state"], "indeterminate");
        assert!(reason(&value, "mailbox_poisoned"));
        assert!(!owner.publish_at(clean(), now));
    }

    #[test]
    fn discovery_has_no_identity_or_domains_and_tracks_owner_liveness() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert_eq!(service.discovery_at(now)["supported"], true);
        assert_eq!(service.discovery_at(now)["availability"], "not_ready");
        assert!(owner.publish_at(clean(), now));
        assert_eq!(service.discovery_at(now)["availability"], "ready");
        assert_eq!(
            service.discovery_at(later(now, 1_001))["availability"],
            "stale"
        );
        let text = service.discovery_at(now).to_string();
        for forbidden in [
            "owner_binding_id",
            "snapshot_seq",
            "ui_frame",
            "domains",
            "sprite_bindings",
        ] {
            assert!(!text.contains(forbidden));
        }
        drop(owner);
        assert_eq!(service.discovery_at(now)["availability"], "owner_absent");
    }

    #[test]
    fn save_or_rollback_host_values_never_refresh_or_clear_local_work() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut input = clean();
        input.domains[0] = DomainRecord::observed(DomainId::SpriteBindings, true);
        assert!(owner.publish_at(input, now));
        let dirty = response_at(
            Some(&service),
            HostScene {
                dirty: true,
                preview_revision: 9,
                ..host()
            },
            1_000,
            now,
        );
        let saved = response_at(
            Some(&service),
            HostScene {
                preview_revision: 9,
                ..host()
            },
            1_000,
            later(now, 10),
        );
        let rolled_back = response_at(
            Some(&service),
            HostScene {
                preview_revision: 8,
                doc_checksum: 1,
                ..host()
            },
            1_000,
            later(now, 20),
        );
        assert_eq!(
            dirty["assessment"]["blockers"],
            json!(["host_scene_unsaved", "sprite_bindings"])
        );
        for value in [&saved, &rolled_back] {
            assert_eq!(value["assessment"]["blockers"], json!(["sprite_bindings"]));
            assert_eq!(value["observation"]["snapshot_seq"], "1");
            assert_eq!(value["observation"]["ui_frame"], "1");
            assert_eq!(value["editor_local"], dirty["editor_local"]);
            assert_eq!(
                value["consistency"],
                "host_at_request_local_at_published_frame"
            );
        }
        assert_eq!(saved["host_scene"]["preview_revision"], "9");
        assert_eq!(rolled_back["host_scene"]["preview_revision"], "8");
        assert_eq!(
            rolled_back["host_scene"]["doc_checksum"],
            "0x0000000000000001"
        );
        assert_eq!(rolled_back["observation"]["age_ms"], 20);
    }

    #[test]
    fn largest_live_counters_are_decimal_strings_then_exhaust_without_reuse() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair_with_binding(Some(u64::MAX));
        owner.snapshot_seq = u64::MAX - 1;
        owner.ui_frame = u64::MAX - 1;
        assert!(owner.publish_at(clean(), now));
        let value = at(Some(&service), now);
        for key in ["owner_binding_id", "snapshot_seq", "ui_frame"] {
            assert_eq!(value["observation"][key], u64::MAX.to_string());
        }
        assert!(!owner.publish_at(clean(), now));
        assert!(reason(&at(Some(&service), now), "counter_exhausted"));
    }

    #[test]
    fn strict_parameter_validation_has_fixed_non_reflective_errors() {
        assert_eq!(parse_max_age(&json!({})).unwrap(), 1_000);
        assert_eq!(parse_max_age(&json!({"max_age_ms": 1})).unwrap(), 1);
        assert_eq!(parse_max_age(&json!({"max_age_ms": 5_000})).unwrap(), 5_000);
        let invalid = [
            J::Null,
            json!([]),
            json!(true),
            json!({"max_age_ms": 0}),
            json!({"max_age_ms": 5_001}),
            json!({"max_age_ms": -1}),
            json!({"max_age_ms": "sentinel-secret-path"}),
            json!({"max_age_ms": 1.5}),
            json!({"max_age_ms": 1.0}),
            json!({"max_age_ms": null}),
            json!({"max_age_ms": true}),
            json!({"max_age_ms": {"path": "sentinel-secret-path"}}),
            json!({"sentinel-secret-path": 1}),
            json!({"max_age_ms": 1, "save": true}),
            serde_json::from_str("{\"max_age_ms\":18446744073709551616}").unwrap(),
        ];
        let reference = parse_max_age(&J::Null).unwrap_err();
        for params in invalid {
            let error = parse_max_age(&params).unwrap_err();
            assert_eq!(error, reference);
            assert_eq!(error.code, crate::INVALID_PARAMS);
            assert!(!error.to_json().to_string().contains("sentinel-secret-path"));
        }
    }

    #[test]
    fn hard_wire_budget_and_no_loss_or_mutation_claims() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut input = clean();
        input.domains[0] = DomainRecord::unknown(DomainId::SpriteBindings);
        input.operations = Operations::OutcomeUncertain;
        assert!(owner.publish_at(input, now));
        let value = response_at(
            Some(&service),
            HostScene {
                dirty: true,
                in_tx: true,
                ..host()
            },
            MAX_AGE_MS,
            now,
        );
        let serialized = serde_json::to_vec(&value).unwrap();
        assert!(validate_response(&value));
        assert!(serialized.len() <= MAX_RESPONSE_BYTES);
        let text = String::from_utf8(serialized).unwrap();
        for forbidden in [
            "scene_path",
            "safe_to_close",
            "close_readiness",
            "last_close_attempt",
            "tx_owner",
            "token",
        ] {
            assert!(!text.contains(forbidden));
        }
        let mut writer = SizeLimit(MAX_RESPONSE_BYTES - 1);
        assert_eq!(writer.write(b"x").unwrap(), 1);
        assert!(writer.write(b"x").is_err());
        assert_eq!(at(Some(&service), now), at(Some(&service), now));
    }

    #[test]
    fn response_validation_accepts_canonical_lifecycle_and_all_finite_reasons() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        for dirty in [false, true] {
            for in_tx in [false, true] {
                let host = HostScene {
                    dirty,
                    in_tx,
                    ..host()
                };
                assert!(validate_response(&response_at(None, host, 1, now)));
                assert!(validate_response(&response_at(
                    Some(&service),
                    host,
                    MAX_AGE_MS,
                    now
                )));
                for (availability, reason) in [
                    (Availability::NotPaired, Reason::NotPaired),
                    (Availability::NotReady, Reason::NotReady),
                    (Availability::OwnerAbsent, Reason::OwnerAbsent),
                    (Availability::Busy, Reason::Busy),
                    (Availability::Incomplete, Reason::CounterExhausted),
                    (Availability::Incomplete, Reason::MailboxPoisoned),
                    (Availability::Incomplete, Reason::ResponseLimitExceeded),
                ] {
                    let value = render(
                        Observation::absent(availability, reason),
                        host,
                        availability != Availability::NotPaired,
                        1,
                    );
                    assert!(validate_response(&value));
                }
            }
        }
        for operations in Operations::ALL {
            let mut input = clean();
            input.operations = operations;
            assert!(owner.publish_at(input, now));
            assert!(validate_response(&at(Some(&service), now)));
            assert!(validate_response(&at(Some(&service), later(now, 1_001))));
        }
        // All four non-overflow defects can coexist. Their stale/operation
        // reasons must still fit the strict response validator's finite bound.
        let input = EditorSnapshot {
            domains: vec![
                DomainRecord::observed(DomainId::Tilemap, false),
                DomainRecord::observed(DomainId::Tilemap, true),
                DomainRecord::unknown(DomainId::Terrain),
                DomainRecord {
                    domain_id: DomainId::SpriteBindings,
                    presence: Presence::Loaded,
                    work_state: WorkState::NotApplicable,
                },
            ],
            operations: Operations::PendingMutation,
        };
        assert!(owner.publish_at(input, now));
        let stale = at(Some(&service), later(now, 1_001));
        assert_eq!(
            stale["assessment"]["reasons"].as_array().unwrap().len(),
            MAX_REASONS
        );
        assert!(validate_response(&stale));
        assert!(owner.publish_at(
            EditorSnapshot {
                domains: vec![DomainRecord::not_loaded(DomainId::Tilemap); MAX_DOMAIN_RECORDS + 1],
                operations: Operations::Idle,
            },
            now,
        ));
        assert!(validate_response(&at(Some(&service), now)));
    }

    #[test]
    fn response_validation_rejects_semantically_contradictory_assessments() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        let mut limited = vec![
            ("not_paired", at(None, now)),
            ("not_ready", at(Some(&service), now)),
        ];
        assert!(owner.publish_at(clean(), now));
        let ready = at(Some(&service), now);
        assert_eq!(ready["assessment"]["state"], "none_observed");
        assert_eq!(ready["assessment"]["reasons"], json!([]));
        for reason in Reason::ALL {
            let mut hostile = ready.clone();
            hostile["assessment"]["reasons"] = json!([reason.as_str()]);
            assert!(!validate_response(&hostile), "ready + {}", reason.as_str());
        }
        for (name, record) in [
            (
                "loaded_unknown",
                DomainRecord {
                    domain_id: DomainId::SpriteBindings,
                    presence: Presence::Loaded,
                    work_state: WorkState::Unknown,
                },
            ),
            (
                "unknown_presence",
                DomainRecord::unknown(DomainId::SpriteBindings),
            ),
        ] {
            let mut input = clean();
            input.domains[0] = record;
            assert!(owner.publish_at(input, now));
            limited.push((name, at(Some(&service), now)));
        }
        let mut incomplete = clean();
        incomplete.domains.pop();
        assert!(owner.publish_at(incomplete, now));
        limited.push(("incomplete", at(Some(&service), now)));
        for operations in [
            Operations::PendingMutation,
            Operations::OutcomeUncertain,
            Operations::Unknown,
        ] {
            let mut input = clean();
            input.operations = operations;
            assert!(owner.publish_at(input, now));
            limited.push((operations.as_str(), at(Some(&service), now)));
        }
        assert!(owner.publish_at(clean(), now));
        limited.push(("stale", at(Some(&service), later(now, 1_001))));
        for (name, dirty, in_tx) in [("host_dirty", true, false), ("host_tx", false, true)] {
            limited.push((
                name,
                response_at(
                    Some(&service),
                    HostScene {
                        dirty,
                        in_tx,
                        ..host()
                    },
                    DEFAULT_MAX_AGE_MS,
                    now,
                ),
            ));
        }
        let (absent_service, absent_owner) = WorkStatusService::pair();
        drop(absent_owner);
        limited.push(("owner_absent", at(Some(&absent_service), now)));
        for (name, canonical) in limited {
            assert!(validate_response(&canonical), "{name}");
            let mut hostile = canonical.clone();
            hostile["assessment"]["state"] = json!("none_observed");
            assert!(!validate_response(&hostile), "{name} as none_observed");
            hostile["assessment"]["blockers"] = json!([]);
            hostile["assessment"]["reasons"] = json!([]);
            assert!(
                !validate_response(&hostile),
                "{name} as empty none_observed"
            );
            // Omitting any required lifecycle, coverage, or operation reason
            // cannot make otherwise valid limited observations acceptable.
            for index in 0..canonical["assessment"]["reasons"].as_array().unwrap().len() {
                let mut hostile = canonical.clone();
                hostile["assessment"]["reasons"]
                    .as_array_mut()
                    .unwrap()
                    .remove(index);
                assert!(
                    !validate_response(&hostile),
                    "{name} without reason {index}"
                );
            }
            let mut hostile = canonical;
            hostile["assessment"]["reasons"] = json!(["local_owner_absent", "outcome_uncertain"]);
            assert!(
                !validate_response(&hostile),
                "{name} with unrelated reasons"
            );
        }
        assert!(owner.publish_at(
            EditorSnapshot {
                domains: vec![DomainRecord::observed(DomainId::Tilemap, true)],
                operations: Operations::OutcomeUncertain,
            },
            now,
        ));
        let stale = at(Some(&service), later(now, 1_001));
        assert_eq!(stale["assessment"]["state"], "indeterminate");
        assert_eq!(stale["assessment"]["blockers"], json!([]));
        for (mut inconsistent, wrong_owner) in [
            (at(None, now), "paired_host"),
            (at(Some(&service), now), "host"),
        ] {
            inconsistent["host_scene"]["owner"] = json!(wrong_owner);
            assert!(!validate_response(&inconsistent));
        }
        let mut hostile = stale;
        hostile["assessment"]["state"] = json!("known_blockers");
        hostile["assessment"]["blockers"] = json!(["tilemap"]);
        assert!(!validate_response(&hostile), "stale local-only blocker");
        for (dirty, in_tx, expected) in [
            (true, false, vec!["host_scene_unsaved"]),
            (false, true, vec!["host_scene_transaction"]),
            (
                true,
                true,
                vec!["host_scene_unsaved", "host_scene_transaction"],
            ),
        ] {
            let canonical = response_at(
                Some(&service),
                HostScene {
                    dirty,
                    in_tx,
                    ..host()
                },
                DEFAULT_MAX_AGE_MS,
                later(now, 1_001),
            );
            assert_eq!(canonical["assessment"]["state"], "known_blockers");
            assert_eq!(canonical["assessment"]["blockers"], json!(expected));
            assert_eq!(
                canonical["assessment"]["reasons"],
                json!([
                    "local_stale",
                    "local_incomplete",
                    "missing_domain",
                    "outcome_uncertain"
                ])
            );
            assert!(validate_response(&canonical));
            let mut hostile = canonical.clone();
            hostile["assessment"]["blockers"]
                .as_array_mut()
                .unwrap()
                .push(json!("tilemap"));
            assert!(!validate_response(&hostile));
            let mut hostile = canonical;
            hostile["assessment"]["reasons"]
                .as_array_mut()
                .unwrap()
                .push(json!("local_owner_absent"));
            assert!(!validate_response(&hostile));
        }
    }

    #[test]
    fn response_validation_rejects_extra_nested_metadata_before_forwarding() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let original = at(Some(&service), now);
        for pointer in [
            "",
            "/observation",
            "/host_scene",
            "/editor_local",
            "/editor_local/domains/0",
            "/editor_operations",
            "/assessment",
        ] {
            for key in ["path", "title", "token", "metadata"] {
                let mut hostile = original.clone();
                hostile
                    .pointer_mut(pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(key.into(), json!({"secret": "sentinel-private-data"}));
                assert!(!validate_response(&hostile), "{pointer}/{key}");
            }
            let mut missing = original.clone();
            let object = missing
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap();
            let key = object.keys().next().unwrap().clone();
            object.remove(&key);
            assert!(!validate_response(&missing));
        }
        let mut hostile = original;
        hostile["host_scene"]["dirty"] = json!({"metadata": {"token": "sentinel-private-data"}});
        assert!(!validate_response(&hostile));
    }

    #[test]
    fn response_validation_rejects_arbitrary_strings_arrays_and_wrong_types() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let original = at(Some(&service), now);
        for pointer in [
            "/scope",
            "/target",
            "/availability",
            "/consistency",
            "/observation/binding_scope",
            "/observation/reconnect_policy",
            "/host_scene/authority",
            "/host_scene/owner",
            "/host_scene/preview_revision_scope",
            "/editor_local/owner",
            "/editor_local/coverage",
            "/editor_local/observation",
            "/editor_local/domains/0/domain_id",
            "/editor_local/domains/0/owner",
            "/editor_local/domains/0/presence",
            "/editor_local/domains/0/work_state",
            "/editor_local/domains/0/persistence_scope",
            "/editor_local/domains/0/domain_revision",
            "/editor_operations/state",
            "/editor_operations/reason",
            "/editor_operations/observation",
            "/assessment/state",
        ] {
            for replacement in [
                json!("sentinel-private-data"),
                json!("x".repeat(MAX_RESPONSE_BYTES + 1)),
                json!([]),
                json!({}),
                json!(true),
                json!(7),
            ] {
                let mut hostile = original.clone();
                *hostile.pointer_mut(pointer).unwrap() = replacement;
                assert!(!validate_response(&hostile), "{pointer}");
            }
        }
        for (pointer, replacement) in [
            ("/schema_version", json!(2)),
            ("/schema_version", json!("1")),
            ("/schema_version", json!(1.0)),
            ("/host_scene/dirty", json!(1)),
            ("/host_scene/in_tx", json!("false")),
            ("/assessment/blockers", json!(["sentinel-private-data"])),
            ("/assessment/reasons", json!(["sentinel-private-data"])),
            ("/assessment/reasons", json!(["local_stale", "local_stale"])),
            (
                "/assessment/reasons",
                json!(vec!["local_stale"; MAX_REASONS + 1]),
            ),
        ] {
            let mut hostile = original.clone();
            *hostile.pointer_mut(pointer).unwrap() = replacement;
            assert!(!validate_response(&hostile), "{pointer}");
        }
        let mut hostile = original.clone();
        hostile["editor_local"]["domains"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(!validate_response(&hostile));
        hostile = original.clone();
        hostile["editor_local"]["domains"]
            .as_array_mut()
            .unwrap()
            .push(original["editor_local"]["domains"][0].clone());
        assert!(!validate_response(&hostile));
        hostile = original.clone();
        hostile["assessment"]["blockers"] = json!(vec!["host_scene_unsaved"; MAX_BLOCKERS + 1]);
        assert!(!validate_response(&hostile));
        hostile = original;
        hostile["editor_local"]["domains"] = json!(vec![J::Null; MAX_DOMAIN_RECORDS + 1]);
        assert!(!validate_response(&hostile));
    }

    #[test]
    fn response_validation_rejects_malformed_counters_checksums_and_observations() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let original = at(Some(&service), now);
        for pointer in [
            "/observation/owner_binding_id",
            "/observation/snapshot_seq",
            "/observation/ui_frame",
            "/host_scene/preview_revision",
        ] {
            for replacement in [
                json!(""),
                json!("01"),
                json!("+1"),
                json!("-1"),
                json!("1.0"),
                json!("1e3"),
                json!("18446744073709551616"),
                json!("１２"),
                json!(1),
                J::Null,
            ] {
                let mut hostile = original.clone();
                *hostile.pointer_mut(pointer).unwrap() = replacement;
                assert!(!validate_response(&hostile), "{pointer}");
            }
        }
        for pointer in [
            "/observation/owner_binding_id",
            "/observation/snapshot_seq",
            "/observation/ui_frame",
        ] {
            let mut hostile = original.clone();
            *hostile.pointer_mut(pointer).unwrap() = json!("0");
            assert!(!validate_response(&hostile));
        }
        for replacement in [
            json!("0x1"),
            json!("0xFFFFFFFFFFFFFFFF"),
            json!("0Xffffffffffffffff"),
            json!("0xgggggggggggggggg"),
            json!(u64::MAX),
            J::Null,
        ] {
            let mut hostile = original.clone();
            hostile["host_scene"]["doc_checksum"] = replacement;
            assert!(!validate_response(&hostile));
        }
        for (pointer, replacement) in [
            ("/observation/age_ms", json!(-1)),
            ("/observation/age_ms", json!(1.0)),
            ("/observation/age_ms", json!("1")),
            ("/observation/max_age_ms", json!(0)),
            ("/observation/max_age_ms", json!(MAX_AGE_MS + 1)),
            ("/observation/max_age_ms", J::Null),
        ] {
            let mut hostile = original.clone();
            *hostile.pointer_mut(pointer).unwrap() = replacement;
            assert!(!validate_response(&hostile));
        }
        let mut malformed = at(None, now);
        malformed["observation"]["snapshot_seq"] = json!("1");
        assert!(!validate_response(&malformed));
        let mut malformed = original;
        malformed["host_scene"]["dirty"] = json!(true);
        // A fixed but internally contradictory claim is rejected using the
        // same assessment classifier as the canonical response renderer.
        assert!(!validate_response(&malformed));
    }
    #[test]
    fn response_validation_rejects_false_freshness_but_allows_stale_truncation() {
        let now = Instant::now();
        let (service, mut owner) = WorkStatusService::pair();
        assert!(owner.publish_at(clean(), now));
        let mut fresh = at(Some(&service), now);
        fresh["observation"]["age_ms"] = json!(1001);
        assert!(!validate_response(&fresh));
        fresh["observation"]["age_ms"] = json!(1000);
        assert!(validate_response(&fresh));
        let mut stale = at(Some(&service), later(now, 1001));
        stale["observation"]["age_ms"] = json!(1000);
        assert!(
            validate_response(&stale),
            "stale duration may truncate to the same integer bound"
        );
    }
}
