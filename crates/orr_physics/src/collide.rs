//! Narrow phase: contact manifolds for circle and convex polygon pairs.
//!
//! Every routine is a pure function of its inputs. Normals always point
//! from shape A to shape B. `separation` is negative for penetration and
//! positive for a small gap (up to the caller's `margin`).

use orr_fp::{FPVec2, FP};

use crate::types::{Shape, MAX_POLY_VERTS, SHAPE_CIRCLE};

/// Rigid transform: position plus rotation given as `(cos, sin)`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Xf {
    pub p: FPVec2,
    pub c: FP,
    pub s: FP,
}

impl Xf {
    #[inline]
    pub fn new(p: FPVec2, angle: FP) -> Xf {
        let (s, c) = angle.sin_cos();
        Xf { p, c, s }
    }
    #[inline]
    pub fn rot(&self, v: FPVec2) -> FPVec2 {
        rotate(self.c, self.s, v)
    }
    #[inline]
    pub fn inv_rot(&self, v: FPVec2) -> FPVec2 {
        rotate(self.c, -self.s, v)
    }
    #[inline]
    pub fn apply(&self, v: FPVec2) -> FPVec2 {
        self.rot(v) + self.p
    }
}

#[inline]
fn rotate(c: FP, s: FP, v: FPVec2) -> FPVec2 {
    FPVec2::new(c * v.x - s * v.y, s * v.x + c * v.y)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ManifoldPoint {
    pub point: FPVec2,
    pub separation: FP,
    pub id: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Manifold {
    /// Unit normal from A to B.
    pub normal: FPVec2,
    pub count: usize,
    pub points: [ManifoldPoint; 2],
}

/// Contact manifold of two shapes, `margin` extra distance still counts.
pub(crate) fn collide(sa: &Shape, xa: &Xf, sb: &Shape, xb: &Xf, margin: FP) -> Manifold {
    match (sa.kind == SHAPE_CIRCLE, sb.kind == SHAPE_CIRCLE) {
        (true, true) => circle_circle(sa.radius, xa.p, sb.radius, xb.p, margin),
        (false, true) => match poly_circle(sa, xa, xb.p, sb.radius, margin) {
            Some((n, sep, pt)) => single(n, sep, pt),
            None => Manifold::default(),
        },
        (true, false) => match poly_circle(sb, xb, xa.p, sa.radius, margin) {
            Some((n, sep, pt)) => single(-n, sep, pt),
            None => Manifold::default(),
        },
        (false, false) => poly_poly(sa, xa, sb, xb, margin),
    }
}

fn single(normal: FPVec2, separation: FP, point: FPVec2) -> Manifold {
    let mut m = Manifold { normal, count: 1, ..Manifold::default() };
    m.points[0] = ManifoldPoint { point, separation, id: 0 };
    m
}

fn circle_circle(ra: FP, pa: FPVec2, rb: FP, pb: FPVec2, margin: FP) -> Manifold {
    let d = pb - pa;
    let rsum = ra + rb;
    let reach = rsum + margin;
    let d2 = d.length_sq();
    if d2 > reach * reach {
        return Manifold::default();
    }
    let dist = d2.sqrt();
    let normal = if dist == FP::ZERO { FPVec2::Y } else { d / dist };
    let sep = dist - rsum;
    single(normal, sep, pa + normal * (ra + sep / 2))
}

/// Polygon vs circle center `cw` (world). Returns `(normal poly->circle
/// (world), separation, point (world))`.
fn poly_circle(poly: &Shape, xp: &Xf, cw: FPVec2, r: FP, margin: FP) -> Option<(FPVec2, FP, FPVec2)> {
    let c = xp.inv_rot(cw - xp.p);
    let n = poly.count as usize;
    let reach = r + margin;
    let mut best = FP::MIN;
    let mut idx = 0usize;
    for i in 0..n {
        let s = poly.normals[i].dot(c - poly.verts[i]);
        if s > reach {
            return None;
        }
        if s > best {
            best = s;
            idx = i;
        }
    }
    let v1 = poly.verts[idx];
    let v2 = poly.verts[(idx + 1) % n];
    let u1 = (c - v1).dot(v2 - v1);
    let u2 = (c - v2).dot(v1 - v2);
    let corner = if u1 <= FP::ZERO {
        Some(v1)
    } else if u2 <= FP::ZERO {
        Some(v2)
    } else {
        None
    };
    if let Some(v) = corner {
        let d = c - v;
        let d2 = d.length_sq();
        if d2 > reach * reach {
            return None;
        }
        let dist = d2.sqrt();
        let nl = if dist == FP::ZERO { poly.normals[idx] } else { d / dist };
        let sep = dist - r;
        return Some((xp.rot(nl), sep, xp.apply(v + nl * (sep / 2))));
    }
    let nl = poly.normals[idx];
    let sep = best - r;
    Some((xp.rot(nl), sep, xp.apply(c - nl * ((r + best) / 2))))
}

/// Largest over `p1` faces of the smallest signed distance of `p2` (given
/// in `p1`'s frame by `pos` and `(rc, rs)`) to that face.
fn find_max_sep(p1: &Shape, p2: &Shape, pos: FPVec2, rc: FP, rs: FP, margin: FP) -> (FP, usize) {
    let n1 = p1.count as usize;
    let n2 = p2.count as usize;
    let mut v2 = [FPVec2::ZERO; MAX_POLY_VERTS];
    for (dst, v) in v2.iter_mut().zip(&p2.verts[..n2]) {
        *dst = rotate(rc, rs, *v) + pos;
    }
    let mut best = FP::MIN;
    let mut best_i = 0usize;
    for i in 0..n1 {
        let n = p1.normals[i];
        let v = p1.verts[i];
        let mut si = FP::MAX;
        for w in v2.iter().take(n2) {
            si = si.min(n.dot(*w - v));
        }
        if si > margin {
            return (si, i);
        }
        if si > best {
            best = si;
            best_i = i;
        }
    }
    (best, best_i)
}

fn poly_poly(sa: &Shape, xa: &Xf, sb: &Shape, xb: &Xf, margin: FP) -> Manifold {
    // B relative to A.
    let pos_ab = xa.inv_rot(xb.p - xa.p);
    let c_ab = xa.c * xb.c + xa.s * xb.s;
    let s_ab = xa.c * xb.s - xa.s * xb.c;
    let (sep_a, edge_a) = find_max_sep(sa, sb, pos_ab, c_ab, s_ab, margin);
    if sep_a > margin {
        return Manifold::default();
    }
    // A relative to B.
    let pos_ba = xb.inv_rot(xa.p - xb.p);
    let (c_ba, s_ba) = (c_ab, -s_ab);
    let (sep_b, edge_b) = find_max_sep(sb, sa, pos_ba, c_ba, s_ba, margin);
    if sep_b > margin {
        return Manifold::default();
    }

    // Prefer A as the reference face unless B is clearly better.
    let tol = FP::from_raw(66);
    let flip = sep_b > sep_a + tol;
    let (p1, p2, xf1, edge1, pos, rc, rs) =
        if flip { (sb, sa, xb, edge_b, pos_ba, c_ba, s_ba) } else { (sa, sb, xa, edge_a, pos_ab, c_ab, s_ab) };
    let n1c = p1.count as usize;
    let n2c = p2.count as usize;
    let normal1 = p1.normals[edge1];

    // Incident edge: the face of p2 most anti-parallel to the reference normal.
    let mut inc = 0usize;
    let mut min_dot = FP::MAX;
    for i in 0..n2c {
        let d = normal1.dot(rotate(rc, rs, p2.normals[i]));
        if d < min_dot {
            min_dot = d;
            inc = i;
        }
    }
    let inc2 = (inc + 1) % n2c;
    let iv = [rotate(rc, rs, p2.verts[inc]) + pos, rotate(rc, rs, p2.verts[inc2]) + pos];

    // Clip the incident edge against the reference face side planes.
    let v11 = p1.verts[edge1];
    let v12 = p1.verts[(edge1 + 1) % n1c];
    let tangent = FPVec2::new(-normal1.y, normal1.x);
    let side1 = -tangent.dot(v11);
    let side2 = tangent.dot(v12);
    let (c1, k1) = clip(iv, -tangent, side1);
    if k1 < 2 {
        return Manifold::default();
    }
    let (c2, k2) = clip(c1, tangent, side2);
    if k2 < 2 {
        return Manifold::default();
    }

    let front = normal1.dot(v11);
    let normal_world = xf1.rot(normal1);
    let mut m = Manifold { normal: if flip { -normal_world } else { normal_world }, ..Manifold::default() };
    for (k, cp) in c2.iter().enumerate() {
        let sep = normal1.dot(*cp) - front;
        if sep <= margin {
            let id = ((flip as u32) << 24) | ((edge1 as u32) << 16) | ((inc as u32) << 8) | k as u32;
            let pt = xf1.apply(*cp - normal1 * (sep / 2));
            m.points[m.count] = ManifoldPoint { point: pt, separation: sep, id };
            m.count += 1;
        }
    }
    m
}

/// Clips a 2-point segment to `dot(n, v) <= offset`. Returns the surviving
/// points (segment order kept) and their count.
fn clip(v: [FPVec2; 2], n: FPVec2, offset: FP) -> ([FPVec2; 2], usize) {
    let d0 = n.dot(v[0]) - offset;
    let d1 = n.dot(v[1]) - offset;
    let mut out = [FPVec2::ZERO; 2];
    let mut k = 0;
    if d0 <= FP::ZERO {
        out[k] = v[0];
        k += 1;
    }
    if d1 <= FP::ZERO {
        out[k] = v[1];
        k += 1;
    }
    if k < 2 && ((d0 < FP::ZERO && d1 > FP::ZERO) || (d0 > FP::ZERO && d1 < FP::ZERO)) {
        let t = d0 / (d0 - d1);
        out[k] = v[0] + (v[1] - v[0]) * t;
        k += 1;
    }
    (out, k)
}

/// Exact boolean overlap test (touching counts as overlapping). Used for
/// sensors, where the manifold itself is not needed.
pub(crate) fn overlap(sa: &Shape, xa: &Xf, sb: &Shape, xb: &Xf) -> bool {
    match (sa.kind == SHAPE_CIRCLE, sb.kind == SHAPE_CIRCLE) {
        (true, true) => {
            let r = sa.radius + sb.radius;
            (xb.p - xa.p).length_sq() <= r * r
        }
        (false, true) => poly_circle(sa, xa, xb.p, sb.radius, FP::ZERO).is_some(),
        (true, false) => poly_circle(sb, xb, xa.p, sa.radius, FP::ZERO).is_some(),
        (false, false) => {
            let pos_ab = xa.inv_rot(xb.p - xa.p);
            let c_ab = xa.c * xb.c + xa.s * xb.s;
            let s_ab = xa.c * xb.s - xa.s * xb.c;
            if find_max_sep(sa, sb, pos_ab, c_ab, s_ab, FP::ZERO).0 > FP::ZERO {
                return false;
            }
            let pos_ba = xb.inv_rot(xa.p - xb.p);
            find_max_sep(sb, sa, pos_ba, c_ab, -s_ab, FP::ZERO).0 <= FP::ZERO
        }
    }
}
