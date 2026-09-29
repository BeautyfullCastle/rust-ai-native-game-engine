//! The physics tick: broad phase, narrow phase, sequential impulse solver,
//! integration and trigger bookkeeping.
//!
//! Everything derived during a tick lives in [`Scratch`] and is rebuilt
//! from the `Frame` at the start of the next tick. The only state that
//! survives a tick is in the `Frame` itself (bodies, the warm-starting
//! cache and the trigger overlap set), so rollback restores it exactly.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};

use crate::collide::{collide, overlap, Xf};
use crate::types::{
    Body, Collider, ContactCache, OverlapPair, PhysicsConfig, PhysicsState, TriggerEvent, BODY_DYNAMIC, BODY_STATIC,
    SHAPE_CIRCLE, TRIGGER_ENTER, TRIGGER_EXIT,
};

#[derive(Clone, Copy, Default)]
struct Aabb {
    min: FPVec2,
    max: FPVec2,
}

#[derive(Clone, Copy, Default)]
struct ContactPt {
    point: FPVec2,
    sep: FP,
    id: u32,
    ra: FPVec2,
    rb: FPVec2,
    normal_mass: FP,
    tangent_mass: FP,
    target: FP,
    jn: FP,
    jt: FP,
}

/// Linear and angular velocity of one body during the solve.
#[derive(Clone, Copy, Default)]
struct Vw {
    v: FPVec2,
    w: FP,
}

#[derive(Clone, Copy, Default)]
struct Constraint {
    a: u32,
    b: u32,
    ima: FP,
    iia: FP,
    imb: FP,
    iib: FP,
    normal: FPVec2,
    friction: FP,
    restitution: FP,
    count: usize,
    pts: [ContactPt; 2],
}

/// Reusable per-tick working memory. Holds no state between ticks: every
/// buffer is cleared and refilled from the `Frame` on each [`step`], so a
/// `Scratch` can be shared across rollbacks and resimulations.
#[derive(Default)]
pub struct Scratch {
    ents: Vec<Entity>,
    bodies: Vec<Body>,
    cols: Vec<Collider>,
    xfs: Vec<Xf>,
    aabbs: Vec<Aabb>,
    order: Vec<u32>,
    pairs: Vec<u64>,
    sensor_pairs: Vec<u64>,
    cons: Vec<Constraint>,
    vw: Vec<Vw>,
    new_cache: Vec<ContactCache>,
    new_overlaps: Vec<OverlapPair>,
}

#[inline]
fn cross_sv(s: FP, v: FPVec2) -> FPVec2 {
    FPVec2::new(-(s * v.y), s * v.x)
}

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

    fn gather(&mut self, frame: &mut Frame) {
        self.ents.clear();
        self.bodies.clear();
        self.cols.clear();
        for (e, (b, c)) in frame.query::<(&Body, &Collider)>() {
            self.ents.push(e);
            self.bodies.push(*b);
            self.cols.push(*c);
        }
        // Canonical order: ascending entity index, independent of the
        // sparse-set dense order (which depends on despawn history).
        if !self.ents.windows(2).all(|w| w[0].index < w[1].index) {
            let mut idx: Vec<u32> = (0..self.ents.len() as u32).collect();
            idx.sort_unstable_by_key(|&i| self.ents[i as usize].index);
            self.ents = idx.iter().map(|&i| self.ents[i as usize]).collect();
            self.bodies = idx.iter().map(|&i| self.bodies[i as usize]).collect();
            self.cols = idx.iter().map(|&i| self.cols[i as usize]).collect();
        }
        for b in &mut self.bodies {
            if b.kind != BODY_DYNAMIC {
                b.inv_mass = FP::ZERO;
                b.inv_inertia = FP::ZERO;
            }
        }
    }

    fn build_transforms(&mut self, margin: FP) {
        let n = self.ents.len();
        self.xfs.clear();
        self.aabbs.clear();
        for i in 0..n {
            let b = &self.bodies[i];
            let xf = if b.kind == BODY_STATIC && b.angle == FP::ZERO {
                Xf { p: b.pos, c: FP::ONE, s: FP::ZERO }
            } else {
                Xf::new(b.pos, b.angle)
            };
            let sh = &self.cols[i].shape;
            let mut bb = if sh.kind == SHAPE_CIRCLE {
                Aabb { min: b.pos - FPVec2::splat(sh.radius), max: b.pos + FPVec2::splat(sh.radius) }
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
        }
    }

    /// Sort-and-sweep along x. Emits canonical `(lo, hi)` pairs sorted
    /// ascending; ties never depend on memory layout because the sort key
    /// ends with the unique body index.
    fn broad_phase(&mut self) {
        let n = self.ents.len();
        self.pairs.clear();
        self.sensor_pairs.clear();
        self.order.clear();
        self.order.extend(0..n as u32);
        let aabbs = &self.aabbs;
        self.order.sort_unstable_by_key(|&i| (aabbs[i as usize].min.x.raw(), i));
        for oi in 0..n {
            let a = self.order[oi] as usize;
            let amax_x = self.aabbs[a].max.x;
            let (amin_y, amax_y) = (self.aabbs[a].min.y, self.aabbs[a].max.y);
            for oj in oi + 1..n {
                let b = self.order[oj] as usize;
                if self.aabbs[b].min.x > amax_x {
                    break;
                }
                if self.aabbs[b].min.y > amax_y || self.aabbs[b].max.y < amin_y {
                    continue;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let (ca, cb) = (&self.cols[lo], &self.cols[hi]);
                if ca.layer & cb.mask == 0 || cb.layer & ca.mask == 0 {
                    continue;
                }
                let key = pair_key(lo as u32, hi as u32);
                if ca.is_sensor() || cb.is_sensor() {
                    self.sensor_pairs.push(key);
                } else if self.bodies[lo].inv_mass > FP::ZERO || self.bodies[hi].inv_mass > FP::ZERO {
                    self.pairs.push(key);
                }
            }
        }
        self.pairs.sort_unstable();
        self.sensor_pairs.sort_unstable();
    }

    fn narrow_phase(&mut self, old_cache: &[ContactCache], margin: FP) {
        self.cons.clear();
        for &key in &self.pairs {
            let (lo, hi) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            let (ca, cb) = (&self.cols[lo], &self.cols[hi]);
            let m = collide(&ca.shape, &self.xfs[lo], &cb.shape, &self.xfs[hi], margin);
            if m.count == 0 {
                continue;
            }
            let mut con = Constraint {
                a: lo as u32,
                b: hi as u32,
                ima: self.bodies[lo].inv_mass,
                iia: self.bodies[lo].inv_inertia,
                imb: self.bodies[hi].inv_mass,
                iib: self.bodies[hi].inv_inertia,
                normal: m.normal,
                friction: mix_friction(ca.friction, cb.friction),
                restitution: ca.restitution.max(cb.restitution),
                count: m.count,
                pts: [ContactPt::default(); 2],
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
            let (ea, eb) = (self.ents[lo].index, self.ents[hi].index);
            for k in 0..con.count {
                let id = con.pts[k].id;
                if let Ok(pos) = old_cache.binary_search_by(|c| (c.a, c.b, c.id).cmp(&(ea, eb, id))) {
                    con.pts[k].jn = old_cache[pos].normal_impulse;
                    con.pts[k].jt = old_cache[pos].tangent_impulse;
                }
            }
            self.cons.push(con);
        }
    }

    fn integrate_velocities(&mut self, cfg: &PhysicsConfig) {
        let n = self.ents.len();
        self.vw.clear();
        let g = cfg.gravity * cfg.dt;
        let max_v = cfg.max_linear_speed;
        let max_w = cfg.max_angular_speed;
        for i in 0..n {
            let b = &self.bodies[i];
            let (mut v, mut w) = (b.vel, b.omega);
            if b.kind == BODY_DYNAMIC && b.inv_mass > FP::ZERO {
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
            self.vw.push(Vw { v, w });
        }
    }

    /// Keeps a badly over-constrained pile (e.g. a body wedged between two
    /// immovable shapes) from producing runaway velocities.
    fn clamp_velocities(&mut self, cfg: &PhysicsConfig) {
        let (max_v, max_w) = (cfg.max_linear_speed, cfg.max_angular_speed);
        for i in 0..self.ents.len() {
            if self.bodies[i].inv_mass > FP::ZERO {
                let x = self.vw[i];
                self.vw[i] = Vw {
                    v: FPVec2::new(x.v.x.clamp(-max_v, max_v), x.v.y.clamp(-max_v, max_v)),
                    w: x.w.clamp(-max_w, max_w),
                };
            }
        }
    }

    fn prepare_and_warm_start(&mut self, cfg: &PhysicsConfig) {
        let inv_dt = FP::ONE / cfg.dt;
        for c in &mut self.cons {
            let (a, b) = (c.a as usize, c.b as usize);
            let (pa, pb) = (self.bodies[a].pos, self.bodies[b].pos);
            let n = c.normal;
            let t = FPVec2::new(n.y, -n.x);
            let (xa, xb) = (self.vw[a], self.vw[b]);
            let (mut va, mut wa, mut vb, mut wb) = (xa.v, xa.w, xb.v, xb.w);
            let (ima, iia, imb, iib) = (c.ima, c.iia, c.imb, c.iib);
            for k in 0..c.count {
                let p = &mut c.pts[k];
                p.ra = p.point - pa;
                p.rb = p.point - pb;
                let rna = p.ra.perp_dot(n);
                let rnb = p.rb.perp_dot(n);
                let kn = ima + imb + iia * rna * rna + iib * rnb * rnb;
                p.normal_mass = if kn > FP::ZERO { FP::ONE / kn } else { FP::ZERO };
                let rta = p.ra.perp_dot(t);
                let rtb = p.rb.perp_dot(t);
                let kt = ima + imb + iia * rta * rta + iib * rtb * rtb;
                p.tangent_mass = if kt > FP::ZERO { FP::ONE / kt } else { FP::ZERO };

                let dv = vb + cross_sv(wb, p.rb) - va - cross_sv(wa, p.ra);
                let vn0 = dv.dot(n);
                let bias = if p.sep >= FP::ZERO {
                    p.sep * inv_dt
                } else {
                    (cfg.baumgarte * (p.sep + cfg.linear_slop).min(FP::ZERO) * inv_dt).max(-cfg.max_correction_speed)
                };
                let mut target = -bias;
                if p.sep <= FP::ZERO && vn0 < -cfg.restitution_threshold {
                    target = target.max(-(c.restitution * vn0));
                }
                p.target = target;

                // Warm start.
                let imp = n * p.jn + t * p.jt;
                va -= imp * ima;
                wa -= iia * p.ra.perp_dot(imp);
                vb += imp * imb;
                wb += iib * p.rb.perp_dot(imp);
            }
            self.vw[a] = Vw { v: va, w: wa };
            self.vw[b] = Vw { v: vb, w: wb };
        }
    }

    fn solve(&mut self, iterations: u32) {
        for _ in 0..iterations {
            for c in &mut self.cons {
                let (a, b) = (c.a as usize, c.b as usize);
                let n = c.normal;
                let t = FPVec2::new(n.y, -n.x);
                let (xa, xb) = (self.vw[a], self.vw[b]);
                let (mut va, mut wa, mut vb, mut wb) = (xa.v, xa.w, xb.v, xb.w);
                let (ima, iia, imb, iib) = (c.ima, c.iia, c.imb, c.iib);
                for k in 0..c.count {
                    let p = &mut c.pts[k];
                    // Friction.
                    let dv = vb + cross_sv(wb, p.rb) - va - cross_sv(wa, p.ra);
                    let vt = dv.dot(t);
                    let max_f = c.friction * p.jn;
                    let new_jt = (p.jt - p.tangent_mass * vt).clamp(-max_f, max_f);
                    let lam = new_jt - p.jt;
                    p.jt = new_jt;
                    let imp = t * lam;
                    va -= imp * ima;
                    wa -= iia * p.ra.perp_dot(imp);
                    vb += imp * imb;
                    wb += iib * p.rb.perp_dot(imp);
                }
                for k in 0..c.count {
                    let p = &mut c.pts[k];
                    // Non-penetration.
                    let dv = vb + cross_sv(wb, p.rb) - va - cross_sv(wa, p.ra);
                    let vn = dv.dot(n);
                    let new_jn = (p.jn + p.normal_mass * (p.target - vn)).max(FP::ZERO);
                    let lam = new_jn - p.jn;
                    p.jn = new_jn;
                    let imp = n * lam;
                    va -= imp * ima;
                    wa -= iia * p.ra.perp_dot(imp);
                    vb += imp * imb;
                    wb += iib * p.rb.perp_dot(imp);
                }
                self.vw[a] = Vw { v: va, w: wa };
                self.vw[b] = Vw { v: vb, w: wb };
            }
        }
    }
}

/// Advances the physics state in `frame` by one fixed step.
///
/// Trigger enter/exit events for this tick are appended to `events`.
/// Panics if [`crate::init`] was not called on this frame.
pub fn step(frame: &mut Frame, sc: &mut Scratch, events: &mut Vec<TriggerEvent>) {
    let st = *frame.singleton::<PhysicsState>();
    let cfg = st.config;
    assert!(cfg.dt.raw() > 0, "orr_physics: call orr_physics::init before step");

    sc.gather(frame);
    sc.build_transforms(cfg.contact_margin);
    sc.broad_phase();

    // Solid contacts (warm-started from last tick's cache).
    sc.narrow_phase(frame.list(st.contacts), cfg.contact_margin);

    // Trigger overlaps.
    sc.new_overlaps.clear();
    for &key in &sc.sensor_pairs {
        let (lo, hi) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
        if overlap(&sc.cols[lo].shape, &sc.xfs[lo], &sc.cols[hi].shape, &sc.xfs[hi]) {
            sc.new_overlaps.push(OverlapPair { a: sc.ents[lo], b: sc.ents[hi] });
        }
    }
    diff_overlaps(frame.list(st.overlaps), &sc.new_overlaps, events);

    // Dynamics.
    sc.integrate_velocities(&cfg);
    sc.prepare_and_warm_start(&cfg);
    sc.solve(cfg.velocity_iterations);
    sc.clamp_velocities(&cfg);

    // Position integration and write-back.
    // `FP::TWO_PI` is one raw unit above `2 * FP::PI`; wrapping by it can
    // land exactly on `-PI`, which `FP::sin_cos` cannot fold.
    let pi = FP::PI;
    let two_pi = FP::PI + FP::PI;
    for i in 0..sc.ents.len() {
        let b = &sc.bodies[i];
        if b.kind == BODY_STATIC {
            continue;
        }
        let (v, w) = (sc.vw[i].v, sc.vw[i].w);
        let mut angle = b.angle + w * cfg.dt;
        if angle > pi {
            angle -= two_pi;
        } else if angle <= -pi {
            angle += two_pi;
        }
        let pos = b.pos + v * cfg.dt;
        if let Some(dst) = frame.get_mut::<Body>(sc.ents[i]) {
            dst.pos = pos;
            dst.angle = angle;
            dst.vel = v;
            dst.omega = w;
        }
    }

    // Persist the warm-starting cache (already sorted by (a, b, id)).
    sc.new_cache.clear();
    for c in &sc.cons {
        let (ea, eb) = (sc.ents[c.a as usize].index, sc.ents[c.b as usize].index);
        for p in &c.pts[..c.count] {
            sc.new_cache.push(ContactCache {
                a: ea,
                b: eb,
                id: p.id,
                _pad: 0,
                normal_impulse: p.jn,
                tangent_impulse: p.jt,
            });
        }
    }
    frame.list_clear(st.contacts);
    for c in &sc.new_cache {
        frame.list_push(st.contacts, *c);
    }
    frame.list_clear(st.overlaps);
    for o in &sc.new_overlaps {
        frame.list_push(st.overlaps, *o);
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
