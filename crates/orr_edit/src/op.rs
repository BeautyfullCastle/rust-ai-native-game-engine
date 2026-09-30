//! The edit vocabulary: [`Op`], who made it ([`Origin`]) and what came of it.

use core::fmt;

use orr_reflect::{Guid, Value};

/// Who made an edit. Kept in the history so a UI can show "the agent changed
/// this" and undo it like any other edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A person at the editor.
    User,
    /// An AI agent (or any remote client), with a name for the log.
    Agent(String),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::User => f.write_str("user"),
            Origin::Agent(n) => write!(f, "agent:{n}"),
        }
    }
}

/// One edit of the scene document.
///
/// Values use the scene form: an entity reference is
/// [`Value::EntityGuid`], never [`Value::Entity`]. A whole-component value may
/// list only some fields of a struct (the rest come from the type's default);
/// the document stores the complete value.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Sets one field of a component. `path` is a reflect path (`"pos.x"`,
    /// `"shape.radius"`); the empty path replaces the whole component value.
    SetField {
        /// Entity.
        guid: Guid,
        /// Component type name.
        component: String,
        /// Field path.
        path: String,
        /// The new value (checked for type and range).
        value: Value,
    },
    /// Adds a component. `None` starts from the type's default value.
    AddComponent {
        /// Entity.
        guid: Guid,
        /// Component type name.
        component: String,
        /// Whole value, or `None` for the default.
        value: Option<Value>,
    },
    /// Removes a component.
    RemoveComponent {
        /// Entity.
        guid: Guid,
        /// Component type name.
        component: String,
    },
    /// Creates an entity. `guid: None` picks a fresh GUID (returned in
    /// [`Applied::guid`]).
    SpawnEntity {
        /// GUID to use, or `None` to have one made.
        guid: Option<Guid>,
        /// Display name.
        name: Option<String>,
        /// Components (each once, any order).
        components: Vec<(String, Value)>,
    },
    /// Deletes an entity with all its components. Refused while another
    /// entity or a singleton still points at it.
    DespawnEntity {
        /// Entity.
        guid: Guid,
    },
    /// Sets or clears the display name.
    Rename {
        /// Entity.
        guid: Guid,
        /// New name.
        name: Option<String>,
    },
    /// Sets one field of a singleton (`path` empty = whole value). A
    /// singleton the scene has no value for yet starts from its default.
    SetSingletonField {
        /// Singleton type name.
        singleton: String,
        /// Field path.
        path: String,
        /// The new value.
        value: Value,
    },
    /// Removes the scene's value for a singleton (it then bakes as default).
    RemoveSingleton {
        /// Singleton type name.
        singleton: String,
    },
}

impl Op {
    /// A short human description, for the history list.
    pub fn describe(&self) -> String {
        match self {
            Op::SetField { guid, component, path, .. } if path.is_empty() => format!("set {component} on {guid}"),
            Op::SetField { guid, component, path, .. } => format!("set {component}.{path} on {guid}"),
            Op::AddComponent { guid, component, .. } => format!("add {component} to {guid}"),
            Op::RemoveComponent { guid, component } => format!("remove {component} from {guid}"),
            Op::SpawnEntity { guid: Some(g), name: Some(n), .. } => format!("spawn {n} ({g})"),
            Op::SpawnEntity { guid: Some(g), .. } => format!("spawn {g}"),
            Op::SpawnEntity { .. } => "spawn entity".to_string(),
            Op::DespawnEntity { guid } => format!("despawn {guid}"),
            Op::Rename { guid, name: Some(n) } => format!("rename {guid} to {n}"),
            Op::Rename { guid, name: None } => format!("clear name of {guid}"),
            Op::SetSingletonField { singleton, path, .. } if path.is_empty() => format!("set singleton {singleton}"),
            Op::SetSingletonField { singleton, path, .. } => format!("set singleton {singleton}.{path}"),
            Op::RemoveSingleton { singleton } => format!("remove singleton {singleton}"),
        }
    }
}

/// What [`EditorDoc::apply`](crate::EditorDoc::apply) reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// For `SpawnEntity`, the GUID of the new entity.
    pub guid: Option<Guid>,
    /// False when the edit left the document as it was (same value, same
    /// name). Such an edit is not recorded in the history.
    pub changed: bool,
}

/// One line of [`EditorDoc::history`](crate::EditorDoc::history).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Unique, increasing id (never reused).
    pub id: u64,
    /// The transaction label, or the description of the single op.
    pub label: String,
    /// Who made it.
    pub origin: Origin,
    /// How many ops the entry holds (after coalescing).
    pub op_count: usize,
    /// True for entries that were undone and can be redone.
    pub undone: bool,
}
