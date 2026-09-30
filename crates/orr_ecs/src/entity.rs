use bytemuck::{Pod, Zeroable};

use crate::codec::{put_pods, put_u32, FrameDecodeError, Reader};

/// A handle to a simulation entity.
///
/// `index` names a slot in the [`Frame`](crate::Frame)'s entity table; `version`
/// distinguishes successive entities that have occupied that slot, so a stale
/// handle to a despawned-and-respawned slot compares unequal to the new one.
/// `Entity` is `Pod` so it can be embedded directly in other components and
/// hashed byte-for-byte as part of a checksum.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Pod, Zeroable)]
pub struct Entity {
    pub index: u32,
    pub version: u32,
}

impl Entity {
    /// A sentinel that never compares equal to a real, allocated entity
    /// (index `u32::MAX` is never handed out by the allocator).
    pub const NONE: Entity = Entity { index: u32::MAX, version: 0 };

    #[inline]
    pub fn is_none(self) -> bool {
        self.index == u32::MAX
    }
}

impl Default for Entity {
    fn default() -> Self {
        Entity::NONE
    }
}

/// Deterministic entity allocator.
///
/// Freed slots are recycled from the back of `free` (LIFO), so a scripted
/// sequence of spawn/despawn calls always reproduces the same index/version
/// assignments, which is required for checksum determinism across platforms.
#[derive(Clone, Debug, Default)]
pub struct EntityAllocator {
    /// Current version of each slot, indexed by `Entity::index`.
    versions: Vec<u32>,
    /// 1 if the slot at this index currently holds a live entity, else 0.
    alive: Vec<u8>,
    /// Free slot indices available for reuse, LIFO order.
    free: Vec<u32>,
    alive_count: u32,
}

impl EntityAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alive_count(&self) -> u32 {
        self.alive_count
    }

    /// Highest number of slots ever allocated (i.e. the exclusive upper bound
    /// on `Entity::index` values ever handed out).
    pub fn slot_count(&self) -> u32 {
        self.versions.len() as u32
    }

    pub fn spawn(&mut self) -> Entity {
        if let Some(idx) = self.free.pop() {
            self.alive[idx as usize] = 1;
            self.alive_count += 1;
            Entity { index: idx, version: self.versions[idx as usize] }
        } else {
            let idx = self.versions.len() as u32;
            self.versions.push(0);
            self.alive.push(1);
            self.alive_count += 1;
            Entity { index: idx, version: 0 }
        }
    }

    /// Frees `e`'s slot if `e` is currently live. Returns whether it was.
    pub fn despawn(&mut self, e: Entity) -> bool {
        let idx = e.index as usize;
        if idx >= self.versions.len() {
            return false;
        }
        if self.alive[idx] == 0 || self.versions[idx] != e.version {
            return false;
        }
        self.alive[idx] = 0;
        self.versions[idx] = self.versions[idx].wrapping_add(1);
        self.free.push(e.index);
        self.alive_count -= 1;
        true
    }

    pub fn exists(&self, e: Entity) -> bool {
        let idx = e.index as usize;
        idx < self.versions.len() && self.alive[idx] != 0 && self.versions[idx] == e.version
    }

    /// Every live entity, in ascending index order.
    pub fn iter_alive(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive
            .iter()
            .enumerate()
            .filter(|(_, a)| **a != 0)
            .map(|(i, _)| Entity { index: i as u32, version: self.versions[i] })
    }

    pub fn copy_from(&mut self, other: &EntityAllocator) {
        self.versions.clone_from(&other.versions);
        self.alive.clone_from(&other.alive);
        self.free.clone_from(&other.free);
        self.alive_count = other.alive_count;
    }

    pub fn hash_into(&self, h: &mut xxhash_rust::xxh3::Xxh3) {
        h.update(&self.alive_count.to_le_bytes());
        h.update(bytemuck::cast_slice(&self.versions));
        h.update(&self.alive);
        h.update(bytemuck::cast_slice(&self.free));
    }

    /// Layout: `alive_count u32`, `slot_count u32`, versions, alive flags,
    /// then `free` as a counted `u32` array.
    pub(crate) fn write_bytes(&self, out: &mut Vec<u8>) {
        put_u32(out, self.alive_count);
        put_pods(out, &self.versions);
        out.extend_from_slice(&self.alive);
        put_pods(out, &self.free);
    }

    /// Reads what [`write_bytes`](Self::write_bytes) wrote and checks the
    /// invariants `spawn`/`despawn` maintain, so a corrupt stream cannot
    /// yield an allocator that later panics or hands out a live index twice.
    pub(crate) fn read_bytes(r: &mut Reader) -> Result<Self, FrameDecodeError> {
        let alive_count = r.u32()?;
        let versions: Vec<u32> = r.counted_pods()?;
        let slots = versions.len() as u32;
        let alive: Vec<u8> = r.pods(slots)?;
        let free: Vec<u32> = r.counted_pods()?;

        if alive.iter().any(|&a| a > 1) {
            return Err(FrameDecodeError::Corrupt("entity alive flag is not 0 or 1"));
        }
        let live = alive.iter().filter(|&&a| a == 1).count();
        if live != alive_count as usize {
            return Err(FrameDecodeError::Corrupt("entity alive_count disagrees with alive flags"));
        }
        if free.len() != alive.len() - live {
            return Err(FrameDecodeError::Corrupt("entity free list does not cover every dead slot"));
        }
        let mut in_free = vec![false; alive.len()];
        for &idx in &free {
            match in_free.get_mut(idx as usize) {
                Some(seen) if !*seen && alive[idx as usize] == 0 => *seen = true,
                _ => return Err(FrameDecodeError::Corrupt("entity free list has a live, duplicate or unknown slot")),
            }
        }
        Ok(Self { versions, alive, free, alive_count })
    }
}
