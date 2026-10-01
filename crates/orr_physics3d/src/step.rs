//! The 3D physics tick: broad phase, narrow phase, sequential impulse
//! solver, integration and sleeping.
//!
//! Everything derived during a tick lives in [`Scratch`] and is rebuilt
//! from the `Frame` at the start of the next tick. The only state that
//! survives a tick is in the `Frame` itself (bodies with their sleep
//! state and the warm-starting cache), so rollback restores it exactly.
//!
//! The read phases work on the frame's dense component slices without
//! copying bodies or colliders. Only after the simulation is done does
//! the step write the results back, for the bodies that moved.

use orr_ecs::{Component, Entity, Frame, FrameList};
use orr_fp::{FPMat3, FPQuat, FPVec3, FP};

use crate::collide::collide;
use crate::fastmath::{div, length, mul_round, sqrt};
use crate::geom::Xf;
use crate::solver::{self, BodyInv, Constraint, ContactPt, Vw};
use crate::types::{
    Body, Collider, ContactCache, PhysicsConfig, PhysicsState, BODY_DYNAMIC, BODY_STATIC, SHAPE_BOX, SHAPE_SPHERE, SLEEP_FLAG,
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

/// Bounding box in sweep order, raw `FP` units.
#[derive(Clone, Copy, Default)]
struct SweepBox {
    min: [i64; 3],
    max: [i64; 3],
    idx: u32,
    layer: u32,
    mask: u32,
    /// Bit 1: dynamic body. Bit 2: moves this tick.
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
    /// Bounding sphere radius per body.
    brad: Vec<FP>,
    sorted: Vec<SweepBox>,
    keys: Vec<(i64, u32)>,
    pairs: Vec<u64>,
    cons: Vec<Constraint>,
    pool: Vec<ContactPt>,
    binv: Vec<BodyInv>,
    vw: Vec<Vw>,
    /// Position and rotation vector accumulated over the substeps.
    pw: Vec<Vw>,
    /// Per mover: new sleep timer, and the island id if it falls asleep.
    mtimer: Vec<u32>,
    mslept: Vec<u32>,
    parent: Vec<u32>,
    imin: Vec<u32>,
    new_cache: Vec<ContactCache>,
    carried: Vec<ContactCache>,
    asleep_end: u32,
}

#[inline]
fn pair_key(lo: u32, hi: u32) -> u64 {
    ((lo as u64) << 32) | hi as u64
}

fn mix_friction(a: FP, b: FP) -> FP {
    if a == b || a == FP::ZERO || b == FP::ZERO {
        // Equal values keep their value; a frictionless side wins.
        return if a == b { a } else { FP::ZERO };
    }
    (a * b).sqrt()
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
            let moving = b.vel != FPVec3::ZERO || b.omega != FPVec3::ZERO;
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

    /// Transforms and sweep boxes (grown by the contact margin).
    fn build_transforms(&mut self, bd: &[Body], cd: &[Collider], margin: FP) {
        let n = self.ents.len();
        self.xfs.clear();
        self.brad.clear();
        self.sorted.clear();
        for i in 0..n {
            let b = &bd[self.bslot[i] as usize];
            let c = &cd[self.cslot[i] as usize];
            let r = FPMat3::from_quat(b.rot);
            let xf = Xf { p: b.pos, r };
            let sh = &c.shape;
            let ext = match sh.kind {
                SHAPE_SPHERE => FPVec3::splat(sh.radius),
                SHAPE_BOX => FPVec3::new(
                    r.r0.abs().dot(sh.half),
                    r.r1.abs().dot(sh.half),
                    r.r2.abs().dot(sh.half),
                ),
                _ => FPVec3::new(r.r0.y.abs(), r.r1.y.abs(), r.r2.y.abs()) * sh.half.y + FPVec3::splat(sh.radius),
            } + FPVec3::splat(margin);
            let (lo, hi) = (b.pos - ext, b.pos + ext);
            let f = self.flags[i];
            self.sorted.push(SweepBox {
                min: [lo.x.raw(), lo.y.raw(), lo.z.raw()],
                max: [hi.x.raw(), hi.y.raw(), hi.z.raw()],
                idx: i as u32,
                layer: c.layer,
                mask: c.mask,
                flags: (((f & F_DYN != 0) as u32) << 1) | (((f & F_ACTIVE != 0) as u32) << 2),
            });
            self.xfs.push(xf);
            self.brad.push(match sh.kind {
                SHAPE_SPHERE => sh.radius,
                SHAPE_BOX => length(sh.half),
                _ => sh.half.y + sh.radius,
            });
        }
    }

    /// Sort and sweep along the axis with the largest spread of box
    /// centers. Pairs come out sorted by `(lo, hi)` body index.
    fn broad_phase(&mut self) {
        self.pairs.clear();
        let n = self.sorted.len();
        if self.movers.is_empty() || n < 2 {
            return;
        }
        // Axis of largest variance (ties: lowest axis).
        let mut best = (0usize, i128::MIN);
        for axis in 0..3 {
            let (mut s, mut s2) = (0i128, 0i128);
            for b in &self.sorted {
                let c = ((b.min[axis] + b.max[axis]) >> 1) as i128 >> 4;
                s += c;
                s2 += c * c;
            }
            let var = s2 * n as i128 - s * s;
            if var > best.1 {
                best = (axis, var);
            }
        }
        let ax = best.0;
        let (ay, az) = ((ax + 1) % 3, (ax + 2) % 3);
        self.keys.clear();
        for b in &self.sorted {
            self.keys.push((b.min[ax], b.idx));
        }
        self.keys.sort_unstable();
        for oi in 0..n {
            let a = self.sorted[self.keys[oi].1 as usize];
            for &(kmin, bi) in &self.keys[oi + 1..] {
                if kmin > a.max[ax] {
                    break;
                }
                let b = &self.sorted[bi as usize];
                let f = a.flags | b.flags;
                if (f & 6) == 6
                    && b.min[ay] <= a.max[ay]
                    && b.max[ay] >= a.min[ay]
                    && b.min[az] <= a.max[az]
                    && b.max[az] >= a.min[az]
                    && a.layer & b.mask != 0
                    && b.layer & a.mask != 0
                {
                    let (lo, hi) = if a.idx < b.idx { (a.idx, b.idx) } else { (b.idx, a.idx) };
                    self.pairs.push(pair_key(lo, hi));
                }
            }
        }
        self.pairs.sort_unstable();
    }

    fn inv_mass_of(&self, i: usize, bd: &[Body]) -> bool {
        self.flags[i] & (F_DYN | F_ASLEEP) == F_DYN && bd[self.bslot[i] as usize].inv_mass > FP::ZERO
    }

    fn narrow_phase(&mut self, cd: &[Collider], old_cache: &[ContactCache], margin: FP) {
        self.cons.clear();
        self.pool.clear();
        let mut ci = 0usize;
        for &key in &self.pairs {
            let (lo, hi) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            let reach = self.brad[lo] + self.brad[hi] + margin;
            if (self.xfs[hi].p - self.xfs[lo].p).length_sq() > reach * reach {
                continue;
            }
            let (ca, cb) = (&cd[self.cslot[lo] as usize], &cd[self.cslot[hi] as usize]);
            let (mut m, flip) = if ca.shape.kind <= cb.shape.kind {
                (collide(&ca.shape, &self.xfs[lo], &cb.shape, &self.xfs[hi], margin), false)
            } else {
                (collide(&cb.shape, &self.xfs[hi], &ca.shape, &self.xfs[lo], margin), true)
            };
            if m.count == 0 {
                continue;
            }
            if flip {
                m.normal = -m.normal;
            }
            let con = Constraint {
                a: lo as u32,
                b: hi as u32,
                normal: m.normal,
                friction: mix_friction(ca.friction, cb.friction),
                restitution: ca.restitution.max(cb.restitution),
                count: m.count as u32,
                first: self.pool.len() as u32,
                ..Constraint::default()
            };
            for k in 0..m.count {
                let mp = m.pts[k];
                self.pool.push(ContactPt { point: mp.point, sep: mp.sep, id: mp.id, ..ContactPt::default() });
            }
            let first = con.first as usize;
            // Warm start: `old_cache` and the pairs are both sorted by
            // (entity a, entity b, id), so one forward walk finds every
            // entry.
            let (ea, eb) = (self.ents[lo].index, self.ents[hi].index);
            while ci < old_cache.len() && (old_cache[ci].a, old_cache[ci].b) < (ea, eb) {
                ci += 1;
            }
            let mut cj = ci;
            for k in 0..con.count as usize {
                let id = self.pool[first + k].id;
                while cj < old_cache.len() && (old_cache[cj].a, old_cache[cj].b) == (ea, eb) && old_cache[cj].id < id {
                    cj += 1;
                }
                if cj < old_cache.len() && (old_cache[cj].a, old_cache[cj].b, old_cache[cj].id) == (ea, eb, id) {
                    self.pool[first + k].jn = old_cache[cj].normal_impulse;
                    self.pool[first + k].jt = old_cache[cj].tangent_impulse;
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

    /// Damping and clamps for the movers; world inverse inertia for every
    /// body. Gravity is added per substep.
    fn integrate_velocities(&mut self, bd: &[Body], cfg: &PhysicsConfig) {
        let n = self.ents.len();
        self.vw.clear();
        self.vw.resize(n, Vw::default());
        self.binv.clear();
        for i in 0..n {
            let b = &bd[self.bslot[i] as usize];
            let mut bi = BodyInv { pos: b.pos, inv_mass: FP::ZERO, inv_inertia: FPMat3::ZERO };
            if self.inv_mass_of(i, bd) {
                bi.inv_mass = b.inv_mass;
                bi.inv_inertia = self.xfs[i].r.rotate_diag(b.inv_inertia);
            }
            self.binv.push(bi);
        }
        let (max_v, max_w) = (cfg.max_linear_speed, cfg.max_angular_speed);
        for &i in &self.movers {
            let i = i as usize;
            let b = &bd[self.bslot[i] as usize];
            let (mut v, mut w) = (b.vel, b.omega);
            if self.flags[i] & F_DYN != 0 {
                if b.linear_damping != FP::ZERO {
                    v = v * (FP::ONE - b.linear_damping * cfg.dt).max(FP::ZERO);
                }
                if b.angular_damping != FP::ZERO {
                    w = w * (FP::ONE - b.angular_damping * cfg.dt).max(FP::ZERO);
                }
                v = clamp_vec(v, max_v);
                w = clamp_vec(w, max_w);
            }
            self.vw[i] = Vw { v, w };
        }
    }

    /// The substep loop: gravity, targets from the current separations,
    /// warm start, sequential impulses, then move the (workspace) positions.
    fn run_substeps(&mut self, cfg: &PhysicsConfig) {
        let subs = cfg.substeps.max(1);
        // The substeps add up to exactly `dt`: the first `rem` are one raw
        // unit longer.
        let (base, rem) = (cfg.dt.raw() / subs as i64, cfg.dt.raw() % subs as i64);
        self.pw.clear();
        self.pw.resize(self.ents.len(), Vw::default());
        for sub in 0..subs {
            let h = FP::from_raw(base + i64::from((sub as i64) < rem));
            let g = cfg.gravity * h;
            for &i in &self.movers {
                let i = i as usize;
                if self.flags[i] & F_DYN != 0 {
                    self.vw[i].v += g;
                }
            }
            if !self.cons.is_empty() {
                solver::update_targets(&self.cons, &mut self.pool, &self.pw, cfg, h);
                solver::warm_start(&self.cons, &self.pool, &mut self.vw);
                solver::solve(&mut self.cons, &mut self.pool, &mut self.vw, cfg.velocity_iterations);
            }
            for &i in &self.movers {
                let i = i as usize;
                let (v, w) = (self.vw[i].v, self.vw[i].w);
                let p = &mut self.pw[i];
                p.v += FPVec3::new(mul_round(v.x, h), mul_round(v.y, h), mul_round(v.z, h));
                p.w += FPVec3::new(mul_round(w.x, h), mul_round(w.y, h), mul_round(w.z, h));
            }
        }
        solver::store_friction(&self.cons, &mut self.pool);
    }

    /// Keeps a badly over-constrained pile from producing runaway
    /// velocities.
    fn clamp_velocities(&mut self, cfg: &PhysicsConfig) {
        let (max_v, max_w) = (cfg.max_linear_speed, cfg.max_angular_speed);
        for &i in &self.movers {
            let i = i as usize;
            if self.flags[i] & F_DYN != 0 {
                let x = self.vw[i];
                self.vw[i] = Vw { v: clamp_vec(x.v, max_v), w: clamp_vec(x.w, max_w) };
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
        let lin2 = cfg.sleep_linear_speed * cfg.sleep_linear_speed;
        let ang2 = cfg.sleep_angular_speed * cfg.sleep_angular_speed;
        let mut candidates = false;
        for &i in &self.movers {
            let i = i as usize;
            let mut t = 0;
            if enabled && self.flags[i] & F_DYN != 0 {
                let x = self.vw[i];
                let still = x.v.length_sq() <= lin2 && x.w.length_sq() <= ang2;
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
            for p in &self.pool[c.first as usize..(c.first + c.count) as usize] {
                self.new_cache.push(ContactCache { a: ea, b: eb, id: p.id, _pad: 0, normal_impulse: p.jn, tangent_impulse: p.jt });
            }
        }
        self.new_cache.extend_from_slice(&self.carried[ci..]);
    }
}

/// Same bits as `FPQuat::integrate_angular`, with the faster square root
/// and division of [`crate::fastmath`] (checked equal by a test).
pub(crate) fn integrate_rot(q: FPQuat, omega: FPVec3, dt: FP) -> FPQuat {
    let (wx, wy, wz) = (omega.x.raw() as i128, omega.y.raw() as i128, omega.z.raw() as i128);
    let (qx, qy, qz, qw) = (q.x.raw() as i128, q.y.raw() as i128, q.z.raw() as i128, q.w.raw() as i128);
    let h = dt.raw() as i128;
    let r = |v: i128| FP::from_raw(((v * h + (1 << 32)) >> 33) as i64);
    let x = q.x + r(wx * qw + wy * qz - wz * qy);
    let y = q.y + r(-wx * qz + wy * qw + wz * qx);
    let z = q.z + r(wx * qy - wy * qx + wz * qw);
    let w = q.w + r(-wx * qx - wy * qy - wz * qz);
    let len = sqrt(x * x + y * y + z * z + w * w);
    if len.raw() == 0 {
        FPQuat::IDENTITY
    } else {
        FPQuat::new(div(x, len), div(y, len), div(z, len), div(w, len))
    }
}

#[inline]
fn clamp_vec(v: FPVec3, m: FP) -> FPVec3 {
    FPVec3::new(v.x.clamp(-m, m), v.y.clamp(-m, m), v.z.clamp(-m, m))
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let p = parent[x as usize];
        parent[x as usize] = parent[p as usize];
        x = parent[x as usize];
    }
    x
}

/// Advances the physics state in `frame` by one fixed step. Panics if
/// [`crate::init`] was not called on this frame.
pub fn step(frame: &mut Frame, sc: &mut Scratch) {
    step_probed(frame, sc, &mut |_| {});
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
pub fn step_probed(frame: &mut Frame, sc: &mut Scratch, probe: &mut impl FnMut(Phase)) {
    let st = *frame.singleton::<PhysicsState>();
    let cfg = st.config;
    assert!(cfg.dt.raw() > 0, "orr_physics3d: call orr_physics3d::init before step");

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
        if sc.movers.is_empty() {
            // Nothing moves: no contacts to solve, no state to change. The
            // cache of the sleeping pairs stays as it is.
            sc.cons.clear();
            sc.pool.clear();
            sc.pairs.clear();
            sc.mslept.clear();
            sc.asleep_end = sc.flags.iter().filter(|&&f| f & F_ASLEEP != 0).count() as u32;
            probe(Phase::Finish);
            return;
        }
        sc.build_transforms(bd, cd, cfg.contact_margin);
        probe(Phase::Transforms);

        // Solid contacts (warm-started from last tick's cache). A contact
        // between a moving body and a sleeping one wakes the sleeper's
        // island, and then the contacts are rebuilt with the island awake.
        let mut rounds = 0;
        loop {
            // The wake flags may have changed the sweep flags.
            for (b, &f) in sc.sorted.iter_mut().zip(&sc.flags) {
                b.flags = (((f & F_DYN != 0) as u32) << 1) | (((f & F_ACTIVE != 0) as u32) << 2);
            }
            sc.broad_phase();
            probe(Phase::Broad);
            sc.narrow_phase(cd, old_cache, cfg.contact_margin);
            probe(Phase::Narrow);
            if sc.wake_touching(bd) && rounds < MAX_WAKE_ROUNDS {
                rounds += 1;
                continue;
            }
            break;
        }

        // Dynamics.
        sc.integrate_velocities(bd, &cfg);
        probe(Phase::Integrate);
        solver::prepare(&mut sc.cons, &mut sc.pool, &sc.binv, &sc.vw, &cfg);
        probe(Phase::Prepare);
        sc.run_substeps(&cfg);
        sc.clamp_velocities(&cfg);
        probe(Phase::Solve);
        sc.update_sleep(bd, &cfg);
        probe(Phase::Sleep);

        let asleep_now = sc.flags.iter().filter(|&&f| f & F_ASLEEP != 0).count() as u32;
        sc.build_cache(old_cache, asleep_now > 0);
        sc.asleep_end = asleep_now + sc.mslept.iter().filter(|&&x| x != 0).count() as u32;
    }

    // Position integration and write-back, for the bodies that moved.
    let limit = cfg.sleep_ticks.min(SLEEP_FLAG - 1);
    for k in 0..sc.movers.len() {
        let i = sc.movers[k] as usize;
        let dynamic = sc.flags[i] & F_DYN != 0;
        let Some(b) = frame.get_mut::<Body>(sc.ents[i]) else { continue };
        if sc.mslept[k] != 0 {
            b.sleep = SLEEP_FLAG | limit;
            b.island = sc.mslept[k];
            b.vel = FPVec3::ZERO;
            b.omega = FPVec3::ZERO;
            continue;
        }
        let (v, w) = (sc.vw[i].v, sc.vw[i].w);
        b.pos += sc.pw[i].v;
        if sc.pw[i].w != FPVec3::ZERO {
            b.rot = integrate_rot(b.rot, sc.pw[i].w, FP::ONE);
        }
        b.vel = v;
        b.omega = w;
        if dynamic {
            b.sleep = sc.mtimer[k];
            b.island = 0;
        }
    }

    // Persist the warm-starting cache (sorted by (a, b, id)).
    store_list(frame, st.contacts, &sc.new_cache);
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
