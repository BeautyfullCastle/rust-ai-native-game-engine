//! Debug commands: byte-level edits of the simulation state that an editor
//! sends while a game runs.
//!
//! A [`DebugCommand`] is applied at a tick boundary, before the systems of
//! the next tick run (see [`crate::Simulation::step_with_debug`]). It is
//! part of the deterministic input stream: a recording stores it next to
//! the inputs of that tick, and a replay applies it at the same place.
//!
//! Every command is checked before it changes anything. A command that
//! fails the check returns a [`DebugError`], leaves the frame as it was,
//! and never panics, whatever the bytes are.
use orr_ecs::{ComponentId, Entity, Frame, SingletonId};

/// One byte-level edit of the simulation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugCommand {
    /// Overwrites `bytes.len()` bytes of a component value, starting at
    /// `offset` bytes from the start of the value.
    SetField { entity: Entity, component: ComponentId, offset: u32, bytes: Vec<u8> },
    /// Like `SetField`, for a singleton value.
    SetSingletonField { singleton: SingletonId, offset: u32, bytes: Vec<u8> },
    /// Spawns a new entity with these whole component values (each `bytes`
    /// is exactly the component size). Each component may appear once.
    Spawn { components: Vec<(ComponentId, Vec<u8>)> },
    Despawn { entity: Entity },
    /// Adds a component to an entity, or replaces its whole value.
    AddComponent { entity: Entity, component: ComponentId, bytes: Vec<u8> },
    RemoveComponent { entity: Entity, component: ComponentId },
}

/// Why a [`DebugCommand`] was refused. Nothing was changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugError {
    /// The entity does not exist (never spawned, or a stale handle).
    EntityNotAlive,
    /// No component type with this id is registered.
    UnknownComponent,
    /// No singleton type with this id is registered.
    UnknownSingleton,
    /// The entity does not have this component.
    MissingComponent,
    /// `offset + len` goes past the end of the value, or `len` is zero.
    OutOfBounds,
    /// The bytes are not exactly the size of the component.
    SizeMismatch,
    /// `Spawn` lists the same component twice.
    DuplicateComponent,
    /// The session cannot take edits now (a read-only replay viewer).
    ReadOnly,
    /// The host has no debug command support.
    Unsupported,
}

impl core::fmt::Display for DebugError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            DebugError::EntityNotAlive => "entity is not alive",
            DebugError::UnknownComponent => "component type is not registered",
            DebugError::UnknownSingleton => "singleton type is not registered",
            DebugError::MissingComponent => "entity does not have this component",
            DebugError::OutOfBounds => "byte range is outside the value",
            DebugError::SizeMismatch => "bytes do not match the component size",
            DebugError::DuplicateComponent => "component listed twice",
            DebugError::ReadOnly => "the session is read-only (branch first)",
            DebugError::Unsupported => "this host does not support debug commands",
        })
    }
}
impl std::error::Error for DebugError {}

const TAG_SET_FIELD: u8 = 1;
const TAG_SET_SINGLETON_FIELD: u8 = 2;
const TAG_SPAWN: u8 = 3;
const TAG_DESPAWN: u8 = 4;
const TAG_ADD: u8 = 5;
const TAG_REMOVE: u8 = 6;

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_entity(out: &mut Vec<u8>, e: Entity) {
    put_u32(out, e.index);
    put_u32(out, e.version);
}
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn entity(&mut self) -> Option<Entity> {
        Some(Entity { index: self.u32()?, version: self.u32()? })
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        Some(self.take(n)?.to_vec())
    }
}

impl DebugCommand {
    /// Appends the wire form (tag byte, little-endian fields, `u32`
    /// lengths) to `out`. Used by the replay file.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            DebugCommand::SetField { entity, component, offset, bytes } => {
                out.push(TAG_SET_FIELD);
                put_entity(out, *entity);
                put_u16(out, component.0);
                put_u32(out, *offset);
                put_bytes(out, bytes);
            }
            DebugCommand::SetSingletonField { singleton, offset, bytes } => {
                out.push(TAG_SET_SINGLETON_FIELD);
                put_u16(out, singleton.0);
                put_u32(out, *offset);
                put_bytes(out, bytes);
            }
            DebugCommand::Spawn { components } => {
                out.push(TAG_SPAWN);
                put_u32(out, components.len() as u32);
                for (id, bytes) in components {
                    put_u16(out, id.0);
                    put_bytes(out, bytes);
                }
            }
            DebugCommand::Despawn { entity } => {
                out.push(TAG_DESPAWN);
                put_entity(out, *entity);
            }
            DebugCommand::AddComponent { entity, component, bytes } => {
                out.push(TAG_ADD);
                put_entity(out, *entity);
                put_u16(out, component.0);
                put_bytes(out, bytes);
            }
            DebugCommand::RemoveComponent { entity, component } => {
                out.push(TAG_REMOVE);
                put_entity(out, *entity);
                put_u16(out, component.0);
            }
        }
    }

    /// Parses what [`DebugCommand::encode`] wrote. `None` on any malformed
    /// or trailing input; never panics and never allocates more than the
    /// input length implies.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut c = Cursor(bytes);
        let cmd = match c.u8()? {
            TAG_SET_FIELD => DebugCommand::SetField {
                entity: c.entity()?,
                component: ComponentId(c.u16()?),
                offset: c.u32()?,
                bytes: c.bytes()?,
            },
            TAG_SET_SINGLETON_FIELD => {
                DebugCommand::SetSingletonField { singleton: SingletonId(c.u16()?), offset: c.u32()?, bytes: c.bytes()? }
            }
            TAG_SPAWN => {
                let n = c.u32()? as usize;
                // Each entry needs at least 6 bytes, so the input bounds the allocation.
                let mut components = Vec::with_capacity(n.min(c.0.len() / 6));
                for _ in 0..n {
                    components.push((ComponentId(c.u16()?), c.bytes()?));
                }
                DebugCommand::Spawn { components }
            }
            TAG_DESPAWN => DebugCommand::Despawn { entity: c.entity()? },
            TAG_ADD => DebugCommand::AddComponent { entity: c.entity()?, component: ComponentId(c.u16()?), bytes: c.bytes()? },
            TAG_REMOVE => DebugCommand::RemoveComponent { entity: c.entity()?, component: ComponentId(c.u16()?) },
            _ => return None,
        };
        if !c.0.is_empty() {
            return None;
        }
        Some(cmd)
    }

    /// Checks the command against `frame` and applies it. Returns the new
    /// entity for `Spawn`, `None` otherwise. On `Err` the frame is unchanged.
    pub fn apply(&self, frame: &mut Frame) -> Result<Option<Entity>, DebugError> {
        let registry = frame.registry().clone();
        let component_size = |id: ComponentId| -> Result<usize, DebugError> {
            if id.0 < registry.component_count() {
                Ok(registry.component_size(id) as usize)
            } else {
                Err(DebugError::UnknownComponent)
            }
        };
        match self {
            DebugCommand::SetField { entity, component, offset, bytes } => {
                let size = component_size(*component)?;
                check_range(size, *offset, bytes.len())?;
                if !frame.exists(*entity) {
                    return Err(DebugError::EntityNotAlive);
                }
                let value = frame.component_bytes_mut(*component, *entity).ok_or(DebugError::MissingComponent)?;
                let start = *offset as usize;
                value[start..start + bytes.len()].copy_from_slice(bytes);
                Ok(None)
            }
            DebugCommand::SetSingletonField { singleton, offset, bytes } => {
                if singleton.0 >= registry.singleton_count() {
                    return Err(DebugError::UnknownSingleton);
                }
                check_range(registry.singleton_size(*singleton) as usize, *offset, bytes.len())?;
                let value = frame.singleton_bytes_mut(*singleton).ok_or(DebugError::UnknownSingleton)?;
                let start = *offset as usize;
                value[start..start + bytes.len()].copy_from_slice(bytes);
                Ok(None)
            }
            DebugCommand::Spawn { components } => {
                for (i, (id, bytes)) in components.iter().enumerate() {
                    if component_size(*id)? != bytes.len() {
                        return Err(DebugError::SizeMismatch);
                    }
                    if components[..i].iter().any(|(other, _)| other == id) {
                        return Err(DebugError::DuplicateComponent);
                    }
                }
                let e = frame.spawn();
                for (id, bytes) in components {
                    let inserted = frame.insert_component_bytes(*id, e, bytes);
                    debug_assert!(inserted, "validated above");
                }
                Ok(Some(e))
            }
            DebugCommand::Despawn { entity } => {
                if frame.despawn(*entity) {
                    Ok(None)
                } else {
                    Err(DebugError::EntityNotAlive)
                }
            }
            DebugCommand::AddComponent { entity, component, bytes } => {
                if component_size(*component)? != bytes.len() {
                    return Err(DebugError::SizeMismatch);
                }
                if !frame.exists(*entity) {
                    return Err(DebugError::EntityNotAlive);
                }
                if frame.insert_component_bytes(*component, *entity, bytes) {
                    Ok(None)
                } else {
                    Err(DebugError::SizeMismatch)
                }
            }
            DebugCommand::RemoveComponent { entity, component } => {
                component_size(*component)?;
                if !frame.exists(*entity) {
                    return Err(DebugError::EntityNotAlive);
                }
                if frame.remove_component_by_id(*component, *entity) {
                    Ok(None)
                } else {
                    Err(DebugError::MissingComponent)
                }
            }
        }
    }
}

fn check_range(size: usize, offset: u32, len: usize) -> Result<(), DebugError> {
    let end = (offset as usize).checked_add(len).ok_or(DebugError::OutOfBounds)?;
    if len == 0 || end > size {
        return Err(DebugError::OutOfBounds);
    }
    Ok(())
}
