//! Sequential impulse contact solver (normal plus two friction
//! directions, warm started).
//!
//! `prepare` precomputes everything that is constant during the iterations:
//! for each contact point and each of the three directions (normal and two
//! tangents) the vectors `r x d` and `I^-1 (r x d)` of both bodies and the
//! effective mass. An iteration row is then a 9-term dot product for the
//! relative velocity and a few vector updates. All arithmetic is integer
//! `FP` math in 64 bits (see [`nmul`] for the range assumption).

use orr_fp::{FPMat3, FPVec3, FP};

use crate::fastmath::{self, nmul, rshift, unit};
use crate::types::PhysicsConfig;

/// Linear and angular velocity of one body during the solve.
#[derive(Clone, Copy, Default)]
pub(crate) struct Vw {
    pub v: FPVec3,
    pub w: FPVec3,
}

/// One direction of one contact point: the lever arm terms and the
/// effective mass.
#[derive(Clone, Copy, Default)]
pub(crate) struct Row {
    /// `ra x d` and `rb x d`.
    pub(crate) rad: FPVec3,
    pub(crate) rbd: FPVec3,
    /// `Ia^-1 (ra x d)` and `Ib^-1 (rb x d)`.
    pub(crate) wad: FPVec3,
    pub(crate) wbd: FPVec3,
    pub(crate) mass: FP,
}

/// One contact point of a manifold.
#[derive(Clone, Copy, Default)]
pub(crate) struct ContactPt {
    pub point: FPVec3,
    pub sep: FP,
    pub id: u32,
    /// Accumulated normal impulse (warm-start value before the solve).
    pub jn: FP,
    /// Accumulated friction impulse along `t1` and `t2` (warm-start value
    /// as a world vector before the solve).
    pub jt: FPVec3,
    // Derived by `prepare`.
    pub(crate) j1: FP,
    pub(crate) j2: FP,
    pub(crate) rows: [Row; 3],
    pub(crate) target: FP,
}

/// A manifold between bodies `a` and `b` (indices into the gathered
/// arrays, `a < b`), normal pointing from `a` to `b`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Constraint {
    pub a: u32,
    pub b: u32,
    pub count: u32,
    pub normal: FPVec3,
    pub friction: FP,
    pub restitution: FP,
    /// Index of the first point in the shared point pool.
    pub first: u32,
    // Derived by `prepare`.
    pub(crate) t1: FPVec3,
    pub(crate) t2: FPVec3,
    pub(crate) ima: FP,
    pub(crate) imb: FP,
}

/// Per-body data the solver needs besides the velocities.
#[derive(Clone, Copy, Default)]
pub(crate) struct BodyInv {
    pub pos: FPVec3,
    pub inv_mass: FP,
    /// World-space inverse inertia tensor.
    pub inv_inertia: FPMat3,
}

#[inline(always)]
fn dot3(a: FPVec3, b: FPVec3) -> i64 {
    a.x.raw().wrapping_mul(b.x.raw()).wrapping_add(a.y.raw().wrapping_mul(b.y.raw())).wrapping_add(a.z.raw().wrapping_mul(b.z.raw()))
}

/// Relative velocity of the contact along a direction `d`:
/// `(vb - va) . d + wb . rbd - wa . rad`, one rounding.
#[inline(always)]
fn rel_vel(va: &Vw, vb: &Vw, d: FPVec3, row: &Row) -> FP {
    let dv = vb.v - va.v;
    let s = dot3(dv, d).wrapping_add(dot3(vb.w, row.rbd)).wrapping_sub(dot3(va.w, row.rad));
    rshift(s)
}

/// Applies the impulse `lam` along `d` to both bodies. A side with zero
/// inverse mass (static, kinematic) is skipped.
#[inline(always)]
fn apply(va: &mut Vw, vb: &mut Vw, d: FPVec3, ima: FP, imb: FP, row: &Row, lam: FP) {
    if ima != FP::ZERO {
        let sa = nmul(ima, lam);
        va.v.x -= nmul(d.x, sa);
        va.v.y -= nmul(d.y, sa);
        va.v.z -= nmul(d.z, sa);
        va.w -= scale(row.wad, lam);
    }
    if imb != FP::ZERO {
        let sb = nmul(imb, lam);
        vb.v.x += nmul(d.x, sb);
        vb.v.y += nmul(d.y, sb);
        vb.v.z += nmul(d.z, sb);
        vb.w += scale(row.wbd, lam);
    }
}

#[inline(always)]
fn scale(v: FPVec3, s: FP) -> FPVec3 {
    FPVec3::new(nmul(v.x, s), nmul(v.y, s), nmul(v.z, s))
}

#[inline(always)]
fn cross(a: FPVec3, b: FPVec3) -> FPVec3 {
    FPVec3::new(nmul(a.y, b.z) - nmul(a.z, b.y), nmul(a.z, b.x) - nmul(a.x, b.z), nmul(a.x, b.y) - nmul(a.y, b.x))
}

#[inline(always)]
fn mat_vec(m: &FPMat3, v: FPVec3) -> FPVec3 {
    FPVec3::new(
        rshift(dot3(m.r0, v)),
        rshift(dot3(m.r1, v)),
        rshift(dot3(m.r2, v)),
    )
}

/// Same bits as `FPVec3::orthonormal_basis`, with the faster square root.
fn basis(n: FPVec3) -> (FPVec3, FPVec3) {
    let t1 = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() {
        FPVec3::new(-n.y, n.x, FP::ZERO)
    } else if n.y.abs() >= n.z.abs() {
        FPVec3::new(FP::ZERO, -n.z, n.y)
    } else {
        FPVec3::new(n.z, FP::ZERO, -n.x)
    };
    let t1 = unit(t1);
    (t1, n.cross(t1))
}

/// Fills the derived fields, warm starts, and updates the velocities in
/// `vw` with the warm start impulses.
pub(crate) fn prepare(cons: &mut [Constraint], pool: &mut [ContactPt], bodies: &[BodyInv], vw: &mut [Vw], cfg: &PhysicsConfig) {
    let inv_dt = fastmath::div(FP::ONE, cfg.dt);
    for c in cons.iter_mut() {
        let (a, b) = (c.a as usize, c.b as usize);
        let (ba, bb) = (&bodies[a], &bodies[b]);
        let (ima, imb) = (ba.inv_mass, bb.inv_mass);
        c.ima = ima;
        c.imb = imb;
        let n = c.normal;
        let (t1, t2) = basis(n);
        c.t1 = t1;
        c.t2 = t2;
        let dirs = [n, t1, t2];
        let (mut va, mut vb) = (vw[a], vw[b]);
        let a_ang = ima > FP::ZERO || va.w != FPVec3::ZERO;
        let b_ang = imb > FP::ZERO || vb.w != FPVec3::ZERO;
        let first = c.first as usize;
        for p in pool[first..first + c.count as usize].iter_mut() {
            let ra = p.point - ba.pos;
            let rb = p.point - bb.pos;
            for (row, &d) in p.rows.iter_mut().zip(&dirs) {
                // A body that neither has inertia nor spins takes no part
                // in the angular terms (static ground: half the work).
                row.rad = if a_ang { cross(ra, d) } else { FPVec3::ZERO };
                row.rbd = if b_ang { cross(rb, d) } else { FPVec3::ZERO };
                row.wad = if ima > FP::ZERO { mat_vec(&ba.inv_inertia, row.rad) } else { FPVec3::ZERO };
                row.wbd = if imb > FP::ZERO { mat_vec(&bb.inv_inertia, row.rbd) } else { FPVec3::ZERO };
                let kk = ima + imb + rshift(dot3(row.wad, row.rad).wrapping_add(dot3(row.wbd, row.rbd)));
                row.mass = if kk > FP::ZERO { fastmath::div(FP::ONE, kk) } else { FP::ZERO };
            }
            let vn0 = rel_vel(&va, &vb, n, &p.rows[0]);
            let bias = if p.sep >= FP::ZERO {
                nmul(p.sep, inv_dt)
            } else {
                nmul(nmul(cfg.baumgarte, (p.sep + cfg.linear_slop).min(FP::ZERO)), inv_dt).max(-cfg.max_correction_speed)
            };
            let mut target = -bias;
            if p.sep <= FP::ZERO && vn0 < -cfg.restitution_threshold {
                target = target.max(-nmul(c.restitution, vn0));
            }
            p.target = target;

            // Warm start: friction vector projected on the new basis, then
            // the normal impulse from last tick.
            p.j1 = rshift(dot3(p.jt, t1));
            p.j2 = rshift(dot3(p.jt, t2));
            let (j1, j2, jn) = (p.j1, p.j2, p.jn);
            if j1 != FP::ZERO {
                apply(&mut va, &mut vb, t1, ima, imb, &p.rows[1], j1);
            }
            if j2 != FP::ZERO {
                apply(&mut va, &mut vb, t2, ima, imb, &p.rows[2], j2);
            }
            if jn != FP::ZERO {
                apply(&mut va, &mut vb, n, ima, imb, &p.rows[0], jn);
            }
        }
        vw[a] = va;
        vw[b] = vb;
    }
}

/// Runs `iterations` sweeps over all constraints (friction, then normal).
/// Odd sweeps walk the constraints backwards, which spreads the
/// information through a stack in both directions.
pub(crate) fn solve(cons: &mut [Constraint], pool: &mut [ContactPt], vw: &mut [Vw], iterations: u32) {
    let n = cons.len();
    for it in 0..iterations {
        for k in 0..n {
            let c = &mut cons[if it & 1 == 0 { k } else { n - 1 - k }];
            let (a, b) = (c.a as usize, c.b as usize);
            let (mut va, mut vb) = (vw[a], vw[b]);
            let (ima, imb) = (c.ima, c.imb);
            let (nrm, t1, t2) = (c.normal, c.t1, c.t2);
            let first = c.first as usize;
            let friction = c.friction;
            let pts = &mut pool[first..first + c.count as usize];
            for p in pts.iter_mut() {
                let max_f = nmul(friction, p.jn);
                if max_f == FP::ZERO && p.j1 == FP::ZERO && p.j2 == FP::ZERO {
                    continue;
                }
                // Both tangent rows read the same velocities: they are
                // nearly decoupled, and this halves the dependency chain.
                let vt1 = rel_vel(&va, &vb, t1, &p.rows[1]);
                let vt2 = rel_vel(&va, &vb, t2, &p.rows[2]);
                let new1 = (p.j1 - nmul(p.rows[1].mass, vt1)).clamp(-max_f, max_f);
                let new2 = (p.j2 - nmul(p.rows[2].mass, vt2)).clamp(-max_f, max_f);
                let (lam1, lam2) = (new1 - p.j1, new2 - p.j2);
                p.j1 = new1;
                p.j2 = new2;
                apply(&mut va, &mut vb, t1, ima, imb, &p.rows[1], lam1);
                apply(&mut va, &mut vb, t2, ima, imb, &p.rows[2], lam2);
            }
            for p in pts.iter_mut() {
                let vn = rel_vel(&va, &vb, nrm, &p.rows[0]);
                let new_jn = (p.jn + nmul(p.rows[0].mass, p.target - vn)).max(FP::ZERO);
                let lam = new_jn - p.jn;
                p.jn = new_jn;
                apply(&mut va, &mut vb, nrm, ima, imb, &p.rows[0], lam);
            }
            vw[a] = va;
            vw[b] = vb;
        }
    }
    // Hand the friction impulses back as world vectors for the cache.
    for c in cons.iter() {
        let (t1, t2) = (c.t1, c.t2);
        let first = c.first as usize;
        for p in pool[first..first + c.count as usize].iter_mut() {
            p.jt = scale(t1, p.j1) + scale(t2, p.j2);
        }
    }
}
