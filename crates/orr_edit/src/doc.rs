//! [`EditorDoc`]: the edit-mode document.

use std::sync::Arc;

use orr_ecs::{ComponentRegistry, Frame};
use orr_fp::FrameRng;
use orr_reflect::{Guid, Scene, SceneIndex, TypeKind, TypeRegistry, Value};
use orr_session::PlayConfig;
use orr_sim::{Game, Simulation};

use crate::error::EditError;
use crate::op::{Applied, HistoryEntry, Op, Origin};
use crate::query::View;
use crate::scene_ops::{self, Effect, Refs};

/// One applied op with what is needed to run it again and to take it back.
#[derive(Clone, Debug)]
struct Step {
    forward: Op,
    inverse: Op,
}

#[derive(Clone, Debug)]
struct Entry {
    id: u64,
    label: String,
    origin: Origin,
    steps: Vec<Step>,
}

struct Tx {
    label: String,
    origin: Origin,
    steps: Vec<Step>,
}

/// The document an editor (or an agent) edits: a [`Scene`] with a baked
/// preview `Frame`, kept in sync after every edit, and one undo history.
///
/// Every change goes through [`apply`](Self::apply), whoever makes it, so a
/// person and an agent share one undo stack. The preview frame is baked from
/// the scene with the same code a play session uses, so the frame you see
/// while editing is the frame play starts from (checksum included).
///
/// Nothing here touches a window, a socket or a clock.
pub struct EditorDoc {
    pub(crate) scene: Scene,
    pub(crate) types: Arc<TypeRegistry>,
    pub(crate) frame_registry: Arc<ComponentRegistry>,
    pub(crate) seed: u64,
    pub(crate) frame: Frame,
    pub(crate) index: SceneIndex,
    /// Text the document was loaded from or last saved as.
    source: Option<String>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// History id at the last load or save (0 = start).
    clean_id: u64,
    next_id: u64,
    pub(crate) next_guid: u32,
    tx: Option<Tx>,
    pub(crate) proposals: crate::proposal::Proposals,
}

impl EditorDoc {
    // ---- creation and loading ----

    /// An empty scene. `frame_registry` must be the one the game's
    /// `Simulation` uses (see [`for_game`](Self::for_game)); `seed` seeds the
    /// `FrameRng` of the preview frame (and so of a play session).
    pub fn new(types: TypeRegistry, frame_registry: Arc<ComponentRegistry>, seed: u64) -> Result<Self, EditError> {
        Self::from_scene(Scene::default(), types, frame_registry, seed)
    }

    /// An empty scene for game `G` (registry from `Simulation::<G>::build_registry()`).
    pub fn for_game<G: Game>(types: TypeRegistry, seed: u64) -> Result<Self, EditError> {
        Self::new(types, Simulation::<G>::build_registry(), seed)
    }

    /// Loads scene text. Comments are kept. The document is not dirty, and
    /// [`to_yaml`](Self::to_yaml) returns `text` byte for byte until the first edit.
    pub fn from_yaml(text: &str, types: TypeRegistry, frame_registry: Arc<ComponentRegistry>, seed: u64) -> Result<Self, EditError> {
        let scene = Scene::parse(text, &types)?;
        let mut doc = Self::from_scene(scene, types, frame_registry, seed)?;
        doc.source = Some(text.to_string());
        Ok(doc)
    }

    /// Wraps a scene (already valid for `types`).
    pub fn from_scene(scene: Scene, types: TypeRegistry, frame_registry: Arc<ComponentRegistry>, seed: u64) -> Result<Self, EditError> {
        let mismatch = types.check_against(&frame_registry);
        if !mismatch.is_empty() {
            return Err(EditError::RegistryMismatch(mismatch));
        }
        let types = Arc::new(types);
        let scene = canonicalize(scene, &types)?;
        let (frame, index) = bake(&scene, &types, &frame_registry, seed)?;
        let next_guid = next_free_guid(&scene, 1);
        Ok(Self {
            scene,
            types,
            frame_registry,
            seed,
            frame,
            index,
            source: None,
            undo: Vec::new(),
            redo: Vec::new(),
            clean_id: 0,
            next_id: 1,
            next_guid,
            tx: None,
            proposals: Default::default(),
        })
    }

    /// Replaces the whole document with scene text. History is cleared and
    /// the document is clean. Refused (and nothing changes) if the text is
    /// invalid or a transaction is open.
    pub fn load_yaml(&mut self, text: &str) -> Result<(), EditError> {
        if self.tx.is_some() {
            return Err(EditError::TxOpen);
        }
        let scene = canonicalize(Scene::parse(text, &self.types)?, &self.types)?;
        let (frame, index) = bake(&scene, &self.types, &self.frame_registry, self.seed)?;
        self.next_guid = next_free_guid(&scene, self.next_guid);
        self.scene = scene;
        self.frame = frame;
        self.index = index;
        self.source = Some(text.to_string());
        self.undo.clear();
        self.redo.clear();
        self.clean_id = 0;
        self.proposals.clear();
        Ok(())
    }

    /// A copy for staging: same scene, frame (same entity handles), index and
    /// GUID counter; empty history, no proposals.
    pub(crate) fn fork(&self) -> Result<EditorDoc, EditError> {
        let frame = Frame::from_bytes(self.frame_registry.clone(), &self.frame.to_bytes())
            .map_err(|e| EditError::Invalid(format!("cannot copy the preview frame: {e}")))?;
        Ok(EditorDoc {
            scene: self.scene.clone(),
            types: self.types.clone(),
            frame_registry: self.frame_registry.clone(),
            seed: self.seed,
            frame,
            index: self.index.clone(),
            source: None,
            undo: Vec::new(),
            redo: Vec::new(),
            clean_id: 0,
            next_id: 1,
            next_guid: self.next_guid,
            tx: None,
            proposals: Default::default(),
        })
    }

    // ---- saving ----

    /// The document as scene text. Unedited since load or save: the exact
    /// text of that load or save. Otherwise regenerated, with the comments
    /// of the entities and components that still exist.
    pub fn to_yaml(&self) -> String {
        if !self.is_dirty() {
            if let Some(s) = &self.source {
                return s.clone();
            }
        }
        let mut out = Scene { singletons: self.scene.singletons.clone(), entities: self.scene.entities.clone(), ..Scene::default() };
        out.carry_comments_from(&self.scene);
        out.to_yaml()
    }

    /// [`to_yaml`](Self::to_yaml), and marks the document clean (as saved).
    pub fn save_yaml(&mut self) -> String {
        let text = self.to_yaml();
        self.source = Some(text.clone());
        self.clean_id = self.top_id();
        text
    }

    /// True if the document differs from the last load or save. Undoing back
    /// to the saved state makes it clean again.
    pub fn is_dirty(&self) -> bool {
        self.top_id() != self.clean_id || self.tx.as_ref().is_some_and(|t| !t.steps.is_empty())
    }

    fn top_id(&self) -> u64 {
        self.undo.last().map_or(0, |e| e.id)
    }

    // ---- reads ----

    /// The scene document (GUID form values).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    /// The type registry (shared with play controllers).
    pub fn types(&self) -> &Arc<TypeRegistry> {
        &self.types
    }
    /// The component registry of the preview frame.
    pub fn frame_registry(&self) -> &Arc<ComponentRegistry> {
        &self.frame_registry
    }
    /// The baked preview frame, always in sync with the scene.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    /// GUID to preview-frame entity map.
    pub fn index(&self) -> &SceneIndex {
        &self.index
    }
    /// Checksum of the preview frame.
    pub fn checksum(&self) -> u64 {
        self.frame.checksum()
    }
    /// The `FrameRng` seed of the preview frame.
    pub fn seed(&self) -> u64 {
        self.seed
    }
    /// Read-only queries on the preview frame.
    pub fn view(&self) -> View<'_> {
        View { types: &self.types, frame: &self.frame, index: &self.index }
    }
    /// A paused [`PlayConfig`] with this document's seed, to pass to
    /// [`PlayController::start_play`](crate::PlayController::start_play).
    pub fn play_config(&self, player_count: u8, tick_rate: u32) -> PlayConfig {
        let mut cfg = PlayConfig::new(player_count, self.seed, tick_rate);
        cfg.start_paused = true;
        cfg
    }

    // ---- editing ----

    /// Applies one edit. Outside a transaction it is one undo step. Inside
    /// one it joins the transaction (consecutive `SetField`s on the same
    /// field are merged into one op). An invalid edit returns `Err` and
    /// changes nothing.
    ///
    /// If a transaction of another origin is open, returns [`EditError::TxBusy`].
    pub fn apply(&mut self, op: Op, origin: Origin) -> Result<Applied, EditError> {
        if let Some(tx) = &self.tx {
            if tx.origin != origin {
                return Err(EditError::TxBusy { owner: tx.origin.to_string() });
            }
        }
        let op = self.with_guid(op);
        let done = scene_ops::apply(&mut self.scene, &self.types, &op)?;
        if !done.changed {
            return Ok(Applied { guid: done.guid, changed: false });
        }
        if let Err(e) = self.sync(std::slice::from_ref(&done.effect)) {
            // Cannot happen for checked values; keep the invariant anyway.
            let _ = scene_ops::apply(&mut self.scene, &self.types, &done.inverse);
            let _ = self.rebake();
            return Err(e);
        }
        self.redo.clear();
        let step = Step { forward: done.forward, inverse: done.inverse };
        match &mut self.tx {
            Some(tx) => push_coalescing(&mut tx.steps, step),
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.undo.push(Entry { id, label: step.forward.describe(), origin, steps: vec![step] });
            }
        }
        Ok(Applied { guid: done.guid, changed: true })
    }

    /// Applies several ops as one transaction: all or nothing, one undo step.
    pub fn apply_batch(&mut self, label: &str, ops: Vec<Op>, origin: Origin) -> Result<Vec<Applied>, EditError> {
        self.begin_tx(label, origin.clone())?;
        let mut out = Vec::with_capacity(ops.len());
        for op in ops {
            match self.apply(op, origin.clone()) {
                Ok(a) => out.push(a),
                Err(e) => {
                    self.rollback_tx()?;
                    return Err(e);
                }
            }
        }
        self.commit_tx()?;
        Ok(out)
    }

    fn with_guid(&mut self, op: Op) -> Op {
        match op {
            Op::SpawnEntity { guid: None, name, components } => {
                let guid = loop {
                    let g = Guid::from_u32(self.next_guid);
                    self.next_guid = self.next_guid.wrapping_add(1);
                    if !self.scene.entities.contains_key(&g) {
                        break g;
                    }
                };
                Op::SpawnEntity { guid: Some(guid), name, components }
            }
            other => other,
        }
    }

    // ---- transactions ----

    /// Starts a transaction: the ops applied until [`commit_tx`](Self::commit_tx)
    /// become one history entry, undone as one. Ops of another origin are
    /// refused meanwhile.
    pub fn begin_tx(&mut self, label: &str, origin: Origin) -> Result<(), EditError> {
        if self.tx.is_some() {
            return Err(EditError::TxOpen);
        }
        self.tx = Some(Tx { label: label.to_string(), origin, steps: Vec::new() });
        Ok(())
    }

    /// Ends the transaction and records it. An empty transaction records nothing.
    pub fn commit_tx(&mut self) -> Result<(), EditError> {
        let tx = self.tx.take().ok_or(EditError::NoTx)?;
        if !tx.steps.is_empty() {
            let id = self.next_id;
            self.next_id += 1;
            self.undo.push(Entry { id, label: tx.label, origin: tx.origin, steps: tx.steps });
            self.redo.clear();
        }
        Ok(())
    }

    /// Ends the transaction and takes back everything it did.
    pub fn rollback_tx(&mut self) -> Result<(), EditError> {
        let tx = self.tx.take().ok_or(EditError::NoTx)?;
        self.revert(&tx.steps)
    }

    /// True while a transaction is open.
    pub fn in_tx(&self) -> bool {
        self.tx.is_some()
    }

    // ---- undo / redo ----

    /// Takes back the last history entry (a whole transaction at once).
    pub fn undo(&mut self) -> Result<(), EditError> {
        if self.tx.is_some() {
            return Err(EditError::TxOpen);
        }
        let entry = self.undo.pop().ok_or(EditError::NothingToUndo)?;
        match self.revert(&entry.steps) {
            Ok(()) => {
                self.redo.push(entry);
                Ok(())
            }
            Err(e) => {
                self.undo.push(entry);
                Err(e)
            }
        }
    }

    /// Repeats the last undone entry.
    pub fn redo(&mut self) -> Result<(), EditError> {
        if self.tx.is_some() {
            return Err(EditError::TxOpen);
        }
        let entry = self.redo.pop().ok_or(EditError::NothingToRedo)?;
        let ops: Vec<&Op> = entry.steps.iter().map(|s| &s.forward).collect();
        match self.run_all(ops) {
            Ok(()) => {
                self.undo.push(entry);
                Ok(())
            }
            Err(e) => {
                self.redo.push(entry);
                Err(e)
            }
        }
    }

    /// True if [`undo`](Self::undo) has something to take back.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// True if [`redo`](Self::redo) has something to repeat.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The history, oldest first: entries that are in effect, then the undone
    /// ones (`undone: true`) in the order redo would repeat them reversed.
    /// An open transaction is not listed until it is committed.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let info = |e: &Entry, undone: bool| HistoryEntry {
            id: e.id,
            label: e.label.clone(),
            origin: e.origin.clone(),
            op_count: e.steps.len(),
            undone,
        };
        self.undo.iter().map(|e| info(e, false)).chain(self.redo.iter().rev().map(|e| info(e, true))).collect()
    }

    /// Undoes `steps` (in reverse) as a unit: on failure the scene is restored.
    fn revert(&mut self, steps: &[Step]) -> Result<(), EditError> {
        let ops: Vec<&Op> = steps.iter().rev().map(|s| &s.inverse).collect();
        self.run_all(ops)
    }

    /// Applies ops in order with one preview sync at the end; all or nothing.
    fn run_all(&mut self, ops: Vec<&Op>) -> Result<(), EditError> {
        let backup = self.scene.clone();
        let mut effects = Vec::with_capacity(ops.len());
        for op in ops {
            match scene_ops::apply(&mut self.scene, &self.types, op) {
                Ok(d) => effects.push(d.effect),
                Err(e) => {
                    self.scene = backup;
                    return Err(e);
                }
            }
        }
        if let Err(e) = self.sync(&effects) {
            self.scene = backup;
            let _ = self.rebake();
            return Err(e);
        }
        Ok(())
    }

    // ---- preview frame ----

    /// Brings the preview frame in line with the scene after ops with these
    /// effects: patches single fields and names in place, rebakes otherwise.
    fn sync(&mut self, effects: &[Effect]) -> Result<(), EditError> {
        if effects.iter().any(|e| matches!(e, Effect::Structural)) {
            return self.rebake();
        }
        for e in effects {
            match e {
                Effect::None => {}
                Effect::Rename { guid } => {
                    let name = self.scene.entities.get(guid).and_then(|e| e.name.clone());
                    self.index.set_name(guid, name);
                }
                Effect::Field { guid, component } => {
                    if !self.patch_field(guid, component)? {
                        return self.rebake();
                    }
                }
                Effect::Structural => unreachable!("handled above"),
            }
        }
        Ok(())
    }

    /// Writes the scene's value of one component into the preview frame.
    /// `false` if the frame does not have it (caller rebakes).
    fn patch_field(&mut self, guid: &Guid, component: &str) -> Result<bool, EditError> {
        let Some(entity) = self.index.entity(guid) else { return Ok(false) };
        let Some((_, value)) = self.scene.entities.get(guid).and_then(|e| e.components.iter().find(|(n, _)| n == component)) else {
            return Ok(false);
        };
        let index = &self.index;
        let resolved = crate::refs::map_refs(value, &mut |r| match r {
            Value::EntityGuid(None) => Ok(Value::Entity(orr_ecs::Entity::NONE)),
            Value::EntityGuid(Some(g)) => {
                let target = Guid::parse(g).ok().and_then(|g| index.entity(&g));
                target.map(Value::Entity).ok_or_else(|| EditError::Invalid(format!("reference to unknown entity '{g}'")))
            }
            other => Ok(other.clone()),
        })?;
        self.types.add_component_value(&mut self.frame, entity, component, &resolved)?;
        Ok(true)
    }

    /// Bakes the scene into a fresh preview frame. Edits do this themselves
    /// (or patch in place); call it only to force a full rebuild.
    pub fn rebake(&mut self) -> Result<(), EditError> {
        let (frame, index) = bake(&self.scene, &self.types, &self.frame_registry, self.seed)?;
        self.frame = frame;
        self.index = index;
        Ok(())
    }
}

/// Adds `step`, merging it into the last one when both set the same field.
fn push_coalescing(steps: &mut Vec<Step>, step: Step) {
    if let (Some(last), Op::SetField { guid, component, path, .. }) = (steps.last_mut(), &step.forward) {
        if let Op::SetField { guid: g, component: c, path: p, .. } = &last.forward {
            if g == guid && c == component && p == path {
                last.forward = step.forward;
                return;
            }
        }
    }
    steps.push(step);
}

fn bake(
    scene: &Scene,
    types: &TypeRegistry,
    registry: &Arc<ComponentRegistry>,
    seed: u64,
) -> Result<(Frame, SceneIndex), EditError> {
    let mut frame = Frame::new(registry.clone());
    if registry.singleton_id::<FrameRng>().is_some() {
        frame.set_singleton(FrameRng::new(seed));
    }
    let index = scene.bake(types, &mut frame)?;
    Ok((frame, index))
}

/// Rewrites every value into its canonical form (all fields present, the
/// form a bake reads back), so later edits and undos compare exactly.
fn canonicalize(mut scene: Scene, types: &TypeRegistry) -> Result<Scene, EditError> {
    let entities = scene.entities.clone();
    let refs = Refs::new(&entities, None);
    for ent in scene.entities.values_mut() {
        for (name, value) in &mut ent.components {
            let t = scene_ops::type_of(types, name, TypeKind::Component)?;
            *value = scene_ops::normalize(t, value, &refs)?;
        }
    }
    for (name, value) in &mut scene.singletons {
        let t = scene_ops::type_of(types, name, TypeKind::Singleton)?;
        *value = scene_ops::normalize(t, value, &refs)?;
    }
    Ok(scene)
}

/// The first GUID counter value at or above `start` that no entity uses.
fn next_free_guid(scene: &Scene, start: u32) -> u32 {
    let mut n = start;
    while scene.entities.contains_key(&Guid::from_u32(n)) {
        n = n.wrapping_add(1);
    }
    n
}
