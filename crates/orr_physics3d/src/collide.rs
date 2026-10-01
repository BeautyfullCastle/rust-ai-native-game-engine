//! Narrow phase: contact manifolds for every shape pair.
//!
//! Spheres and capsules are both segments grown by a radius (a sphere is a
//! degenerate segment), so only three routines exist: segment/segment,
//! segment/box and box/box. [`collide`] expects `a.kind <= b.kind` and
//! returns a normal pointing from `a` to `b`.
//!
//! Every point carries a feature id that stays the same while the same
//! features touch, which keys the warm-starting cache. Ties are always
//! broken by a fixed axis order, never by data layout.

use orr_fp::{fp, FPVec3, FP};

use crate::geom::{any_perpendicular, closest_seg_seg, dotr, ratio_q16, segment_of, Xf};
use crate::types::{Shape, SHAPE_BOX};

/// One contact point of a manifold.
#[derive(Clone, Copy, Default)]
pub(crate) struct CPoint {
    pub point: FPVec3,
    /// Signed separation along the normal (negative = penetration).
    pub sep: FP,
    pub id: u32,
}

/// Up to four contact points sharing one normal (from `a` to `b`).
#[derive(Clone, Copy, Default)]
pub(crate) struct Manifold {
    pub normal: FPVec3,
    pub count: usize,
    pub pts: [CPoint; 4],
}

impl Manifold {
    #[inline]
    fn push(&mut self, point: FPVec3, sep: FP, id: u32) {
        if self.count < 4 {
            self.pts[self.count] = CPoint { point, sep, id };
            self.count += 1;
        }
    }

    /// Sorts the points by id and makes the ids unique.
    fn finish(&mut self) {
        let n = self.count;
        for i in 1..n {
            let mut j = i;
            while j > 0 && self.pts[j - 1].id > self.pts[j].id {
                self.pts.swap(j - 1, j);
                j -= 1;
            }
        }
        for i in 1..n {
            if self.pts[i].id <= self.pts[i - 1].id {
                self.pts[i].id = self.pts[i - 1].id + 1;
            }
        }
    }
}

/// Contact manifold of two shapes, `a.kind <= b.kind`. Contacts exist up
/// to `margin` of separation.
pub(crate) fn collide(a: &Shape, xa: &Xf, b: &Shape, xb: &Xf, margin: FP) -> Manifold {
    debug_assert!(a.kind <= b.kind);
    if b.kind == SHAPE_BOX {
        if a.kind == SHAPE_BOX {
            box_box(a, xa, b, xb, margin)
        } else {
            seg_box(a, xa, b, xb, margin)
        }
    } else {
        seg_seg(a, xa, b, xb, margin)
    }
}

/// `(a + b) / 2`.
#[inline]
fn mid(a: FPVec3, b: FPVec3) -> FPVec3 {
    (a + b) * FP::HALF
}

// ---------------------------------------------------------------------
// segment / segment (sphere and capsule pairs)
// ---------------------------------------------------------------------

fn seg_seg(a: &Shape, xa: &Xf, b: &Shape, xb: &Xf, margin: FP) -> Manifold {
    let (p1, q1) = segment_of(a, xa);
    let (p2, q2) = segment_of(b, xb);
    let (ra, rb) = (a.radius, b.radius);
    let rsum = ra + rb;
    let mut m = Manifold::default();
    let (s, t) = closest_seg_seg(p1, q1, p2, q2);
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let c1 = p1 + d1 * s;
    let c2 = p2 + d2 * t;
    let delta = c2 - c1;
    let lim = rsum + margin;
    let dist2 = delta.length_sq();
    if dist2 > lim * lim {
        return m;
    }
    let dist = dist2.sqrt();
    let n = if dist.raw() >= 16 {
        delta / dist
    } else {
        let c = d1.cross(d2).normalize_or_zero();
        if c != FPVec3::ZERO {
            c
        } else if d1 != FPVec3::ZERO {
            any_perpendicular(d1)
        } else {
            any_perpendicular(d2)
        }
    };
    m.normal = n;

    // Two near-parallel capsules lying side by side: clip the overlap of
    // the two axes into two contact points so they rest without rocking.
    let (la, le, lb) = (dotr(d1, d1), dotr(d2, d2), dotr(d1, d2));
    if la > 1 << 10 && le > 1 << 10 && lb * lb * 10_000 >= 9604 * la * le {
        let l1 = d1.length();
        let u = d1 / l1;
        let xp = (p2 - p1).dot(u);
        let xq = (q2 - p1).dot(u);
        let lo = xp.min(xq).max(FP::ZERO);
        let hi = xp.max(xq).min(l1);
        let span = xq - xp;
        if hi - lo > fp!(0.01) && span.abs() > fp!(0.01) {
            for (k, x) in [lo, hi].into_iter().enumerate() {
                let tau = FP::from_raw(ratio_q16((x - xp).raw() as i128, span.raw() as i128).clamp(0, 65536));
                let e1 = p1 + u * x;
                let e2 = p2 + d2 * tau;
                let sep = (e2 - e1).dot(n) - rsum;
                if sep <= margin {
                    m.push(mid(e1 + n * ra, e2 - n * rb), sep, k as u32);
                }
            }
            if m.count > 0 {
                return m;
            }
        }
    }
    m.push(mid(c1 + n * ra, c2 - n * rb), dist - rsum, 0);
    m
}

// ---------------------------------------------------------------------
// segment / box (sphere and capsule against box)
// ---------------------------------------------------------------------

/// Parameter range of the segment `p0 + t d`, `t` in `[0, 1]`, inside the
/// slabs `|x_j| <= h_j` of the listed axes.
fn clip_axes(p0: FPVec3, d: FPVec3, h: FPVec3, axes: &[usize]) -> Option<(FP, FP)> {
    let (mut tmin, mut tmax) = (0i64, 65536i64);
    for &j in axes {
        let (pj, dj, hj) = (p0.get(j).raw(), d.get(j).raw(), h.get(j).raw());
        if dj == 0 {
            if pj.abs() > hj {
                return None;
            }
        } else {
            let mut t1 = ratio_q16((-hj - pj) as i128, dj as i128);
            let mut t2 = ratio_q16((hj - pj) as i128, dj as i128);
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some((FP::from_raw(tmin), FP::from_raw(tmax)))
}

fn clamp_to_box(p: FPVec3, h: FPVec3) -> FPVec3 {
    FPVec3::new(p.x.clamp(-h.x, h.x), p.y.clamp(-h.y, h.y), p.z.clamp(-h.z, h.z))
}

fn seg_box(a: &Shape, xa: &Xf, b: &Shape, xb: &Xf, margin: FP) -> Manifold {
    let (w0, w1) = segment_of(a, xa);
    let r = a.radius;
    let h = b.half;
    let p0 = xb.to_local(w0);
    let p1 = xb.to_local(w1);
    let d = p1 - p0;
    let mut m = Manifold::default();
    let lim = r + margin;

    // Quick reject: segment box against the box grown by the limit.
    for j in 0..3 {
        let (lo, hi) = (p0.get(j).min(p1.get(j)), p0.get(j).max(p1.get(j)));
        if lo > h.get(j) + lim || hi < -(h.get(j) + lim) {
            return m;
        }
    }

    // Local normal pointing from the box to the segment.
    let mut tmid = FP::HALF;
    let mut closest: Option<(FPVec3, FPVec3)> = None; // (box point, segment point)
    if let Some((t0, t1)) = clip_axes(p0, d, h, &[0, 1, 2]) {
        tmid = (t0 + t1) * FP::HALF;
    } else {
        // Disjoint: the closest pair involves a segment end point or one of
        // the 12 box edges.
        let mut best = FP::MAX;
        let mut consider = |qb: FPVec3, qs: FPVec3, t: FP, best: &mut FP, tmid: &mut FP| {
            let d2 = (qs - qb).length_sq();
            if d2 < *best {
                *best = d2;
                *tmid = t;
                closest = Some((qb, qs));
            }
        };
        consider(clamp_to_box(p0, h), p0, FP::ZERO, &mut best, &mut tmid);
        consider(clamp_to_box(p1, h), p1, FP::ONE, &mut best, &mut tmid);
        for axis in 0..3 {
            let (j, k) = ((axis + 1) % 3, (axis + 2) % 3);
            for sj in [-1i32, 1] {
                for sk in [-1i32, 1] {
                    let mut e0 = FPVec3::ZERO;
                    e0.set(j, h.get(j) * sj);
                    e0.set(k, h.get(k) * sk);
                    let mut e1 = e0;
                    e0.set(axis, -h.get(axis));
                    e1.set(axis, h.get(axis));
                    let (s, t) = closest_seg_seg(p0, p1, e0, e1);
                    consider(e0 + (e1 - e0) * t, p0 + d * s, s, &mut best, &mut tmid);
                }
            }
        }
        if best > lim * lim {
            return m;
        }
    }

    let n_world_sign = FP::MINUS_ONE; // manifold normal = -(box to segment)
    let emit_face = |m: &mut Manifold, i: usize, neg: bool, tmid: FP| {
        // Normal along +-axis i (box to segment).
        let s: i32 = if neg { -1 } else { 1 };
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let mut nl = FPVec3::ZERO;
        nl.set(i, FP::from_int(s));
        let face_code = (i as u32) * 2 + u32::from(!neg);
        let range = clip_axes(p0, d, h, &[j, k]);
        let mut pts = 0;
        if let Some((ta, tb)) = range {
            let ends: [(FP, u32); 2] = if tb - ta > fp!(0.004) { [(ta, 0), (tb, 1)] } else { [((ta + tb) * FP::HALF, 2), (FP::ZERO, 3)] };
            let cnt = if tb - ta > fp!(0.004) { 2 } else { 1 };
            for &(t, sub) in ends.iter().take(cnt) {
                let pt = p0 + d * t;
                let sep = pt.get(i) * s - h.get(i) - r;
                if sep <= margin {
                    let pl = pt - nl * (r + sep * FP::HALF);
                    m.push(xb.to_world(pl), sep, face_code * 4 + sub);
                    pts += 1;
                }
            }
        }
        if pts == 0 {
            let pt = p0 + d * tmid;
            let sep = pt.get(i) * s - h.get(i) - r;
            let pl = pt - nl * (r + sep * FP::HALF);
            m.push(xb.to_world(pl), sep, face_code * 4 + 2);
        }
        m.normal = xb.r.mul_vec(nl) * n_world_sign;
    };

    if let Some((qb, qs)) = closest {
        let delta = qs - qb;
        let dist = delta.length();
        if dist.raw() >= 16 {
            let nl = delta / dist;
            // Face aligned: use the exact face normal and a clipped overlap.
            let (mut ai, mut av) = (0usize, nl.x.abs());
            for j in 1..3 {
                if nl.get(j).abs() > av {
                    ai = j;
                    av = nl.get(j).abs();
                }
            }
            // Only when part of the segment is really over the face; a
            // segment beyond the edge gets the true (tilted) normal.
            if av >= fp!(0.98) && clip_axes(p0, d, h, &[(ai + 1) % 3, (ai + 2) % 3]).is_some() {
                emit_face(&mut m, ai, nl.get(ai) < FP::ZERO, tmid);
                m.finish();
                return m;
            }
            let sep = dist - r;
            let pl = mid(qs - nl * r, qb);
            m.push(xb.to_world(pl), sep, 24);
            m.normal = xb.r.mul_vec(nl) * n_world_sign;
            return m;
        }
    }

    // The segment reaches into the box: least penetration axis (SAT).
    let (mut best_pen, mut best_i, mut best_neg) = (FP::MAX, 0usize, false);
    for i in 0..3 {
        let (lo, hi) = (p0.get(i).min(p1.get(i)), p0.get(i).max(p1.get(i)));
        let plus = h.get(i) + r - lo;
        let minus = hi + r + h.get(i);
        if plus < best_pen {
            (best_pen, best_i, best_neg) = (plus, i, false);
        }
        if minus < best_pen {
            (best_pen, best_i, best_neg) = (minus, i, true);
        }
    }
    // Axes perpendicular to the segment and a box axis (edge contacts).
    let dl = d.length();
    let mut cross_best: Option<(FP, FPVec3)> = None;
    if dl > fp!(0.01) {
        for i in 0..3 {
            let mut e = FPVec3::ZERO;
            e.set(i, FP::ONE);
            let c = e.cross(d);
            let cl = c.length();
            if cl <= dl / 10 {
                continue;
            }
            let ax = c / cl;
            let ha = ax.abs().dot(h);
            let s0 = p0.dot(ax);
            let plus = ha + r - s0;
            let minus = s0 + r + ha;
            let (pen, nl) = if plus <= minus { (plus, ax) } else { (minus, -ax) };
            if cross_best.is_none_or(|(bp, _)| pen < bp) {
                cross_best = Some((pen, nl));
            }
        }
    }
    if let Some((pen, nl)) = cross_best {
        if pen + fp!(0.005) < best_pen {
            let pt = p0 + d * tmid;
            let sep = -pen;
            let pl = pt - nl * (r + sep * FP::HALF);
            m.push(xb.to_world(pl), sep, 25);
            m.normal = xb.r.mul_vec(nl) * n_world_sign;
            return m;
        }
    }
    emit_face(&mut m, best_i, best_neg, tmid);
    m.finish();
    m
}

// ---------------------------------------------------------------------
// box / box
// ---------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Poly {
    p: [FPVec3; 8],
    n: usize,
}

/// Clips `poly` against `signed_dist <= 0` where the distance of a vertex
/// `v` is `(v - c).dot(w) - h`.
fn clip_poly(poly: &Poly, c: FPVec3, w: FPVec3, h: FP) -> Poly {
    let mut out = Poly { p: [FPVec3::ZERO; 8], n: 0 };
    let n = poly.n;
    for i in 0..n {
        let (cur, nxt) = (poly.p[i], poly.p[(i + 1) % n]);
        let dc = (cur - c).dot(w) - h;
        let dn = (nxt - c).dot(w) - h;
        if dc <= FP::ZERO && out.n < 8 {
            out.p[out.n] = cur;
            out.n += 1;
        }
        if ((dc < FP::ZERO && dn > FP::ZERO) || (dc > FP::ZERO && dn < FP::ZERO)) && out.n < 8 {
            let t = dc / (dc - dn);
            out.p[out.n] = cur + (nxt - cur) * t;
            out.n += 1;
        }
    }
    out
}

/// Contact points of the incident face of box `y` on the reference face of
/// box `x`. `nref` is the outward reference normal (along axis `ri` of
/// `x`). Points go into `m`; the manifold normal is set by the caller.
#[allow(clippy::too_many_arguments)]
fn face_points(m: &mut Manifold, xx: &Xf, hx: FPVec3, xy: &Xf, hy: FPVec3, nref: FPVec3, ri: usize, refcode: u32, margin: FP) {
    let fc = xx.p + nref * hx.get(ri);
    // Incident face: the face of y most anti-parallel to nref.
    let (mut j, mut best) = (0usize, FP::MINUS_ONE);
    let mut dots = [FP::ZERO; 3];
    for q in 0..3 {
        dots[q] = nref.dot(xy.axis(q));
        if dots[q].abs() > best {
            best = dots[q].abs();
            j = q;
        }
    }
    let f = if dots[j] >= FP::ZERO { -xy.axis(j) } else { xy.axis(j) };
    let ic = xy.p + f * hy.get(j);
    let (l1, l2) = ((j + 1) % 3, (j + 2) % 3);
    let u = xy.axis(l1) * hy.get(l1);
    let v = xy.axis(l2) * hy.get(l2);
    let mut poly = Poly { p: [FPVec3::ZERO; 8], n: 4 };
    poly.p[0] = ic + u + v;
    poly.p[1] = ic - u + v;
    poly.p[2] = ic - u - v;
    poly.p[3] = ic + u - v;
    let (k1, k2) = ((ri + 1) % 3, (ri + 2) % 3);
    let (a1, a2) = (xx.axis(k1), xx.axis(k2));
    for (w, h) in [(a1, hx.get(k1)), (-a1, hx.get(k1)), (a2, hx.get(k2)), (-a2, hx.get(k2))] {
        poly = clip_poly(&poly, xx.p, w, h);
        if poly.n == 0 {
            return;
        }
    }
    // Keep points within the margin; positions midway between the faces.
    let mut cand: [(FPVec3, FP); 8] = [(FPVec3::ZERO, FP::ZERO); 8];
    let mut nc = 0;
    for i in 0..poly.n {
        let sep = (poly.p[i] - fc).dot(nref);
        if sep <= margin {
            cand[nc] = (poly.p[i] - nref * (sep * FP::HALF), sep);
            nc += 1;
        }
    }
    // Feature ids are the diagonal quadrant of the point in the reference
    // face frame, so they stay the same when the clipped polygon changes
    // shape slightly (a stack that is a hair off axis clips to an octagon
    // whose vertex identities flicker, its four corners do not).
    if nc == 0 {
        return;
    }
    let base = 1024 * refcode;
    if nc > 4 {
        // Four points: the extreme one in each diagonal direction.
        let mut taken = [false; 8];
        for (q, &(s1, s2)) in DIAGONALS.iter().enumerate() {
            let (mut pick, mut best) = (usize::MAX, FP::MIN);
            for i in 0..nc {
                if taken[i] {
                    continue;
                }
                let rel = cand[i].0 - fc;
                let score = rel.dot(a1) * s1 + rel.dot(a2) * s2;
                if score > best {
                    best = score;
                    pick = i;
                }
            }
            taken[pick] = true;
            m.push(cand[pick].0, cand[pick].1, base + q as u32);
        }
        return;
    }
    let mut cen = FPVec3::ZERO;
    for &(p, _) in &cand[..nc] {
        cen += p;
    }
    cen = cen / FP::from_int(nc as i32);
    for &(p, s) in &cand[..nc] {
        let rel = p - cen;
        let q = match (rel.dot(a1) >= FP::ZERO, rel.dot(a2) >= FP::ZERO) {
            (true, true) => 0,
            (true, false) => 1,
            (false, false) => 2,
            (false, true) => 3,
        };
        m.push(p, s, base + q);
    }
}

/// Diagonal directions of the reference face frame, in id order.
const DIAGONALS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, -1), (-1, 1)];

fn box_box(a: &Shape, xa: &Xf, b: &Shape, xb: &Xf, margin: FP) -> Manifold {
    let (ha, hb) = (a.half, b.half);
    let mut m = Manifold::default();
    let rm = xa.r.transpose().mul_mat(&xb.r); // rm[i][j] = A_i . B_j
    let tw = xb.p - xa.p;
    let t = xa.r.tmul_vec(tw);
    let eps = FP::from_raw(64);
    let mut r = [[FP::ZERO; 3]; 3];
    let mut ar = [[FP::ZERO; 3]; 3];
    for i in 0..3 {
        let row = rm.row(i);
        for j in 0..3 {
            r[i][j] = row.get(j);
            ar[i][j] = r[i][j].abs() + eps;
        }
    }

    // Faces of A.
    let (mut sep_a, mut axis_a) = (FP::MIN, 0usize);
    for i in 0..3 {
        let rb_ = hb.x * ar[i][0] + hb.y * ar[i][1] + hb.z * ar[i][2];
        let s = t.get(i).abs() - (ha.get(i) + rb_);
        if s > margin {
            return m;
        }
        if s > sep_a {
            (sep_a, axis_a) = (s, i);
        }
    }
    // Faces of B.
    let (mut sep_b, mut axis_b) = (FP::MIN, 0usize);
    for j in 0..3 {
        let ra_ = ha.x * ar[0][j] + ha.y * ar[1][j] + ha.z * ar[2][j];
        let tj = t.x * r[0][j] + t.y * r[1][j] + t.z * r[2][j];
        let s = tj.abs() - (hb.get(j) + ra_);
        if s > margin {
            return m;
        }
        if s > sep_b {
            (sep_b, axis_b) = (s, j);
        }
    }
    // Edge pairs.
    let (mut sep_e, mut axis_e) = (FP::MIN, (usize::MAX, usize::MAX));
    for i in 0..3 {
        let (i1, i2) = ((i + 1) % 3, (i + 2) % 3);
        for j in 0..3 {
            let (j1, j2) = ((j + 1) % 3, (j + 2) % 3);
            let len2 = FP::ONE - r[i][j] * r[i][j];
            if len2 < fp!(0.0025) {
                continue;
            }
            let ra_ = ha.get(i1) * ar[i2][j] + ha.get(i2) * ar[i1][j];
            let rb_ = hb.get(j1) * ar[i][j2] + hb.get(j2) * ar[i][j1];
            let proj = (t.get(i2) * r[i1][j] - t.get(i1) * r[i2][j]).abs();
            let s = (proj - (ra_ + rb_)) / len2.sqrt();
            if s > margin {
                return m;
            }
            if s > sep_e {
                (sep_e, axis_e) = (s, (i, j));
            }
        }
    }

    let tol = fp!(0.01);
    let use_b = sep_b > sep_a + tol;
    let (face_sep, _) = if use_b { (sep_b, axis_b) } else { (sep_a, axis_a) };
    let use_edge = axis_e.0 != usize::MAX && sep_e > face_sep + tol;

    if use_edge {
        let (i, j) = axis_e;
        let mut n = xa.axis(i).cross(xb.axis(j)).normalize_or_zero();
        if n.dot(tw) < FP::ZERO {
            n = -n;
        }
        let mut pa = xa.p;
        for k in 0..3 {
            if k != i {
                let s = if n.dot(xa.axis(k)) >= FP::ZERO { FP::ONE } else { FP::MINUS_ONE };
                pa += xa.axis(k) * (ha.get(k) * s);
            }
        }
        let mut pb = xb.p;
        for k in 0..3 {
            if k != j {
                let s = if n.dot(xb.axis(k)) > FP::ZERO { FP::MINUS_ONE } else { FP::ONE };
                pb += xb.axis(k) * (hb.get(k) * s);
            }
        }
        let (ea, eb) = (xa.axis(i) * ha.get(i), xb.axis(j) * hb.get(j));
        let (s, tt) = closest_seg_seg(pa - ea, pa + ea, pb - eb, pb + eb);
        let ca = pa - ea + ea * (s * 2);
        let cb = pb - eb + eb * (tt * 2);
        m.normal = n;
        m.push(mid(ca, cb), (cb - ca).dot(n), 200 + (i * 3 + j) as u32);
        return m;
    }

    if !use_b {
        let i = axis_a;
        let neg = t.get(i) < FP::ZERO;
        let nref = if neg { -xa.axis(i) } else { xa.axis(i) };
        m.normal = nref;
        face_points(&mut m, xa, ha, xb, hb, nref, i, (i as u32) * 2 + u32::from(!neg), margin);
    } else {
        let j = axis_b;
        let tj = t.x * r[0][j] + t.y * r[1][j] + t.z * r[2][j];
        let neg = tj < FP::ZERO; // B axis points away from A when tj >= 0
        // Outward reference normal of B points toward A.
        let nref = if neg { xb.axis(j) } else { -xb.axis(j) };
        m.normal = -nref;
        face_points(&mut m, xb, hb, xa, ha, nref, j, 6 + (j as u32) * 2 + u32::from(!neg), margin);
    }
    if m.count == 0 && axis_e.0 != usize::MAX {
        // Clipped away entirely: fall back to the edge pair.
        let (i, j) = axis_e;
        let mut n = xa.axis(i).cross(xb.axis(j)).normalize_or_zero();
        if n.dot(tw) < FP::ZERO {
            n = -n;
        }
        let mut pa = xa.p;
        for k in 0..3 {
            if k != i {
                let s = if n.dot(xa.axis(k)) >= FP::ZERO { FP::ONE } else { FP::MINUS_ONE };
                pa += xa.axis(k) * (ha.get(k) * s);
            }
        }
        let mut pb = xb.p;
        for k in 0..3 {
            if k != j {
                let s = if n.dot(xb.axis(k)) > FP::ZERO { FP::MINUS_ONE } else { FP::ONE };
                pb += xb.axis(k) * (hb.get(k) * s);
            }
        }
        let (ea, eb) = (xa.axis(i) * ha.get(i), xb.axis(j) * hb.get(j));
        let (s, tt) = closest_seg_seg(pa - ea, pa + ea, pb - eb, pb + eb);
        let ca = pa - ea + ea * (s * 2);
        let cb = pb - eb + eb * (tt * 2);
        let sep = (cb - ca).dot(n);
        if sep <= margin {
            m.normal = n;
            m.push(mid(ca, cb), sep, 200 + (i * 3 + j) as u32);
        }
    }
    m.finish();
    m
}
