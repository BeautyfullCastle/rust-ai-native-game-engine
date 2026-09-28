use crate::component::Component;
use crate::entity::Entity;
use crate::frame::Frame;

/// A [`Commands`]-produced handle to an entity that will be spawned when the
/// buffer is applied. Not yet a real [`Entity`] — resolved by
/// [`Commands::apply`] to whatever slot the deferred `spawn` actually lands
/// on, so it stays valid even if other entities are spawned/despawned in
/// between recording and applying.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingEntity(u32);

/// Either an already-real `Entity` or a `Commands`-reserved [`PendingEntity`]
/// not yet resolved.
#[derive(Clone, Copy, Debug)]
pub enum EntityRef {
    Real(Entity),
    Pending(PendingEntity),
}

impl From<Entity> for EntityRef {
    fn from(e: Entity) -> Self {
        EntityRef::Real(e)
    }
}
impl From<PendingEntity> for EntityRef {
    fn from(p: PendingEntity) -> Self {
        EntityRef::Pending(p)
    }
}

type MutateFn = Box<dyn FnOnce(&mut Frame, Entity) + Send>;

enum Op {
    Spawn(u32),
    Despawn(EntityRef),
    Mutate(EntityRef, MutateFn),
}

/// A deferred buffer of structural operations (spawn/despawn/add/remove),
/// recorded now and applied later via [`Commands::apply`]. This lets code
/// that is iterating a [`Query`](crate::QueryIter) over a `Frame` (which
/// holds `&mut Frame`, so no other structural mutation of the frame is
/// possible at the same time) record the changes it wants and apply them
/// afterward, in the order they were recorded.
#[derive(Default)]
pub struct Commands {
    ops: Vec<Op>,
    next_pending: u32,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self) -> PendingEntity {
        let id = self.next_pending;
        self.next_pending += 1;
        self.ops.push(Op::Spawn(id));
        PendingEntity(id)
    }

    pub fn despawn(&mut self, e: impl Into<EntityRef>) {
        self.ops.push(Op::Despawn(e.into()));
    }

    pub fn add<T: Component>(&mut self, e: impl Into<EntityRef>, v: T) {
        let e = e.into();
        self.ops.push(Op::Mutate(
            e,
            Box::new(move |frame, real| {
                frame.add::<T>(real, v);
            }),
        ));
    }

    pub fn remove<T: Component>(&mut self, e: impl Into<EntityRef>) {
        let e = e.into();
        self.ops.push(Op::Mutate(
            e,
            Box::new(move |frame, real| {
                frame.remove::<T>(real);
            }),
        ));
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Applies every recorded op to `frame`, in recorded order, then clears
    /// the buffer for reuse. `Spawn` ops are resolved to real entities as
    /// they run, so any op recorded after a `spawn()` that references its
    /// `PendingEntity` (or a nested one) resolves correctly.
    pub fn apply(&mut self, frame: &mut Frame) {
        let mut resolved: Vec<Option<Entity>> = vec![None; self.next_pending as usize];
        for op in self.ops.drain(..) {
            match op {
                Op::Spawn(id) => {
                    let e = frame.spawn();
                    resolved[id as usize] = Some(e);
                }
                Op::Despawn(er) => {
                    if let Some(e) = resolve(er, &resolved) {
                        frame.despawn(e);
                    }
                }
                Op::Mutate(er, f) => {
                    if let Some(e) = resolve(er, &resolved) {
                        f(frame, e);
                    }
                }
            }
        }
        self.next_pending = 0;
    }
}

fn resolve(er: EntityRef, resolved: &[Option<Entity>]) -> Option<Entity> {
    match er {
        EntityRef::Real(e) => Some(e),
        EntityRef::Pending(p) => resolved[p.0 as usize],
    }
}
