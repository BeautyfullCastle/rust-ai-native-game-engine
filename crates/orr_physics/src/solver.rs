//! Sequential impulse contact solver.
//!
//! Per contact point, `prepare` precomputes everything that does not
//! change during the iterations: the cross products `r x n` and `r x t`,
//! those products scaled by the inverse inertias, the effective masses and
//! the restitution/bias target. An iteration row then needs one fused
//! 4-term dot product for the relative velocity and a few multiplies to
//! apply the impulse. All arithmetic is integer `FP` math, in 64 bits (see
//! [`nmul`]); the dot products round once instead of once per term.

use orr_fp::{FPVec2, FP};

use crate::fastmath::{self, nmul};
use crate::collide::Xf;
use crate::types::PhysicsConfig;

/// Linear and angular velocity of one body during the solve.
#[derive(Clone, Copy, Default)]
pub(crate) struct Vw {
    pub v: FPVec2,
    pub w: FP,
}

/// One contact point of a manifold.
#[derive(Clone, Copy, Default)]
pub(crate) struct ContactPt {
    pub point: FPVec2,
    pub sep: FP,
    pub id: u32,
    /// Accumulated normal impulse (warm-start value before the solve).
    pub jn: FP,
    /// Accumulated friction impulse.
    pub jt: FP,
    // Derived by `prepare`.
    rna: FP,
    rnb: FP,
    rta: FP,
    rtb: FP,
    wna: FP,
    wnb: FP,
    wta: FP,
    wtb: FP,
    normal_mass: FP,
    tangent_mass: FP,
    target: FP,
}

/// A manifold between bodies `a` and `b` (indices into the gathered
/// arrays, `a < b`), normal pointing from `a` to `b`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Constraint {
    pub a: u32,
    pub b: u32,
    pub count: u32,
    pub ima: FP,
    pub iia: FP,
    pub imb: FP,
    pub iib: FP,
    pub normal: FPVec2,
    pub friction: FP,
    pub restitution: FP,
    pub pts: [ContactPt; 2],
    // Derived by `prepare`: the normal scaled by the inverse masses.
    pub nax: FP,
    pub nay: FP,
    pub nbx: FP,
    pub nby: FP,
}

/// `(a0*b0 + a1*b1 + a2*b2 + a3*b3) >> 16` with one rounding, in 64 bits
/// (see [`nmul`] for the range assumption; the sum is checked the same way).
#[inline(always)]
fn dot4(a: [FP; 4], b: [FP; 4]) -> FP {
    let [a0, a1, a2, a3] = a.map(FP::raw);
    let [b0, b1, b2, b3] = b.map(FP::raw);
    debug_assert!(
        a0.checked_mul(b0)
            .zip(a1.checked_mul(b1))
            .zip(a2.checked_mul(b2).zip(a3.checked_mul(b3)))
            .and_then(|((p0, p1), (p2, p3))| p0.checked_add(p1)?.checked_add(p2)?.checked_add(p3))
            .is_some(),
        "orr_physics: solver dot product out of range"
    );
    let s = a0.wrapping_mul(b0).wrapping_add(a1.wrapping_mul(b1)).wrapping_add(a2.wrapping_mul(b2)).wrapping_add(a3.wrapping_mul(b3));
    FP::from_raw(s >> 16)
}

#[inline(always)]
fn neg(x: FP) -> FP {
    FP::from_raw(-x.raw())
}

/// Fills the derived fields, warm starts and returns nothing; velocities
/// in `vw` are updated by the warm start impulses.
pub(crate) fn prepare(cons: &mut [Constraint], xfs: &[Xf], vw: &mut [Vw], cfg: &PhysicsConfig) {
    let inv_dt = fastmath::div(FP::ONE, cfg.dt);
    for c in cons.iter_mut() {
        let (a, b) = (c.a as usize, c.b as usize);
        let (pa, pb) = (xfs[a].p, xfs[b].p);
        let (nx, ny) = (c.normal.x, c.normal.y);
        let (ima, iia, imb, iib) = (c.ima, c.iia, c.imb, c.iib);
        c.nax = nmul(nx, ima);
        c.nay = nmul(ny, ima);
        c.nbx = nmul(nx, imb);
        c.nby = nmul(ny, imb);
        let (mut va, mut vb) = (vw[a], vw[b]);
        for k in 0..c.count as usize {
            let p = &mut c.pts[k];
            let ra = p.point - pa;
            let rb = p.point - pb;
            p.rna = nmul(ra.x, ny) - nmul(ra.y, nx);
            p.rnb = nmul(rb.x, ny) - nmul(rb.y, nx);
            // ra x t = -(ra . n) with t = (ny, -nx).
            p.rta = neg(nmul(ra.x, nx) + nmul(ra.y, ny));
            p.rtb = neg(nmul(rb.x, nx) + nmul(rb.y, ny));
            p.wna = nmul(iia, p.rna);
            p.wnb = nmul(iib, p.rnb);
            p.wta = nmul(iia, p.rta);
            p.wtb = nmul(iib, p.rtb);
            let kn = ima + imb + nmul(p.wna, p.rna) + nmul(p.wnb, p.rnb);
            p.normal_mass = if kn > FP::ZERO { fastmath::div(FP::ONE, kn) } else { FP::ZERO };
            let kt = ima + imb + nmul(p.wta, p.rta) + nmul(p.wtb, p.rtb);
            p.tangent_mass = if kt > FP::ZERO { fastmath::div(FP::ONE, kt) } else { FP::ZERO };

            let vn0 = dot4([vb.v.x - va.v.x, vb.v.y - va.v.y, vb.w, neg(va.w)], [nx, ny, p.rnb, p.rna]);
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

            // Warm start: friction then normal impulse from last tick.
            let (jt, jn) = (p.jt, p.jn);
            va.v.x -= nmul(jt, c.nay) + nmul(jn, c.nax);
            va.v.y += nmul(jt, c.nax) - nmul(jn, c.nay);
            va.w -= nmul(jt, p.wta) + nmul(jn, p.wna);
            vb.v.x += nmul(jt, c.nby) + nmul(jn, c.nbx);
            vb.v.y += nmul(jn, c.nby) - nmul(jt, c.nbx);
            vb.w += nmul(jt, p.wtb) + nmul(jn, p.wnb);
        }
        vw[a] = va;
        vw[b] = vb;
    }
}

/// Runs `iterations` sweeps over all constraints (friction, then normal).
pub(crate) fn solve(cons: &mut [Constraint], vw: &mut [Vw], iterations: u32) {
    for _ in 0..iterations {
        for c in cons.iter_mut() {
            let (a, b) = (c.a as usize, c.b as usize);
            let (mut va, mut vb) = (vw[a], vw[b]);
            let (nx, ny) = (c.normal.x, c.normal.y);
            let (nax, nay, nbx, nby) = (c.nax, c.nay, c.nbx, c.nby);
            let count = c.count as usize;
            let friction = c.friction;
            for p in c.pts[..count].iter_mut() {
                // Friction along t = (ny, -nx).
                let vt = dot4([vb.v.x - va.v.x, va.v.y - vb.v.y, vb.w, neg(va.w)], [ny, nx, p.rtb, p.rta]);
                let max_f = nmul(friction, p.jn);
                let new_jt = (p.jt - nmul(p.tangent_mass, vt)).clamp(-max_f, max_f);
                let lam = new_jt - p.jt;
                p.jt = new_jt;
                va.v.x -= nmul(lam, nay);
                va.v.y += nmul(lam, nax);
                va.w -= nmul(lam, p.wta);
                vb.v.x += nmul(lam, nby);
                vb.v.y -= nmul(lam, nbx);
                vb.w += nmul(lam, p.wtb);
            }
            for p in c.pts[..count].iter_mut() {
                // Non-penetration along n.
                let vn = dot4([vb.v.x - va.v.x, vb.v.y - va.v.y, vb.w, neg(va.w)], [nx, ny, p.rnb, p.rna]);
                let new_jn = (p.jn + nmul(p.normal_mass, p.target - vn)).max(FP::ZERO);
                let lam = new_jn - p.jn;
                p.jn = new_jn;
                va.v.x -= nmul(lam, nax);
                va.v.y -= nmul(lam, nay);
                va.w -= nmul(lam, p.wna);
                vb.v.x += nmul(lam, nbx);
                vb.v.y += nmul(lam, nby);
                vb.w += nmul(lam, p.wnb);
            }
            vw[a] = va;
            vw[b] = vb;
        }
    }
}
