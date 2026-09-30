//! `Editor`: the state machine behind the window. No egui in here.
//!
//! It wraps an `EditorDoc` (edit mode) and, while playing, a
//! `PlayController` (play mode), plus what a person has selected. Every edit
//! a person makes goes through these methods with `Origin::User`:
//!
//! - edit mode: `EditorDoc::apply` (undoable, one history entry per gesture);
//! - play mode: `PlayController::set_field` and friends (recorded
//!   `DebugCommand`s, so the play stays deterministic and can be rewound).
//!
//! The play session runs on the UI thread in this MVP: [`Editor::advance`]
//! runs the ticks the frame time and speed ask for. A 1000-body physics tick
//! is under a millisecond, so this is fine at 60 Hz. A threaded host would
//! be the next step.
//!
//! An ERP server can run inside the editor ([`Editor::start_erp`]). Its
//! requests run in [`Editor::poll_erp`] on the UI thread, against the same
//! `EditorDoc` and play session, so an AI agent's edits land in the same
//! undo history (origin `agent:<name>`) and show up in the window at once.

use std::path::{Path, PathBuf};

use orr_edit::{EditError, EditorDoc, Op, Origin, PlayController, StoppedPlay, Target, View};
use orr_fp::{FPVec2, FP};
use orr_reflect::{decimal, Guid, TypeRegistry, Value};
use orr_remote::{ErpServer, ErpTarget, ServerConfig};
use orr_render::Camera;
use orr_sample::physics_game::{register_reflect, PhysGame};
use orr_session::{ControlOp, Speed, Timeline};
use orr_sim::Simulation;

/// Seed of the preview frame and of play sessions started from the editor.
pub const SEED: u64 = 7;
/// Sim rate of play sessions (the sample's rate).
pub const TICK_RATE: u32 = orr_sample::physics_game::TICK_RATE;
/// Players of a play session (`PhysGame` has one paddle per player).
pub const PLAYERS: u8 = 2;
/// Most ticks one [`Editor::advance`] call runs. More would mean the machine
/// cannot keep up; the extra time is dropped instead of piling up.
pub const MAX_TICKS_PER_ADVANCE: u32 = 8;

const LOG_LIMIT: usize = 200;

/// The type registry of `PhysGame` (physics types, `PaddleTag`, `Scene`).
pub fn make_types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    register_reflect(&mut t);
    t
}

/// Loads scene text into a document for `PhysGame`.
pub fn doc_from_text(text: &str) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml(text, make_types(), Simulation::<PhysGame>::build_registry(), SEED)
}

/// An empty document for `PhysGame`.
pub fn empty_doc() -> Result<EditorDoc, EditError> {
    EditorDoc::for_game::<PhysGame>(make_types(), SEED)
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

/// Edit mode or play mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The scene document is being edited.
    Edit,
    /// A play session runs (or is paused / rewound).
    Play,
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

/// See the module docs.
pub struct Editor {
    doc: EditorDoc,
    path: Option<PathBuf>,
    /// The play session. Its type is what `orr_remote::ErpTarget` borrows,
    /// so ERP clients drive the same session.
    play: Option<PlayController<PhysGame>>,
    /// Seconds of (speed scaled) real time not yet turned into ticks.
    play_acc: f64,
    /// The embedded ERP server, if started (see [`Editor::start_erp`]).
    erp: Option<ErpServer>,
    stopped: Option<StoppedPlay>,
    selection: Option<Target>,
    /// The viewport camera (world units, y up).
    pub camera: Camera,
    status: Option<Message>,
    log: Vec<Message>,
}

impl Editor {
    /// An editor on `doc`, saved as `path` (if any).
    pub fn new(doc: EditorDoc, path: Option<PathBuf>) -> Self {
        let mut e = Self { doc, path, play: None, play_acc: 0.0, erp: None, stopped: None, selection: None, camera: Camera::new([0.0, 0.0], 20.0), status: None, log: Vec::new() };
        e.fit_camera();
        e
    }

    /// Reads a scene file.
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let doc = doc_from_text(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::new(doc, Some(path.to_path_buf())))
    }

    // ---- reads ----

    /// The scene document (edit mode state; play never changes it).
    pub fn doc(&self) -> &EditorDoc {
        &self.doc
    }

    /// The mode.
    pub fn mode(&self) -> Mode {
        if self.play.is_some() {
            Mode::Play
        } else {
            Mode::Edit
        }
    }

    /// True while a play session exists.
    pub fn is_playing_mode(&self) -> bool {
        self.play.is_some()
    }

    /// Queries on what the viewport shows: the live play frame in play mode,
    /// else the preview frame.
    pub fn view(&self) -> View<'_> {
        match &self.play {
            Some(p) => p.view(),
            None => self.doc.view(),
        }
    }

    /// The play controller, in play mode.
    pub fn play_controller(&self) -> Option<&PlayController<PhysGame>> {
        self.play.as_ref()
    }

    /// The timeline of the play session.
    pub fn timeline(&self) -> Option<Timeline> {
        self.play.as_ref().map(|p| p.timeline())
    }

    /// The recording of the last play that was stopped.
    pub fn last_stopped(&self) -> Option<&StoppedPlay> {
        self.stopped.as_ref()
    }

    /// The scene file path.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The selected entity.
    pub fn selection(&self) -> Option<&Target> {
        self.selection.as_ref()
    }

    /// The selected entity's GUID, if it has one.
    pub fn selected_guid(&self) -> Option<Guid> {
        match self.selection.as_ref()? {
            Target::Guid(g) => Some(g.clone()),
            Target::Entity(e) => self.view().guid_of(*e).cloned(),
        }
    }

    /// True if the document has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    /// The window title, with a `*` when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(|| "untitled".to_string(), |n| n.to_string_lossy().into_owned());
        let mode = if self.play.is_some() { " [play]" } else { "" };
        format!("{}{name}{mode} - Orrery Editor", if self.is_dirty() { "*" } else { "" })
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
        self.view().checksum()
    }

    /// Display text of an entity in lists: its name, else its GUID, else its frame handle.
    pub fn entity_label(info: &orr_edit::EntityInfo) -> String {
        match (&info.name, &info.guid) {
            (Some(n), _) => n.clone(),
            (None, Some(g)) => g.to_string(),
            (None, None) => format!("entity {}v{} (play)", info.entity.index, info.entity.version),
        }
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

    fn report<T>(&mut self, what: &str, r: Result<T, EditError>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.error(format!("{what}: {e}"));
                None
            }
        }
    }

    // ---- selection and camera ----

    /// Selects an entity (or clears the selection).
    pub fn select(&mut self, target: Option<Target>) {
        self.selection = target;
    }

    /// Selects the entity with this display name or GUID text.
    pub fn select_named(&mut self, name: &str) -> bool {
        let found = self.view().entities().into_iter().find(|e| e.name.as_deref() == Some(name) || e.guid.as_ref().is_some_and(|g| g.to_string() == name));
        match found {
            Some(info) => {
                self.selection = Some(info.guid.map_or(Target::Entity(info.entity), Target::Guid));
                true
            }
            None => false,
        }
    }

    /// Drops a selection that no longer exists (after an undo, a despawn or a rewind).
    pub fn sanitize_selection(&mut self) {
        if let Some(t) = &self.selection {
            if self.view().resolve(t).is_err() {
                self.selection = None;
            }
        }
    }

    /// Frames the camera on the scene box (`Scene` singleton), like the sample does.
    pub fn fit_camera(&mut self) {
        let get = |name: &str| match self.view().singleton("Scene", name) {
            Ok(Value::Fixed(f)) => Some(fp_to_f64(f)),
            _ => None,
        };
        if let (Some(half_w), Some(height)) = (get("half_w"), get("height")) {
            self.camera = Camera::new([0.0, (height / 2.0) as f32], (half_w.max(height / 2.0) * 1.06) as f32);
        }
    }

    // ---- files ----

    /// Replaces the document with a scene file. Refused in play mode.
    pub fn open_path(&mut self, path: &Path) -> bool {
        if self.play.is_some() {
            self.error("stop play before opening a scene");
            return false;
        }
        match Self::open(path) {
            Ok(mut fresh) => {
                fresh.log = std::mem::take(&mut self.log);
                fresh.camera = self.camera;
                *self = fresh;
                self.fit_camera();
                self.info(format!("opened {}", path.display()));
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }

    /// Saves to the current path. `false` (and a message) if there is none or it fails.
    pub fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_as(&p),
            None => {
                self.error("no file name yet: use Save As");
                false
            }
        }
    }

    /// Saves the scene text to `path` and makes it the current path.
    pub fn save_as(&mut self, path: &Path) -> bool {
        if self.doc.in_tx() {
            self.error("finish the current edit before saving");
            return false;
        }
        let text = self.doc.to_yaml();
        match std::fs::write(path, &text) {
            Ok(()) => {
                self.doc.save_yaml();
                self.path = Some(path.to_path_buf());
                self.info(format!("saved {}", path.display()));
                true
            }
            Err(e) => {
                self.error(format!("cannot write {}: {e}", path.display()));
                false
            }
        }
    }

    // ---- editing ----

    /// Starts a gesture (a drag): everything until [`end_edit`](Self::end_edit)
    /// is one undo step. Edit mode only; in play mode edits are recorded
    /// debug commands and there is nothing to group.
    pub fn begin_edit(&mut self, label: &str) {
        if self.play.is_none() && !self.doc.in_tx() {
            let r = self.doc.begin_tx(label, Origin::User);
            self.report("edit", r);
        }
    }

    /// Ends the gesture started by [`begin_edit`](Self::begin_edit).
    pub fn end_edit(&mut self) {
        if self.doc.in_tx() {
            let r = self.doc.commit_tx();
            self.report("edit", r);
        }
    }

    /// Sets one field of the selected entity's component (or a singleton).
    /// `path` is a reflect path (`"pos.x"`, `""` = whole). Returns true if
    /// something changed. A refusal is reported as an error message.
    pub fn set_field(&mut self, owner: &Owner, path: &str, value: Value) -> bool {
        let what = match owner {
            Owner::Component(c) | Owner::Singleton(c) => format!("set {c}.{path}"),
        };
        let guid = self.selected_guid();
        let sel = self.selection.clone();
        let result = match (&mut self.play, owner) {
            (None, Owner::Component(c)) => match guid {
                Some(guid) => self.doc.apply(Op::SetField { guid, component: c.clone(), path: path.to_string(), value }, Origin::User).map(|a| a.changed),
                None => Err(EditError::Invalid("nothing selected".into())),
            },
            (None, Owner::Singleton(s)) => {
                self.doc.apply(Op::SetSingletonField { singleton: s.clone(), path: path.to_string(), value }, Origin::User).map(|a| a.changed)
            }
            (Some(p), Owner::Component(c)) => match sel {
                Some(t) => p.set_field(&t, c, path, value),
                None => Err(EditError::Invalid("nothing selected".into())),
            },
            (Some(p), Owner::Singleton(s)) => p.set_singleton_field(s, path, value),
        };
        self.report(&what, result).unwrap_or(false)
    }

    /// Adds a component (default value) to the selected entity.
    pub fn add_component(&mut self, component: &str) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        let what = format!("add {component}");
        let guid = self.selected_guid();
        let r = match &mut self.play {
            None => match guid {
                Some(guid) => self.doc.apply(Op::AddComponent { guid, component: component.to_string(), value: None }, Origin::User).map(|_| ()),
                None => Err(EditError::UnknownEntity("selection".into())),
            },
            Some(p) => p.add_component(&t, component, None),
        };
        self.report(&what, r).is_some()
    }

    /// Removes a component from the selected entity.
    pub fn remove_component(&mut self, component: &str) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        let what = format!("remove {component}");
        let guid = self.selected_guid();
        let r = match &mut self.play {
            None => match guid {
                Some(guid) => self.doc.apply(Op::RemoveComponent { guid, component: component.to_string() }, Origin::User).map(|_| ()),
                None => Err(EditError::UnknownEntity("selection".into())),
            },
            Some(p) => p.remove_component(&t, component),
        };
        self.report(&what, r).is_some()
    }

    /// Spawns a dynamic circle body at `at` (world units) and selects it.
    pub fn spawn_body(&mut self, at: [f32; 2]) -> bool {
        let pos = FPVec2::new(fp_of_f64(f64::from(at[0])).unwrap_or(FP::ZERO), fp_of_f64(f64::from(at[1])).unwrap_or(FP::ZERO));
        let components = vec![
            (
                "orr_physics::Body".to_string(),
                Value::Struct(vec![
                    ("pos".into(), Value::Vec2(pos)),
                    ("inv_mass".into(), Value::Fixed(FP::ONE)),
                    // A unit-mass disc of radius 0.5: I = m r^2 / 2 = 0.125, so 1 / I = 8.
                    ("inv_inertia".into(), Value::Fixed(FP::from_int(8))),
                    ("kind".into(), Value::Enum("dynamic".into())),
                ]),
            ),
            ("orr_physics::Collider".to_string(), Value::Struct(Vec::new())),
        ];
        match &mut self.play {
            None => {
                let name = self.fresh_name();
                let r = self.doc.apply(Op::SpawnEntity { guid: None, name: Some(name), components }, Origin::User);
                match self.report("spawn", r) {
                    Some(a) => {
                        self.selection = a.guid.map(Target::Guid);
                        true
                    }
                    None => false,
                }
            }
            Some(p) => {
                let r = p.spawn(&components);
                match self.report("spawn", r) {
                    Some(e) => {
                        self.selection = Some(Target::Entity(e));
                        true
                    }
                    None => false,
                }
            }
        }
    }

    fn fresh_name(&self) -> String {
        let names: Vec<Option<String>> = self.doc.view().entities().into_iter().map(|e| e.name).collect();
        (1..)
            .map(|n| format!("new_body_{n}"))
            .find(|c| !names.iter().any(|n| n.as_deref() == Some(c.as_str())))
            .expect("an unused name exists")
    }

    /// Deletes the selected entity. Refusals (something still points at it) are reported.
    pub fn delete_selected(&mut self) -> bool {
        let Some(t) = self.selection.clone() else {
            self.error("nothing selected");
            return false;
        };
        let guid = self.selected_guid();
        let r = match &mut self.play {
            None => match guid {
                Some(guid) => self.doc.apply(Op::DespawnEntity { guid }, Origin::User).map(|_| ()),
                None => Err(EditError::UnknownEntity("selection".into())),
            },
            Some(p) => p.despawn(&t),
        };
        let ok = self.report("delete", r).is_some();
        if ok {
            self.selection = None;
        }
        ok
    }

    /// Takes back the last edit (edit mode only).
    pub fn undo(&mut self) -> bool {
        if self.play.is_some() {
            self.error("undo works in edit mode; stop play first");
            return false;
        }
        let r = self.doc.undo();
        let ok = self.report("undo", r).is_some();
        self.sanitize_selection();
        ok
    }

    /// Repeats the last undone edit (edit mode only).
    pub fn redo(&mut self) -> bool {
        if self.play.is_some() {
            self.error("redo works in edit mode; stop play first");
            return false;
        }
        let r = self.doc.redo();
        let ok = self.report("redo", r).is_some();
        self.sanitize_selection();
        ok
    }

    // ---- ERP ----

    /// Starts the embedded ERP server. The play defaults (players, tick rate)
    /// and the scene path for `scene.save {write: true}` are the editor's.
    /// Returns the URL clients connect to.
    pub fn start_erp(&mut self, mut cfg: ServerConfig) -> Result<String, String> {
        if self.erp.is_some() {
            return Err("the ERP server is already running".into());
        }
        cfg.limits.player_count = PLAYERS;
        cfg.limits.tick_rate = TICK_RATE;
        cfg.limits.scene_path = self.path.clone();
        let server = ErpServer::start(cfg).map_err(|e| format!("ERP: {e}"))?;
        let url = server.url();
        self.erp = Some(server);
        self.info(format!("ERP listening on {url}"));
        Ok(url)
    }

    /// The ERP server's URL and connection count, if it runs.
    pub fn erp_status(&self) -> Option<(String, usize)> {
        self.erp.as_ref().map(|s| (s.url(), s.connection_count()))
    }

    /// Runs the ERP requests that arrived since the last call (call once per
    /// frame). Returns how many ran.
    pub fn poll_erp(&mut self) -> usize {
        let Some(server) = &mut self.erp else { return 0 };
        let had_play = self.play.is_some();
        let report = server.poll(&mut ErpTarget { doc: &mut self.doc, play: &mut self.play });
        if had_play != self.play.is_some() {
            self.play_acc = 0.0;
        }
        if report.requests > 0 {
            self.sanitize_selection();
        }
        report.requests
    }

    // ---- play ----

    /// Starts a play session from the scene, paused at tick 0. Refused if
    /// one already runs or an edit gesture is open.
    pub fn start_play(&mut self) -> bool {
        if self.play.is_some() {
            return true;
        }
        self.end_edit();
        let r = PlayController::<PhysGame>::start_play(&self.doc, self.doc.play_config(PLAYERS, TICK_RATE));
        match self.report("play", r) {
            Some(ctl) => {
                self.play = Some(ctl);
                self.play_acc = 0.0;
                self.info("play started");
                true
            }
            None => false,
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
        if let Some(p) = &mut self.play {
            p.control(op);
            self.play_acc = 0.0;
            for note in p.session_mut().take_notes() {
                if let orr_session::PlayNote::DebugRejected(e) = note {
                    let text = format!("debug edit refused: {e}");
                    self.status = Some(Message { text: text.clone(), error: true });
                    self.log.push(Message { text, error: true });
                }
            }
        }
    }

    /// Goes to a recorded tick (pauses).
    pub fn seek(&mut self, tick: u64) {
        self.control(ControlOp::Seek(tick));
        self.sanitize_selection();
    }

    /// Sets the play speed (0.25 to 4).
    pub fn set_speed(&mut self, factor: f32) {
        let permille = (f64::from(factor) * 1000.0).round().max(0.0) as u32;
        self.control(ControlOp::SetSpeed(Speed::from_permille(permille)));
    }

    /// Drops the recorded future after the head tick.
    pub fn branch(&mut self) {
        self.control(ControlOp::Branch);
    }

    /// Ends play. The document is exactly as it was before play; the
    /// recording stays available from [`last_stopped`](Self::last_stopped).
    pub fn stop(&mut self) -> Option<&StoppedPlay> {
        let p = self.play.take()?;
        self.stopped = Some(p.stop_play());
        self.sanitize_selection();
        self.info("play stopped");
        self.stopped.as_ref()
    }

    /// Runs the ticks that `dt` seconds of wall clock call for (a fixed step
    /// accumulator scaled by the play speed). Returns how many ran.
    pub fn advance(&mut self, dt: f64) -> u32 {
        let Some(p) = &mut self.play else { return 0 };
        let session = p.session();
        if !session.is_playing() {
            self.play_acc = 0.0;
            return 0;
        }
        let speed = f64::from(session.speed().permille()) / 1000.0;
        let step = 1.0 / f64::from(session.tick_rate().max(1));
        self.play_acc += dt.clamp(0.0, 1.0) * speed;
        let mut ran = 0;
        while self.play_acc >= step && ran < MAX_TICKS_PER_ADVANCE {
            p.session_mut().tick();
            self.play_acc -= step;
            ran += 1;
        }
        if ran == MAX_TICKS_PER_ADVANCE {
            self.play_acc = 0.0;
        }
        ran
    }
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
