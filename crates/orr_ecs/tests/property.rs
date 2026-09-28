mod common;
use common::*;
use orr_ecs::Frame;
use std::collections::BTreeMap;

/// Tiny deterministic xorshift32 PRNG so the test needs no `rand` dependency.
struct Xorshift32(u32);
impl Xorshift32 {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn range(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// Naive reference model: entity liveness + Pos/Vel components in
/// `BTreeMap`s, keyed by (index, version) so stale handles are rejected the
/// same way the real `Frame` rejects them.
#[derive(Default)]
struct Model {
    versions: BTreeMap<u32, u32>, // index -> current version, only while alive
    pos: BTreeMap<(u32, u32), Pos>,
    vel: BTreeMap<(u32, u32), Vel>,
    next_index: u32,
    free: Vec<u32>,
    version_of_freed: BTreeMap<u32, u32>,
}

#[derive(Clone, Copy, Debug)]
struct ModelEntity {
    index: u32,
    version: u32,
}

impl Model {
    fn spawn(&mut self) -> ModelEntity {
        if let Some(idx) = self.free.pop() {
            let v = self.version_of_freed[&idx];
            self.versions.insert(idx, v);
            ModelEntity { index: idx, version: v }
        } else {
            let idx = self.next_index;
            self.next_index += 1;
            self.versions.insert(idx, 0);
            ModelEntity { index: idx, version: 0 }
        }
    }
    fn despawn(&mut self, e: ModelEntity) -> bool {
        if self.versions.get(&e.index) != Some(&e.version) {
            return false;
        }
        self.versions.remove(&e.index);
        self.pos.remove(&(e.index, e.version));
        self.vel.remove(&(e.index, e.version));
        let next_v = e.version.wrapping_add(1);
        self.version_of_freed.insert(e.index, next_v);
        self.free.push(e.index);
        true
    }
    fn exists(&self, e: ModelEntity) -> bool {
        self.versions.get(&e.index) == Some(&e.version)
    }
    fn add_pos(&mut self, e: ModelEntity, v: Pos) {
        if self.exists(e) {
            self.pos.insert((e.index, e.version), v);
        }
    }
    fn add_vel(&mut self, e: ModelEntity, v: Vel) {
        if self.exists(e) {
            self.vel.insert((e.index, e.version), v);
        }
    }
    fn remove_pos(&mut self, e: ModelEntity) {
        self.pos.remove(&(e.index, e.version));
    }
    fn has_pos(&self, e: ModelEntity) -> bool {
        self.exists(e) && self.pos.contains_key(&(e.index, e.version))
    }
    fn alive_count(&self) -> u32 {
        self.versions.len() as u32
    }
    fn pos_count(&self) -> u32 {
        self.pos.len() as u32
    }
}

#[test]
fn randomized_ops_match_naive_model() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let mut model = Model::default();
    let mut live: Vec<orr_ecs::Entity> = Vec::new();
    let mut live_model: Vec<ModelEntity> = Vec::new();

    let mut rng = Xorshift32(0xC0FFEE01);

    for _ in 0..20_000 {
        let op = rng.range(6);
        match op {
            0 => {
                // spawn
                let e = f.spawn();
                let me = model.spawn();
                assert_eq!(e.index, me.index);
                assert_eq!(e.version, me.version);
                live.push(e);
                live_model.push(me);
            }
            1 if !live.is_empty() => {
                // despawn a random live entity
                let i = rng.range(live.len() as u32) as usize;
                let e = live.swap_remove(i);
                let me = live_model.swap_remove(i);
                assert_eq!(f.despawn(e), model.despawn(me));
            }
            2 if !live.is_empty() => {
                // add Pos
                let i = rng.range(live.len() as u32) as usize;
                let e = live[i];
                let me = live_model[i];
                let v = Pos { x: rng.next() as i64, y: rng.next() as i64 };
                f.add(e, v);
                model.add_pos(e_to_model(e), v);
                let _ = me;
            }
            3 if !live.is_empty() => {
                // add Vel
                let i = rng.range(live.len() as u32) as usize;
                let e = live[i];
                let v = Vel { x: rng.next() as i64, y: rng.next() as i64 };
                f.add(e, v);
                model.add_vel(e_to_model(e), v);
            }
            4 if !live.is_empty() => {
                // remove Pos
                let i = rng.range(live.len() as u32) as usize;
                let e = live[i];
                f.remove::<Pos>(e);
                model.remove_pos(e_to_model(e));
            }
            _ => {
                // no-op this round (also covers op==1..4 when live is empty)
            }
        }
    }

    assert_eq!(f.alive_count(), model.alive_count());
    assert_eq!(f.count::<Pos>(), model.pos_count());
    for e in &live {
        let me = e_to_model(*e);
        assert_eq!(f.exists(*e), model.exists(me));
        assert_eq!(f.get::<Pos>(*e).copied(), model.pos.get(&(me.index, me.version)).copied());
        assert_eq!(f.has::<Pos>(*e), model.has_pos(me));
    }
}

fn e_to_model(e: orr_ecs::Entity) -> ModelEntity {
    ModelEntity { index: e.index, version: e.version }
}
