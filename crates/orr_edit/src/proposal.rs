//! Proposals: named sets of [`Op`]s staged against the document without
//! touching it, for an agent to build while the person keeps editing.
//!
//! A proposal owns a private copy of the document (a scene plus its preview
//! frame). Every op is validated against that copy as it is added, so what
//! the agent sees (`proposal_preview`, `proposal_diff`) is exactly what
//! `accept` will produce, as long as the document has not changed
//! meanwhile. `accept` applies all ops to the real document as one history
//! entry with the proposal's origin, all or nothing; if the document
//! changed and an op no longer applies, `accept` fails with
//! [`EditError::ProposalConflict`] and changes nothing.

use core::fmt;
use std::collections::BTreeMap;

use orr_reflect::{Guid, Scene};

use crate::diff::{self, ProposalSummary};
use crate::doc::EditorDoc;
use crate::error::EditError;
use crate::op::{Applied, Op, Origin};
use crate::query::View;

/// Identifies a proposal of one [`EditorDoc`]. Ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProposalId(pub u64);

impl fmt::Display for ProposalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// A document-local concurrency guard for a proposal's verification inputs.
/// Capture it while holding the same shared document borrow as verification,
/// then pass it to [`EditorDoc::accept_if_unchanged`]. This is not proof that
/// verification ran or that its checks passed; the caller must judge the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalState {
    /// Opaque document-instance identity; a new host/document cannot reuse it.
    pub document_id: u128,
    /// The proposal whose ops were captured.
    pub id: ProposalId,
    /// Revision of the base document.
    pub document_revision: u64,
    /// Revision of the proposal's staged document.
    pub proposal_revision: u64,
}

/// One line of [`EditorDoc::list_proposals`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalInfo {
    /// The proposal.
    pub id: ProposalId,
    /// Its name.
    pub label: String,
    /// Who made it; the origin of the history entry `accept` records.
    pub origin: Origin,
    /// Ops staged so far.
    pub op_count: usize,
    /// True if the document has changed since the proposal was made (the
    /// proposal may still accept cleanly; see
    /// [`EditorDoc::proposal_check`]).
    pub stale: bool,
}

/// A proposal's change against the document it was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalDiff {
    /// Unified diff of the scene text: `--- base` / `+++ proposal`, hunks of
    /// `@@ -l,n +l,n @@` with 3 lines of context. Empty if nothing changed.
    pub text: String,
    /// The same change, structurally.
    pub summary: ProposalSummary,
}

/// What [`EditorDoc::accept`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The history entry the proposal became (`None` if its ops changed nothing).
    pub history_id: Option<u64>,
    /// One result per op, in order (`SpawnEntity` reports its GUID).
    pub applied: Vec<Applied>,
}

struct Proposal {
    label: String,
    origin: Origin,
    ops: Vec<Op>,
    staged: Box<EditorDoc>,
    /// The document's scene when the proposal was made (no comments).
    base: Scene,
}

/// The proposals of one document.
#[derive(Default)]
pub(crate) struct Proposals {
    map: BTreeMap<u64, Proposal>,
    last_id: u64,
}

impl Proposals {
    /// Drops every proposal (ids keep counting).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }
}

/// The scene without comments, the form used for diffs.
fn plain(scene: &Scene) -> Scene {
    Scene { singletons: scene.singletons.clone(), entities: scene.entities.clone(), ..Scene::default() }
}

fn same_content(a: &Scene, b: &Scene) -> bool {
    a.singletons == b.singletons && a.entities == b.entities
}

impl EditorDoc {
    fn proposal(&self, id: ProposalId) -> Result<&Proposal, EditError> {
        self.proposals.map.get(&id.0).ok_or(EditError::UnknownProposal(id.0))
    }

    /// Starts an empty proposal: a private copy of the document to stage
    /// ops on. Cheap; the document is not touched and stays editable.
    pub fn propose(&mut self, label: &str, origin: Origin) -> Result<ProposalId, EditError> {
        let staged = Box::new(self.fork()?);
        self.proposals.last_id += 1;
        let id = self.proposals.last_id;
        let base = plain(&self.scene);
        self.proposals.map.insert(id, Proposal { label: label.to_string(), origin, ops: Vec::new(), staged, base });
        Ok(ProposalId(id))
    }

    /// Stages one op. It is checked against the proposal's private copy
    /// (type, range, references) and an invalid op returns `Err` and stages
    /// nothing. An op that changes nothing is not recorded. A `SpawnEntity`
    /// without a GUID gets one now (returned in [`Applied::guid`]), so
    /// `accept` creates the entity under the GUID the preview showed.
    pub fn proposal_apply(&mut self, id: ProposalId, op: Op) -> Result<Applied, EditError> {
        self.proposal(id)?;
        let op = match op {
            Op::SpawnEntity { guid: None, name, components } => {
                Op::SpawnEntity { guid: Some(self.fresh_guid()), name, components }
            }
            other => other,
        };
        let p = self.proposals.map.get_mut(&id.0).ok_or(EditError::UnknownProposal(id.0))?;
        let applied = p.staged.apply(op.clone(), p.origin.clone())?;
        if applied.changed {
            p.ops.push(op);
        }
        Ok(applied)
    }

    /// Stages several ops as one step: all or nothing. If any op is invalid,
    /// none is staged and the proposal is unchanged. Otherwise like
    /// [`proposal_apply`](Self::proposal_apply) for each op, in order.
    pub fn proposal_apply_all(&mut self, id: ProposalId, ops: Vec<Op>) -> Result<Vec<Applied>, EditError> {
        self.proposal(id)?;
        let mut filled = Vec::with_capacity(ops.len());
        for op in ops {
            filled.push(match op {
                Op::SpawnEntity { guid: None, name, components } => Op::SpawnEntity { guid: Some(self.fresh_guid()), name, components },
                other => other,
            });
        }
        let p = self.proposals.map.get_mut(&id.0).ok_or(EditError::UnknownProposal(id.0))?;
        let applied = p.staged.apply_batch(&p.label, filled.clone(), p.origin.clone())?;
        for (op, a) in filled.into_iter().zip(&applied) {
            if a.changed {
                p.ops.push(op);
            }
        }
        Ok(applied)
    }

    /// A GUID no entity of the document or of any proposal uses.
    fn fresh_guid(&mut self) -> Guid {
        loop {
            let g = Guid::from_u32(self.next_guid);
            self.next_guid = self.next_guid.wrapping_add(1);
            let used = self.scene.entities.contains_key(&g)
                || self.proposals.map.values().any(|p| p.staged.scene.entities.contains_key(&g));
            if !used {
                return g;
            }
        }
    }

    /// The ops staged so far, in order (GUIDs filled in).
    pub fn proposal_ops(&self, id: ProposalId) -> Result<&[Op], EditError> {
        Ok(&self.proposal(id)?.ops)
    }

    /// Captures the current base and proposal revisions. Edits, undo, redo
    /// and rollback invalidate this state even if they restore old values.
    /// Reading or successfully staging no-op edits does not invalidate it.
    pub fn proposal_state(&self, id: ProposalId) -> Result<ProposalState, EditError> {
        Ok(ProposalState {
            document_id: self.instance_id,
            id,
            document_revision: self.revision(),
            proposal_revision: self.proposal(id)?.staged.revision(),
        })
    }

    /// Name, origin and size of one proposal.
    pub fn proposal_info(&self, id: ProposalId) -> Result<ProposalInfo, EditError> {
        Ok(self.info_of(id.0, self.proposal(id)?))
    }

    fn info_of(&self, id: u64, p: &Proposal) -> ProposalInfo {
        ProposalInfo {
            id: ProposalId(id),
            label: p.label.clone(),
            origin: p.origin.clone(),
            op_count: p.ops.len(),
            stale: !same_content(&self.scene, &p.base),
        }
    }

    /// All open proposals, oldest first.
    pub fn list_proposals(&self) -> Vec<ProposalInfo> {
        self.proposals.map.iter().map(|(&id, p)| self.info_of(id, p)).collect()
    }

    /// Read-only queries on the proposal's preview frame (its entities,
    /// components and singletons as they would be after `accept`), and the
    /// frame to draw in the viewport. The document's own view is unchanged.
    pub fn proposal_preview(&self, id: ProposalId) -> Result<View<'_>, EditError> {
        Ok(self.proposal(id)?.staged.view())
    }

    /// The proposal's staged scene.
    pub fn proposal_scene(&self, id: ProposalId) -> Result<&Scene, EditError> {
        Ok(&self.proposal(id)?.staged.scene)
    }

    /// The change the proposal makes to the document it was made from: a
    /// unified text diff and a structural summary. It does not include
    /// edits made to the document since the proposal was made.
    pub fn proposal_diff(&self, id: ProposalId) -> Result<ProposalDiff, EditError> {
        let p = self.proposal(id)?;
        let text = diff::unified_diff(&p.base.to_yaml(), &plain(&p.staged.scene).to_yaml(), "base", "proposal", 3);
        Ok(ProposalDiff { text, summary: diff::summarize(&p.base, &p.staged.scene) })
    }

    /// Would `accept` succeed now? Runs the ops on a copy of the current
    /// document; the document and the proposal are unchanged.
    pub fn proposal_check(&self, id: ProposalId) -> Result<(), EditError> {
        let p = self.proposal(id)?;
        let mut copy = self.fork()?;
        run_ops(&mut copy, id, &p.ops, &p.origin).map(|_| ())
    }

    /// Accepts only if neither the base document nor the proposal has changed
    /// since `expected` was captured. The comparison and acceptance happen
    /// under one exclusive borrow. A stale state leaves both untouched;
    /// verify again before retrying. Like [`accept`](Self::accept), this does
    /// not decide whether the caller's verification checks passed.
    pub fn accept_if_unchanged(&mut self, id: ProposalId, expected: ProposalState) -> Result<Accepted, EditError> {
        if self.in_tx() {
            return Err(EditError::TxOpen);
        }
        if self.proposal_state(id)? != expected {
            return Err(EditError::StaleVerification { proposal: id.0 });
        }
        self.accept(id)
    }

    /// Applies the proposal to the document as one history entry (label of
    /// the proposal, its origin), all or nothing, and removes it. Undo takes
    /// it back like any edit. If a transaction is open, returns
    /// [`EditError::TxOpen`]. If the document changed since the proposal was
    /// made and an op no longer applies, returns
    /// [`EditError::ProposalConflict`]; the document changes not at all and
    /// the proposal stays. Ops that still apply, do so even if the person
    /// edited other things (or the same field: the proposal's value wins).
    pub fn accept(&mut self, id: ProposalId) -> Result<Accepted, EditError> {
        if self.in_tx() {
            return Err(EditError::TxOpen);
        }
        let p = self.proposal(id)?;
        let (label, origin, ops) = (p.label.clone(), p.origin.clone(), p.ops.clone());
        let before = self.top_entry_id();
        self.begin_tx(&label, origin.clone())?;
        let applied = match run_ops(self, id, &ops, &origin) {
            Ok(a) => a,
            Err(e) => {
                self.rollback_tx()?;
                return Err(e);
            }
        };
        self.commit_tx()?;
        self.proposals.map.remove(&id.0);
        let now = self.top_entry_id();
        let history_id = if now != before { now } else { None };
        Ok(Accepted { history_id, applied })
    }

    /// The id of the newest history entry in effect.
    fn top_entry_id(&self) -> Option<u64> {
        self.history().iter().rev().find(|h| !h.undone).map(|h| h.id)
    }

    /// Discards a proposal.
    pub fn reject(&mut self, id: ProposalId) -> Result<(), EditError> {
        self.proposals.map.remove(&id.0).map(|_| ()).ok_or(EditError::UnknownProposal(id.0))
    }
}

/// Applies `ops` in order to `doc` (which must be in a transaction of
/// `origin`, or a scratch copy), mapping a failure to a conflict.
fn run_ops(doc: &mut EditorDoc, id: ProposalId, ops: &[Op], origin: &Origin) -> Result<Vec<Applied>, EditError> {
    let mut out = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        match doc.apply(op.clone(), origin.clone()) {
            Ok(a) => out.push(a),
            Err(cause) => return Err(EditError::ProposalConflict { proposal: id.0, op_index: i, cause: Box::new(cause) }),
        }
    }
    Ok(out)
}
