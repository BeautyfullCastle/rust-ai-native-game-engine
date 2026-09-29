//! Exact translation sweeps of convex shapes.
//!
//! Every shape is a *core* (a point, a segment or a convex polygon) grown
//! by a radius. Sweeping a caster along a ray against a target is the same
//! as casting the ray origin against the Minkowski sum of the target core
//! and the mirrored caster core, grown by the sum of both radii. That sum
//! is a convex polygon (at most 16 vertices for two 8-gons) with round
//! corners, and a ray against it has a closed form: one plane per edge,
//! offset by the radius, plus one circle per vertex. There is no
//! iteration, so the result is a pure function of the inputs, and its error
//! is a few raw units (1/65536) from the square roots and divisions.

use orr_fp::{FPVec2, FP};

use crate::collide::Xf;
use crate::geom::closest_on_segment;
use crate::types::{Shape, SHAPE_CAPSULE, SHAPE_CIRCLE};

/// Most core points of one shape (a polygon's vertex count).
pub(crate) const MAX_CORE: usize = crate::types::MAX_POLY_VERTS;
const MAX_HULL: usize = MAX_CORE * MAX_CORE;

/// Core points of `shape` under `xf`: the center of a circle, the two end
/// points of a capsule or the vertices of a polygon.
pub(crate) fn core(shape: &Shape, xf: &Xf) -> ([FPVec2; MAX_CORE], usize) {
    let mut out = [FPVec2::ZERO; MAX_CORE];
    if shape.kind == SHAPE_CIRCLE {
        out[0] = xf.p;
        return (out, 1);
    }
    let n = if shape.kind == SHAPE_CAPSULE { 2 } else { shape.count as usize };
    for (dst, v) in out.iter_mut().zip(&shape.verts[..n]) {
        *dst = xf.apply(*v);
    }
    (out, n)
}

/// A caster: its core points relative to its own origin, and its radius.
pub(crate) struct Caster {
    rel: [FPVec2; MAX_CORE],
    n: usize,
    r: FP,
}

impl Caster {
    pub fn new(shape: &Shape, angle: FP) -> Caster {
        let xf = Xf::new(FPVec2::ZERO, angle);
        let (rel, n) = core(shape, &xf);
        Caster { rel, n, r: shape.radius }
    }
}

/// Result of a sweep against one target.
pub(crate) struct HullHit {
    /// Travel distance along the unit direction.
    pub t: FP,
    /// Unit normal from the target toward the caster.
    pub normal: FPVec2,
    /// The caster already touched the target at `t == 0`.
    pub inside: bool,
}

#[inline]
fn cross(o: FPVec2, a: FPVec2, b: FPVec2) -> i128 {
    let (ax, ay) = ((a.x.raw() - o.x.raw()) as i128, (a.y.raw() - o.y.raw()) as i128);
    let (bx, by) = ((b.x.raw() - o.x.raw()) as i128, (b.y.raw() - o.y.raw()) as i128);
    ax * by - ay * bx
}

/// Convex hull in place (counter-clockwise, no collinear points, no
/// duplicates). Returns the point count: 1 for a point, 2 for a segment.
fn hull(pts: &mut [FPVec2; MAX_HULL], n: usize) -> usize {
    pts[..n].sort_unstable_by_key(|p| (p.x.raw(), p.y.raw()));
    let mut m = 0;
    for i in 0..n {
        if m == 0 || pts[i] != pts[m - 1] {
            pts[m] = pts[i];
            m += 1;
        }
    }
    if m <= 2 {
        return m;
    }
    let mut out = [FPVec2::ZERO; MAX_HULL + 2];
    let mut k = 0;
    for &p in &pts[..m] {
        while k >= 2 && cross(out[k - 2], out[k - 1], p) <= 0 {
            k -= 1;
        }
        out[k] = p;
        k += 1;
    }
    let lower = k + 1;
    for i in (0..m - 1).rev() {
        while k >= lower && cross(out[k - 2], out[k - 1], pts[i]) <= 0 {
            k -= 1;
        }
        out[k] = pts[i];
        k += 1;
    }
    k -= 1;
    pts[..k].copy_from_slice(&out[..k]);
    k
}

fn edge_normal(v: FPVec2, w: FPVec2) -> FPVec2 {
    let e = w - v;
    FPVec2::new(e.y, -e.x).normalize_or_zero()
}

/// Signed distance from `o` to the polygon (or segment, or point) `pts`
/// (negative inside a polygon) and the unit direction away from it.
fn point_vs_hull(pts: &[FPVec2], o: FPVec2) -> (FP, FPVec2) {
    let n = pts.len();
    if n == 1 {
        let dv = o - pts[0];
        let dist = dv.length();
        return (dist, if dist == FP::ZERO { FPVec2::Y } else { dv / dist });
    }
    if n == 2 {
        let dv = o - closest_on_segment(o, pts[0], pts[1]);
        let dist = dv.length();
        return (dist, if dist == FP::ZERO { edge_normal(pts[0], pts[1]) } else { dv / dist });
    }
    let mut best = FP::MIN;
    let mut idx = 0usize;
    let mut nrm = FPVec2::ZERO;
    for i in 0..n {
        let ni = edge_normal(pts[i], pts[(i + 1) % n]);
        let s = ni.dot(o - pts[i]);
        if s > best {
            best = s;
            idx = i;
            nrm = ni;
        }
    }
    if best <= FP::ZERO {
        return (best, nrm);
    }
    let (v1, v2) = (pts[idx], pts[(idx + 1) % n]);
    let corner = if (o - v1).dot(v2 - v1) <= FP::ZERO {
        Some(v1)
    } else if (o - v2).dot(v1 - v2) <= FP::ZERO {
        Some(v2)
    } else {
        None
    };
    match corner {
        Some(v) => {
            let dv = o - v;
            let dist = dv.length();
            (dist, if dist == FP::ZERO { nrm } else { dv / dist })
        }
        None => (best, nrm),
    }
}

/// First hit of the ray `o + d * t` (`d` unit, `t <= max`) with the circle
/// at `c`, for a ray that starts outside it.
fn ray_circle_out(c: FPVec2, r: FP, o: FPVec2, d: FPVec2, max: FP) -> Option<(FP, FPVec2)> {
    let m = o - c;
    let b = m.dot(d);
    if b >= FP::ZERO {
        return None;
    }
    let disc = b * b - (m.dot(m) - r * r);
    if disc < FP::ZERO {
        return None;
    }
    let t = -b - disc.sqrt();
    if t > max {
        return None;
    }
    Some((t, (o + d * t - c) / r))
}

/// Ray against the set of points within `r` of the convex polygon (or
/// segment, or point) `pts`. A start inside reports `t = 0` with the
/// direction that separates the two; a start exactly on the surface counts
/// as a hit only when the ray moves inward. Touching along the way (grazing)
/// is a hit.
pub(crate) fn ray_rounded_hull(pts: &[FPVec2], r: FP, o: FPVec2, d: FPVec2, max: FP) -> Option<HullHit> {
    let n = pts.len();
    if n == 0 {
        return None;
    }
    let (dist, nrm) = point_vs_hull(pts, o);
    if dist < r {
        return Some(HullHit { t: FP::ZERO, normal: nrm, inside: true });
    }
    if dist == r {
        return if d.dot(nrm) < FP::ZERO { Some(HullHit { t: FP::ZERO, normal: nrm, inside: false }) } else { None };
    }
    let mut best: Option<(FP, FPVec2)> = None;
    let mut consider = |t: FP, nl: FPVec2| {
        if best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, nl));
        }
    };
    if n == 1 {
        if r > FP::ZERO {
            if let Some((t, nl)) = ray_circle_out(pts[0], r, o, d, max) {
                consider(t, nl);
            }
        }
    } else {
        for i in 0..n {
            let v = pts[i];
            let w = pts[(i + 1) % n];
            let nrm = edge_normal(v, w);
            let den = nrm.dot(d);
            if den < FP::ZERO {
                let t = (r + nrm.dot(v - o)) / den;
                if t >= FP::ZERO && t <= max {
                    let e = w - v;
                    let s = (o + d * t - v).dot(e);
                    if s >= FP::ZERO && s <= e.dot(e) {
                        consider(t, nrm);
                    }
                }
            }
            if r > FP::ZERO {
                if let Some((t, nl)) = ray_circle_out(v, r, o, d, max) {
                    consider(t, nl);
                }
            }
        }
    }
    best.map(|(t, normal)| HullHit { t, normal, inside: false })
}

/// Sweeps `caster` from its origin `o` along the unit direction `d` for at
/// most `max` against `target` (placed by `xt`).
pub(crate) fn sweep(caster: &Caster, target: &Shape, xt: &Xf, o: FPVec2, d: FPVec2, max: FP) -> Option<HullHit> {
    let (tp, tn) = core(target, xt);
    let rr = caster.r + target.radius;

    // Reject on boxes: the swept caster against the target.
    let (mut tmin, mut tmax) = (tp[0], tp[0]);
    for p in &tp[1..tn] {
        tmin = tmin.min(*p);
        tmax = tmax.max(*p);
    }
    let (mut cmin, mut cmax) = (caster.rel[0], caster.rel[0]);
    for p in &caster.rel[1..caster.n] {
        cmin = cmin.min(*p);
        cmax = cmax.max(*p);
    }
    let end = o + d * max;
    let lo = o.min(end) + cmin - FPVec2::splat(rr);
    let hi = o.max(end) + cmax + FPVec2::splat(rr);
    if hi.x < tmin.x || lo.x > tmax.x || hi.y < tmin.y || lo.y > tmax.y {
        return None;
    }

    let mut pts = [FPVec2::ZERO; MAX_HULL];
    let mut m = 0;
    for t in &tp[..tn] {
        for c in &caster.rel[..caster.n] {
            pts[m] = *t - *c;
            m += 1;
        }
    }
    let k = hull(&mut pts, m);
    ray_rounded_hull(&pts[..k], rr, o, d, max)
}
