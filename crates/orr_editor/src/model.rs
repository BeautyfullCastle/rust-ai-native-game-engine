//! What the editor knows about the simulation side, as plain data parsed from
//! ERP answers. The editor holds no document and no play session: these are
//! copies the host sent (see [`crate::editor`]).

use orr_ecs::Entity;
use orr_reflect::Guid;
use orr_remote::json::{handle_text, parse_handle};
use orr_remote::wire::parse_checksum;
use serde_json::Value as J;

/// Edit mode or play mode (of the host).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// The scene document is being edited.
    #[default]
    Edit,
    /// A play session runs (or is paused / rewound).
    Play,
}

/// Names an entity: by scene GUID, or by frame handle (for entities that
/// have no GUID, such as ones spawned during play).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A scene entity.
    Guid(Guid),
    /// A frame entity handle (in edit mode, of the preview frame).
    Entity(Entity),
}

impl Target {
    /// The text an ERP `entity` parameter takes: the GUID, or the handle (`12v0`).
    pub fn param(&self) -> String {
        match self {
            Target::Guid(g) => g.to_string(),
            Target::Entity(e) => handle_text(*e),
        }
    }

    /// The target a GUID or handle text names.
    pub fn parse(text: &str) -> Option<Target> {
        match Guid::parse(text) {
            Ok(g) => Some(Target::Guid(g)),
            Err(_) => parse_handle(text).map(Target::Entity),
        }
    }
}

impl From<Guid> for Target {
    fn from(g: Guid) -> Self {
        Target::Guid(g)
    }
}
impl From<Entity> for Target {
    fn from(e: Entity) -> Self {
        Target::Entity(e)
    }
}

/// One entity of the hierarchy (a row of `world.query`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRow {
    /// The handle in the frame the host reads (the preview frame in edit mode).
    pub entity: Entity,
    /// The scene GUID; `None` for an entity created during play.
    pub guid: Option<Guid>,
    /// Display name.
    pub name: Option<String>,
    /// Reflected component type names, sorted.
    pub components: Vec<String>,
}

impl EntityRow {
    /// Display text of an entity in lists: its name, else its GUID, else its frame handle.
    pub fn label(&self) -> String {
        match (&self.name, &self.guid) {
            (Some(n), _) => n.clone(),
            (None, Some(g)) => g.to_string(),
            (None, None) => format!("entity {}v{} (play)", self.entity.index, self.entity.version),
        }
    }

    /// The target that names this row: its GUID if it has one.
    pub fn target(&self) -> Target {
        self.guid.clone().map_or(Target::Entity(self.entity), Target::Guid)
    }

    pub(crate) fn from_json(j: &J) -> Option<EntityRow> {
        let entity = parse_handle(j.get("handle")?.as_str()?)?;
        let guid = j.get("guid").and_then(J::as_str).and_then(|g| Guid::parse(g).ok());
        let name = j.get("name").and_then(J::as_str).map(str::to_string);
        let components = j.get("components")?.as_array()?.iter().filter_map(|c| c.as_str().map(str::to_string)).collect();
        Some(EntityRow { entity, guid, name, components })
    }
}

/// The host's state (`sim.state`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimState {
    /// Edit or play.
    pub mode: Mode,
    /// A play session runs by the clock.
    pub playing: bool,
    /// The tick the session is at (0 in edit mode).
    pub head_tick: u64,
    /// First and last recorded tick.
    pub first_tick: u64,
    /// See `first_tick`.
    pub last_tick: u64,
    /// Checksum of the live frame (the play head, or the scene's preview frame).
    pub checksum: u64,
    /// Play speed in thousandths.
    pub speed_permille: u32,
    /// How often the recording was branched.
    pub branches: u32,
    /// Changes when the frame of an unchanged tick may differ.
    pub epoch: u64,
    /// Ticks per second of the session.
    pub tick_rate: u32,
    /// Players of the session.
    pub player_count: u8,
    /// The document has unsaved changes.
    pub dirty: bool,
    /// A transaction is open.
    pub in_tx: bool,
    /// The scene file of the host, if it has one.
    pub scene_path: Option<String>,
    /// Live entities of the frame.
    pub entities: u32,
}

impl SimState {
    pub(crate) fn from_json(j: &J) -> SimState {
        let u = |k: &str| j.get(k).and_then(J::as_u64).unwrap_or(0);
        let b = |k: &str| j.get(k).and_then(J::as_bool).unwrap_or(false);
        SimState {
            mode: if j.get("mode").and_then(J::as_str) == Some("play") { Mode::Play } else { Mode::Edit },
            playing: b("playing"),
            head_tick: u("head_tick"),
            first_tick: u("first_tick"),
            last_tick: u("last_tick"),
            checksum: j.get("checksum").and_then(parse_checksum).unwrap_or(0),
            speed_permille: u("speed_permille") as u32,
            branches: u("branches") as u32,
            epoch: u("epoch"),
            tick_rate: u("tick_rate") as u32,
            player_count: u("player_count") as u8,
            dirty: b("dirty"),
            in_tx: b("in_tx"),
            scene_path: j.get("scene_path").and_then(J::as_str).map(str::to_string),
            entities: u("entities") as u32,
        }
    }
}

/// One entry of the undo history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Entry id.
    pub id: u64,
    /// What it did.
    pub label: String,
    /// Who did it: `user` or `agent:<name>`.
    pub origin: String,
    /// Ops in the entry (a transaction has several).
    pub op_count: usize,
    /// Taken back (redo would repeat it).
    pub undone: bool,
}

/// The undo history (`history.list`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// Oldest first.
    pub entries: Vec<HistoryEntry>,
    /// Undo has something to take back.
    pub can_undo: bool,
    /// Redo has something to repeat.
    pub can_redo: bool,
    /// The document differs from the last save.
    pub dirty: bool,
    /// A transaction is open.
    pub in_tx: bool,
}

impl History {
    pub(crate) fn from_json(j: &J) -> History {
        let entries = j
            .get("entries")
            .and_then(J::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(HistoryEntry {
                            id: e.get("id")?.as_u64()?,
                            label: e.get("label")?.as_str()?.to_string(),
                            origin: e.get("origin")?.as_str()?.to_string(),
                            op_count: e.get("op_count")?.as_u64()? as usize,
                            undone: e.get("undone")?.as_bool()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let b = |k: &str| j.get(k).and_then(J::as_bool).unwrap_or(false);
        History { entries, can_undo: b("can_undo"), can_redo: b("can_redo"), dirty: b("dirty"), in_tx: b("in_tx") }
    }
}

/// An open proposal (`proposal.list`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalInfo {
    /// `p1`.
    pub id: String,
    /// Its label.
    pub label: String,
    /// Who made it: `agent:<name>`.
    pub origin: String,
    /// Staged ops.
    pub op_count: usize,
    /// The document changed since it was begun.
    pub stale: bool,
}

impl ProposalInfo {
    pub(crate) fn from_json(j: &J) -> Option<ProposalInfo> {
        Some(ProposalInfo {
            id: j.get("id")?.as_str()?.to_string(),
            label: j.get("label")?.as_str()?.to_string(),
            origin: j.get("origin")?.as_str()?.to_string(),
            op_count: j.get("op_count")?.as_u64()? as usize,
            stale: j.get("stale").and_then(J::as_bool).unwrap_or(false),
        })
    }
}

/// An entity named in a proposal summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRef {
    /// Its GUID.
    pub guid: Guid,
    /// Its name.
    pub name: Option<String>,
}

/// A rename in a proposal summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    /// The entity.
    pub guid: Guid,
    /// Name before.
    pub old: Option<String>,
    /// Name after.
    pub new: Option<String>,
}

/// One changed field in a proposal summary.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// The entity (`None` for a singleton).
    pub entity: Option<Guid>,
    /// Component or singleton type.
    pub component: String,
    /// Field path (empty = whole).
    pub path: String,
    /// Value before, as ERP JSON.
    pub old: J,
    /// Value after, as ERP JSON.
    pub new: J,
}

/// What a proposal changes, structurally (the `summary` of `proposal.get`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Entities that exist only in the staged scene.
    pub entities_added: Vec<EntityRef>,
    /// Entities that exist only in the base scene.
    pub entities_removed: Vec<EntityRef>,
    /// Entities whose name differs.
    pub entities_renamed: Vec<Renamed>,
    /// `(entity, component)` added.
    pub components_added: Vec<(Guid, String)>,
    /// `(entity, component)` removed.
    pub components_removed: Vec<(Guid, String)>,
    /// Changed fields.
    pub fields_changed: Vec<FieldChange>,
    /// Singletons only the staged scene has.
    pub singletons_added: Vec<String>,
    /// Singletons only the base scene has.
    pub singletons_removed: Vec<String>,
}

impl Summary {
    pub(crate) fn from_json(j: &J) -> Summary {
        let list = |k: &str| j.get(k).and_then(J::as_array).cloned().unwrap_or_default();
        let guid = |v: &J, k: &str| v.get(k).and_then(J::as_str).and_then(|g| Guid::parse(g).ok());
        let text = |v: &J, k: &str| v.get(k).and_then(J::as_str).map(str::to_string);
        let eref = |v: &J| Some(EntityRef { guid: guid(v, "guid")?, name: text(v, "name") });
        Summary {
            entities_added: list("entities_added").iter().filter_map(eref).collect(),
            entities_removed: list("entities_removed").iter().filter_map(eref).collect(),
            entities_renamed: list("entities_renamed").iter().filter_map(|v| Some(Renamed { guid: guid(v, "guid")?, old: text(v, "old"), new: text(v, "new") })).collect(),
            components_added: list("components_added").iter().filter_map(|v| Some((guid(v, "entity")?, text(v, "component")?))).collect(),
            components_removed: list("components_removed").iter().filter_map(|v| Some((guid(v, "entity")?, text(v, "component")?))).collect(),
            fields_changed: list("fields_changed")
                .iter()
                .filter_map(|v| {
                    Some(FieldChange {
                        entity: guid(v, "entity"),
                        component: text(v, "component")?,
                        path: text(v, "path").unwrap_or_default(),
                        old: v.get("old").cloned().unwrap_or(J::Null),
                        new: v.get("new").cloned().unwrap_or(J::Null),
                    })
                })
                .collect(),
            singletons_added: list("singletons_added").iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            singletons_removed: list("singletons_removed").iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        }
    }

    /// True if base and staged are the same document.
    pub fn is_empty(&self) -> bool {
        *self == Summary::default()
    }
}

/// A proposal with its diff (`proposal.get`).
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalDetail {
    /// Name, origin, size.
    pub info: ProposalInfo,
    /// Unified text diff of the scene.
    pub diff: String,
    /// The structural summary.
    pub summary: Summary,
}

impl ProposalDetail {
    pub(crate) fn from_json(j: &J) -> Option<ProposalDetail> {
        Some(ProposalDetail { info: ProposalInfo::from_json(j)?, diff: j.get("diff")?.as_str()?.to_string(), summary: Summary::from_json(j.get("summary")?) })
    }
}

/// The recording of a play session that was stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    /// The tick the session ended at.
    pub tick: u64,
    /// The frame checksum there.
    pub checksum: u64,
    /// The `.orrp` recording.
    pub replay: Vec<u8>,
}

/// A client connected to the host (from `activity.list`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Its name.
    pub name: String,
    /// What it may do, comma separated.
    pub caps: String,
    /// Requests it made.
    pub requests: u64,
}

/// An ERP value as short text: `1.5`, `[6, 18]`, `"name"`, `{kind: circle, radius: 0.5}`.
pub fn format_json(j: &J) -> String {
    match j {
        J::Null => "null".to_string(),
        J::String(s) => s.clone(),
        J::Array(a) => format!("[{}]", a.iter().map(format_json).collect::<Vec<_>>().join(", ")),
        J::Object(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{k}: {}", format_json(v))).collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}
