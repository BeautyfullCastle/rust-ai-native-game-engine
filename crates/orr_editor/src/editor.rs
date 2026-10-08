//! `Editor`: the state machine behind the window. No egui in here.
//!
//! # The editor is a client of a simulation host
//!
//! It owns no `EditorDoc`, no play session and no ERP server. Everything
//! that simulates or edits lives in a **host** (`orr_remote::Host`), on a
//! thread of this process or in another process ([`HostSpec`]). The editor
//! reaches it through exactly two channels (see [`crate::backend`]):
//!
//! - **ERP** for everything a person does: edits (`world.*`, one `tx.*`
//!   transaction per gesture), undo and redo, save and open, play control,
//!   proposals and their diffs, and the queries that fill the panels;
//! - the **bridge** (`RemoteBridge`: `Bridge` + `SimControl` + `Snapshot`) for
//!   the frames the viewport draws.
//!
//! What the panels show are **caches** of host answers ([`SimState`], the
//! hierarchy rows, the inspected entity, the history, ...). They are filled by
//! requests that never wait for the answer ([`Editor::pump`], once per UI
//! frame) and refreshed when a notification (`watch.tick`, `watch.history`,
//! `watch.proposals`, `watch.activity`) or a new frame says the host changed.
//! The viewport never waits for a request: it draws the newest snapshot.
//!
//! A person's actions (Undo, Play, a typed value, ...) are blocking calls:
//! they return the host's answer, which is a round trip (microseconds for a
//! host thread, a network round trip for a remote host). A drag is the
//! exception: its begin, coalesced field edits and commit are all posted
//! without waiting. Replies advance a bounded gesture queue, preserving one
//! undo step per accepted drag. The host remains authoritative.
//!
//! If the host thread panics, or the remote host goes away, the editor keeps
//! running: [`Editor::down`] says why and [`Editor::restart`] starts the host
//! again (local: the scene file is opened afresh, so edits that were not
//! saved are lost) or reconnects (remote).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{BridgeEvent, ControlOp, Lifecycle, Snapshot, Timeline, ViewResync, ViewUpdate};
use orr_ecs::Entity;
use orr_fp::{FPVec2, FP};
use orr_reflect::{decimal, Guid, TypeRegistry, Value};
use orr_remote::codec::b64_decode;
use orr_remote::json::{json_to_value, value_to_json};
use orr_remote::{CaptureError, CaptureRequest, CapturedImage, ClientError, ErpClient, RemoteViewDelivery, ViewMode, ViewState};
use orr_render::{Camera, RenderList};
use crate::game::{Drawable, EditorGame, EditorStream};
use serde_json::{json, Value as J};

use crate::agent::{AgentState, Feed, FeedEntry};
use crate::backend::{Backend, HostSpec};
use crate::diagnostics::{EditorDiagnostics, Telemetry};
use crate::model::{ClientInfo, EntityRow, GuidSelection, History, ProposalDetail, ProposalInfo, SimState, Stopped, Target};
pub use crate::model::Mode;
use crate::viewport::{self, Scene};

/// Most entities one hierarchy refresh asks for.
const ROWS_LIMIT: u64 = 20_000;
/// Least time between two refreshes of the inspector while the host changes all the time.
const INSPECT_EVERY: Duration = Duration::from_millis(50);
/// How often the list of connected clients is refreshed.
const CLIENTS_EVERY: Duration = Duration::from_millis(500);
/// Most messages the log keeps.
const LOG_LIMIT: usize = 200;

/// The type registry of `PhysGame` (physics types, `PaddleTag`, `Scene`): the
/// shipped descriptors the inspector draws its widgets from.
pub fn make_types() -> TypeRegistry {
    EditorGame::PhysGame.types()
}

/// The demo scene: `scenes/physics_demo.scene.yaml` of the working directory
/// if there is one, else the one in the source tree this was built from.
pub fn default_scene_path() -> PathBuf {
    let local = PathBuf::from("scenes/physics_demo.scene.yaml");
    if local.exists() {
        return local;
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml"))
}

/// A line of the status bar and the log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    /// The text.
    pub text: String,
    /// True for a refused action.
    pub error: bool,
}

/// What an inspector widget edits: a component of the selected entity, or a singleton.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Owner {
    /// A component type name (of the selected entity).
    Component(String),
    /// A singleton type name.
    Singleton(String),
}

/// Why the editor cannot reach its host any more.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Down {
    /// `simulation host stopped: <reason>` or `disconnected: <reason>`.
    pub reason: String,
    /// True if the host was started by this editor (a panic), false if it is a remote host.
    pub local: bool,
}

/// The components of the selected entity, as the host last reported them.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspect {
    /// The entity.
    pub target: Target,
    /// Its list row.
    pub row: Option<EntityRow>,
    /// `(type name, value)` of its reflected components.
    pub components: Vec<(String, Value)>,
}

/// What the answer to a request that was sent without waiting is for.
pub mod input;

enum Pend {
    Input(u64, input::Op),
    Rows(u64),
    Inspect(Target),
    Singletons,
    History,
    State(u64),
    Clients,
    Detail(String),
    PreviewRows(String, u64),
    Gesture(u64, gestures::Reply),
}

struct Pending {
    kind: Pend,
    posted_at: Instant,
}

#[derive(Default)]
struct Dirty {
    rows: bool,
    inspect: bool,
    singletons: bool,
    history: bool,
    state: bool,
}

#[derive(Default)]
struct InFlight {
    rows: bool,
    inspect: bool,
    singletons: bool,
    history: bool,
    state: bool,
    clients: bool,
    details: Vec<String>,
    preview_rows: Option<u64>,
}

#[derive(Clone, Copy)]
enum ScreenshotQueryKind {
    State,
    View,
}

struct ScreenshotValidation {
    requested: ViewState,
    state_id: u64,
    view_id: u64,
    state: Option<Result<ViewState, CaptureError>>,
    view: Option<Result<(u64, u64), CaptureError>>,
}

/// The proposal whose staged frame the viewport shows.
struct Preview {
    id: String,
    stream: EditorStream,
    seq: u64,
    generation: u64,
    rows_dirty: bool,
    bodies: Vec<Drawable>,
    rows: Vec<EntityRow>,
}

/// See the module docs.
pub struct Editor {
    backend: Backend,
    input: input::Input,
    types: Arc<TypeRegistry>,
    down: Option<Down>,
    selection: Option<Target>,
    guid_selection: GuidSelection,
    batch_supported: bool,
    scene_edit_allowed: bool,
    batch_uncertain: bool,
    /// The viewport camera (world units, y up).
    pub camera: Camera,
    pub camera3d: orr_render::OrbitCamera,
    #[cfg(feature="room-project")]
    room_camera: Option<orr_sample::room_camera::Document>,
    #[cfg(feature="room-project")]
    room_camera_scene: Option<std::path::PathBuf>,
    #[cfg(feature="room-ui")]
    room_ui_source: std::sync::Arc<std::sync::atomic::AtomicU64>,
    yard_frame: crate::viewport3d::YardFrame,
    #[cfg(feature = "terrain-physics")]
    terrain_view: crate::terrain_physics::TerrainPhysicsView,
    #[cfg(feature = "navigation")]
    navigation_view: crate::navigation_view::NavigationView,
    #[cfg(feature = "navigation")]
    navigation_blocker: Option<String>,
    status: Option<Message>,
    log: Vec<Message>,
    // caches of host answers
    sim: SimState,
    history: History,
    rows: Vec<EntityRow>,
    yard_rows_checksum: Option<u64>,
    yard_rows_tick: u64,
    yard_rows_generation: u64,
    inspect: Option<Inspect>,
    singletons: Vec<(String, Value)>,
    proposals: Vec<ProposalInfo>,
    details: BTreeMap<String, ProposalDetail>,
    clients: Vec<ClientInfo>,
    server_now_ms: u64,
    stopped: Option<Stopped>,
    // the newest frame
    snapshot: Option<Snapshot>,
    bodies: Vec<Drawable>,
    checksum: u64,
    snap_entities: u32,
    preview: Option<Preview>,
    preview_generation: u64,
    // requests that are in flight, and what to ask for next
    pending: BTreeMap<u64, Pending>,
    screenshot_queries: BTreeMap<u64, (u64, ScreenshotQueryKind)>,
    screenshot_validations: BTreeMap<u64, ScreenshotValidation>,
    telemetry: Telemetry,
    dirty: Dirty,
    inflight: InFlight,
    change_gen: u64,
    state_generation: u64,
    last_inspect: Option<Instant>,
    last_clients: Option<Instant>,
    gesture: gestures::Gestures,
    refit: bool,
    initial_camera_fit: bool,
    refit_checksum: Option<u64>,
    agent: AgentState,
    feed: Feed,
    agent_clients: BTreeMap<String, ErpClient>,
    script_owner: BTreeMap<String, String>,
}

impl Editor {
    /// An editor on a host thread of this process that loads `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_game(path, EditorGame::PhysGame)
    }

    /// An editor on a local host for the selected game that loads `path`.
    pub fn open_game(path: &Path, game: EditorGame) -> Result<Self, String> {
        Self::start(&HostSpec::local_game(path, game))
    }

    /// An editor attached to the host at `url` (`ws://host:port`), for example an
    /// `orr_remote_host`.
    pub fn attach(url: &str, token: Option<&str>) -> Result<Self, String> {
        Self::start(&HostSpec::remote(url, token))
    }

    /// Starts the host (or attaches to it) and reads its state.
    pub fn start(spec: &HostSpec) -> Result<Self, String> {
        let backend = Backend::connect(spec)?;
        let mut e = Self::on_backend(backend)?;
        let local = spec.is_local();
        if local {
            e.info("ready");
        } else {
            e.info(format!("attached to {}", e.backend.url.clone().unwrap_or_default()));
        }
        e.report_view_delivery();
        Ok(e)
    }

    fn report_view_delivery(&mut self) {
        if self.backend.bridge.view_delivery() == RemoteViewDelivery::Legacy {
            self.info("host uses a legacy frame stream; bounded presentation recovery is unavailable");
        }
    }

    fn on_backend(mut backend: Backend) -> Result<Self, String> {
        let discovery = backend.erp.call("rpc.discover", J::Null).map_err(|e| format!("rpc.discover: {e}"))?;
        let batch_supported = discovery["methods"].as_array().is_some_and(|methods|
            methods.iter().any(|m| m["name"] == "world.patch_batch"));
        let scene_edit_allowed = discovery["you"]["capabilities"].as_array().is_some_and(|caps|
            caps.iter().any(|cap| cap == "scene_edit"));
        let mut feed = Feed::default();
        feed.own.clone_from(&backend.own_client);
        let types = Arc::new(backend.game.types());
        let mut input = input::Input::new(backend.managed_input);
        input.collect_dodge = backend.game.is_collect();
        input.room_escape = backend.game.is_room();
        let mut e = Self {
            input,
            backend,
            types,
            down: None,
            selection: None,
            guid_selection: GuidSelection::default(),
            batch_supported,
            scene_edit_allowed,
            batch_uncertain: false,
            camera: Camera::new([0.0, 0.0], 20.0),
            #[cfg(feature="room-project")] room_camera: None,
            #[cfg(feature="room-project")] room_camera_scene: None,
            #[cfg(feature="room-ui")] room_ui_source: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            camera3d: orr_render::OrbitCamera::new([0.0, 2.0, 0.0], 0.55, 0.5, 16.0),
            yard_frame: crate::viewport3d::YardFrame::default(),
            #[cfg(feature = "terrain-physics")]
            terrain_view: crate::terrain_physics::TerrainPhysicsView::default(),
            #[cfg(feature = "navigation")]
            navigation_view: crate::navigation_view::NavigationView::default(),
            #[cfg(feature = "navigation")]
            navigation_blocker: None,
            status: None,
            log: Vec::new(),
            sim: SimState::default(),
            history: History::default(),
            rows: Vec::new(),
            yard_rows_checksum: None,
            yard_rows_tick: 0,
            yard_rows_generation: 0,
            inspect: None,
            singletons: Vec::new(),
            proposals: Vec::new(),
            details: BTreeMap::new(),
            clients: Vec::new(),
            server_now_ms: 0,
            stopped: None,
            snapshot: None,
            bodies: Vec::new(),
            checksum: 0,
            snap_entities: 0,
            preview: None,
            preview_generation: 0,
            pending: BTreeMap::new(),
            screenshot_queries: BTreeMap::new(),
            screenshot_validations: BTreeMap::new(),
            telemetry: Telemetry::default(),
            dirty: Dirty::default(),
            inflight: InFlight::default(),
            change_gen: 0,
            state_generation: 0,
            last_inspect: None,
            last_clients: None,
            gesture: gestures::Gestures::default(),
            refit: false,
            initial_camera_fit: true,
            refit_checksum: None,
            agent: AgentState::default(),
            feed,
            agent_clients: BTreeMap::new(),
            script_owner: BTreeMap::new(),
        };
        e.backend.subscribe()?;
        // Everything the panels need, once, so the first frame is not empty.
        e.read_state()?;
        e.read_history()?;
        e.read_rows()?;
        e.read_singletons()?;
        e.read_proposals()?;
        e.read_clients(true)?;
        e.fit_camera();
        e.ingest();
        e.refresh_view();
        Ok(e)
    }

    fn read_state(&mut self) -> Result<(), String> {
        let r = self.timed_erp_call("sim.state", J::Null).map_err(|e| format!("sim.state: {e}"))?;
        self.sim = SimState::from_json(&r);
        Ok(())
    }

    fn read_history(&mut self) -> Result<(), String> {
        let r = self.timed_erp_call("history.list", J::Null).map_err(|e| format!("history.list: {e}"))?;
        self.history = History::from_json(&r);
        Ok(())
    }

    fn read_rows(&mut self) -> Result<(), String> {
        let r = self.timed_erp_call("world.query", json!({"limit": ROWS_LIMIT})).map_err(|e| format!("world.query: {e}"))?;
        self.rows = rows_of(&r);
        self.yard_rows_checksum=r.get("checksum").and_then(orr_remote::wire::parse_checksum);
        self.yard_rows_tick=r.get("tick").and_then(J::as_u64).unwrap_or(u64::MAX);
        self.yard_rows_generation=self.change_gen;
        Ok(())
    }

    fn read_singletons(&mut self) -> Result<(), String> {
        let r = self.timed_erp_call("world.singleton.get", J::Null).map_err(|e| format!("world.singleton.get: {e}"))?;
        self.singletons = singletons_of(&self.types, &r);
        Ok(())
    }

    fn read_proposals(&mut self) -> Result<(), String> {
        let r = self.timed_erp_call("proposal.list", J::Null).map_err(|e| format!("proposal.list: {e}"))?;
        self.proposals = r["proposals"].as_array().map(|a| a.iter().filter_map(ProposalInfo::from_json).collect()).unwrap_or_default();
        Ok(())
    }

    fn read_clients(&mut self, backlog: bool) -> Result<(), String> {
        let params = if backlog { json!({"since": 0, "include_reads": true, "limit": 2000}) } else { json!({"since": self.feed.last_seq, "include_reads": true, "limit": 200}) };
        let r = self.timed_erp_call("activity.list", params).map_err(|e| format!("activity.list: {e}"))?;
        self.apply_activity_list(&r);
        Ok(())
    }

    // ---- reads ----

    /// Bounded CPU wall-clock diagnostics, with no host calls or heap allocation.
    /// See [`EditorDiagnostics`] for exact boundaries and rolling-window semantics.
    pub fn diagnostics(&self) -> EditorDiagnostics {
        self.telemetry.summary(self.pending.len())
    }

    pub(crate) fn record_ui_frame(&mut self, elapsed: Duration) {
        self.telemetry.ui_frame.record(elapsed);
    }

    /// Where the editor's host is and how it was started.
    pub fn spec(&self) -> &HostSpec {
        &self.backend.spec
    }

    /// The compiled game adapter selected by discovery.
    pub fn game(&self) -> EditorGame { self.backend.game }

    /// Replay viewers and disconnected hosts never accept scene/debug mutations.
    pub fn is_viewer(&self) -> bool {
        self.sim.viewer || self.timeline().is_some_and(|t| t.mode == orr_bridge::PlayMode::Viewer)
    }

    pub fn can_mutate(&self) -> bool { self.down.is_none() && !self.is_viewer() }

    /// Field moved by the viewport, only when selected entity has it.
    pub fn movement_owner(&self) -> Option<Owner> {
        let component = self.game().position_component();
        let row = self.selection.as_ref().and_then(|t| self.row_of(t))?;
        row.components.iter().any(|c| c == component).then(|| Owner::Component(component.to_string()))
    }

    /// The type registry the inspector draws its widgets from.
    pub fn types(&self) -> &TypeRegistry {
        &self.types
    }

    /// Set once the host is gone (see the module docs); `None` while it is reachable.
    pub fn down(&self) -> Option<&Down> {
        self.down.as_ref()
    }

    /// The host's state (mode, ticks, dirty flag, scene file), as last reported.
    pub fn sim(&self) -> &SimState {
        &self.sim
    }

    /// The mode.
    pub fn mode(&self) -> Mode {
        self.sim.mode
    }

    /// True while a play session exists on the host.
    pub fn is_playing_mode(&self) -> bool {
        self.sim.mode == Mode::Play
    }

    /// The timeline of the play session, from the newest frame (`None` in edit mode).
    pub fn timeline(&self) -> Option<Timeline> {
        if self.sim.mode != Mode::Play {
            return None;
        }
        self.snapshot.as_ref().and_then(|s| s.timeline().cloned())
    }

    /// The newest frame the host published.
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    /// Nonblocking readiness for an opt-in settled screenshot. Never waits
    /// for ERP or changes playback; the ordinary UI pump resolves this state.
    pub(crate) fn screenshot_waiting_for(&self) -> Option<&'static str> {
        if self.down.is_some() { return Some("host connection"); }
        let Some(snapshot) = &self.snapshot else { return Some("frame snapshot") };
        if self.sim.playing || snapshot.timeline().is_some_and(|t| t.playing) { return Some("paused playback"); }
        if self.checksum != self.sim.checksum
            || snapshot.timeline().is_some() != (self.sim.mode == Mode::Play)
            || (self.sim.mode == Mode::Play && snapshot.timeline().is_none_or(|t| t.tick != self.sim.head_tick)) {
            return Some("current frame snapshot");
        }
        if self.dirty.rows || self.dirty.inspect || self.dirty.singletons || self.dirty.history || self.dirty.state
            || !self.pending.is_empty() || self.gesture.busy() || self.refit || self.refit_checksum.is_some() {
            return Some("model refresh");
        }
        if self.selection.as_ref().is_some_and(|t| self.inspect.as_ref().is_none_or(|i| &i.target != t)) {
            return Some("selected entity inspector");
        }
        if self.preview.as_ref().is_some_and(|p| p.seq == 0 || p.rows_dirty) { return Some("proposal snapshot"); }
        None
    }

    /// Takes the next editor-owned screenshot request from the local host.
    pub(crate) fn take_screenshot_request(&self) -> Option<CaptureRequest> {
        self.backend.screenshot_owner.as_ref()?.take_request()
    }

    pub(crate) fn has_local_screenshot_owner(&self) -> bool {
        self.backend.screenshot_owner.is_some()
    }

    pub(crate) fn screenshot_gesture_busy(&self) -> bool {
        self.gesture.busy()
    }

    pub(crate) fn screenshot_is_active(&self, serial: u64) -> bool {
        self.backend.screenshot_owner.as_ref().is_some_and(|owner| owner.is_active(serial))
    }

    pub(crate) fn screenshot_complete(&self, serial: u64, result: Result<CapturedImage, CaptureError>) {
        if let Some(owner) = &self.backend.screenshot_owner {
            owner.complete(serial, result);
        }
    }

    pub(crate) fn screenshot_begin_encoder(&self, serial: u64) -> Result<(), CaptureError> {
        self.backend.screenshot_owner.as_ref().ok_or(CaptureError::Unavailable)?.begin_encoder(serial)
    }

    pub(crate) fn screenshot_begin_capture(&self, serial: u64) -> Result<(), CaptureError> {
        self.backend.screenshot_owner.as_ref().ok_or(CaptureError::Unavailable)?.begin_capture(serial)
    }

    pub(crate) fn screenshot_end_capture(&self, serial: u64) {
        if let Some(owner) = &self.backend.screenshot_owner {
            owner.end_capture(serial);
        }
    }

    pub(crate) fn screenshot_end_encoder(&self, serial: u64) {
        if let Some(owner) = &self.backend.screenshot_owner {
            owner.end_encoder(serial);
        }
    }

    /// Begins fresh host reads after the broker delivered this request.
    pub(crate) fn begin_screenshot_validation(&mut self, request: &CaptureRequest) -> Result<(), CaptureError> {
        if self.down.is_some() || self.preview.is_some() || !request.requested.paused || !self.screenshot_is_active(request.serial) {
            return Err(CaptureError::Unavailable);
        }
        let state_id = self.backend.erp.post("sim.state", J::Null).map_err(|_| CaptureError::Unavailable)?;
        let view_id = match self.backend.erp.post("world.query", json!({"limit": 1, "values": false})) {
            Ok(id) => id,
            Err(_) => return Err(CaptureError::Unavailable),
        };
        self.screenshot_queries.insert(state_id, (request.serial, ScreenshotQueryKind::State));
        self.screenshot_queries.insert(view_id, (request.serial, ScreenshotQueryKind::View));
        self.screenshot_validations.insert(request.serial, ScreenshotValidation {
            requested: request.requested,
            state_id,
            view_id,
            state: None,
            view: None,
        });
        Ok(())
    }

    /// Returns the exact admitted stamp only after fresh host queries and the
    /// editor's current primary snapshot agree and all presentation caches settle.
    pub(crate) fn screenshot_capture_ready(&self, serial: u64) -> Result<Option<ViewState>, CaptureError> {
        if self.down.is_some() || !self.screenshot_is_active(serial) {
            return Err(CaptureError::Unavailable);
        }
        if self.preview.is_some() {
            return Err(CaptureError::Unavailable);
        }
        let validation = self.screenshot_validations.get(&serial).ok_or(CaptureError::Stale)?;
        let (Some(state), Some(view)) = (&validation.state, &validation.view) else { return Ok(None) };
        let state = state.as_ref().map_err(|error| *error)?;
        let (tick, checksum) = view.as_ref().map_err(|error| *error)?;
        let requested = validation.requested;
        if *state != requested || *tick != requested.tick || *checksum != requested.checksum {
            return Err(CaptureError::Stale);
        }
        if self.capture_view_state() != requested || !self.capture_snapshot_matches(requested) {
            return Ok(None);
        }
        if self.screenshot_waiting_for().is_some() || self.gesture.busy() {
            return Ok(None);
        }
        Ok(Some(requested))
    }

    pub(crate) fn cancel_screenshot_validation(&mut self, serial: u64) {
        if let Some(validation) = self.screenshot_validations.remove(&serial) {
            self.screenshot_queries.remove(&validation.state_id);
            self.screenshot_queries.remove(&validation.view_id);
        }
    }

    pub(crate) fn capture_view_state(&self) -> ViewState {
        ViewState {
            mode: if self.sim.mode == Mode::Play { ViewMode::Play } else { ViewMode::Edit },
            paused: !self.sim.playing,
            tick: self.sim.head_tick,
            epoch: self.sim.epoch,
            checksum: self.sim.checksum,
        }
    }

    pub(crate) fn capture_snapshot_seq(&self) -> Option<u64> {
        self.snapshot.as_ref().map(Snapshot::seq)
    }

    pub(crate) fn capture_snapshot_matches(&self, requested: ViewState) -> bool {
        if self.checksum != requested.checksum {
            return false;
        }
        let Some(snapshot) = &self.snapshot else { return false };
        match (requested.mode, snapshot.timeline()) {
            (ViewMode::Edit, None) => requested.tick == 0,
            (ViewMode::Play, Some(timeline)) => {
                !timeline.playing && timeline.tick == requested.tick && timeline.epoch == requested.epoch
            }
            _ => false,
        }
    }

    /// The drawable bodies of the frame on screen (the scene's preview frame in
    /// edit mode, the live frame in play mode).
    pub fn bodies(&self) -> &[Drawable] {
        &self.bodies
    }

    /// The drawable bodies of the staged frame of the previewed proposal, once it has arrived.
    pub fn preview_bodies(&self) -> Option<&[Drawable]> {
        self.preview.as_ref().filter(|p| p.seq > 0).map(|p| p.bodies.as_slice())
    }

    /// The recording of the last play that was stopped.
    pub fn last_stopped(&self) -> Option<&Stopped> {
        self.stopped.as_ref()
    }

    /// The scene file of the host.
    pub fn path(&self) -> Option<PathBuf> {
        self.sim.scene_path.as_ref().map(PathBuf::from)
    }

    /// The hierarchy: every entity of the frame on screen.
    pub fn rows(&self) -> &[EntityRow] {
        &self.rows
    }

    /// The row of an entity.
    pub fn row_of(&self, t: &Target) -> Option<&EntityRow> {
        match t {
            Target::Guid(g) => self.rows.iter().find(|r| r.guid.as_ref() == Some(g)),
            Target::Entity(e) => self.rows.iter().find(|r| r.entity == *e),
        }
    }

    /// The frame entity a target names, as far as the hierarchy knows.
    pub fn entity_of(&self, t: &Target) -> Option<Entity> {
        self.row_of(t).map(|r| r.entity)
    }

    /// The components of the selected entity.
    pub fn inspect(&self) -> Option<&Inspect> {
        self.inspect.as_ref().filter(|i| Some(&i.target) == self.selection.as_ref())
    }

    /// The singletons and their values.
    pub fn singletons(&self) -> &[(String, Value)] {
        &self.singletons
    }

    /// The undo history.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// The selected entity.
    pub fn selection(&self) -> Option<&Target> {
        self.selection.as_ref()
    }

    /// Document GUIDs in stable click order. Runtime handle-only selections
    /// remain inspect-only and are never admitted to a batch.
    pub fn selected_guids(&self) -> &[Guid] { self.guid_selection.guids() }

    /// Membership used by hierarchy and viewport highlighting.
    pub fn is_selected(&self, target: &Target) -> bool {
        match target {
            Target::Guid(guid) => self.guid_selection.contains(guid),
            Target::Entity(_) => self.selection.as_ref() == Some(target),
        }
    }

    /// A batch may have reached the host even though its acknowledgement was
    /// lost. Reconnect reads a freshly fenced document and complete history;
    /// no operation is retried automatically.
    pub fn batch_outcome_uncertain(&self) -> bool { self.batch_uncertain }

    /// Read-only enablement; the host repeats all admission checks.
    pub fn can_nudge_selection(&self) -> bool {
        !self.game().is_3d() && self.can_mutate() && self.mode() == Mode::Edit && self.preview.is_none()
            && self.batch_supported && self.scene_edit_allowed && !self.batch_uncertain
            && !self.history.in_tx && !self.sim.in_tx && !self.gesture.busy()
            && !self.guid_selection.guids().is_empty()
            && self.guid_selection.guids().iter().all(|guid| self.rows.iter().any(|row|
                row.guid.as_ref() == Some(guid)
                    && row.components.iter().any(|c| c == self.game().position_component())))
    }

    /// The selected entity's GUID, if it has one.
    pub fn selected_guid(&self) -> Option<Guid> {
        match self.selection.as_ref()? {
            Target::Guid(g) => Some(g.clone()),
            Target::Entity(e) => self.rows.iter().find(|r| r.entity == *e).and_then(|r| r.guid.clone()),
        }
    }

    /// True if the document has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.history.dirty || self.sim.dirty
    }

    /// The window title, with a `*` when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self.sim.scene_path.as_deref().and_then(|p| Path::new(p).file_name()).map_or_else(|| "untitled".to_string(), |n| n.to_string_lossy().into_owned());
        let mode = if self.sim.mode == Mode::Play { " [play]" } else { "" };
        let host = match (&self.backend.spec, &self.backend.url) {
            (HostSpec::Remote { .. }, Some(u)) => format!(" @ {u}"),
            _ => String::new(),
        };
        format!("{}{name}{mode}{host} - Orrery Editor", if self.is_dirty() { "*" } else { "" })
    }

    /// The last message (status bar).
    pub fn status(&self) -> Option<&Message> {
        self.status.as_ref()
    }

    /// Every message, oldest first (capped).
    pub fn log(&self) -> &[Message] {
        &self.log
    }

    /// Checksum of the frame on screen.
    pub fn checksum(&self) -> u64 {
        self.checksum
    }

    // ---- messages ----

    /// Records an informational message.
    pub fn info(&mut self, text: impl Into<String>) {
        self.push_message(Message { text: text.into(), error: false });
    }

    /// Records a refusal or failure.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push_message(Message { text: text.into(), error: true });
    }

    fn push_message(&mut self, m: Message) {
        self.status = Some(m.clone());
        self.log.push(m);
        if self.log.len() > LOG_LIMIT {
            self.log.remove(0);
        }
    }

    // ---- selection and camera ----

    /// Selects an entity (or clears the selection).
    pub fn select(&mut self, target: Option<Target>) {
        match &target {
            Some(Target::Guid(guid)) => self.guid_selection.replace(guid.clone()),
            _ => self.guid_selection.clear(),
        }
        self.set_primary_selection(target);
    }

    /// Ctrl-click toggles a document GUID; a handle-only target keeps the
    /// original single inspect path. Refusing a 129th GUID changes nothing.
    pub fn toggle_selection(&mut self, target: Target) -> bool {
        if self.game().is_3d() { self.select(Some(target)); return true; }
        let Target::Guid(guid) = target else { self.select(Some(target)); return true };
        if self.rows.iter().all(|row| row.guid.as_ref() != Some(&guid)) {
            self.error("cannot select a GUID absent from the current document rows");
            return false;
        }
        if let Err(error) = self.guid_selection.toggle(guid) {
            self.error(error.to_string());
            return false;
        }
        let primary = self.guid_selection.primary().cloned().map(Target::Guid);
        self.set_primary_selection(primary);
        true
    }

    fn set_primary_selection(&mut self, target: Option<Target>) {
        if self.selection != target {
            self.cancel_edit();
            self.selection = target;
            self.inspect = None;
            self.dirty.inspect = true;
            self.last_inspect = None;
        }
    }

    /// Selects the entity with this display name or GUID text.
    pub fn select_named(&mut self, name: &str) -> bool {
        let found = self.rows.iter().find(|r| r.name.as_deref() == Some(name) || r.guid.as_ref().is_some_and(|g| g.to_string() == name)).map(EntityRow::target);
        match found {
            Some(t) => {
                self.select(Some(t));
                true
            }
            None => false,
        }
    }

    /// Drops a selection that no longer exists (after an undo, a despawn or a rewind).
    pub fn sanitize_selection(&mut self) {
        if !self.dirty.rows && !self.inflight.rows {
            self.guid_selection.retain_rows(&self.rows);
            if matches!(self.selection, Some(Target::Guid(_))) {
                self.set_primary_selection(self.guid_selection.primary().cloned().map(Target::Guid));
            }
        }
        if let Some(t) = &self.selection {
            if !self.rows.is_empty() && self.row_of(t).is_none() {
                self.select(None);
            }
        }
        if let Some(id) = self.agent.preview.clone() {
            if !self.proposals.iter().any(|p| p.id == id) {
                self.set_preview(None);
            }
        }
    }

    /// Frames the camera on the scene box (`Scene` singleton), like the sample does.
    pub fn fit_camera(&mut self) {
        if self.game().is_room() {
            #[cfg(feature="room-project")] if self.has_room_camera() { self.reset_room_camera(); return; }
        }
        if self.game().is_3d() { self.camera3d = orr_render::OrbitCamera::new([0.0,2.0,0.0],0.55,0.5,16.0); return; }
        if self.game().is_collect() {
            let mut low = [f32::INFINITY; 2]; let mut high = [f32::NEG_INFINITY; 2];
            for body in &self.bodies { for axis in 0..2 { let radius = body.outline.extent(); low[axis] = low[axis].min(body.pos[axis] - radius); high[axis] = high[axis].max(body.pos[axis] + radius); } }
            self.camera = if self.bodies.is_empty() { Camera::new([0.0, 0.0], 270.0) } else { Camera::new([(low[0]+high[0])*0.5,(low[1]+high[1])*0.5],((high[0]-low[0]).max(high[1]-low[1])*0.6).max(40.0)) };
            return;
        }
        if self.game() == EditorGame::Arena {
            let half = orr_view::fp_to_f32(orr_sample::arena_game::ARENA_HALF);
            self.camera = Camera::new([0.0, 0.0], half * 1.06);
            // Authoring commonly uses a few players near the center. Frame
            // those drawables so their hit targets are useful immediately.
            if !self.bodies.is_empty() {
                let mut low = [f32::INFINITY; 2];
                let mut high = [f32::NEG_INFINITY; 2];
                for b in &self.bodies {
                    let r = b.outline.extent();
                    for i in 0..2 { low[i] = low[i].min(b.pos[i] - r); high[i] = high[i].max(b.pos[i] + r); }
                }
                self.camera = Camera::new([(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5],
                    ((high[0] - low[0]).max(high[1] - low[1]) * 0.6).max(100.0));
            }
            return;
        }
        let get = |name: &str| {
            let (_, v) = self.singletons.iter().find(|(n, _)| n == "Scene")?;
            let Value::Struct(fields) = v else { return None };
            match fields.iter().find(|(n, _)| n == name)? {
                (_, Value::Fixed(f)) => Some(fp_to_f64(*f)),
                _ => None,
            }
        };
        if let (Some(half_w), Some(height)) = (get("half_w"), get("height")) {
            self.camera = Camera::new([0.0, (height / 2.0) as f32], (half_w.max(height / 2.0) * 1.06) as f32);
        }
    }

    // ---- talking to the host ----

    fn timed_erp_call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        let started = Instant::now();
        let result = self.backend.erp.call(method, params);
        self.telemetry.sync_erp_wait.record(started.elapsed());
        result
    }

    /// Runs a method and waits for the answer. `Err` carries the host's message.
    fn call(&mut self, method: &str, params: J) -> Result<J, String> {
        if let Some(d) = &self.down {
            return Err(d.reason.clone());
        }
        if self.is_viewer() && !gestures::is_read(method) && !matches!(method, "sim.play" | "sim.pause" | "sim.step" | "sim.seek" | "sim.speed" | "sim.stop") {
            return Err("replay Viewer is read-only; editor mutations and branching are disabled".into());
        }
        if self.gesture.busy() && !gestures::is_read(method) {
            return Err("an edit is still pending; wait for it to finish or cancel the drag".into());
        }
        if method.starts_with("sim.") && method != "sim.state" { self.state_generation += 1; }
        let r = self.timed_erp_call(method, params);
        self.ingest();
        match r {
            Ok(v) => Ok(v),
            Err(ClientError::Rpc(e)) => Err(e.message),
            Err(other) => {
                self.connection_lost(&other.to_string());
                Err(other.to_string())
            }
        }
    }

    /// Sends a request without waiting; the answer is handled by [`pump`](Self::pump).
    fn post(&mut self, method: &str, params: J, kind: Pend) {
        if self.down.is_some() {
            return;
        }
        let posted_at = Instant::now();
        match self.backend.erp.post(method, params) {
            Ok(id) => {
                self.pending.insert(id, Pending { kind, posted_at });
                self.telemetry.pending_high_water = self.telemetry.pending_high_water.max(self.pending.len());
            }
            Err(e) => self.connection_lost(&e.to_string()),
        }
    }

    /// Takes in what the ERP client collected: notifications and answers.
    fn ingest(&mut self) {
        let notes: Vec<J> = self.backend.erp.notifications.drain(..).collect();
        for n in notes {
            let (Some(method), Some(params)) = (n.get("method").and_then(J::as_str), n.get("params")) else { continue };
            self.on_notification(method, params);
        }
        let answers: Vec<_> = self.backend.erp.responses.drain(..).collect();
        for (id, r) in answers {
            if let Some((serial, kind)) = self.screenshot_queries.remove(&id) {
                self.on_screenshot_answer(serial, kind, r);
            } else if let Some(pending) = self.pending.remove(&id) {
                self.telemetry.async_request.record(pending.posted_at.elapsed());
                self.on_answer(pending.kind, r);
            }
        }
        // Frames are not asked for on this channel.
        self.backend.erp.frames.clear();
        self.backend.erp.local_frames.clear();
    }

    fn on_screenshot_answer(&mut self, serial: u64, kind: ScreenshotQueryKind, result: Result<J, orr_remote::RpcError>) {
        let Some(validation) = self.screenshot_validations.get_mut(&serial) else { return };
        match (kind, result) {
            (ScreenshotQueryKind::State, Ok(value)) => validation.state = Some(parse_capture_state(&value).ok_or(CaptureError::Failed)),
            (ScreenshotQueryKind::View, Ok(value)) => validation.view = Some(parse_capture_view(&value).ok_or(CaptureError::Failed)),
            (ScreenshotQueryKind::State, Err(_)) => validation.state = Some(Err(CaptureError::Failed)),
            (ScreenshotQueryKind::View, Err(_)) => validation.view = Some(Err(CaptureError::Failed)),
        }
    }

    fn on_notification(&mut self, method: &str, params: &J) {
        match method {
            "watch.tick" => {
                let u = |k: &str| params.get(k).and_then(J::as_u64).unwrap_or(0);
                let mode = if params.get("mode").and_then(J::as_str) == Some("play") { Mode::Play } else { Mode::Edit };
                let epoch = u("epoch");
                let changed = mode != self.sim.mode || (mode == Mode::Play && epoch != self.sim.epoch);
                self.sim.mode = mode;
                self.sim.head_tick = u("tick");
                self.sim.last_tick = u("last_tick");
                self.sim.epoch = epoch;
                self.sim.playing = params.get("playing").and_then(J::as_bool).unwrap_or(false);
                // A paused tick can change the checksum within the same
                // Play epoch. Refresh after ingesting this batch: an older
                // in-flight State reply may follow these notifications.
                if changed || !self.sim.playing { self.dirty.state = true; }
                if changed {
                    self.mark_changed();
                } else if mode == Mode::Play {
                    self.dirty.inspect = true;
                }
            }
            "watch.history" => {
                let b = |k: &str| params.get(k).and_then(J::as_bool).unwrap_or(false);
                self.history.can_undo = b("can_undo");
                self.history.can_redo = b("can_redo");
                self.history.dirty = b("dirty");
                self.history.in_tx = b("in_tx");
                self.observe_gesture_transaction(b("in_tx"));
                self.dirty.history = true;
                self.dirty.state = true;
                self.mark_changed();
            }
            "watch.proposals" => {
                if let Some(open) = params.get("open").and_then(J::as_array) {
                    self.proposals = open.iter().filter_map(ProposalInfo::from_json).collect();
                    let ids: Vec<String> = self.proposals.iter().map(|p| p.id.clone()).collect();
                    self.details.retain(|id, d| ids.contains(id) && self.proposals.iter().any(|p| p.id == *id && p.op_count == d.info.op_count && p.stale == d.info.stale));
                    self.sanitize_selection();
                }
            }
            "watch.activity" => {
                if let Some(list) = params.get("entries").and_then(J::as_array) {
                    if list.iter().any(|entry| entry["ok"] == true
                        && (entry["method"] == "scene.load"
                            || (entry["method"] == "scene.save" && entry["scene_path_changed"] == true))) {
                        self.clear_room_camera();
                        self.select(None);
                    }
                    self.feed.push(list.iter().filter_map(FeedEntry::from_json).collect());
                }
            }
            _ => {}
        }
    }

    fn on_answer(&mut self, kind: Pend, r: Result<J, orr_remote::RpcError>) {
        match kind {
            Pend::Input(intent, op) => {
                if let Some(message) = self.input.answer(intent, op, r, Instant::now()) { self.error(message); }
            }
            Pend::Gesture(id, reply) => self.gesture_answer(id, reply, r),
            Pend::Rows(gen) => {
                self.inflight.rows = false;
                if let Ok(v) = r {
                    self.rows = rows_of(&v);
                    self.yard_rows_checksum=v.get("checksum").and_then(orr_remote::wire::parse_checksum);
                    self.yard_rows_tick=v.get("tick").and_then(J::as_u64).unwrap_or(u64::MAX);
                    self.yard_rows_generation=gen;
                    if gen != self.change_gen { self.dirty.rows=true; }
                    if gen == self.change_gen {
                        self.sanitize_selection();
                    }
                } else { self.dirty.rows=true; }
            }
            Pend::Inspect(t) => {
                self.inflight.inspect = false;
                match r {
                    Ok(v) => self.apply_entity(&t, &v),
                    Err(_) => {
                        if self.inspect.as_ref().is_some_and(|i| i.target == t) {
                            self.inspect = None;
                        }
                    }
                }
            }
            Pend::Singletons => {
                self.inflight.singletons = false;
                if let Ok(v) = r {
                    self.singletons = singletons_of(&self.types, &v);
                    if self.refit && self.game() == EditorGame::PhysGame {
                        self.refit = false;
                        self.fit_camera();
                    }
                }
            }
            Pend::History => {
                self.inflight.history = false;
                if let Ok(v) = r {
                    self.history = History::from_json(&v);
                }
            }
            Pend::State(generation) => {
                self.inflight.state = false;
                if generation != self.state_generation { self.dirty.state = true; return; }
                if let Ok(v) = r {
                    self.apply_state(&v);
                }
            }
            Pend::Clients => {
                self.inflight.clients = false;
                if let Ok(v) = r {
                    self.apply_activity_list(&v);
                }
            }
            Pend::Detail(id) => {
                self.inflight.details.retain(|d| *d != id);
                if let Ok(v) = r {
                    if let Some(d) = ProposalDetail::from_json(&v) {
                        self.details.insert(id, d);
                    }
                }
            }
            Pend::PreviewRows(id, generation) => {
                if self.inflight.preview_rows == Some(generation) {
                    self.inflight.preview_rows = None;
                }
                if let (Ok(v), Some(p)) = (r, self.preview.as_mut()) {
                    if p.id == id && p.generation == generation {
                        p.rows = rows_of(&v);
                    }
                }
            }
        }
    }

    fn apply_state(&mut self, v: &J) {
        let was = self.sim.mode;
        let next=SimState::from_json(v);
        if self.sim.scene_path != next.scene_path { self.clear_room_camera(); }
        self.sim = next;
        if was != self.sim.mode {
            self.mark_changed();
        }
        if self.sim.dirty != self.history.dirty {
            self.history.dirty = self.sim.dirty;
        }
        self.history.in_tx = self.sim.in_tx;
    }

    fn apply_activity_list(&mut self, v: &J) {
        if let Some(list) = v.get("entries").and_then(J::as_array) {
            self.feed.push(list.iter().filter_map(FeedEntry::from_json).collect());
        }
        if let Some(ms) = v.get("now_ms").and_then(J::as_u64) {
            self.server_now_ms = ms;
        }
        if let Some(list) = v.get("clients").and_then(J::as_array) {
            self.clients = list
                .iter()
                .filter_map(|c| {
                    let caps = c.get("capabilities")?.as_array()?.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",");
                    Some(ClientInfo { name: c.get("client")?.as_str()?.to_string(), caps, requests: c.get("requests").and_then(J::as_u64).unwrap_or(0) })
                })
                .filter(|c| c.name != self.backend.own_client)
                .collect();
        }
    }

    /// The answer to `world.get` of the selected entity.
    fn apply_entity(&mut self, t: &Target, v: &J) {
        if self.selection.as_ref() != Some(t) {
            return;
        }
        let row = v.get("entity").and_then(EntityRow::from_json);
        let mut components = Vec::new();
        if let Some(obj) = v.get("components").and_then(J::as_object) {
            for (name, j) in obj {
                let Some(ty) = self.types.get(name) else { continue };
                if let Ok(val) = json_to_value(ty.desc(), j, false) {
                    components.push((name.clone(), val));
                }
            }
        }
        self.inspect = Some(Inspect { target: t.clone(), row, components });
    }

    /// The host says something changed that the caches may not know: ask again.
    fn mark_changed(&mut self) {
        self.change_gen += 1;
        self.dirty.rows = true;
        self.dirty.inspect = true;
        self.dirty.singletons = true;
    }

    /// Schedule authoritative cache/frame refresh after a successful prefab ERP edit.
    #[cfg(feature = "linked-prefabs")]
    pub(crate) fn prefab_mark_edited(&mut self) {
        self.mark_edited();
    }

    fn mark_edited(&mut self) {
        self.mark_changed();
        self.dirty.history = true;
        self.dirty.state = true;
    }

    fn connection_lost(&mut self, err: &str) {
        if self.down.is_some() {
            return;
        }
        let down = match &self.backend.host {
            Some(h) => {
                let why = h.stopped_reason(Duration::ZERO);
                match why {
                    Some(w) => Down { reason: format!("simulation host stopped: {w}"), local: true },
                    None => Down { reason: format!("disconnected: {err}"), local: true },
                }
            }
            None => Down { reason: format!("disconnected: {err}"), local: false },
        };
        self.error(down.reason.clone());
        self.gesture = gestures::Gestures::default();
        self.input = input::Input::default();
        self.pending.clear();
        self.screenshot_queries.clear();
        self.screenshot_validations.clear();
        self.down = Some(down);
    }

    // ---- per frame ----

    /// Call once per UI frame: takes in notifications, answers and the newest
    /// frame, and sends the requests that refresh what changed. Never waits
    /// for the host.
    pub fn pump(&mut self) {
        let started = Instant::now();
        self.pump_inner();
        self.telemetry.pump.record(started.elapsed());
    }

    fn pump_inner(&mut self) {
        if self.down.is_some() {
            // A panic closes the channel just before its thread records why.
            // Keep the initial error, then enrich it on a later frame instead
            // of waiting for the local host (which might still be healthy).
            if let Some(why) = self.backend.host.as_ref().and_then(|h| h.stopped_reason(Duration::ZERO)) {
                let reason = format!("simulation host stopped: {why}");
                if self.down.as_ref().is_some_and(|d| d.reason != reason) {
                    self.down.as_mut().expect("checked above").reason = reason.clone();
                    self.error(reason);
                }
            }
            return;
        }
        if let Err(e) = self.backend.erp.poll() {
            self.ingest();
            self.connection_lost(&e.to_string());
            return;
        }
        self.ingest();
        self.check_gesture_timeout();
        self.pump_input();
        self.refresh_view();
        if self.down.is_some() {
            return;
        }
        self.refresh_preview();
        self.send_refreshes();
    }

    /// Consume the frame, notifications and recovery fence as one pinned read.
    fn refresh_view(&mut self) {
        let update = self.backend.bridge.poll_view();
        let errors = self.backend.bridge.take_errors();
        let failure = errors.iter().rev().find(|e| e.kind() == Some("view_delivery_invalid")).map(|e| e.message.clone());
        for error in errors {
            self.error(format!("frame stream: {error}"));
        }
        // Preserve the actionable protocol failure before consuming the
        // terminal Disconnected notification, which otherwise has no reason.
        if let Some(reason) = failure {
            self.connection_lost(&reason);
        }
        self.apply_view_update(update);
        if !self.backend.bridge.is_alive() {
            self.connection_lost("the frame stream closed");
        }
    }

    fn apply_view_update<E>(&mut self, update: ViewUpdate<E>) {
        let ViewUpdate { snapshot, events, resync } = update;
        let disconnected = resync.as_ref().is_some_and(|r| r.disconnected);
        if let Some(reset) = &resync {
            // The editor draws the current predicted pose directly and owns no
            // interpolator or speculative VFX handles. Rebuild that complete
            // presentation baseline even when its sequence has not changed.
            // Do not discard pending ERP responses: these diagnostics are not
            // acknowledgements of the requests on that independent channel.
            self.snapshot = None;
            self.bodies.clear();
            self.checksum = 0;
            self.mark_changed();
            self.dirty.state = true;
            for message in recovery_messages("view", reset) {
                self.push_message(message);
            }
        }
        if let Some(snapshot) = snapshot {
            self.apply_snapshot(snapshot);
        }
        if disconnected {
            self.connection_lost("the frame stream closed");
            return;
        }
        for event in events {
            match event {
                BridgeEvent::Lifecycle(Lifecycle::DebugRejected(e)) => self.error(format!("debug edit refused: {e}")),
                BridgeEvent::Lifecycle(Lifecycle::Disconnected) => {
                    self.connection_lost("the frame stream closed");
                    return;
                }
                _ => {}
            }
        }
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
        if self.snapshot.as_ref().is_some_and(|o| o.seq() == s.seq()) {
            return;
        }
        let started = Instant::now();
        if self.game().is_3d() && self.snapshot.as_ref().is_some_and(|old| old.timeline().is_some()!=s.timeline().is_some() || (s.timeline().is_some()&&s.tick()<old.tick())) { self.mark_changed(); }
        let frame = s.predicted();
        self.bodies = self.backend.game.drawables(frame);
        if self.game().is_3d() {
            self.yard_frame = crate::viewport3d::YardFrame::extract(frame);
            #[cfg(feature = "room-project")]
            if self.game().is_room() {
                self.yard_frame.items = orr_sample::room_view::items(frame);
                self.yard_frame.poses.retain(|(entity,_)| self.yard_frame.items.iter().any(|item|item.entity==*entity));
            }
            if (s.timeline().is_none()&&self.yard_rows_checksum!=Some(frame.checksum())) || (s.timeline().is_some()&&self.yard_rows_tick>s.tick()) {self.dirty.rows=true;}
        }
        #[cfg(feature = "terrain-physics")]
        if self.game().is_terrain() {
            let previous = self.terrain_view.error.clone();
            self.terrain_view.update(frame);
            if previous != self.terrain_view.error {
                if let Some(error) = self.terrain_view.error.clone() { self.error(error); }
            }
        }
        #[cfg(feature = "navigation")]
        if self.game().is_navigation() { self.navigation_view.update(frame); }
        self.checksum = frame.checksum();
        let alive = frame.alive_count();
        self.telemetry.snapshot_extract.record(started.elapsed());
        if alive != self.snap_entities {
            self.snap_entities = alive;
            if alive as usize != self.rows.len() {
                self.dirty.rows = true;
            }
        }
        self.dirty.inspect = true;
        if self.sim.mode == Mode::Play {
            self.dirty.singletons = true;
        }
        self.snapshot = Some(s);
        if self.refit_checksum == Some(self.checksum) {
            self.refit_checksum = None;
            self.refit = false;
            self.fit_camera();
        }
        if self.initial_camera_fit {
            self.initial_camera_fit = false;
            if self.game().has_keyboard() { self.fit_camera(); }
        }
    }

    fn refresh_preview(&mut self) {
        let Some(p) = self.preview.as_mut() else { return };
        // Preview streams receive lifecycle notifications too. Drain them on
        // every render, even when the proposal's paused snapshot is unchanged.
        let update = p.stream.poll_view();
        let errors = p.stream.take_errors();
        let mut messages = update.resync.as_ref().map(|r| recovery_messages("preview", r)).unwrap_or_default();
        let had_error = !errors.is_empty();
        messages.extend(errors.into_iter().map(|e| Message { text: format!("preview frame stream: {e}"), error: true }));
        for event in &update.events {
            if let BridgeEvent::Lifecycle(life @ (Lifecycle::DebugRejected(_) | Lifecycle::SeekRejected { .. } | Lifecycle::Desync { .. })) = event {
                messages.push(Message { text: format!("preview frame stream diagnostic: {life:?}"), error: true });
            }
        }
        let disconnected = !p.stream.is_alive()
            || update.resync.as_ref().is_some_and(|r| r.disconnected)
            || update.events.iter().any(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Disconnected)));
        if disconnected {
            self.preview = None;
            for message in messages {
                self.push_message(message);
            }
            if !had_error {
                self.error("preview frame stream closed");
            }
            return;
        }
        let recovered = update.resync.is_some();
        if recovered {
            p.seq = 0;
            p.bodies.clear();
            p.rows.clear();
        }
        let changed = if let Some(s) = update.snapshot {
            if s.seq() == p.seq && !recovered {
                false
            } else {
                p.seq = s.seq();
                p.bodies = self.backend.game.drawables(s.predicted());
                true
            }
        } else {
            false
        };
        if recovered || changed {
            self.preview_generation += 1;
            p.generation = self.preview_generation;
            p.rows_dirty = true;
        }
        let refresh_rows = p.seq > 0 && p.rows_dirty && self.inflight.preview_rows.is_none();
        let id = p.id.clone();
        let generation = p.generation;
        if refresh_rows {
            p.rows_dirty = false;
        }
        for message in messages {
            self.push_message(message);
        }
        if refresh_rows {
            self.inflight.preview_rows = Some(generation);
            self.post("proposal.preview", json!({"id": id, "values": false, "limit": ROWS_LIMIT}), Pend::PreviewRows(id, generation));
        }
    }

    fn send_refreshes(&mut self) {
        let now = Instant::now();
        if self.dirty.state && !self.inflight.state {
            self.dirty.state = false;
            self.inflight.state = true;
            self.post("sim.state", J::Null, Pend::State(self.state_generation));
        }
        if self.dirty.history && !self.inflight.history {
            self.dirty.history = false;
            self.inflight.history = true;
            self.post("history.list", J::Null, Pend::History);
        }
        if self.dirty.rows && !self.inflight.rows {
            self.dirty.rows = false;
            self.inflight.rows = true;
            let gen = self.change_gen;
            self.post("world.query", json!({"limit": ROWS_LIMIT}), Pend::Rows(gen));
        }
        let due = self.last_inspect.is_none_or(|t| now.duration_since(t) >= INSPECT_EVERY);
        if due && self.dirty.inspect {
            if let (Some(t), false) = (self.selection.clone(), self.inflight.inspect) {
                self.dirty.inspect = false;
                self.inflight.inspect = true;
                self.last_inspect = Some(now);
                self.post("world.get", json!({"entity": t.param()}), Pend::Inspect(t));
            } else if self.selection.is_none() {
                self.dirty.inspect = false;
            }
        }
        if due && self.dirty.singletons && !self.inflight.singletons {
            self.dirty.singletons = false;
            self.inflight.singletons = true;
            self.last_inspect = Some(now);
            self.post("world.singleton.get", J::Null, Pend::Singletons);
        }
        let want: Vec<String> = self.proposals.iter().filter(|p| !self.details.contains_key(&p.id) && !self.inflight.details.contains(&p.id)).map(|p| p.id.clone()).collect();
        for id in want {
            self.inflight.details.push(id.clone());
            self.post("proposal.get", json!({"id": id}), Pend::Detail(id));
        }
        // A new periodic poll would make a settled capture appear busy between
        // its GPU ticket and readback/encoding completion. Keep existing replies
        // and all state refreshes flowing; only defer this timer for live tickets.
        if !self.inflight.clients
            && self.last_clients.is_none_or(|t| now.duration_since(t) >= CLIENTS_EVERY)
            && !self.screenshot_validations.keys().any(|serial| self.screenshot_is_active(*serial))
        {
            self.last_clients = Some(now);
            self.inflight.clients = true;
            let since = self.feed.last_seq;
            self.post("activity.list", json!({"since": since, "include_reads": true, "limit": 200}), Pend::Clients);
        }
    }

    /// Brings every cache up to date and waits for the frame of the host's
    /// current state: what a test (or a script) calls before it looks at the
    /// editor. The UI never calls it.
    pub fn sync(&mut self) {
        // Notifications lag a little behind the host (it checks the history a few times a second):
        // ask for everything once, so the caches are what the host has now.
        self.mark_edited();
        for _ in 0..12 {
            self.last_inspect = None;
            self.last_clients = None;
            self.pump();
            if self.down.is_some() {
                return;
            }
            // The host answers in order: this returns after every request sent so far.
            let Ok(state) = self.call("sim.state", J::Null) else { return };
            self.apply_state(&state);
            // The frame the host publishes for that state. The checksum alone is
            // not enough: right after a pause the tick and checksum are the same
            // as the last frame that was still marked playing.
            if !self.sim.playing {
                let want = self.sim.checksum;
                let end = Instant::now() + Duration::from_secs(3);
                while Instant::now() < end {
                    self.refresh_view();
                    let timeline_matches = self.sim.mode != Mode::Play
                        || self.timeline().is_some_and(|t| !t.playing && t.tick == self.sim.head_tick);
                    if self.down.is_some() || (self.checksum == want && timeline_matches) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            self.wait_preview();
            self.pump();
            let quiet = !(self.dirty.rows || self.dirty.inspect || self.dirty.singletons || self.dirty.history || self.dirty.state) && self.pending.is_empty();
            if quiet {
                break;
            }
            // One more barrier, so the answers to what `pump` just sent have arrived.
            if self.call("sim.state", J::Null).is_err() {
                return;
            }
        }
        let _ = self.backend.erp.poll();
        self.ingest();
    }

    /// Waits (briefly) until the staged frame of the previewed proposal and its entities have arrived.
    fn wait_preview(&mut self) {
        let end = Instant::now() + Duration::from_secs(3);
        while self.down.is_none()
            && self.previewing().is_some()
            && self.preview.as_ref().is_some_and(|p| p.seq == 0 || p.rows.is_empty() || self.inflight.preview_rows.is_some())
        {
            if Instant::now() > end {
                break;
            }
            self.pump();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // ---- files ----

    /// Replaces the document with a scene file. Refused in play mode.
    pub fn open_path(&mut self, path: &Path) -> bool {
        if self.sim.mode == Mode::Play {
            self.error("stop play before opening a scene");
            return false;
        }
        if self.previewing().is_some() {
            self.error("clear the proposal preview before opening a scene");
            return false;
        }
        if self.game().has_pinned_scene() && self.path().as_deref() != Some(path) {
            self.error("Pinned terrain/navigation hosts are bound to their original scene directory. Open the other scene in a new local host");
            return false;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.error(format!("cannot read {}: {e}", path.display()));
                return false;
            }
        };
        match self.call("scene.load", if self.game().has_pinned_scene() { json!({"text": text}) } else { json!({"text": text, "path": path.display().to_string()}) }) {
            Ok(r) => {
                self.clear_room_camera();
                self.select(None);
                self.refit = true;
                if self.game() == EditorGame::Arena {
                    self.refit_checksum = r.get("checksum").and_then(orr_remote::wire::parse_checksum);
                }
                self.backend.spec.set_local_scene_path(path.to_path_buf());
                self.mark_edited();
                self.info(format!("opened {}", path.display()));
                true
            }
            Err(e) => {
                self.error(format!("{}: {e}", path.display()));
                false
            }
        }
    }

    /// Saves to the current path. `false` (and a message) if there is none or it fails.
    pub fn save(&mut self) -> bool {
        if self.sim.scene_path.is_none() {
            self.error("no file name yet: use Save As");
            return false;
        }
        self.save_with(json!({"write": true}))
    }

    /// Saves the scene text to `path` and makes it the current path.
    pub fn save_as(&mut self, path: &Path) -> bool {
        if self.game().has_pinned_scene() {
            self.error("Pinned terrain/navigation scene paths are fixed for this host; use Save to preserve its pinned source directory");
            return false;
        }
        self.save_with(json!({"write": true, "path": path.display().to_string()}))
    }

    fn save_with(&mut self, params: J) -> bool {
        if self.history.in_tx || self.gesture.busy() {
            self.error("finish the current edit before saving");
            return false;
        }
        match self.call("scene.save", params) {
            Ok(r) => {
                let written = r.get("written").and_then(J::as_str).unwrap_or_default().to_string();
                if self.sim.scene_path.as_deref() != Some(written.as_str()) { self.clear_room_camera(); }
                self.sim.scene_path = Some(written.clone());
                if !written.is_empty() {
                    self.backend.spec.set_local_scene_path(PathBuf::from(&written));
                }
                self.sim.dirty = false;
                self.history.dirty = false;
                self.dirty.state = true;
                self.dirty.history = true;
                self.info(format!("saved {written}"));
                true
            }
            Err(e) => {
                self.error(format!("cannot save: {e}"));
                false
            }
        }
    }

    // ---- editing ----

    /// Applies an exact fixed-point displacement to every selected GUID in
    /// one guarded RPC. Values are read from the host, never reconstructed
    /// from floating-point drawables. Any intervening document mutation makes
    /// the captured checksum stale and the entire request is refused.
    pub fn nudge_selected(&mut self, offset: FPVec2) -> bool {
        if !self.can_nudge_selection() {
            self.error("selection move requires editable scene mode, current document GUIDs, SceneEdit and no pending transaction or preview");
            return false;
        }
        if offset == FPVec2::ZERO { return false; }
        let state = match self.call("sim.state", J::Null) {
            Ok(state) => state,
            Err(error) => { self.error(error); return false; }
        };
        let fresh = SimState::from_json(&state);
        self.apply_state(&state);
        if fresh.mode != Mode::Edit || fresh.viewer || fresh.in_tx {
            self.error("selection move refused: host is not an idle editable document");
            return false;
        }
        let Some(expected_checksum) = state.get("doc_checksum").and_then(orr_remote::wire::parse_checksum) else {
            self.error("host supplied no document checksum"); return false;
        };
        let guids = self.guid_selection.guids().to_vec();
        let component = self.game().position_component();
        let mut patches = Vec::with_capacity(guids.len());
        for guid in &guids {
            let position = match self.field_of(&Target::Guid(guid.clone()), component, self.game().position_field()) {
                Ok(Value::Vec2(position)) => position,
                Ok(_) => { self.error("selected position is not a two-dimensional fixed-point value"); return false; }
                Err(error) => { self.error(error); return false; }
            };
            let (Some(x), Some(y)) = (position.x.raw().checked_add(offset.x.raw()), position.y.raw().checked_add(offset.y.raw())) else {
                self.error("selection move would overflow a fixed-point position"); return false;
            };
            patches.push(json!({"guid":guid.to_string(), "value":value_to_json(&Value::Vec2(FPVec2::new(FP::from_raw(x), FP::from_raw(y))))}));
        }
        let params = json!({"label":"move selected entities", "expected_checksum":format!("0x{expected_checksum:016x}"),
            "component":component, "path":self.game().position_field(), "patches":patches});
        if match serde_json::to_vec(&params) { Ok(bytes) => bytes.len() > 64 * 1024, Err(_) => true } {
            self.error("selection move exceeds the 64 KiB request limit"); return false;
        }
        if !self.can_nudge_selection() || self.guid_selection.guids() != guids.as_slice() {
            self.error("selection or document changed during position reads; move was not submitted"); return false;
        }
        match self.call("world.patch_batch", params) {
            Ok(result) => {
                let Some(changed) = result["changed"].as_bool().filter(|_| result.get("checksum").and_then(orr_remote::wire::parse_checksum).is_some()) else {
                    self.batch_uncertain = true;
                    self.connection_lost("selection move acknowledgement was malformed; outcome uncertain, reconnect before editing");
                    return false;
                };
                if changed { self.mark_edited(); }
                changed
            }
            Err(error) => {
                if self.down.is_some() {
                    self.batch_uncertain = true;
                    self.error("selection move outcome is uncertain; reconnect and read the document and history before editing; the request will not be retried");
                } else { self.error(format!("selection move: {error}")); }
                false
            }
        }
    }

    /// Sets one field of the selected entity's component (or a singleton).
    /// `path` is a reflect path (`"pos.x"`, `""` = whole). Returns true if
    /// something changed. A refusal is reported as an error message. Inside a
    /// gesture, true means queued, not accepted: latest values for the same
    /// field coalesce until sent. A host refusal rolls the gesture back and is
    /// reported when its reply arrives. Caches only show host-confirmed values.
    pub fn set_field(&mut self, owner: &Owner, path: &str, value: Value) -> bool {
        if self.is_viewer() {
            self.error("replay Viewer is read-only");
            return false;
        }
        if let Some(down) = &self.down {
            self.error(down.reason.clone());
            return false;
        }
        let (path, value) = self.variant_switch(owner, path, value);
        let (path, value) = (path.as_str(), value);
        let (what, method, params) = match owner {
            Owner::Component(c) => {
                let Some(t) = &self.selection else {
                    self.error(format!("set {c}.{path}: nothing selected"));
                    return false;
                };
                (format!("set {c}.{path}"), "world.patch", json!({"entity": t.param(), "component": c, "path": path, "value": value_to_json(&value)}))
            }
            Owner::Singleton(s) => (format!("set {s}.{path}"), "world.singleton.patch", json!({"name": s, "path": path, "value": value_to_json(&value)})),
        };
        if self.gesture.has_input() {
            return self.queue_gesture_edit(what, method, params);
        }
        if self.gesture.busy() {
            self.error("an edit is still pending; wait before changing another field");
            return false;
        }
        self.patch_cached(owner, path, &value);
        match self.call(method, params) {
            Ok(r) => {
                let changed = r.get("changed").and_then(J::as_bool).unwrap_or(false);
                if changed {
                    if self.sim.mode == Mode::Edit {
                        self.history.dirty = true;
                    }
                    self.mark_edited();
                }
                changed
            }
            Err(e) => {
                self.error(format!("{what}: {e}"));
                // The cache was changed ahead of the host: take it back.
                self.dirty.inspect = true;
                self.dirty.singletons = true;
                false
            }
        }
    }

    /// ERP writes a tagged value whole: switching `shape.kind` to `circle`
    /// becomes writing `shape` as the circle variant with its default fields
    /// (what the widget's combo box means).
    fn variant_switch(&self, owner: &Owner, path: &str, value: Value) -> (String, Value) {
        let (Owner::Component(name) | Owner::Singleton(name)) = owner;
        let Value::Enum(variant) = &value else { return (path.to_string(), value) };
        let Some(parent) = path.strip_suffix(".kind").or_else(|| (path == "kind").then_some("")) else { return (path.to_string(), value) };
        let Some(ty) = self.types.get(name) else { return (path.to_string(), value) };
        let Ok(desc) = orr_remote::json::desc_at_path(ty.desc(), parent) else { return (path.to_string(), value) };
        if let orr_reflect::Kind::Tagged(t) = &desc.kind {
            if let Some(v) = t.variants.iter().find(|v| &v.name == variant) {
                return (parent.to_string(), Value::Variant(v.name.clone(), v.default.clone()));
            }
        }
        (path.to_string(), value)
    }

    /// Changes the cached value the widgets show, ahead of the host's answer.
    fn patch_cached(&mut self, owner: &Owner, path: &str, value: &Value) {
        let (list, name): (&mut Vec<(String, Value)>, &str) = match owner {
            Owner::Component(c) => match self.inspect.as_mut() {
                Some(i) if Some(&i.target) == self.selection.as_ref() => (&mut i.components, c.as_str()),
                _ => return,
            },
            Owner::Singleton(s) => (&mut self.singletons, s.as_str()),
        };
        let Some((_, cur)) = list.iter_mut().find(|(n, _)| n == name) else { return };
        if path.is_empty() {
            *cur = value.clone();
            return;
        }
        let Some(ty) = self.types.get(name) else { return };
        let desc = ty.desc();
        let mut bytes = vec![0u8; desc.size()];
        if desc.write_unranged(&mut bytes, cur).is_err() || ty.set(&mut bytes, path, value.clone()).is_err() {
            return;
        }
        *cur = desc.read(&bytes);
    }

    /// Reads one field of the selected entity (or a singleton) from the host.
    pub fn read_field(&mut self, owner: &Owner, path: &str) -> Result<Value, String> {
        match owner {
            Owner::Component(c) => {
                let t = self.selection.clone().ok_or("nothing selected")?;
                self.field_of(&t, c, path)
            }
            Owner::Singleton(s) => {
                let r = self.call("world.singleton.get", json!({"name": s, "path": path}))?;
                self.decode_field(s, path, &r["value"])
            }
        }
    }

    /// Reads one field of any entity from the host (`path` like `pos.x`, empty = the whole component).
    pub fn field_of(&mut self, t: &Target, component: &str, path: &str) -> Result<Value, String> {
        let r = self.call("world.get", json!({"entity": t.param(), "component": component, "path": path}))?;
        self.decode_field(component, path, &r["value"])
    }

    fn decode_field(&self, type_name: &str, path: &str, j: &J) -> Result<Value, String> {
        let ty = self.types.get(type_name).ok_or_else(|| format!("unknown type '{type_name}'"))?;
        let desc = orr_remote::json::desc_at_path(ty.desc(), path)?;
        json_to_value(&desc, j, false)
    }

    /// Runs any ERP method on the host and waits for the answer: the escape
    /// hatch for tools and tests (the editor's own actions have methods of
    /// their own). `Err` carries the host's message.
    pub fn host_call(&mut self, method: &str, params: J) -> Result<J, String> {
        self.call(method, params)
    }

    /// Parses text as a value of the field `path` of type `type_name` (a
    /// component or singleton), like a person typing it.
    pub fn parse_field_text(&self, type_name: &str, path: &str, text: &str) -> Result<Value, String> {
        let ty = self.types.get(type_name).ok_or_else(|| format!("unknown type '{type_name}'"))?;
        let desc = orr_remote::json::desc_at_path(ty.desc(), path)?;
        crate::script::parse_like(&crate::inspector::zero_value(&desc), text)
    }

    /// Adds a component (default value) to the selected entity.
    pub fn add_component(&mut self, component: &str) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        match self.call("world.insert", json!({"entity": t.param(), "component": component})) {
            Ok(_) => {
                self.mark_edited();
                true
            }
            Err(e) => {
                self.error(format!("add {component}: {e}"));
                false
            }
        }
    }

    /// Removes a component from the selected entity.
    pub fn remove_component(&mut self, component: &str) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        match self.call("world.remove", json!({"entity": t.param(), "component": component})) {
            Ok(_) => {
                self.mark_edited();
                true
            }
            Err(e) => {
                self.error(format!("remove {component}: {e}"));
                false
            }
        }
    }

    /// The terrain geometry reconstructed from the authoritative host snapshot.
    #[cfg(feature = "terrain-physics")]
    pub fn admitted_terrain(&self) -> &crate::terrain_physics::TerrainPhysicsView { &self.terrain_view }

    /// The current authoritative 3D snapshot mapped to presentation data.
    pub fn yard_frame(&self) -> &crate::viewport3d::YardFrame { &self.yard_frame }

    /// The asynchronously queried GUID map must describe this presentation
    /// generation. Edit rebakes recycle handles, so checksum equality is also
    /// required. Play preserves the seeded GUID map and versions runtime handles;
    /// queries from a future tick are withheld after a backwards seek.
    pub fn yard_rows_coherent(&self)->bool {
        let Some(snapshot)=&self.snapshot else{return false};
        !self.dirty.rows&&!self.inflight.rows&&self.yard_rows_generation==self.change_gen&&self.down.is_none()
            && if snapshot.timeline().is_some(){self.yard_rows_tick<=snapshot.tick()}else{self.yard_rows_checksum==Some(snapshot.predicted().checksum())}
    }

    #[cfg(feature = "navigation")]
    pub fn admitted_navigation(&self) -> &crate::navigation_view::NavigationView { &self.navigation_view }
    #[cfg(feature = "navigation")]
    pub(crate) fn navigation_mark_edited(&mut self) { self.mark_edited(); }
    #[cfg(feature = "navigation")]
    pub fn set_navigation_blocker(&mut self, blocker: Option<String>) { self.navigation_blocker = blocker; }

    #[cfg(feature="room-project")]
    pub fn install_room_camera(&mut self, document: orr_sample::room_camera::Document) -> Result<(),String> {
        document.validate()?;
        for size in [(1,16384),(16384,1),(800,600)] { document.camera(&document.orbit(),size)?; }
        if !self.game().is_room() || !self.spec().is_local() { return Err("camera requires a local Room project".into()); }
        self.camera3d = document.orbit();
        self.room_camera = Some(document);
        self.room_camera_scene = self.path();
        Ok(())
    }
    pub fn has_room_camera(&self) -> bool {
        #[cfg(feature="room-project")]
        { self.game().is_room() && self.spec().is_local() && self.room_camera.is_some()
            && self.room_camera_scene == self.path() }
        #[cfg(not(feature="room-project"))]
        { false }
    }
    pub fn presentation_camera3d(&self, size:(u32,u32)) -> Result<orr_render::Camera3D,String> {
        #[cfg(feature="room-project")]
        if self.has_room_camera() { return self.room_camera.as_ref().unwrap().camera(&self.camera3d,size); }
        let _ = size;
        Ok(self.camera3d.camera())
    }
    pub fn orbit3d(&mut self, delta:[f32;2]) {
        #[cfg(feature="room-project")]
        if self.has_room_camera() { self.room_camera.as_ref().unwrap().orbit_delta(&mut self.camera3d,delta); return; }
        self.camera3d.orbit(delta);
    }
    pub fn pan3d(&mut self, delta:[f32;2],size:(u32,u32)) {
        #[cfg(feature="room-project")]
        if self.has_room_camera() { self.room_camera.as_ref().unwrap().pan(&mut self.camera3d,delta,size); return; }
        self.camera3d.pan(delta,size);
    }
    pub fn zoom3d(&mut self, delta:f32) {
        #[cfg(feature="room-project")]
        if self.has_room_camera() { self.room_camera.as_ref().unwrap().zoom(&mut self.camera3d,delta); return; }
        self.camera3d.zoom(delta);
    }
    #[cfg(feature="room-ui")]
    pub(crate) fn room_ui_source_token(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> { self.room_ui_source.clone() }

    fn clear_room_camera(&mut self) {
        // Reuse the source-retirement hook for camera-free authored Room UI too.
        #[cfg(feature="room-ui")]
        { let _ = self.room_ui_source.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |value| Some(value.saturating_add(1))); }
        #[cfg(feature="room-project")]
        if self.room_camera.take().is_some() {
            self.room_camera_scene = None;
            self.camera3d = orr_render::OrbitCamera::new([0.0,2.0,0.0],0.55,0.5,16.0);
        }
    }
    pub fn reset_room_camera(&mut self) {
        #[cfg(feature="room-project")]
        if self.has_room_camera() { self.camera3d=self.room_camera.as_ref().unwrap().orbit(); }
    }

    /// Nearest collider-proxy selection in the main Yard3D viewport.
    pub fn pick3d(&self, pixel: [f32;2], size:(u32,u32)) -> Option<Target> {
        if !self.yard_rows_coherent(){return None;}
        self.yard_frame.pick(&self.presentation_camera3d(size).ok()?,pixel,size,&self.rows)
    }

    /// Creates a persisted Yard3D box, or a sphere in the admitted terrain game.
    pub fn spawn_yard_body(&mut self, pos: orr_fp::FPVec3) -> bool {
        if self.game().is_navigation() { self.error("Point navigation scenes do not support physics bodies or colliders"); return false; }
        if !self.game().is_3d() || self.mode()!=Mode::Edit || !self.can_mutate() || self.previewing().is_some() {
            self.error("Yard bodies require editable Yard3D scene mode"); return false;
        }
        let params=json!({"name":self.fresh_name(),"components":{
            "orr_physics3d::Body":{"pos":value_to_json(&Value::Vec3(pos)),"kind":"dynamic","inv_mass":1,"inv_inertia":if self.game().is_terrain() {[10,10,10]}else{[6,6,6]}},
            "orr_physics3d::Collider":{"shape":if self.game().is_terrain() { json!({"kind":"sphere","radius":0.5}) } else { json!({"kind":"box","half_extents":[0.5,0.5,0.5]}) }}
        }});
        match self.call("world.spawn",params) {
            Ok(r)=>{ let t=r.get("guid").and_then(J::as_str).and_then(Target::parse); self.mark_edited();self.select(t);self.inspect=None;self.dirty.inspect=true;true }
            Err(e)=>{self.error(format!("spawn Yard body: {e}"));false}
        }
    }

    /// Atomic exact XYZ and whole unit-quaternion edit, one document undo entry.
    /// No float presentation transform is ever written back to the host.
    pub fn set_yard_transform(&mut self, pos:orr_fp::FPVec3, rotation:[FP;4])->bool {
        if !self.game().is_3d() || self.mode()!=Mode::Edit || !self.can_mutate() || self.selected_guids().len()!=1 || self.previewing().is_some() {
            self.error("Yard transform requires one editable persistent entity");return false;
        }
        let Some(target)=self.selection.clone() else {return false};
        if let Err(e)=self.call("tx.begin",json!({"label":"Yard XYZ and quaternion"})){self.error(e);return false;}
        let rot=Value::Variant("unit".into(),["x","y","z","w"].into_iter().zip(rotation).map(|(key,value)|(key.into(),Value::Fixed(value))).collect());
        let result=(||{
            self.call("world.patch",json!({"entity":target.param(),"component":"orr_physics3d::Body","path":"pos","value":value_to_json(&Value::Vec3(pos))}))?;
            self.call("world.patch",json!({"entity":target.param(),"component":"orr_physics3d::Body","path":"rot","value":value_to_json(&rot)}))?;
            self.call("tx.commit",J::Null)?;Ok::<_,String>(())
        })();
        match result {Ok(())=>{self.mark_edited();true},Err(e)=>{let _=self.call("tx.rollback",J::Null);self.error(e);self.mark_changed();false}}
    }

    /// Spawns a dynamic circle body at `at` (world units) and selects it.
    pub fn spawn_body(&mut self, at: [f32; 2]) -> bool {
        if self.game() != EditorGame::PhysGame {
            self.error("Body creation is only available for PhysGame");
            return false;
        }
        let pos = FPVec2::new(fp_of_f64(f64::from(at[0])).unwrap_or(FP::ZERO), fp_of_f64(f64::from(at[1])).unwrap_or(FP::ZERO));
        let body = Value::Struct(vec![
            ("pos".into(), Value::Vec2(pos)),
            ("inv_mass".into(), Value::Fixed(FP::ONE)),
            // A unit-mass disc of radius 0.5: I = m r^2 / 2 = 0.125, so 1 / I = 8.
            ("inv_inertia".into(), Value::Fixed(FP::from_int(8))),
            ("kind".into(), Value::Enum("dynamic".into())),
        ]);
        let mut params = json!({"components": {"orr_physics::Body": value_to_json(&body), "orr_physics::Collider": {}}});
        if self.sim.mode == Mode::Edit {
            params["name"] = json!(self.fresh_name());
        }
        match self.call("world.spawn", params) {
            Ok(r) => {
                let t = r.get("guid").and_then(J::as_str).or_else(|| r.get("handle").and_then(J::as_str)).and_then(Target::parse);
                self.mark_edited();
                // The new entity is not in the hierarchy yet: keep the selection through the next refresh.
                self.select(t);
                self.inspect = None;
                self.dirty.inspect = true;
                true
            }
            Err(e) => {
                self.error(format!("spawn: {e}"));
                false
            }
        }
    }

    /// Adds a locally authored Arena player in an unused configured slot.
    /// The slot layout is read from the host after acquiring a transaction so
    /// concurrent ERP edits cannot race the duplicate check.
    pub fn spawn_arena_player(&mut self, slot: u8, at: [f32; 2]) -> bool {
        if self.game() != EditorGame::Arena {
            self.error("Arena player creation is only available for Arena");
            return false;
        }
        if self.sim.mode != Mode::Edit || !self.can_mutate() {
            self.error("Arena players can only be created in editable scene mode");
            return false;
        }
        if self.previewing().is_some() {
            self.error("clear the proposal preview before creating an Arena player");
            return false;
        }
        if self.history.in_tx || self.sim.in_tx || self.gesture.busy() {
            self.error("finish the current edit before creating an Arena player");
            return false;
        }
        if slot >= 8 {
            self.error(format!("Arena player slot {slot} is outside the supported range 0..=7"));
            return false;
        }
        let Some(x) = fp_of_f64(f64::from(at[0])) else {
            self.error("Arena player position must be a finite representable value");
            return false;
        };
        let Some(y) = fp_of_f64(f64::from(at[1])) else {
            self.error("Arena player position must be a finite representable value");
            return false;
        };
        let position = value_to_json(&Value::Vec2(FPVec2::new(x, y)));
        let label = format!("create Arena player slot {slot}");
        if let Err(e) = self.call("tx.begin", json!({"label": label})) {
            self.error(format!("cannot create Arena player: {e}"));
            return false;
        }

        let result = (|| -> Result<Target, String> {
            let state = self.call("sim.state", J::Null)?;
            if state.get("mode").and_then(J::as_str) != Some("edit")
                || state.get("in_tx").and_then(J::as_bool) != Some(true)
            {
                return Err("Arena scene changed mode before player creation".into());
            }
            let player_count = state.get("player_count").and_then(J::as_u64)
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| (1..=8).contains(n))
                .ok_or_else(|| "host returned an invalid Arena player count".to_string())?;
            if slot >= player_count {
                return Err(format!("Arena player slot {slot} is outside the configured player range"));
            }
            let query = self.call(
                "world.query",
                json!({"components":["PlayerTag"],"values":true,"limit":9}),
            )?;
            if query.get("truncated").and_then(J::as_bool) != Some(false) {
                return Err("Arena player slot layout is too large or incomplete to validate".into());
            }
            let entities = query.get("entities").and_then(J::as_array)
                .ok_or_else(|| "Arena player slot query returned no entity list".to_string())?;
            if query.get("total").and_then(J::as_u64) != Some(entities.len() as u64) {
                return Err("Arena player slot query returned an incomplete entity list".into());
            }
            let mut slots = [false; 8];
            for entity in entities {
                let found = entity.pointer("/values/PlayerTag/slot").and_then(J::as_u64)
                    .and_then(|n| u8::try_from(n).ok())
                    .filter(|n| *n < player_count && *n < 8)
                    .ok_or_else(|| "Arena scene has a PlayerTag outside the configured player range".to_string())?;
                if std::mem::replace(&mut slots[usize::from(found)], true) {
                    return Err(format!("Arena scene has duplicate PlayerTag slot {found}"));
                }
            }
            if slots[usize::from(slot)] {
                return Err(format!("Arena player slot {slot} is already occupied"));
            }
            let created = self.call("world.spawn", json!({
                "name": format!("player_{slot}"),
                "components": {
                    "Position": {"pos": position},
                    "PlayerTag": {"slot": slot},
                }
            }))?;
            created.get("guid").and_then(J::as_str)
                .or_else(|| created.get("handle").and_then(J::as_str))
                .and_then(Target::parse)
                .ok_or_else(|| "Arena player creation returned no entity identifier".to_string())
        })();

        let target = match result {
            Ok(target) => target,
            Err(e) => {
                let _ = self.call("tx.rollback", J::Null);
                self.error(format!("cannot create Arena player: {e}"));
                return false;
            }
        };
        if let Err(e) = self.call("tx.commit", J::Null) {
            let _ = self.call("tx.rollback", J::Null);
            self.error(format!("cannot finish Arena player creation: {e}"));
            return false;
        }
        self.mark_edited();
        self.select(Some(target));
        self.inspect = None;
        self.dirty.inspect = true;
        true
    }

    fn fresh_name(&self) -> String {
        (1..)
            .map(|n| format!("new_body_{n}"))
            .find(|c| !self.rows.iter().any(|r| r.name.as_deref() == Some(c.as_str())))
            .expect("an unused name exists")
    }

    /// Deletes the selected entity. Refusals (something still points at it) are reported.
    pub fn delete_selected(&mut self) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        match self.call("world.despawn", json!({"entity": t.param()})) {
            Ok(_) => {
                self.select(None);
                self.inspect = None;
                self.mark_edited();
                true
            }
            Err(e) => {
                self.error(format!("delete: {e}"));
                false
            }
        }
    }

    /// Takes back the last edit (edit mode only).
    pub fn undo(&mut self) -> bool {
        self.history_step("history.undo", "undo")
    }

    /// Repeats the last undone edit (edit mode only).
    pub fn redo(&mut self) -> bool {
        self.history_step("history.redo", "redo")
    }

    fn history_step(&mut self, method: &str, what: &str) -> bool {
        if self.sim.mode == Mode::Play {
            self.error(format!("{what} works in edit mode; stop play first"));
            return false;
        }
        match self.call(method, J::Null) {
            Ok(r) => {
                if let Some(h) = r.get("history") {
                    self.history = History::from_json(h);
                    self.sim.dirty = self.history.dirty;
                }
                self.mark_edited();
                true
            }
            Err(e) => {
                self.error(format!("{what}: {e}"));
                false
            }
        }
    }

    // ---- agent activity and proposals ----

    /// The window state of the Agent tab.
    pub fn agent(&self) -> &AgentState {
        &self.agent
    }

    /// The Agent tab state, mutably.
    pub fn agent_mut(&mut self) -> &mut AgentState {
        &mut self.agent
    }

    /// The activity feed: what agents did through ERP.
    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    /// The activity feed, mutably (filters, marking as seen).
    pub fn feed_mut(&mut self) -> &mut Feed {
        &mut self.feed
    }

    /// Milliseconds since the host's ERP server started (the clock of
    /// activity entries), as of the last refresh.
    pub fn erp_elapsed_ms(&self) -> Option<u64> {
        (self.server_now_ms > 0).then_some(self.server_now_ms)
    }

    /// The other clients connected to the host right now (not this editor).
    pub fn agents(&self) -> &[ClientInfo] {
        &self.clients
    }

    /// The ERP address agents can connect to, and how many are connected. For
    /// a local host that is the `--erp` listener (none without it); for an
    /// attached one, the host's address.
    pub fn erp_status(&self) -> Option<(String, usize)> {
        self.backend.url.clone().map(|u| (u, self.clients.len()))
    }

    /// The open proposals.
    pub fn proposals(&self) -> &[ProposalInfo] {
        &self.proposals
    }

    /// The diff and summary of an open proposal, once the host has sent them.
    pub fn proposal_detail(&self, id: &str) -> Option<&ProposalDetail> {
        self.details.get(id)
    }

    /// The proposal the viewport currently previews (edit mode only).
    pub fn previewing(&self) -> Option<&str> {
        if self.sim.mode == Mode::Play {
            return None;
        }
        self.preview.as_ref().map(|p| p.id.as_str()).filter(|id| self.proposals.iter().any(|p| p.id == *id))
    }

    /// Turns the preview of a proposal on (`Some`) or off (`None`). Refused in
    /// play mode. The staged frame reaches the viewport through a second frame
    /// stream of the host (`watch.subscribe` source `proposal:<id>`).
    pub fn set_preview(&mut self, id: Option<String>) -> bool {
        self.release_control();
        match id {
            Some(_) if self.sim.mode == Mode::Play => {
                self.error("preview needs edit mode; stop play first");
                false
            }
            Some(i) if !self.proposals.iter().any(|p| p.id == i) => {
                self.error(format!("unknown proposal {i}"));
                false
            }
            Some(i) => {
                if self.preview.as_ref().is_some_and(|p| p.id == i) {
                    return true;
                }
                match self.backend.frame_stream(&format!("proposal:{i}")) {
                    Ok(stream) => {
                        self.preview_generation += 1;
                        self.preview = Some(Preview { id: i.clone(), stream, seq: 0, generation: self.preview_generation, rows_dirty: true, bodies: Vec::new(), rows: Vec::new() });
                        self.agent.preview = Some(i);
                        true
                    }
                    Err(e) => {
                        self.preview = None;
                        self.agent.preview = None;
                        self.inflight.preview_rows = None;
                        if e.contains("remote identity check:") {
                            self.snapshot = None;
                            self.bodies.clear();
                            self.checksum = 0;
                            self.connection_lost(&e);
                        } else {
                            self.error(format!("preview: {e}"));
                        }
                        false
                    }
                }
            }
            None => {
                self.preview = None;
                self.agent.preview = None;
                self.inflight.preview_rows = None;
                true
            }
        }
    }

    /// The render list of what the viewport shows: the scene's frame, or the
    /// staged frame of the previewed proposal with its changes marked, plus
    /// the pulses (`(entity, fade)`) of entities an agent just edited.
    pub fn viewport_list(&self, vp: (u32, u32), pulses: &[(Target, f32)]) -> RenderList {
        let selected = self.selection.as_ref().and_then(|t| self.entity_of(t));
        let previewing = self.previewing();
        let staged = self.preview.as_ref().filter(|p| Some(p.id.as_str()) == previewing && p.seq > 0);
        let (mut list, bodies, rows): (RenderList, &[Drawable], &[EntityRow]) = match staged {
            Some(p) => {
                let scene = Scene { bodies: &p.bodies, rows: &p.rows };
                let base = Scene { bodies: &self.bodies, rows: &self.rows };
                let sel = self.selection.as_ref().and_then(|t| match t {
                    Target::Guid(g) => scene.entity_of(g),
                    Target::Entity(e) => Some(*e),
                });
                let list = match self.details.get(&p.id) {
                    Some(d) => viewport::build_preview_list(&scene, &base, &d.summary, sel, &self.camera, vp),
                    None => viewport::build_list(&p.bodies, sel, &self.camera, vp),
                };
                (list, &p.bodies, &p.rows)
            }
            None => (viewport::build_list(&self.bodies, selected, &self.camera, vp), &self.bodies, &self.rows),
        };
        if !pulses.is_empty() {
            let live: Vec<(Entity, f32)> = pulses
                .iter()
                .filter_map(|(t, f)| {
                    let e = match t {
                        Target::Entity(e) => Some(*e),
                        Target::Guid(g) => rows.iter().find(|r| r.guid.as_ref() == Some(g)).map(|r| r.entity),
                    }?;
                    Some((e, *f))
                })
                .collect();
            viewport::add_pulses(&mut list, bodies, &live);
        }
        // Re-resolve each GUID in the currently drawn source, including a
        // rebaked proposal. Only its selection outline is added; shapes and
        // grid from the small helper list are never duplicated in the frame.
        for guid in self.guid_selection.guids() {
            let Some(entity) = rows.iter().find(|row| row.guid.as_ref() == Some(guid)).map(|row| row.entity) else { continue };
            if self.guid_selection.primary() == Some(guid) { continue; }
            if let Some(body) = bodies.iter().find(|body| body.entity == entity) {
                let mut outline = viewport::build_list(std::slice::from_ref(body), Some(entity), &self.camera, vp);
                list.lines.append(&mut outline.lines);
            }
        }
        list
    }

    /// The body under a world point (of the frame on screen), as a target.
    pub fn pick(&self, world: [f32; 2]) -> Option<Target> {
        let e = viewport::pick(&self.bodies, world)?;
        Some(self.row_of(&Target::Entity(e)).map_or(Target::Entity(e), EntityRow::target))
    }

    /// World position of the body of a target, as the frame on screen has it.
    pub fn body_pos(&self, t: &Target) -> Option<[f32; 2]> {
        viewport::body_pos(&self.bodies, self.entity_of(t)?)
    }

    /// A client with the name of an agent, on the editor's own host (scripts
    /// stage proposals through it, the way an ERP agent would).
    pub fn agent_client(&mut self, name: &str) -> Result<&mut ErpClient, String> {
        if !self.agent_clients.contains_key(name) {
            let c = self.backend.agent_client(name)?;
            self.agent_clients.insert(name.to_string(), c);
        }
        Ok(self.agent_clients.get_mut(name).expect("inserted above"))
    }

    /// Remembers which agent made a proposal a script staged.
    pub(crate) fn note_script_proposal(&mut self, id: &str, agent: &str) {
        self.script_owner.insert(id.to_string(), agent.to_string());
    }

    /// The agent a script staged proposal `id` as.
    pub(crate) fn script_proposal_owner(&self, id: &str) -> Option<&str> {
        self.script_owner.get(id).map(String::as_str)
    }

    // ---- play ----

    /// Starts a play session from the scene, paused at tick 0. Does nothing
    /// if one already runs.
    pub fn start_play(&mut self) -> bool {
        #[cfg(feature = "navigation")]
        if self.game().is_navigation() && self.sim.mode != Mode::Play {
            let blocker = self.navigation_blocker.clone().or_else(|| {
                (!self.navigation_view.admitted).then(|| "Build and admit a terrain route before Play".to_string())
            }).or_else(|| self.navigation_view.error.clone());
            if let Some(error) = blocker { self.error(error); return false; }
        }
        #[cfg(feature = "terrain-physics")]
        if self.game().is_terrain() && (!self.terrain_view.admitted || self.terrain_view.error.is_some()) {
            self.error(self.terrain_view.error.clone().unwrap_or_else(|| "Wait for a valid admitted terrain snapshot before Play".into()));
            return false;
        }
        if self.sim.mode == Mode::Play {
            return true;
        }
        self.end_edit();
        match self.call("sim.start", json!({})) {
            Ok(r) => {
                self.apply_state(&r);
                self.mark_changed();
                self.info("play started");
                true
            }
            Err(e) => {
                self.error(format!("play: {e}"));
                false
            }
        }
    }

    /// Play button: starts the session if needed and runs by the clock.
    pub fn play(&mut self) {
        if self.start_play() {
            self.control(ControlOp::Play);
        }
    }

    /// Pause button.
    pub fn pause(&mut self) {
        self.control(ControlOp::Pause);
    }

    /// Step button: runs `n` ticks now (starting the session if needed).
    pub fn step(&mut self, n: u32) {
        if self.start_play() {
            self.control(ControlOp::Pause);
            self.control(ControlOp::Step(n));
        }
    }

    /// Runs a timeline control (play mode only).
    pub fn control(&mut self, op: ControlOp) {
        if !matches!(op, ControlOp::Play | ControlOp::SetSpeed(_)) { self.release_control(); }
        if self.sim.mode != Mode::Play {
            return;
        }
        let (method, params) = match op {
            ControlOp::Play => ("sim.play", J::Null),
            ControlOp::Pause => ("sim.pause", J::Null),
            ControlOp::Step(n) => ("sim.step", json!({"n": n})),
            ControlOp::SetSpeed(s) => ("sim.speed", json!({"permille": s.permille()})),
            ControlOp::Seek(t) => ("sim.seek", json!({"tick": t})),
            ControlOp::Branch => ("sim.branch", J::Null),
        };
        match self.call(method, params) {
            Ok(r) => {
                self.apply_state(&r);
                if matches!(op, ControlOp::Seek(_) | ControlOp::Branch | ControlOp::Step(_)) {
                    self.mark_changed();
                }
            }
            Err(e) => self.error(format!("{method}: {e}")),
        }
    }

    /// Goes to a recorded tick (pauses).
    pub fn seek(&mut self, tick: u64) {
        self.control(ControlOp::Seek(tick));
    }

    /// Sets the play speed (0.25 to 4).
    pub fn set_speed(&mut self, factor: f32) {
        let permille = (f64::from(factor) * 1000.0).round().max(0.0) as u32;
        self.control(ControlOp::SetSpeed(orr_bridge::Speed::from_permille(permille)));
    }

    /// Drops the recorded future after the head tick.
    pub fn branch(&mut self) {
        self.control(ControlOp::Branch);
    }

    /// Ends play. The document is exactly as it was before play; the
    /// recording stays available from [`last_stopped`](Self::last_stopped).
    pub fn stop(&mut self) -> Option<&Stopped> {
        self.release_control();
        if self.sim.mode != Mode::Play {
            return None;
        }
        match self.call("sim.stop", json!({"include_replay": true})) {
            Ok(r) => {
                let replay = r.get("replay").and_then(J::as_str).and_then(b64_decode).unwrap_or_default();
                let checksum = r.get("checksum").and_then(orr_remote::wire::parse_checksum).unwrap_or(0);
                self.stopped = Some(Stopped { tick: r.get("tick").and_then(J::as_u64).unwrap_or(0), checksum, replay });
                self.sim.mode = Mode::Edit;
                self.sim.viewer = false;
                self.sim.playing = false;
                self.mark_edited();
                self.info("play stopped");
                self.stopped.as_ref()
            }
            Err(e) => {
                self.error(format!("stop: {e}"));
                None
            }
        }
    }

    // ---- when the host is gone ----

    /// Starts the host again and reconnects: a local host reopens the scene file
    /// (edits that were not saved are lost), a remote one is connected to afresh.
    /// On failure the editor stays down and says why.
    pub fn restart(&mut self) -> bool {
        let was_uncertain = self.batch_uncertain;
        self.release_control();
        let spec = self.backend.spec.clone();
        let local = spec.is_local();
        let backend = match Backend::connect(&spec) {
            Ok(b) => b,
            Err(e) => {
                self.error(format!("restart failed: {e}"));
                return false;
            }
        };
        let mut fresh = match Self::on_backend(backend) {
            Ok(f) => f,
            Err(e) => {
                self.error(format!("restart failed: {e}"));
                return false;
            }
        };
        fresh.log = std::mem::take(&mut self.log);
        if fresh.game() == self.game() {
            fresh.camera = self.camera;
            fresh.initial_camera_fit = false;
        }
        #[cfg(feature="room-project")]
        if self.has_room_camera() && fresh.game().is_room() && fresh.path()==self.path() {
            let _ = fresh.install_room_camera(self.room_camera.as_ref().unwrap().clone());
        }
        fresh.feed.filter = self.feed.filter;
        fresh.agent.expand_methods = std::mem::take(&mut self.agent.expand_methods);
        *self = fresh;
        if local {
            self.info("simulation host restarted: the scene was opened again; edits that were not saved are lost");
        } else {
            self.info("reconnected");
        }
        self.report_view_delivery();
        if was_uncertain {
            self.info(if local { "new local host reopened the saved file and read its document/history; the old host's uncertain batch was not retried" }
                else { "fresh fenced connection, authoritative document and history read; inspect the confirmed result before a new edit (no batch was retried)" });
        }
        true
    }

    /// Makes the host thread panic (needs a host started with debug hooks):
    /// the test of crash isolation.
    pub fn debug_crash_host(&mut self) {
        let _ = self.call("debug.panic", J::Null);
    }
}

/// A bounded diagnostic history, never a replay of lifecycle transitions or
/// request acknowledgements. Current state comes from the pinned snapshot and
/// ERP, and actual request results continue through their normal response path.
fn recovery_messages(source: &str, reset: &ViewResync) -> Vec<Message> {
    let mut messages = vec![Message {
        text: format!("{source} recovered at tick {}: {} presentation notifications discarded (generation {})", reset.head_tick, reset.discarded_events, reset.generation),
        error: false,
    }];
    for summary in &reset.lifecycle {
        messages.push(Message {
            text: format!("{source} recovery diagnostics: {} coalesced occurrence(s), latest {:?}", summary.count, summary.last),
            error: matches!(summary.last, Lifecycle::DebugRejected(_) | Lifecycle::SeekRejected { .. } | Lifecycle::Desync { .. } | Lifecycle::Disconnected),
        });
    }
    if let Some(tick) = reset.last_desync {
        if !reset.lifecycle.iter().any(|s| matches!(s.last, Lifecycle::Desync { tick: t } if t == tick)) {
            messages.push(Message { text: format!("{source} recovery diagnostics: last desync at tick {tick}"), error: true });
        }
    }
    messages
}

fn parse_capture_state(value: &J) -> Option<ViewState> {
    let mode = match value.get("mode")?.as_str()? {
        "edit" => ViewMode::Edit,
        "play" => ViewMode::Play,
        _ => return None,
    };
    let playing = value.get("playing")?.as_bool()?;
    Some(ViewState {
        mode,
        paused: !playing,
        tick: value.get("head_tick")?.as_u64()?,
        epoch: value.get("epoch")?.as_u64()?,
        checksum: orr_remote::wire::parse_checksum(value.get("checksum")?)?,
    })
}

fn parse_capture_view(value: &J) -> Option<(u64, u64)> {
    Some((
        value.get("tick")?.as_u64()?,
        orr_remote::wire::parse_checksum(value.get("checksum")?)?,
    ))
}

fn rows_of(v: &J) -> Vec<EntityRow> {
    v.get("entities").and_then(J::as_array).map(|a| a.iter().filter_map(EntityRow::from_json).collect()).unwrap_or_default()
}

fn singletons_of(types: &TypeRegistry, v: &J) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(obj) = v.get("singletons").and_then(J::as_object) {
        for (name, j) in obj {
            let Some(ty) = types.get(name) else { continue };
            if let Ok(val) = json_to_value(ty.desc(), j, false) {
                out.push((name.clone(), val));
            }
        }
    }
    out
}

/// `FP` as `f64`, for the view layer only (drag speeds, camera).
pub fn fp_to_f64(v: FP) -> f64 {
    v.raw() as f64 / 65536.0
}

/// `FP` from a view-layer number, through exact decimal parsing: the number
/// is printed with 5 decimals and parsed by `orr_reflect::decimal`, never
/// converted bit by bit. `None` for NaN, infinity or values that do not fit.
pub fn fp_of_f64(v: f64) -> Option<FP> {
    if !v.is_finite() {
        return None;
    }
    decimal::parse_fp(&format!("{v:.5}")).ok()
}

#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod diagnostics_tests;

mod gestures;
#[cfg(test)]
mod gesture_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod multi_selection_tests;

#[cfg(test)]
mod tick_state_refresh_tests {
    use super::*;

    #[test]
    fn same_epoch_paused_tick_refreshes_state_after_an_older_reply() {
        let mut editor = Editor::open(&default_scene_path()).unwrap();
        editor.sync();
        assert!(editor.start_play());
        editor.sync();
        let old = editor.call("sim.state", J::Null).unwrap();
        assert_eq!(old["mode"], "play");
        assert_eq!(old["playing"], false);
        let current = editor.call("sim.step", json!({"n":20})).unwrap();
        editor.sync();
        let wanted = parse_capture_state(&current).unwrap();
        assert_eq!(wanted.epoch, old["epoch"].as_u64().unwrap());
        assert!(editor.capture_snapshot_matches(wanted));
        assert_ne!(parse_capture_state(&old).unwrap().checksum, wanted.checksum);
        editor.apply_state(&old);
        assert!(!editor.dirty.state);
        // Reproduce ingest order without a scheduler race: a same-epoch
        // paused tick arrives, then an older in-flight State reply is drained.
        editor.on_notification("watch.tick", &json!({
            "mode":"play", "playing":false, "tick":wanted.tick,
            "last_tick":wanted.tick, "epoch":wanted.epoch,
            "checksum":current["checksum"]
        }));
        editor.on_answer(Pend::State(editor.state_generation), Ok(old));
        assert_ne!(editor.capture_view_state(), wanted);
        assert!(editor.dirty.state, "the old response must not strand capture readiness at an earlier checksum");
        editor.send_refreshes();
        assert!(editor.inflight.state);
        assert!(editor.pending.values().any(|pending| matches!(pending.kind, Pend::State(_))));
        // Ordinary UI pumping must recover; sync() would hide the regression
        // by unconditionally making another blocking sim.state call.
        let deadline = Instant::now() + Duration::from_secs(5);
        while editor.capture_view_state() != wanted || editor.screenshot_waiting_for().is_some() {
            assert!(Instant::now() < deadline, "state refresh did not settle");
            editor.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(editor.capture_snapshot_matches(wanted));
    }
}

#[cfg(test)]
mod screenshot_poll_tests {
    use super::*;
    use crate::app::EditorApp;
    use egui::{Color32, ColorImage, Event, Pos2, Rect, ViewportId};
    use egui_kittest::Harness;
    use orr_remote::{Auth, ScreenshotOptions, ScreenshotService, ServerConfig};

    // No renderer/readback: use the real UI pump and ERP owner with synthetic
    // screenshot events. The native framebuffer smoke remains the GPU check.
    fn paused_play_harness() -> (Harness<'static, EditorApp>, ErpClient) {
        let spec = HostSpec::Local {
            scene: default_scene_path(),
            listen: Some(ServerConfig::new(Auth::DevNoAuth)),
            debug_hooks: false,
        };
        let mut editor = Editor::start(&spec).unwrap();
        assert!(editor.start_play());
        editor.call("sim.step", json!({"n":20})).unwrap();
        editor.sync();
        assert_eq!(editor.capture_view_state().mode, ViewMode::Play);
        assert_eq!(editor.capture_view_state().tick, 20);
        assert!(editor.capture_view_state().paused);
        let url = editor.erp_status().unwrap().0;
        let mut harness = Harness::builder()
            .with_size([960.0, 720.0])
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(move |_cc| EditorApp::new(editor, None));
        harness.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect =
            Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(960.0, 720.0)));
        harness.run_steps(3);
        (harness, ErpClient::connect(&url, None).unwrap())
    }

    fn screenshot_command(harness: &Harness<'_, EditorApp>) -> Option<u64> {
        harness.output().viewport_output.get(&ViewportId::ROOT)?.commands.iter().find_map(|command| {
            let egui::ViewportCommand::Screenshot(data) = command else { return None };
            data.data.as_ref()?.downcast_ref::<u64>().copied()
        })
    }

    fn request_capture(harness: &mut Harness<'_, EditorApp>, agent: &mut ErpClient, timeout_ms: u64) -> (u64, u64) {
        let id = agent.post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":timeout_ms})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            harness.run_steps(1);
            if let Some(ticket) = screenshot_command(harness) {
                assert!(!harness.state().editor.inflight.clients);
                assert_eq!(harness.state().editor.screenshot_waiting_for(), None);
                return (id, ticket);
            }
            agent.poll().unwrap();
            assert!(agent.take_response(id).is_none(), "capture completed before issuing its screenshot ticket");
            assert!(Instant::now() < deadline, "editor never issued a screenshot ticket");
            std::thread::yield_now();
        }
    }

    fn deliver_screenshot(harness: &mut Harness<'_, EditorApp>, ticket: u64) {
        harness.input_mut().events.retain(|event| !matches!(event, Event::Screenshot { .. }));
        harness.input_mut().events.push(Event::Screenshot {
            viewport_id: ViewportId::ROOT,
            user_data: egui::UserData::new(ticket),
            image: Arc::new(ColorImage::filled([960, 720], Color32::from_rgb(24, 80, 144))),
        });
        harness.run_steps(1);
    }

    fn wait_response(harness: &mut Harness<'_, EditorApp>, agent: &mut ErpClient, id: u64) -> Result<J, orr_remote::RpcError> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            agent.poll().unwrap();
            if let Some(response) = agent.take_response(id) {
                return response;
            }
            harness.run_steps(1);
            assert!(screenshot_command(harness).is_none(), "capture must finish using its original ticket");
            assert!(Instant::now() < deadline, "screenshot response did not arrive");
            std::thread::yield_now();
        }
    }

    #[test]
    fn paused_capture_survives_due_client_poll_and_resumes_polling() {
        let (mut harness, mut agent) = paused_play_harness();
        let state = agent.call("sim.state", J::Null).unwrap();
        let (id, ticket) = request_capture(&mut harness, &mut agent, 5000);
        let frame_seq = harness.state().editor.capture_snapshot_seq();
        // Force the periodic deadline between the ticket and its event. The
        // event frame pumps first, so an eager poll leaves Pend::Clients queued
        // exactly when capture_pass_still_current checks model readiness.
        harness.state_mut().editor.last_clients = None;
        deliver_screenshot(&mut harness, ticket);
        let poll_deferred_while_encoding = harness.state().editor.last_clients.is_none();
        let image = wait_response(&mut harness, &mut agent, id).expect("unchanged paused capture must succeed without retry");
        assert!(poll_deferred_while_encoding, "defer the due poll while the image encodes");
        assert_eq!(image["status"], "captured");
        assert_eq!(image["mode"], "play");
        assert_eq!(image["paused"], true);
        assert_eq!(image["tick"], "20");
        assert_eq!(image["epoch"], state["epoch"].as_u64().unwrap().to_string());
        assert_eq!(image["checksum"], state["checksum"]);
        assert_eq!(image["frame_seq"], frame_seq.unwrap().to_string());
        assert_eq!(parse_capture_state(&agent.call("sim.state", J::Null).unwrap()), parse_capture_state(&state));
        harness.run_steps(1);
        assert!(harness.state().editor.screenshot_validations.is_empty());
        assert!(harness.state().editor.last_clients.is_some(), "the overdue poll must resume after completion");
    }

    #[test]
    fn paused_capture_still_rejects_sim_step_after_ticket() {
        let (mut harness, mut agent) = paused_play_harness();
        let before = agent.call("sim.state", J::Null).unwrap();
        let (id, ticket) = request_capture(&mut harness, &mut agent, 5000);
        harness.state_mut().editor.last_clients = None;
        let changed = agent.call("sim.step", json!({"n":1})).unwrap();
        assert_eq!(changed["head_tick"], 21);
        assert_ne!(changed["checksum"], before["checksum"]);
        deliver_screenshot(&mut harness, ticket);
        let error = wait_response(&mut harness, &mut agent, id).unwrap_err();
        assert_eq!(error.kind(), Some("view_stale"), "{error}");
    }

    #[test]
    fn client_poll_resumes_when_screenshot_validation_is_cancelled() {
        let mut editor = Editor::open(&default_scene_path()).unwrap();
        editor.sync();
        let (service, owner) = ScreenshotService::pair();
        editor.backend.screenshot_owner = Some(owner);
        let request = service.submit(
            ScreenshotOptions { max_width: 960, max_height: 720 },
            editor.capture_view_state(),
            Instant::now() + Duration::from_secs(5),
        ).unwrap();
        editor.begin_screenshot_validation(&request).unwrap();
        editor.last_clients = None;
        editor.send_refreshes();
        assert!(!editor.inflight.clients, "defer periodic polling during validation");
        service.cancel(request.serial);
        // Cancellation must unblock the timer even before the UI removes the
        // abandoned validation entry or consumes a late screenshot event.
        assert!(editor.screenshot_validations.contains_key(&request.serial));
        editor.send_refreshes();
        assert!(editor.inflight.clients);
        assert!(editor.pending.values().any(|pending| matches!(pending.kind, Pend::Clients)));
        editor.cancel_screenshot_validation(request.serial);
    }

    #[test]
    fn client_poll_resumes_when_screenshot_ticket_expires() {
        let (mut harness, mut agent) = paused_play_harness();
        let (id, ticket) = request_capture(&mut harness, &mut agent, 1000);
        harness.state_mut().editor.last_clients = None;
        harness.run_steps(1);
        assert!(harness.state().editor.last_clients.is_none());
        let error = wait_response(&mut harness, &mut agent, id).unwrap_err();
        assert_eq!(error.kind(), Some("view_timeout"), "{error}");
        harness.run_steps(1);
        assert!(harness.state().editor.screenshot_validations.is_empty());
        assert!(harness.state().editor.last_clients.is_some(), "an expired ticket must not suspend client polling");
        // Expiry resumes polling but must retain the physical GPU permit until
        // its exact late event arrives, as in the integration timeout coverage.
        let busy = agent.call_err("view.screenshot", json!({"target":"app_framebuffer"}));
        assert_eq!(busy.kind(), Some("view_busy"));
        deliver_screenshot(&mut harness, ticket);
    }
}
