//! [`PlayController`]: play mode of the editor.

use std::collections::BTreeSet;
use std::sync::Arc;

use orr_ecs::{ComponentId, Entity, Frame, SingletonId};
use orr_reflect::{Scene, SceneIndex, TypeInfo, TypeKind, TypeRegistry, Value};
use orr_session::{ControlOp, PlayConfig, PlaySession, Timeline};
use orr_sim::{DebugCommand, Game, SimEvent};

use crate::doc::EditorDoc;
use crate::error::EditError;
use crate::query::{Target, View};
use crate::refs::map_refs;
use crate::scene_ops::{type_of, with_defaults};

/// What is left when play stops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoppedPlay {
    /// The recording (`.orrp`, including every debug edit). Open it with
    /// `PlaySession::open_replay` to watch or verify it.
    pub replay: Vec<u8>,
    /// The tick the session was at.
    pub tick: u64,
    /// Checksum of that tick.
    pub checksum: u64,
}

/// A running play session started from an [`EditorDoc`].
///
/// The document is not touched: play works on a copy of the baked frame, so
/// [`stop_play`](Self::stop_play) returns to the scene exactly as it was.
/// Inspector edits go in as [`DebugCommand`]s at a tick boundary. They are
/// recorded in the replay and replay identically, so play stays deterministic
/// and can be scrubbed (seek, step, branch) through
/// [`session_mut`](Self::session_mut) or [`control`](Self::control).
///
/// Entities created during play have no GUID. Address them with
/// [`Target::Entity`].
pub struct PlayController<G: Game> {
    session: PlaySession<G>,
    types: Arc<TypeRegistry>,
    index: SceneIndex,
    allow_play_edits: bool,
}

impl<G: Game> PlayController<G> {
    /// Bakes nothing new: copies the document's preview frame into a
    /// `PlaySession<G>`. Use [`EditorDoc::play_config`] for `cfg`.
    ///
    /// Fails if the document's frame registry is not the layout of `G`.
    pub fn start_play(doc: &EditorDoc, cfg: PlayConfig) -> Result<Self, EditError> {
        let session = PlaySession::<G>::from_frame(cfg, doc.frame())
            .map_err(|e| EditError::PlayStart(e.to_string()))?;
        Ok(Self {
            session,
            types: doc.types().clone(),
            index: doc.index().clone(),
            allow_play_edits: doc.admission.as_ref().is_none_or(|a| a.allow_play_edits()),
        })
    }

    /// Ends play and returns the recording. Drop the controller instead to
    /// discard it. The document was never changed.
    pub fn stop_play(self) -> StoppedPlay {
        let tick = self.session.head_tick();
        StoppedPlay {
            replay: self.session.save_replay(),
            tick,
            checksum: self.session.frame().checksum(),
        }
    }

    /// The session (timeline, frames, checksums, replay).
    pub fn session(&self) -> &PlaySession<G> {
        &self.session
    }
    /// The session, for play/pause/step/seek/branch, inputs and commands.
    pub fn session_mut(&mut self) -> &mut PlaySession<G> {
        &mut self.session
    }
    /// Runs a timeline control (same as `session_mut().control(op)`).
    pub fn control(&mut self, op: ControlOp) -> Vec<SimEvent<G::Event>> {
        self.session.control(op)
    }
    /// The timeline bar state.
    pub fn timeline(&self) -> Timeline {
        self.session.timeline()
    }
    /// Read-only queries on the live frame (the head tick of the session).
    pub fn view(&self) -> View<'_> {
        View {
            types: &self.types,
            frame: self.session.frame(),
            index: &self.index,
        }
    }

    /// Reads the live frame back into a scene document (entities made in
    /// play get generated GUIDs). Use it to keep a state found in play.
    pub fn capture_scene(&self) -> Result<Scene, EditError> {
        if !self.allow_play_edits {
            return Err(EditError::Invalid("This asset-backed host does not support capturing play as an authored scene; save the edit document instead".into()));
        }
        Ok(Scene::unbake(
            &self.types,
            self.session.frame(),
            Some(&self.index),
        )?)
    }

    /// Whether this host's admitted asset contract permits runtime debug edits.
    pub fn allow_play_edits(&self) -> bool { self.allow_play_edits }

    fn check_play_edit(&self) -> Result<(), EditError> {
        if self.allow_play_edits {
            Ok(())
        } else {
            Err(EditError::Invalid(
                "This asset-backed host does not support live play edits; stop play before editing"
                    .into(),
            ))
        }
    }

    // ---- edits ----

    /// Sets one field of a component with a `DebugCommand::SetField` (the
    /// smallest byte span that changes). `Ok(false)` when the value is
    /// already there (nothing is recorded). `value` uses scene form: entity
    /// references are `Value::EntityGuid`.
    pub fn set_field(
        &mut self,
        target: &Target,
        component: &str,
        path: &str,
        value: Value,
    ) -> Result<bool, EditError> {
        self.check_play_edit()?;
        let t = type_of(&self.types, component, TypeKind::Component)?;
        let e = self.view().resolve(target)?;
        let id = component_id(self.session.frame(), component)?;
        let old = self
            .session
            .frame()
            .component_bytes(id, e)
            .ok_or_else(|| EditError::NoComponent {
                entity: target.describe(),
                component: component.into(),
            })?
            .to_vec();
        let mut new = old.clone();
        t.set(&mut new, path, self.to_frame_form(&value)?)?;
        let Some((offset, bytes)) = diff_span(&old, &new) else {
            return Ok(false);
        };
        self.session.debug(DebugCommand::SetField {
            entity: e,
            component: id,
            offset,
            bytes,
        })?;
        Ok(true)
    }

    /// Sets one field of a singleton (`DebugCommand::SetSingletonField`).
    pub fn set_singleton_field(
        &mut self,
        singleton: &str,
        path: &str,
        value: Value,
    ) -> Result<bool, EditError> {
        self.check_play_edit()?;
        let t = type_of(&self.types, singleton, TypeKind::Singleton)?;
        let id = singleton_id(self.session.frame(), singleton)?;
        let old = self
            .session
            .frame()
            .singleton_bytes(id)
            .ok_or_else(|| EditError::UnknownType(singleton.to_string()))?
            .to_vec();
        let mut new = old.clone();
        t.set(&mut new, path, self.to_frame_form(&value)?)?;
        let Some((offset, bytes)) = diff_span(&old, &new) else {
            return Ok(false);
        };
        self.session.debug(DebugCommand::SetSingletonField {
            singleton: id,
            offset,
            bytes,
        })?;
        Ok(true)
    }

    /// Adds a component (`None` = default value). Fails if the entity has it.
    pub fn add_component(
        &mut self,
        target: &Target,
        component: &str,
        value: Option<Value>,
    ) -> Result<(), EditError> {
        self.check_play_edit()?;
        let t = type_of(&self.types, component, TypeKind::Component)?;
        let e = self.view().resolve(target)?;
        let id = component_id(self.session.frame(), component)?;
        if self.session.frame().component_bytes(id, e).is_some() {
            return Err(EditError::HasComponent {
                entity: target.describe(),
                component: component.into(),
            });
        }
        let bytes = self.component_bytes(t, value.as_ref())?;
        self.session.debug(DebugCommand::AddComponent {
            entity: e,
            component: id,
            bytes,
        })?;
        Ok(())
    }

    /// Removes a component.
    pub fn remove_component(&mut self, target: &Target, component: &str) -> Result<(), EditError> {
        self.check_play_edit()?;
        type_of(&self.types, component, TypeKind::Component)?;
        let e = self.view().resolve(target)?;
        let id = component_id(self.session.frame(), component)?;
        self.session.debug(DebugCommand::RemoveComponent {
            entity: e,
            component: id,
        })?;
        Ok(())
    }

    /// Spawns an entity with these components. Returns its frame handle
    /// (it has no GUID).
    pub fn spawn(&mut self, components: &[(String, Value)]) -> Result<Entity, EditError> {
        self.check_play_edit()?;
        let mut list = Vec::new();
        for (name, value) in components {
            let t = type_of(&self.types, name, TypeKind::Component)?;
            list.push((
                component_id(self.session.frame(), name)?,
                self.component_bytes(t, Some(value))?,
            ));
        }
        let before: BTreeSet<Entity> = self.session.frame().entities().collect();
        self.session
            .debug(DebugCommand::Spawn { components: list })?;
        self.session
            .frame()
            .entities()
            .find(|e| !before.contains(e))
            .ok_or_else(|| EditError::Invalid("spawn did not create an entity".into()))
    }

    /// Despawns an entity.
    pub fn despawn(&mut self, target: &Target) -> Result<(), EditError> {
        self.check_play_edit()?;
        let e = self.view().resolve(target)?;
        self.session.debug(DebugCommand::Despawn { entity: e })?;
        Ok(())
    }

    // ---- helpers ----

    /// Scene form (GUID references) to frame form (`Entity`, checked alive).
    fn to_frame_form(&self, v: &Value) -> Result<Value, EditError> {
        let view = self.view();
        map_refs(v, &mut |r| match r {
            Value::EntityGuid(None) => Ok(Value::Entity(Entity::NONE)),
            Value::EntityGuid(Some(g)) => orr_reflect::Guid::parse(g)
                .ok()
                .and_then(|g| view.entity_of(&g))
                .map(Value::Entity)
                .ok_or_else(|| EditError::UnknownEntity(g.clone())),
            other => Ok(other.clone()),
        })
    }

    /// Whole component bytes from a value (or the default).
    fn component_bytes(&self, t: &TypeInfo, value: Option<&Value>) -> Result<Vec<u8>, EditError> {
        let mut bytes = t.default_bytes();
        if let Some(v) = value {
            t.desc()
                .write(&mut bytes, &self.to_frame_form(&with_defaults(t, v))?)?;
        }
        Ok(bytes)
    }
}

/// The smallest byte range that differs, as `(offset, new bytes)`.
fn diff_span(old: &[u8], new: &[u8]) -> Option<(u32, Vec<u8>)> {
    let first = old.iter().zip(new).position(|(a, b)| a != b)?;
    let last = old.iter().zip(new).rposition(|(a, b)| a != b)?;
    Some((first as u32, new[first..=last].to_vec()))
}

/// The frame's id of a component type, by its registered name.
fn component_id(frame: &Frame, name: &str) -> Result<ComponentId, EditError> {
    let reg = frame.registry();
    (0..reg.component_count())
        .map(ComponentId)
        .find(|&id| reg.component_name(id) == name)
        .ok_or_else(|| EditError::UnknownType(name.to_string()))
}

fn singleton_id(frame: &Frame, name: &str) -> Result<SingletonId, EditError> {
    let reg = frame.registry();
    (0..reg.singleton_count())
        .map(SingletonId)
        .find(|&id| reg.singleton_name(id) == name)
        .ok_or_else(|| EditError::UnknownType(name.to_string()))
}
