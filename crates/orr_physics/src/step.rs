//! The physics tick: broad phase, narrow phase, sequential impulse solver,
//! integration, sleeping and trigger bookkeeping.
//!
//! Everything derived during a tick lives in [`Scratch`] and is rebuilt
//! from the `Frame` at the start of the next tick. The only state that
//! survives a tick is in the `Frame` itself (bodies with their sleep
//! state, the warm-starting cache and the trigger overlap set), so
//! rollback restores it exactly. A `Scratch` may keep the previous tick's
//! sweep order as a sort hint, but the sort result never depends on it.
//!
//! The read phases work on the frame's dense component slices without
//! copying bodies or colliders. Only after the simulation is done does
//! the step write the results back, for the bodies that moved.

use orr_ecs::{Component, Entity, Frame, FrameList};
use orr_fp::{FPVec2, FP};

use crate::collide::{collide, overlap, Xf};
use crate::fastmath::{self, mul, FastVec};
use crate::solver::{self, Constraint, ContactPt, Vw};
use crate::types::{
    Body, Collider, ContactCache, OverlapPair, PhysicsConfig, PhysicsState, TriggerEvent, BODY_DYNAMIC, BODY_STATIC,
    SHAPE_CAPSULE, SHAPE_CIRCLE, SLEEP_FLAG, TRIGGER_ENTER, TRIGGER_EXIT,
};

/// Body has mass and is simulated as a dynamic body (asleep or not).
const F_DYN: u8 = 1;
/// Body sleeps: it is skipped by the solver and integration.
const F_ASLEEP: u8 = 2;
/// Body moves this tick (awake dynamic, or non-static with a velocity).
const F_ACTIVE: u8 = 4;

/// Sleeping bodies that still touch an awake body after this many wake
/// rounds are handled as immovable for the tick and wake up in the next.
const MAX_WAKE_ROUNDS: u32 = 3;

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Default)]
struct Aabb {
    min: FPVec2,
    max: FPVec2,
}

/// Bounding box in sweep order, raw `FP` units.
#[derive(Clone, Copy, Default)]
struct SweepBox {
    min_x: i64,
    max_x: i64,
    min_y: i64,
    max_y: i64,
    idx: u32,
    layer: u32,
    mask: u32,
    /// Bit 0: sensor. Bit 1: dynamic body. Bit 2: moves this tick.
    flags: u32,
}

/// Counts of the last [`step`], for tuning and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepStats {
    /// Bodies with a collider.
    pub bodies: u32,
    /// Awake dynamic bodies (the ones the solver worked on).
    pub awake: u32,
    /// Sleeping dynamic bodies at the end of the tick.
    pub asleep: u32,
    /// Candidate pairs the broad phase produced.
    pub pairs: u32,
    /// Pairs that had at least one contact point.
    pub manifolds: u32,
    /// Contact points solved.
    pub points: u32,
}

/// Reusable per-tick working memory. Holds no state between ticks: every
/// buffer is cleared and refilled from the `Frame` on each [`step`], so a
/// `Scratch` can be shared across rollbacks and resimulations.
#[derive(Default)]
pub struct Scratch {
    /// Bodies with body and collider, ascending by entity index.
    ents: Vec<Entity>,
    /// Slot of each body in the dense `Body` slice.
    bslot: Vec<u32>,
    /// Slot of each body in the dense `Collider` slice.
    cslot: Vec<u32>,
    perm_b: Vec<u32>,
    perm_c: Vec<u32>,
    flags: Vec<u8>,
    /// Indices of bodies that move this tick, ascending.
    movers: Vec<u32>,
    /// Entity index to body index (`NONE` if absent). Built only while
    /// something sleeps.
    slot_of_index: Vec<u32>,
    wake_ids: Vec<u32>,
    wake_single: Vec<u32>,
    xfs: Vec<Xf>,
    aabbs: Vec<Aabb>,
    /// Per body: layer, mask, sensor bit.
    filters: Vec<[u32; 3]>,
    has_sensor: bool,
    order: Vec<u32>,
    sorted: Vec<SweepBox>,
    cand: Vec<u64>,
    starts: Vec<u32>,
    cursor: Vec<u32>,
    pairs: Vec<u64>,
    sensor_pairs: Vec<u64>,
    cons: Vec<Constraint>,
    vw: Vec<Vw>,
    /// Per mover: new sleep timer, and the island id if it falls asleep.
    mtimer: Vec<u32>,
    mslept: Vec<u32>,
    parent: Vec<u32>,
    imin: Vec<u32>,
    new_cache: Vec<ContactCache>,
    carried: Vec<ContactCache>,
    new_overlaps: Vec<OverlapPair>,
    asleep_end: u32,
}

fn pair_key(lo: u32, hi: u32) -> u64 {
    ((lo as u64) << 32) | hi as u64
}

fn mix_friction(a: FP, b: FP) -> FP {
    if a == b || a == FP::ZERO || b == FP::ZERO {
        // Equal values keep their value; a frictionless side wins.
        return if a == b { a } else { FP::ZERO };
    }
    fastmath::sqrt(a * b)
}

impl Scratch {
    /// Creates empty working memory.
    pub fn new() -> Scratch {
        Scratch::default()
    }

    /// Counts of the last [`step`].
    pub fn stats(&self) -> StepStats {
        let awake = self
            .movers
            .iter()
            .zip(&self.mslept)
            .filter(|&(&i, &slept)| slept == 0 && self.flags[i as usize] & F_DYN != 0)
            .count() as u32;
        StepStats {
            bodies: self.ents.len() as u32,
            awake,
            asleep: self.asleep_end,
            pairs: self.pairs.len() as u32,
            manifolds: self.cons.len() as u32,
            points: self.cons.iter().map(|c| c.count).sum(),
        }
    }

    /// Orders the bodies by entity index and finds each one's dense slots.
    /// Only entities that have both a `Body` and a `Collider` take part.
    fn order_bodies(&mut self, be: &[Entity], ce: &[Entity]) {
        self.ents.clear();
        self.bslot.clear();
        self.cslot.clear();
        if be == ce {
            // Same entities in the same dense order (the usual case).
            if be.windows(2).all(|w| w[0].index < w[1].index) {
                for (i, &e) in be.iter().enumerate() {
                    self.ents.push(e);
                    self.bslot.push(i as u32);
                    self.cslot.push(i as u32);
                }
            } else {
                // Dense order depends on despawn history; sort it out.
                self.perm_b.clear();
                self.perm_b.extend(0..be.len() as u32);
                self.perm_b.sort_unstable_by_key(|&i| be[i as usize].index);
                for &i in &self.perm_b {
                    self.ents.push(be[i as usize]);
                    self.bslot.push(i);
                    self.cslot.push(i);
                }
            }
            return;
        }
        self.perm_b.clear();
        self.perm_b.extend(0..be.len() as u32);
        self.perm_b.sort_unstable_by_key(|&i| be[i as usize].index);
        self.perm_c.clear();
        self.perm_c.extend(0..ce.len() as u32);
        self.perm_c.sort_unstable_by_key(|&i| ce[i as usize].index);
        let (mut x, mut y) = (0, 0);
        while x < self.perm_b.len() && y < self.perm_c.len() {
            let (eb, ec) = (be[self.perm_b[x] as usize], ce[self.perm_c[y] as usize]);
            match eb.index.cmp(&ec.index) {
                core::cmp::Ordering::Less => x += 1,
                core::cmp::Ordering::Greater => y += 1,
                core::cmp::Ordering::Equal => {
                    if eb == ec {
                        self.ents.push(eb);
                        self.bslot.push(self.perm_b[x]);
                        self.cslot.push(self.perm_c[y]);
                    }
                    x += 1;
                    y += 1;
                }
            }
        }
    }

    /// Orders the bodies, classifies them (dynamic, asleep, moving) and
    /// applies the sleep wake rules that need no contacts: an awake body
    /// that still carries an island id, or a sleeping body with a velocity
    /// (both mean game code woke it), wakes its whole island.
    fn gather(&mut self, be: &[Entity], bd: &[Body], ce: &[Entity], cfg: &PhysicsConfig) {
        self.order_bodies(be, ce);
        let n = self.ents.len();
        let enabled = cfg.sleep_ticks != 0;
        self.flags.clear();
        self.wake_ids.clear();
        let mut any_asleep = false;
        for i in 0..n {
            let b = &bd[self.bslot[i] as usize];
            let moving = b.vel != FPVec2::ZERO || b.omega != FP::ZERO;
            let mut f = 0u8;
            if b.kind == BODY_DYNAMIC && b.inv_mass > FP::ZERO {
                f |= F_DYN;
                if enabled && b.sleep & SLEEP_FLAG != 0 && !moving {
                    f |= F_ASLEEP;
                    any_asleep = true;
                } else {
                    f |= F_ACTIVE;
                    if b.island != 0 {
                        self.wake_ids.push(b.island);
                    }
                }
            } else if b.kind != BODY_STATIC && moving {
                f |= F_ACTIVE;
            }
            self.flags.push(f);
        }
        if any_asleep {
            let max = self.ents[n - 1].index as usize;
            self.slot_of_index.clear();
            self.slot_of_index.resize(max + 1, NONE);
            for (i, e) in self.ents.iter().enumerate() {
                self.slot_of_index[e.index as usize] = i as u32;
            }
            if !self.wake_ids.is_empty() {
                self.wake_single.clear();
                self.apply_wakes(bd);
            }
        }
        self.rebuild_movers();
    }

    fn rebuild_movers(&mut self) {
        self.movers.clear();
        for (i, &f) in self.flags.iter().enumerate() {
            if f & F_ACTIVE != 0 {
                self.movers.push(i as u32);
            }
        }
    }

    /// Wakes every sleeping body whose island id is in `wake_ids` and
    /// every body in `wake_single`.
    fn apply_wakes(&mut self, bd: &[Body]) {
        self.wake_ids.sort_unstable();
        self.wake_ids.dedup();
        if !self.wake_ids.is_empty() {
            for i in 0..self.ents.len() {
                if self.flags[i] & F_ASLEEP != 0 {
                    let id = bd[self.bslot[i] as usize].island;
                    if id != 0 && self.wake_ids.binary_search(&id).is_ok() {
                        self.flags[i] = (self.flags[i] & !F_ASLEEP) | F_ACTIVE;
                    }
                }
            }
        }
        for k in 0..self.wake_single.len() {
            let i = self.wake_single[k] as usize;
            if self.flags[i] & F_ASLEEP != 0 {
                self.flags[i] = (self.flags[i] & !F_ASLEEP) | F_ACTIVE;
            }
        }
    }

    /// A sleeping body loses its support when the other body of a cached
    /// contact no longer exists: wake it (and so its island).
    fn wake_orphans(&mut self, old_cache: &[ContactCache], bd: &[Body]) {
        self.wake_ids.clear();
        self.wake_single.clear();
        for c in old_cache {
            let sa = self.slot_of_index.get(c.a as usize).copied().unwrap_or(NONE);
            let sb = self.slot_of_index.get(c.b as usize).copied().unwrap_or(NONE);
            if (sa == NONE) == (sb == NONE) {
                continue;
            }
            let alive = if sa == NONE { sb } else { sa } as usize;
            if self.flags[alive] & F_ASLEEP != 0 {
                let id = bd[self.bslot[alive] as usize].island;
                if id != 0 {
                    self.wake_ids.push(id);
                } else {
                    self.wake_single.push(alive as u32);
                }
            }
        }
        if !self.wake_ids.is_empty() || !self.wake_single.is_empty() {
            self.apply_wakes(bd);
            self.rebuild_movers();
        }
    }

    fn build_transforms(&mut self, bd: &[Body], cd: &[Collider], margin: FP) {
        let n = self.ents.len();
        self.xfs.clear();
        self.aabbs.clear();
        self.filters.clear();
        self.has_sensor = false;
        for i in 0..n {
            let b = &bd[self.bslot[i] as usize];
            let c = &cd[self.cslot[i] as usize];
            let xf = if b.kind == BODY_STATIC && b.angle == FP::ZERO {
                Xf { p: b.pos, c: FP::ONE, s: FP::ZERO }
            } else {
                Xf::new(b.pos, b.angle)
            };
            let sh = &c.shape;
            let mut bb = if sh.kind == SHAPE_CIRCLE {
                Aabb { min: b.pos - FPVec2::splat(sh.radius), max: b.pos + FPVec2::splat(sh.radius) }
            } else if sh.kind == SHAPE_CAPSULE {
                let (p, q) = (xf.apply(sh.verts[0]), xf.apply(sh.verts[1]));
                Aabb { min: p.min(q) - FPVec2::splat(sh.radius), max: p.max(q) + FPVec2::splat(sh.radius) }
            } else {
                let mut mn = FPVec2::splat(FP::MAX);
                let mut mx = FPVec2::splat(FP::MIN);
                for k in 0..sh.count as usize {
                    let w = xf.apply(sh.verts[k]);
                    mn = mn.min(w);
                    mx = mx.max(w);
                }
                Aabb { min: mn, max: mx }
            };
            bb.min -= FPVec2::splat(margin);
            bb.max += FPVec2::splat(margin);
            self.xfs.push(xf);
            self.aabbs.push(bb);
            self.filters.push([c.layer, c.mask, c.is_sensor() as u32]);
            self.has_sensor |= c.is_sensor();
        }
    }

    /// Inverse mass and inertia the solver may use for body `i`: zero for
    /// anything that does not move as a dynamic body this tick.
    #[inline]
    fn inv_mass_of(&self, i: usize, bd: &[Body]) -> (FP, FP) {
        if self.flags[i] & (F_DYN | F_ASLEEP) == F_DYN {
            let b = &bd[self.bslot[i] as usize];
            (b.inv_mass, b.inv_inertia)
        } else {
            (FP::ZERO, FP::ZERO)
        }
    }

    /// Sort-and-sweep along x. Emits canonical `(lo, hi)` pairs sorted
    /// ascending. The sort key ends with the unique body index, so the
    /// result never depends on memory layout or on the previous tick's
    /// order (which is only used as a starting point for the sort).
    ///
    /// A solid pair is emitted only if one body is dynamic and one body
    /// moves this tick: pairs of sleeping, static and idle bodies cost no
    /// narrow phase. Sensor pairs are always emitted.
    fn broad_phase(&mut self) {
        let n = self.ents.len();
        self.pairs.clear();
        self.sensor_pairs.clear();
        self.cand.clear();
        if self.movers.is_empty() && !self.has_sensor {
            return;
        }

        // Sort by (min x, index), starting from last tick's order when the
        // body count is unchanged (bodies barely move between ticks, so
        // insertion sort is near linear). Any start gives the same result.
        let aabbs = &self.aabbs;
        let key = |i: u32| (aabbs[i as usize].min.x.raw(), i);
        let mut full = self.order.len() != n;
        if full {
            self.order.clear();
            self.order.extend(0..n as u32);
        } else {
            let mut budget = 8 * n + 64;
            for i in 1..n {
                let cur = self.order[i];
                let kc = key(cur);
                let mut j = i;
                while j > 0 && key(self.order[j - 1]) > kc {
                    self.order[j] = self.order[j - 1];
                    j -= 1;
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                }
                self.order[j] = cur;
                if budget == 0 {
                    full = true;
                    break;
                }
            }
        }
        if full {
            self.order.sort_unstable_by_key(|&i| key(i));
        }

        // Boxes in sweep order, contiguous.
        self.sorted.clear();
        for &i in &self.order {
            let iu = i as usize;
            let bb = &self.aabbs[iu];
            let [layer, mask, sensor] = self.filters[iu];
            let f = self.flags[iu];
            self.sorted.push(SweepBox {
                min_x: bb.min.x.raw(),
                max_x: bb.max.x.raw(),
                min_y: bb.min.y.raw(),
                max_y: bb.max.y.raw(),
                idx: i,
                layer,
                mask,
                flags: sensor | (((f & F_DYN != 0) as u32) << 1) | (((f & F_ACTIVE != 0) as u32) << 2),
            });
        }

        // Branchless inner loop: the y test outcome is unpredictable, so
        // every candidate writes its key and only advances the output
        // cursor when it is a hit. Sensor pairs carry bit 63.
        let mut cnt = 0usize;
        for oi in 0..n {
            let a = self.sorted[oi];
            let mut e = oi + 1;
            while e < n && self.sorted[e].min_x <= a.max_x {
                e += 1;
            }
            let m = e - oi - 1;
            if m == 0 {
                continue;
            }
            if self.cand.len() < cnt + m {
                self.cand.resize(cnt + m, 0);
            }
            let out = &mut self.cand[cnt..cnt + m];
            let mut k = 0usize;
            for b in &self.sorted[oi + 1..e] {
                let f = a.flags | b.flags;
                let solid = (f & 6) == 6;
                let hit = (b.min_y <= a.max_y)
                    & (b.max_y >= a.min_y)
                    & (a.layer & b.mask != 0)
                    & (b.layer & a.mask != 0)
                    & ((f & 1 != 0) | solid);
                let (lo, hi) = if a.idx < b.idx { (a.idx, b.idx) } else { (b.idx, a.idx) };
                out[k] = pair_key(lo, hi) | ((f as u64 & 1) << 63);
                k += hit as usize;
            }
            cnt += k;
        }
        self.cand.truncate(cnt);
        if self.has_sensor {
            let mut w = 0;
            for r in 0..self.cand.len() {
                let k = self.cand[r];
                if k >> 63 != 0 {
                    self.sensor_pairs.push(k & !(1 << 63));
                } else {
                    self.cand[w] = k;
                    w += 1;
                }
            }
            self.cand.truncate(w);
        }
        self.sensor_pairs.sort_unstable();

        // Counting sort by `lo`, then by `hi` inside each bucket.
        self.starts.clear();
        self.starts.resize(n + 1, 0);
        for &k in &self.cand {
            self.starts[(k >> 32) as usize + 1] += 1;
        }
        for i in 0..n {
            self.starts[i + 1] += self.starts[i];
        }
        self.pairs.resize(self.cand.len(), 0);
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.starts[..n]);
        for &k in &self.cand {
            let lo = (k >> 32) as usize;
            self.pairs[self.cursor[lo] as usize] = k;
            self.cursor[lo] += 1;
        }
        for i in 0..n {
            let (s0, s1) = (self.starts[i] as usize, self.starts[i + 1] as usize);
            if s1 - s0 > 16 {
                self.pairs[s0..s1].sort_unstable();
            } else if s1 - s0 > 1 {
                // Small buckets: insertion sort beats a general sort.
                for j in s0 + 1..s1 {
                    let cur = self.pairs[j];
                    let mut m = j;
                    while m > s0 && self.pairs[m - 1] > cur {
                        self.pairs[m] = self.pairs[m - 1];
                        m -= 1;
                    }
                    self.pairs[m] = cur;
                }
            }
        }
    }

    fn narrow_phase(&mut self, bd: &[Body], cd: &[Collider], old_cache: &[ContactCache], margin: FP) {
        self.cons.clear();
        let mut ci = 0usize;
        for &key in &self.pairs {
            let (lo, hi) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            let (ca, cb) = (&cd[self.cslot[lo] as usize], &cd[self.cslot[hi] as usize]);
            let m = collide(&ca.shape, &self.xfs[lo], &cb.shape, &self.xfs[hi], margin);
            if m.count == 0 {
                continue;
            }
            let (ima, iia) = self.inv_mass_of(lo, bd);
            let (imb, iib) = self.inv_mass_of(hi, bd);
            let mut con = Constraint {
                a: lo as u32,
                b: hi as u32,
                ima,
                iia,
                imb,
                iib,
                normal: m.normal,
                friction: mix_friction(ca.friction, cb.friction),
                restitution: ca.restitution.max(cb.restitution),
                count: m.count as u32,
                pts: [ContactPt::default(); 2],
                ..Constraint::default()
            };
            for k in 0..m.count {
                let mp = m.points[k];
                con.pts[k].point = mp.point;
                con.pts[k].sep = mp.separation;
                con.pts[k].id = mp.id;
            }
            if con.count == 2 && con.pts[0].id > con.pts[1].id {
                con.pts.swap(0, 1);
            }
            // Warm start: `old_cache` and the pairs are both sorted by
            // (entity a, entity b, id), so one forward walk finds every
            // entry.
            let (ea, eb) = (self.ents[lo].index, self.ents[hi].index);
            while ci < old_cache.len() && (old_cache[ci].a, old_cache[ci].b) < (ea, eb) {
                ci += 1;
            }
            let mut cj = ci;
            for k in 0..con.count as usize {
                let id = con.pts[k].id;
                while cj < old_cache.len() && (old_cache[cj].a, old_cache[cj].b) == (ea, eb) && old_cache[cj].id < id {
                    cj += 1;
                }
                if cj < old_cache.len() && (old_cache[cj].a, old_cache[cj].b, old_cache[cj].id) == (ea, eb, id) {
                    con.pts[k].jn = old_cache[cj].normal_impulse;
                    con.pts[k].jt = old_cache[cj].tangent_impulse;
                }
            }
            self.cons.push(con);
        }
    }

    /// Wakes the islands of sleeping bodies that touch a moving body.
    /// Returns true if any body woke (contacts must then be rebuilt).
    fn wake_touching(&mut self, bd: &[Body]) -> bool {
        self.wake_ids.clear();
        self.wake_single.clear();
        for c in &self.cons {
            for x in [c.a as usize, c.b as usize] {
                if self.flags[x] & F_ASLEEP != 0 {
                    let id = bd[self.bslot[x] as usize].island;
                    if id != 0 {
                        self.wake_ids.push(id);
                    } else {
                        self.wake_single.push(x as u32);
                    }
                }
            }
        }
        if self.wake_ids.is_empty() && self.wake_single.is_empty() {
            return false;
        }
        self.apply_wakes(bd);
        self.rebuild_movers();
        true
    }

    fn integrate_velocities(&mut self, bd: &[Body], cfg: &PhysicsConfig) {
        let n = self.ents.len();
        self.vw.clear();
        self.vw.resize(n, Vw::default());
        let g = cfg.gravity * cfg.dt;
        let max_v = cfg.max_linear_speed;
        let max_w = cfg.max_angular_speed;
        for &i in &self.movers {
            let i = i as usize;
            let b = &bd[self.bslot[i] as usize];
            let (mut v, mut w) = (b.vel, b.omega);
            if self.flags[i] & F_DYN != 0 {
                v += g;
                if b.linear_damping != FP::ZERO {
                    v = v * (FP::ONE - b.linear_damping * cfg.dt).max(FP::ZERO);
                }
                if b.angular_damping != FP::ZERO {
                    w *= (FP::ONE - b.angular_damping * cfg.dt).max(FP::ZERO);
                }
                v = FPVec2::new(v.x.clamp(-max_v, max_v), v.y.clamp(-max_v, max_v));
                w = w.clamp(-max_w, max_w);
            }
            self.vw[i] = Vw { v, w };
        }
    }

    /// Keeps a badly over-constrained pile (e.g. a body wedged between two
    /// immovable shapes) from producing runaway velocities.
    fn clamp_velocities(&mut self, cfg: &PhysicsConfig) {
        let (max_v, max_w) = (cfg.max_linear_speed, cfg.max_angular_speed);
        for &i in &self.movers {
            let i = i as usize;
            if self.flags[i] & F_DYN != 0 {
                let x = self.vw[i];
                self.vw[i] = Vw {
                    v: FPVec2::new(x.v.x.clamp(-max_v, max_v), x.v.y.clamp(-max_v, max_v)),
                    w: x.w.clamp(-max_w, max_w),
                };
            }
        }
    }

    /// Updates the sleep timers of the awake dynamic bodies and finds the
    /// islands (groups linked by contacts between dynamic bodies) in which
    /// every body has been still long enough to sleep.
    fn update_sleep(&mut self, bd: &[Body], cfg: &PhysicsConfig) {
        let enabled = cfg.sleep_ticks != 0;
        let limit = cfg.sleep_ticks.min(SLEEP_FLAG - 1);
        self.mtimer.clear();
        self.mslept.clear();
        let lin2 = mul(cfg.sleep_linear_speed, cfg.sleep_linear_speed);
        let mut candidates = false;
        for &i in &self.movers {
            let i = i as usize;
            let mut t = 0;
            if enabled && self.flags[i] & F_DYN != 0 {
                let x = self.vw[i];
                let still = x.v.dotf(x.v) <= lin2 && x.w.abs() <= cfg.sleep_angular_speed;
                if still {
                    let prev = bd[self.bslot[i] as usize].sleep & !SLEEP_FLAG;
                    t = (prev + 1).min(limit);
                    candidates |= t >= limit;
                }
            }
            self.mtimer.push(t);
            self.mslept.push(0);
        }
        if !candidates {
            return;
        }
        let n = self.ents.len();
        self.parent.clear();
        self.parent.extend(0..n as u32);
        self.imin.clear();
        self.imin.resize(n, u32::MAX);
        // Union by lowest index, so a root is the lowest body of its island.
        for k in 0..self.cons.len() {
            let (a, b) = (self.cons[k].a as usize, self.cons[k].b as usize);
            if self.flags[a] & (F_DYN | F_ASLEEP) == F_DYN && self.flags[b] & (F_DYN | F_ASLEEP) == F_DYN {
                let (ra, rb) = (find(&mut self.parent, a as u32), find(&mut self.parent, b as u32));
                if ra != rb {
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    self.parent[hi as usize] = lo;
                }
            }
        }
        for k in 0..self.movers.len() {
            let i = self.movers[k];
            if self.flags[i as usize] & F_DYN != 0 {
                let r = find(&mut self.parent, i) as usize;
                self.imin[r] = self.imin[r].min(self.mtimer[k]);
            }
        }
        for k in 0..self.movers.len() {
            let i = self.movers[k];
            if self.flags[i as usize] & F_DYN != 0 {
                let r = find(&mut self.parent, i) as usize;
                if self.imin[r] >= limit {
                    self.mslept[k] = self.ents[r].index + 1;
                }
            }
        }
    }

    /// Merges the contact cache of the pairs solved this tick with the old
    /// entries of pairs that were skipped because both bodies were idle
    /// (sleeping, static): their impulses stay for the wake-up.
    fn build_cache(&mut self, old_cache: &[ContactCache], any_asleep: bool) {
        self.new_cache.clear();
        self.carried.clear();
        if any_asleep {
            for c in old_cache {
                let sa = self.slot_of_index.get(c.a as usize).copied().unwrap_or(NONE);
                let sb = self.slot_of_index.get(c.b as usize).copied().unwrap_or(NONE);
                if sa != NONE && sb != NONE && (self.flags[sa as usize] | self.flags[sb as usize]) & F_ACTIVE == 0 {
                    self.carried.push(*c);
                }
            }
        }
        let mut ci = 0;
        for c in &self.cons {
            let (ea, eb) = (self.ents[c.a as usize].index, self.ents[c.b as usize].index);
            while ci < self.carried.len() && (self.carried[ci].a, self.carried[ci].b) < (ea, eb) {
                self.new_cache.push(self.carried[ci]);
                ci += 1;
            }
            for p in &c.pts[..c.count as usize] {
                self.new_cache.push(ContactCache {
                    a: ea,
                    b: eb,
                    id: p.id,
                    _pad: 0,
                    normal_impulse: p.jn,
                    tangent_impulse: p.jt,
                });
            }
        }
        self.new_cache.extend_from_slice(&self.carried[ci..]);
    }
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let p = parent[x as usize];
        parent[x as usize] = parent[p as usize];
        x = parent[x as usize];
    }
    x
}

/// Advances the physics state in `frame` by one fixed step.
///
/// Trigger enter/exit events for this tick are appended to `events`.
/// Panics if [`crate::init`] was not called on this frame.
pub fn step(frame: &mut Frame, sc: &mut Scratch, events: &mut Vec<TriggerEvent>) {
    step_probed(frame, sc, events, &mut |_| {});
}

/// A phase of [`step`], reported to the probe of [`step_probed`] when the
/// phase ends. Broad and narrow phase repeat when a contact wakes a
/// sleeping island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Body gather, ordering and wake rules.
    Gather,
    /// Transforms and bounding boxes.
    Transforms,
    /// Broad phase pair search.
    Broad,
    /// Contact manifolds and warm-start lookup.
    Narrow,
    /// Sensor overlaps and trigger events.
    Sensors,
    /// Velocity integration.
    Integrate,
    /// Constraint setup and warm starting.
    Prepare,
    /// Sequential impulse iterations.
    Solve,
    /// Sleep timers and islands.
    Sleep,
    /// Position integration, write-back and cache store.
    Finish,
}

/// [`step`] that calls `probe` at the end of every [`Phase`]. The probe is
/// how benchmarks time phases without the sim crate touching a clock. It
/// must not affect the simulation.
pub fn step_probed(frame: &mut Frame, sc: &mut Scratch, events: &mut Vec<TriggerEvent>, probe: &mut impl FnMut(Phase)) {
    let st = *frame.singleton::<PhysicsState>();
    let cfg = st.config;
    assert!(cfg.dt.raw() > 0, "orr_physics: call orr_physics::init before step");

    // Read phase: everything is computed from shared borrows of the frame.
    {
        let (be, bd) = frame.dense::<Body>();
        let (ce, cd) = frame.dense::<Collider>();
        let old_cache = frame.list(st.contacts);

        sc.gather(be, bd, ce, &cfg);
        if sc.flags.iter().any(|&f| f & F_ASLEEP != 0) {
            sc.wake_orphans(old_cache, bd);
        }
        probe(Phase::Gather);
        sc.build_transforms(bd, cd, cfg.contact_margin);
        probe(Phase::Transforms);

        // Solid contacts (warm-started from last tick's cache). A contact
        // between a moving body and a sleeping one wakes the sleeper's
        // island, and then the contacts are rebuilt with the island awake.
        let mut rounds = 0;
        loop {
            sc.broad_phase();
            probe(Phase::Broad);
            sc.narrow_phase(bd, cd, old_cache, cfg.contact_margin);
            probe(Phase::Narrow);
            if sc.wake_touching(bd) && rounds < MAX_WAKE_ROUNDS {
                rounds += 1;
                continue;
            }
            break;
        }

        // Trigger overlaps.
        sc.new_overlaps.clear();
        for &key in &sc.sensor_pairs {
            let (lo, hi) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            let (ca, cb) = (&cd[sc.cslot[lo] as usize], &cd[sc.cslot[hi] as usize]);
            if overlap(&ca.shape, &sc.xfs[lo], &cb.shape, &sc.xfs[hi]) {
                sc.new_overlaps.push(OverlapPair { a: sc.ents[lo], b: sc.ents[hi] });
            }
        }
        diff_overlaps(frame.list(st.overlaps), &sc.new_overlaps, events);
        probe(Phase::Sensors);

        // Dynamics.
        sc.integrate_velocities(bd, &cfg);
        probe(Phase::Integrate);
        solver::prepare(&mut sc.cons, &sc.xfs, &mut sc.vw, &cfg);
        probe(Phase::Prepare);
        solver::solve(&mut sc.cons, &mut sc.vw, cfg.velocity_iterations);
        sc.clamp_velocities(&cfg);
        probe(Phase::Solve);
        sc.update_sleep(bd, &cfg);
        probe(Phase::Sleep);

        let asleep_now = sc.flags.iter().filter(|&&f| f & F_ASLEEP != 0).count() as u32;
        sc.build_cache(old_cache, asleep_now > 0);
        sc.asleep_end = asleep_now + sc.mslept.iter().filter(|&&x| x != 0).count() as u32;
    }

    // Position integration and write-back, for the bodies that moved.
    // `FP::TWO_PI` is one raw unit above `2 * FP::PI`; wrapping by it can
    // land exactly on `-PI`, which `FP::sin_cos` cannot fold.
    let pi = FP::PI;
    let two_pi = FP::PI + FP::PI;
    let limit = cfg.sleep_ticks.min(SLEEP_FLAG - 1);
    for k in 0..sc.movers.len() {
        let i = sc.movers[k] as usize;
        let dynamic = sc.flags[i] & F_DYN != 0;
        let Some(b) = frame.get_mut::<Body>(sc.ents[i]) else { continue };
        if sc.mslept[k] != 0 {
            b.sleep = SLEEP_FLAG | limit;
            b.island = sc.mslept[k];
            b.vel = FPVec2::ZERO;
            b.omega = FP::ZERO;
            continue;
        }
        let (v, w) = (sc.vw[i].v, sc.vw[i].w);
        let mut angle = b.angle + w * cfg.dt;
        if angle > pi {
            angle -= two_pi;
        } else if angle <= -pi {
            angle += two_pi;
        }
        b.pos += v * cfg.dt;
        b.angle = angle;
        b.vel = v;
        b.omega = w;
        if dynamic {
            b.sleep = sc.mtimer[k];
            b.island = 0;
        }
    }

    // Persist the warm-starting cache (sorted by (a, b, id)) and the
    // trigger overlap set.
    store_list(frame, st.contacts, &sc.new_cache);
    store_list(frame, st.overlaps, &sc.new_overlaps);
    probe(Phase::Finish);
}

/// Replaces the contents of a frame list, in place when the length is
/// unchanged.
fn store_list<T: Component>(frame: &mut Frame, h: FrameList<T>, new: &[T]) {
    if frame.list(h).len() == new.len() {
        frame.list_mut(h).copy_from_slice(new);
    } else {
        frame.list_clear(h);
        for v in new {
            frame.list_push(h, *v);
        }
    }
}

/// Merges the old and new (both sorted by entity index) overlap sets and
/// appends the enter/exit events, in pair order.
fn diff_overlaps(old: &[OverlapPair], new: &[OverlapPair], events: &mut Vec<TriggerEvent>) {
    let key = |p: &OverlapPair| (p.a.index, p.b.index);
    let ev = |p: &OverlapPair, kind: u32| TriggerEvent { a: p.a, b: p.b, kind, _pad: 0 };
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        match (old.get(i), new.get(j)) {
            (Some(o), Some(n)) => {
                let (ko, kn) = (key(o), key(n));
                if ko < kn {
                    events.push(ev(o, TRIGGER_EXIT));
                    i += 1;
                } else if ko > kn {
                    events.push(ev(n, TRIGGER_ENTER));
                    j += 1;
                } else {
                    if o != n {
                        events.push(ev(o, TRIGGER_EXIT));
                        events.push(ev(n, TRIGGER_ENTER));
                    }
                    i += 1;
                    j += 1;
                }
            }
            (Some(o), None) => {
                events.push(ev(o, TRIGGER_EXIT));
                i += 1;
            }
            (None, Some(n)) => {
                events.push(ev(n, TRIGGER_ENTER));
                j += 1;
            }
            (None, None) => break,
        }
    }
}
