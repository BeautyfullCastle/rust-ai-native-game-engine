//! `orr_edit`: the GUI-free, transport-free core of the Orrery editor.
//!
//! Two front ends drive this one model: the egui editor (a person) and the
//! ERP server (AI agents). Both call the same methods, so their edits share
//! one undo stack and one history.
//!
//! # Pieces
//!
//! - [`EditorDoc`]: edit mode. A [`Scene`](orr_reflect::Scene), the type
//!   registry, a baked preview `Frame` kept in sync, a dirty flag and undo/redo.
//!   Edits are [`Op`]s applied through [`EditorDoc::apply`] with an [`Origin`].
//!   Transactions ([`EditorDoc::begin_tx`], [`commit_tx`](EditorDoc::commit_tx),
//!   [`rollback_tx`](EditorDoc::rollback_tx), [`apply_batch`](EditorDoc::apply_batch))
//!   group ops into one undo step.
//! - [`PlayController`]: play mode. Copies the baked frame into a
//!   `PlaySession`, sends edits as recorded `DebugCommand`s, and exposes the
//!   timeline (play, pause, step, seek, branch).
//! - Proposals ([`EditorDoc::propose`]): an agent stages [`Op`]s on a private
//!   copy of the document (preview via [`EditorDoc::proposal_preview`], text and
//!   structural diff via [`EditorDoc::proposal_diff`]) while the person keeps
//!   editing; [`EditorDoc::accept`] applies them as one history entry.
//! - Verification ([`verify_frames`], [`EditorDoc::verify_proposal`]): run the
//!   base and the candidate headlessly on recorded or scripted inputs and
//!   compare checksums and [`Metrics`]; [`Check`] rules turn the
//!   [`VerifyReport`] into pass/fail.
//! - [`View`]: read-only queries (entities, components as `Value`s, singletons,
//!   JSON Schema) over the preview frame or the live play frame.
//!
//! # Loop
//!
//! ```text
//! EditorDoc::from_yaml -> apply(Op, Origin) ... undo/redo ... save_yaml
//!        \-- PlayController::start_play(&doc, doc.play_config(..))
//!               set_field / control(Seek..) ... stop_play() -> replay bytes
//! ```
//!
//! # Notes
//!
//! - Values in [`Op`]s and queries are scene form: entity references are
//!   `Value::EntityGuid`.
//! - The preview frame is patched in place for field edits and rebaked for
//!   structural ones. Entity handles of the preview frame can change on a
//!   rebake (entities are numbered in GUID order); GUIDs never change.
//! - This is a tool crate: no floats for sim values, `BTreeMap` for order.

mod checks;
mod diff;
mod doc;
mod error;
mod fragment;
#[cfg(feature = "linked-prefabs")]
mod linked_prefab;
mod op;
mod play;
mod proposal;
mod query;
mod refs;
mod scene_ops;
mod verify;

pub use checks::{evaluate_checks, Check, CheckOutcome, CheckResult, Cmp, MetricStat, Side};
pub use diff::{
    format_value, summarize, unified_diff, EntityRef, FieldChange, ProposalSummary, Renamed,
};
pub use doc::{BakeAdmission, EditorDoc};
pub use error::EditError;
pub use fragment::{
    FragmentInstance, FragmentTranslation2D, SceneFragment, FRAGMENT_MAX_BYTES,
    FRAGMENT_MAX_COMPONENTS, FRAGMENT_MAX_DEPTH, FRAGMENT_MAX_ENTITIES, FRAGMENT_MAX_STRING_BYTES,
    FRAGMENT_MAX_VALUES,
};
pub use op::{Applied, HistoryEntry, Op, Origin};
pub use play::{PlayController, StoppedPlay};
pub use proposal::{Accepted, ProposalDiff, ProposalId, ProposalInfo, ProposalState};
pub use query::{EntityInfo, Target, View};
pub use verify::{
    verify_frames, verify_frames_cancellable, ChecksumSample, MetricComparison, MetricStats,
    RecordingCheck, ReflectMetrics, VerifyInputs, VerifyOptions, VerifyReport,
};

pub use orr_sim::{MetricValue, Metrics, NoMetrics};
