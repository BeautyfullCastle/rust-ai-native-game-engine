//! Errors of the editor core.

use core::fmt;

use orr_reflect::{BakeError, Guid, ReflectError, SceneError};
use orr_sim::DebugError;

/// Why an edit, undo, load or play-mode call was refused. A refused call
/// changes nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum EditError {
    /// No entity with this GUID (or entity handle) exists.
    UnknownEntity(String),
    /// A type name that is not registered (or is the wrong kind: component vs singleton).
    UnknownType(String),
    /// The entity has no such component.
    NoComponent {
        /// Entity (GUID text, or `entity 12v0` for one without a GUID).
        entity: String,
        /// Component type name.
        component: String,
    },
    /// The entity already has the component (`AddComponent`, `SpawnEntity`).
    HasComponent {
        /// Entity (same form as in `NoComponent`).
        entity: String,
        /// Component type name.
        component: String,
    },
    /// The GUID is already used by another entity.
    GuidExists(Guid),
    /// The entity cannot be despawned: other entities point at it.
    Referenced {
        /// The entity that was to be despawned.
        guid: Guid,
        /// One entity that refers to it.
        by: Guid,
    },
    /// A value or path was refused by reflection (type, range, path).
    Reflect(ReflectError),
    /// Anything else that makes the request invalid.
    Invalid(String),
    /// A transaction is already open (`begin_tx`, `undo`, `redo`, `load`).
    TxOpen,
    /// No transaction is open (`commit_tx`, `rollback_tx`).
    NoTx,
    /// A transaction of another origin is open; wait for it to end.
    TxBusy {
        /// Who owns the open transaction.
        owner: String,
    },
    /// Nothing to undo.
    NothingToUndo,
    /// Nothing to redo.
    NothingToRedo,
    /// The scene text was rejected.
    Parse(SceneError),
    /// The scene could not be baked into a frame.
    Bake(BakeError),
    /// The type registry and the frame's component registry disagree.
    RegistryMismatch(Vec<String>),
    /// A play-mode debug command was refused by the session.
    Debug(DebugError),
    /// The play session could not start.
    PlayStart(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::UnknownEntity(g) => write!(f, "unknown entity '{g}'"),
            EditError::UnknownType(n) => write!(f, "unknown or wrong kind of type '{n}'"),
            EditError::NoComponent { entity, component } => write!(f, "entity '{entity}' has no component '{component}'"),
            EditError::HasComponent { entity, component } => write!(f, "entity '{entity}' already has component '{component}'"),
            EditError::GuidExists(g) => write!(f, "entity '{g}' already exists"),
            EditError::Referenced { guid, by } => write!(f, "entity '{guid}' is referenced by '{by}'"),
            EditError::Reflect(e) => write!(f, "{e}"),
            EditError::Invalid(m) => write!(f, "{m}"),
            EditError::TxOpen => write!(f, "a transaction is open"),
            EditError::NoTx => write!(f, "no transaction is open"),
            EditError::TxBusy { owner } => write!(f, "a transaction of {owner} is open"),
            EditError::NothingToUndo => write!(f, "nothing to undo"),
            EditError::NothingToRedo => write!(f, "nothing to redo"),
            EditError::Parse(e) => write!(f, "{e}"),
            EditError::Bake(e) => write!(f, "{e}"),
            EditError::RegistryMismatch(m) => write!(f, "type registry does not match the frame: {}", m.join("; ")),
            EditError::Debug(e) => write!(f, "debug command refused: {e}"),
            EditError::PlayStart(m) => write!(f, "cannot start play: {m}"),
        }
    }
}

impl std::error::Error for EditError {}

impl From<ReflectError> for EditError {
    fn from(e: ReflectError) -> Self {
        EditError::Reflect(e)
    }
}
impl From<BakeError> for EditError {
    fn from(e: BakeError) -> Self {
        EditError::Bake(e)
    }
}
impl From<SceneError> for EditError {
    fn from(e: SceneError) -> Self {
        EditError::Parse(e)
    }
}
impl From<DebugError> for EditError {
    fn from(e: DebugError) -> Self {
        EditError::Debug(e)
    }
}
