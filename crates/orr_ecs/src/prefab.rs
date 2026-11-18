//! Prefabs: an entity that a system can create from a recipe.
//!
//! A [`Prefab`] is a list of components as raw bytes. It is baked before the simulation starts
//! (from a scene fragment, see `orr_reflect::Scene::bake_prefab`, or in code with
//! [`Prefab::with`]). [`install`] copies the prefabs into the frame, in [`Game::setup`]-time
//! order, so they are part of every snapshot, checksum and join snapshot. A system then calls
//! [`spawn_prefab`] with a [`PrefabId`]. The spawn reads only the frame: no file, no clock, no
//! hash map. Two peers that start from the same prefabs and replay the same ticks get the same
//! entities.
//!
//! A game opts in with [`register`] in `Game::register` and [`install`] in `Game::setup`.

use bytemuck::{Pod, Zeroable};

use crate::{Component, ComponentId, ComponentRegistry, ComponentRegistryBuilder, Entity, Frame, FrameList};

/// Bytes of component data in one [`PrefabPart`]. A bigger component takes several parts.
pub const PREFAB_PART_BYTES: usize = 128;

/// The name of [`PrefabPart`] in the component registry.
pub const PREFAB_PART_NAME: &str = "orr_ecs::PrefabPart";

/// The name of [`PrefabLibrary`] in the component registry.
pub const PREFAB_LIBRARY_NAME: &str = "orr_ecs::PrefabLibrary";

/// A piece of one component of one prefab, as stored in the frame. The parts of a prefab are
/// stored in order, and the parts of one component follow each other.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct PrefabPart {
    /// The prefab this part belongs to (its [`PrefabId`]).
    pub prefab: u32,
    /// The [`ComponentId`] of the component.
    pub component: u16,
    /// How many of the `bytes` are used.
    pub len: u16,
    /// Component data, zero padded.
    pub bytes: [u8; PREFAB_PART_BYTES],
}

/// All prefabs of a frame: a singleton that holds their parts.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct PrefabLibrary {
    /// The parts of all prefabs, ordered by prefab.
    pub parts: FrameList<PrefabPart>,
    /// How many prefabs are installed.
    pub count: u32,
    /// Keeps the layout without padding.
    pub reserved: u32,
}

/// The number of a prefab: its place in the list given to [`install`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PrefabId(pub u32);

/// Why a prefab was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PrefabError {
    /// The component is not registered in this frame, or its type has another size.
    UnknownComponent(String),
    /// The bytes do not have the size of the component.
    WrongSize {
        /// Component name.
        component: &'static str,
        /// Size of the component.
        expected: u32,
        /// Size of the bytes.
        got: usize,
    },
    /// The frame has no [`PrefabLibrary`]: the game did not call [`register`].
    NotRegistered,
    /// The frame already has prefabs.
    AlreadyInstalled,
}

impl core::fmt::Display for PrefabError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownComponent(name) => write!(f, "prefab component {name} is not registered in this game"),
            Self::WrongSize { component, expected, got } => write!(f, "prefab component {component} has {got} bytes, expected {expected}"),
            Self::NotRegistered => f.write_str("the game did not register prefabs (orr_ecs::prefab::register)"),
            Self::AlreadyInstalled => f.write_str("prefabs are already installed in this frame"),
        }
    }
}

impl std::error::Error for PrefabError {}

/// A recipe for one entity: component bytes, in the order they are added.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Prefab {
    components: Vec<(ComponentId, Vec<u8>)>,
}

impl Prefab {
    /// A prefab without components.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds component `value` (replaces an earlier value of the same component type).
    /// `registry` is the registry of the game's frame.
    pub fn with<T: Component>(self, registry: &ComponentRegistry, value: T) -> Result<Self, PrefabError> {
        let id = registry.component_id::<T>().ok_or_else(|| PrefabError::UnknownComponent(core::any::type_name::<T>().to_string()))?;
        self.with_bytes(registry, id, bytemuck::bytes_of(&value))
    }

    /// Adds the component `id` from raw bytes of the component's size.
    pub fn with_bytes(mut self, registry: &ComponentRegistry, id: ComponentId, bytes: &[u8]) -> Result<Self, PrefabError> {
        if id.0 >= registry.component_count() {
            return Err(PrefabError::UnknownComponent(format!("#{}", id.0)));
        }
        let expected = registry.component_size(id);
        if bytes.len() != expected as usize {
            return Err(PrefabError::WrongSize { component: registry.component_name(id), expected, got: bytes.len() });
        }
        self.components.retain(|(c, _)| *c != id);
        self.components.push((id, bytes.to_vec()));
        Ok(self)
    }

    /// The components with their bytes.
    pub fn components(&self) -> &[(ComponentId, Vec<u8>)] {
        &self.components
    }
}

/// Registers the prefab singleton and its list. Call it from `Game::register`.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_list::<PrefabPart>(PREFAB_PART_NAME);
    builder.register_singleton::<PrefabLibrary>(PREFAB_LIBRARY_NAME);
}

/// The id of the component named `name` in `registry`, for baking a prefab from a scene.
pub fn component_id_by_name(registry: &ComponentRegistry, name: &str) -> Option<ComponentId> {
    (0..registry.component_count()).map(ComponentId).find(|id| registry.component_name(*id) == name)
}

/// Copies `prefabs` into the frame. Call it once, from `Game::setup`. The prefab at index `i` of
/// the slice gets `PrefabId(i)`.
pub fn install(frame: &mut Frame, prefabs: &[Prefab]) -> Result<(), PrefabError> {
    if frame.registry().singleton_id::<PrefabLibrary>().is_none() || frame.registry().list_id::<PrefabPart>().is_none() {
        return Err(PrefabError::NotRegistered);
    }
    if frame.singleton::<PrefabLibrary>().count != 0 {
        return Err(PrefabError::AlreadyInstalled);
    }
    let list = frame.alloc_list::<PrefabPart>();
    for (number, prefab) in prefabs.iter().enumerate() {
        for (id, bytes) in prefab.components() {
            let mut chunks: Vec<&[u8]> = bytes.chunks(PREFAB_PART_BYTES).collect();
            if chunks.is_empty() {
                // A component without data (a marker) still needs one part.
                chunks.push(&[]);
            }
            for chunk in chunks {
                let mut part = PrefabPart { prefab: number as u32, component: id.0, len: chunk.len() as u16, bytes: [0; PREFAB_PART_BYTES] };
                part.bytes[..chunk.len()].copy_from_slice(chunk);
                frame.list_push(list, part);
            }
        }
    }
    *frame.singleton_mut::<PrefabLibrary>() = PrefabLibrary { parts: list, count: prefabs.len() as u32, reserved: 0 };
    Ok(())
}

/// How many prefabs the frame has, 0 when the game has none.
pub fn prefab_count(frame: &Frame) -> u32 {
    if frame.registry().singleton_id::<PrefabLibrary>().is_none() {
        return 0;
    }
    frame.singleton::<PrefabLibrary>().count
}

/// Creates an entity with the components of prefab `id`. Returns `None`, and changes nothing,
/// when the frame has no such prefab. Call it from a system.
pub fn spawn_prefab(frame: &mut Frame, id: PrefabId) -> Option<Entity> {
    if id.0 >= prefab_count(frame) {
        return None;
    }
    let list = frame.singleton::<PrefabLibrary>().parts;
    // Join the parts of each component first, because the frame is borrowed while it is read.
    let mut components: Vec<(u16, Vec<u8>)> = Vec::new();
    for part in frame.list(list).iter().filter(|part| part.prefab == id.0) {
        match components.last_mut() {
            Some((component, bytes)) if *component == part.component => bytes.extend_from_slice(&part.bytes[..usize::from(part.len)]),
            _ => components.push((part.component, part.bytes[..usize::from(part.len)].to_vec())),
        }
    }
    let entity = frame.spawn();
    for (component, bytes) in components {
        frame.insert_component_bytes(ComponentId(component), entity, &bytes);
    }
    Some(entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
    struct Spot {
        x: i64,
        y: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
    struct Big {
        data: [u8; 256],
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
    struct Tag {
        value: u8,
    }

    fn registry() -> std::sync::Arc<ComponentRegistry> {
        let mut builder = ComponentRegistryBuilder::new();
        register(&mut builder);
        builder.register_component::<Spot>("Spot");
        builder.register_component::<Big>("Big");
        builder.register_component::<Tag>("Tag");
        builder.build()
    }

    fn big() -> Big {
        let mut data = [0; 256];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        Big { data }
    }

    fn prefabs(registry: &ComponentRegistry) -> Vec<Prefab> {
        vec![
            Prefab::new().with(registry, Spot { x: 3, y: -4 }).unwrap().with(registry, Tag { value: 9 }).unwrap(),
            Prefab::new().with(registry, big()).unwrap(),
        ]
    }

    #[test]
    fn a_spawned_entity_has_the_components_of_its_prefab() {
        let registry = registry();
        let mut frame = Frame::new(registry.clone());
        install(&mut frame, &prefabs(&registry)).unwrap();

        let a = spawn_prefab(&mut frame, PrefabId(0)).unwrap();
        let b = spawn_prefab(&mut frame, PrefabId(1)).unwrap();

        assert_eq!(frame.get::<Spot>(a), Some(&Spot { x: 3, y: -4 }));
        assert_eq!(frame.get::<Tag>(a), Some(&Tag { value: 9 }));
        assert!(!frame.has::<Big>(a));
        assert_eq!(frame.get::<Big>(b), Some(&big()), "a component bigger than one part is joined again");
    }

    #[test]
    fn an_unknown_prefab_spawns_nothing() {
        let registry = registry();
        let mut frame = Frame::new(registry.clone());
        install(&mut frame, &prefabs(&registry)).unwrap();
        let before = frame.checksum();

        assert_eq!(spawn_prefab(&mut frame, PrefabId(2)), None);

        assert_eq!(frame.checksum(), before);
    }

    #[test]
    fn two_frames_with_the_same_prefabs_spawn_the_same_state_and_the_snapshot_keeps_them() {
        let registry = registry();
        let make = || {
            let mut frame = Frame::new(registry.clone());
            install(&mut frame, &prefabs(&registry)).unwrap();
            frame
        };
        let (mut one, mut two) = (make(), make());
        for frame in [&mut one, &mut two] {
            spawn_prefab(frame, PrefabId(1));
            spawn_prefab(frame, PrefabId(0));
            spawn_prefab(frame, PrefabId(0));
        }
        assert_eq!(one.checksum(), two.checksum());

        // A frame rebuilt from bytes can spawn too: the prefabs are part of the state.
        let mut restored = Frame::from_bytes(registry.clone(), &one.to_bytes()).unwrap();
        assert_eq!(restored.checksum(), one.checksum());
        let a = spawn_prefab(&mut restored, PrefabId(0)).unwrap();
        let b = spawn_prefab(&mut one, PrefabId(0)).unwrap();
        assert_eq!(a, b);
        assert_eq!(restored.checksum(), one.checksum());
    }

    #[test]
    fn a_prefab_with_a_wrong_size_or_an_unknown_component_is_refused() {
        let registry = registry();
        let spot = component_id_by_name(&registry, "Spot").unwrap();
        assert!(matches!(Prefab::new().with_bytes(&registry, spot, &[1, 2, 3]), Err(PrefabError::WrongSize { expected: 16, got: 3, .. })));
        assert!(matches!(Prefab::new().with_bytes(&registry, ComponentId(99), &[]), Err(PrefabError::UnknownComponent(_))));
        assert_eq!(component_id_by_name(&registry, "Missing"), None);
    }

    #[test]
    fn install_needs_registration_and_runs_once() {
        let mut bare = ComponentRegistryBuilder::new();
        bare.register_component::<Spot>("Spot");
        let mut frame = Frame::new(bare.build());
        assert_eq!(install(&mut frame, &[]), Err(PrefabError::NotRegistered));
        assert_eq!(prefab_count(&frame), 0);
        assert_eq!(spawn_prefab(&mut frame, PrefabId(0)), None);

        let registry = registry();
        let mut frame = Frame::new(registry.clone());
        install(&mut frame, &prefabs(&registry)).unwrap();
        assert_eq!(install(&mut frame, &prefabs(&registry)), Err(PrefabError::AlreadyInstalled));
    }
}
