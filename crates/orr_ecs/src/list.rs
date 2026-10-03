use core::any::Any;
use core::marker::PhantomData;

use bytemuck::{Pod, Zeroable};
use xxhash_rust::xxh3::Xxh3;

use crate::codec::{hash_len, put_len, put_pods, put_u32, FrameDecodeError, Reader};
use crate::component::Component;

/// A `Pod` handle (index + version, like [`Entity`](crate::Entity)) into a
/// per-type list pool owned by the [`Frame`](crate::Frame). Lets a component
/// reference variable-length data without holding a heap pointer, so it
/// stays `Pod`/checksummable/snapshot-safe.
#[repr(C)]
pub struct FrameList<T: 'static> {
    pub index: u32,
    pub version: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> FrameList<T> {
    pub const NONE: FrameList<T> = FrameList { index: u32::MAX, version: 0, _marker: PhantomData };

    pub fn is_none(&self) -> bool {
        self.index == u32::MAX
    }
}

impl<T: 'static> Clone for FrameList<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for FrameList<T> {}
impl<T: 'static> PartialEq for FrameList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.version == other.version
    }
}
impl<T: 'static> Eq for FrameList<T> {}
impl<T: 'static> core::fmt::Debug for FrameList<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameList").field("index", &self.index).field("version", &self.version).finish()
    }
}

// SAFETY: layout is exactly `{u32, u32, PhantomData}`, a `#[repr(C)]` struct
// whose only real fields are two `u32`s with no padding; `PhantomData` never
// occupies memory. This is a plain handle, not real ownership of `T`, so it
// is `Pod`/`Zeroable` regardless of what `T` is (the bound merely restricts
// which element types the API is usable with).
unsafe impl<T: 'static> Zeroable for FrameList<T> {}
unsafe impl<T: 'static> Pod for FrameList<T> {}

struct Slot<T> {
    version: u32,
    alive: bool,
    items: Vec<T>,
}

/// Pool of variable-length lists for one registered element type. Slots are
/// recycled from `free` (LIFO) on `free_list`, incrementing `version` so a
/// stale [`FrameList`] handle is detected rather than silently aliasing a
/// reused slot.
pub struct ListPool<T: Component> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T: Component> Default for ListPool<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new() }
    }
}

impl<T: Component> ListPool<T> {
    pub fn alloc(&mut self) -> FrameList<T> {
        if let Some(idx) = self.free.pop() {
            let slot = &mut self.slots[idx as usize];
            slot.alive = true;
            slot.items.clear();
            FrameList { index: idx, version: slot.version, _marker: PhantomData }
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Slot { version: 1, alive: true, items: Vec::new() });
            FrameList { index: idx, version: 1, _marker: PhantomData }
        }
    }

    fn slot(&self, h: FrameList<T>) -> Option<&Slot<T>> {
        self.slots.get(h.index as usize).filter(|s| s.alive && s.version == h.version)
    }

    fn slot_mut(&mut self, h: FrameList<T>) -> Option<&mut Slot<T>> {
        self.slots.get_mut(h.index as usize).filter(|s| s.alive && s.version == h.version)
    }

    pub fn get(&self, h: FrameList<T>) -> &[T] {
        self.slot(h).map(|s| s.items.as_slice()).unwrap_or_default()
    }

    pub fn get_mut(&mut self, h: FrameList<T>) -> &mut [T] {
        self.slot_mut(h).map(|s| s.items.as_mut_slice()).unwrap_or_default()
    }

    pub fn push(&mut self, h: FrameList<T>, v: T) {
        if let Some(s) = self.slot_mut(h) {
            s.items.push(v);
        }
    }

    pub fn clear(&mut self, h: FrameList<T>) {
        if let Some(s) = self.slot_mut(h) {
            s.items.clear();
        }
    }

    pub fn free(&mut self, h: FrameList<T>) {
        if let Some(s) = self.slots.get_mut(h.index as usize) {
            if s.alive && s.version == h.version {
                s.alive = false;
                s.items.clear();
                s.version = s.version.wrapping_add(1);
                self.free.push(h.index);
            }
        }
    }
}

/// Type-erased handle to a [`ListPool<T>`].
pub trait AnyListPool: Send + Sync {
    fn hash_into(&self, h: &mut Xxh3);
    fn copy_from(&mut self, other: &dyn AnyListPool);
    /// Appends `slot_count u32`, per slot `version u32, alive u8, items`
    /// (counted), then `free` as a counted `u32` array.
    fn write_bytes(&self, out: &mut Vec<u8>);
    /// Replaces `self` with what [`write_bytes`](Self::write_bytes) wrote,
    /// checking the slot/free-list invariants `alloc`/`free` maintain.
    fn read_bytes(&mut self, r: &mut Reader) -> Result<(), FrameDecodeError>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Component> AnyListPool for ListPool<T> {
    fn hash_into(&self, h: &mut Xxh3) {
        hash_len(h, self.slots.len());
        for s in &self.slots {
            h.update(&s.version.to_le_bytes());
            h.update(&[s.alive as u8]);
            hash_len(h, s.items.len());
            h.update(bytemuck::cast_slice(&s.items));
        }
        // LIFO order affects the next handle returned by alloc.
        hash_len(h, self.free.len());
        h.update(bytemuck::cast_slice(&self.free));
    }

    fn copy_from(&mut self, other: &dyn AnyListPool) {
        let other = other
            .as_any()
            .downcast_ref::<ListPool<T>>()
            .expect("orr_ecs: copy_from between mismatched list pool types");
        self.free.clone_from(&other.free);
        if self.slots.len() < other.slots.len() {
            self.slots.resize_with(other.slots.len(), || Slot { version: 0, alive: false, items: Vec::new() });
        } else {
            self.slots.truncate(other.slots.len());
        }
        for (dst, src) in self.slots.iter_mut().zip(other.slots.iter()) {
            dst.version = src.version;
            dst.alive = src.alive;
            dst.items.clone_from(&src.items);
        }
    }

    fn write_bytes(&self, out: &mut Vec<u8>) {
        put_len(out, self.slots.len());
        for s in &self.slots {
            put_u32(out, s.version);
            out.push(s.alive as u8);
            put_pods(out, &s.items);
        }
        put_pods(out, &self.free);
    }

    fn read_bytes(&mut self, r: &mut Reader) -> Result<(), FrameDecodeError> {
        let slot_count = r.u32()?;
        let mut slots = Vec::new();
        for _ in 0..slot_count {
            let version = r.u32()?;
            let alive = match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(FrameDecodeError::Corrupt("list slot alive flag is not 0 or 1")),
            };
            let items: Vec<T> = r.counted_pods()?;
            if !alive && !items.is_empty() {
                return Err(FrameDecodeError::Corrupt("freed list slot still holds items"));
            }
            slots.push(Slot { version, alive, items });
        }
        let free: Vec<u32> = r.counted_pods()?;

        let dead = slots.iter().filter(|s| !s.alive).count();
        if free.len() != dead {
            return Err(FrameDecodeError::Corrupt("list free list does not cover every dead slot"));
        }
        let mut in_free = vec![false; slots.len()];
        for &idx in &free {
            match in_free.get_mut(idx as usize) {
                Some(seen) if !*seen && !slots[idx as usize].alive => *seen = true,
                _ => return Err(FrameDecodeError::Corrupt("list free list has a live, duplicate or unknown slot")),
            }
        }
        self.slots = slots;
        self.free = free;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
