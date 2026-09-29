//! Narrow phase for capsules: capsule/circle, capsule/capsule and
//! capsule/polygon manifolds.
//!
//! A capsule is a segment with a radius. Every manifold has a unit normal
//! from A to B and up to two points, like the circle and polygon routines
//! in `collide`.
//!
//! - One point: the closest points of the two core shapes (segment,
//!   polygon boundary or circle center), moved out by the radii. This is
//!   exact for every feature pair, so a capsule cap rolls smoothly over a
//!   polygon corner.
//! - Two points: when a capsule axis is within about 6 degrees of a flat
//!   feature (another capsule axis or a polygon edge) and the two overlap
//!   along it, the overlap range is clipped to two points so a capsule
//!   can rest on its side.
//! - Deep overlap (the core segment touches the polygon or another
//!   capsule axis): the axis of least penetration, as in SAT.

use orr_fp::{FPVec2, FP};

use crate::collide::{Manifold, ManifoldPoint, Xf};
use crate::fastmath::{self, FastVec};
use crate::geom::{closest_on_segment, perp, point_at, seg_seg_closest};
use crate::types::{Shape, SHAPE_CAPSULE, SHAPE_CIRCLE};

/// `sin` of the largest angle between a capsule axis and a flat feature
/// that still makes a two-point manifold (0.1, about 5.7 degrees).
const PARALLEL_TOL: FP = FP::from_raw(6554);
/// Least `dot` of the closest-point normal and a face normal for that face
/// to serve as the reference of a two-point manifold (0.9).
const FACE_DOT: FP = FP::from_raw(58982);

const ID_ONE: u32 = 0x10;
const ID_CAPSULE_PAIR: u32 = 0x20;
const ID_FACE: u32 = 0x30;
const ID_AXIS: u32 = 0x40;

/// A capsule in world space.
struct Cap {
    a: FPVec2,
    b: FPVec2,
    /// Unit vector `a -> b`.
    u: FPVec2,
    len: FP,
    r: FP,
}

fn world_cap(s: &Shape, xf: &Xf) -> Cap {
    Cap {
        a: xf.apply(s.verts[0]),
        b: xf.apply(s.verts[1]),
        u: xf.rot(s.normals[0]),
        len: s.normals[1].x,
        r: s.radius,
    }
}

#[inline]
fn vdiv(v: FPVec2, s: FP) -> FPVec2 {
    FPVec2::new(fastmath::div(v.x, s), fastmath::div(v.y, s))
}

fn single(normal: FPVec2, separation: FP, point: FPVec2, id: u32) -> Manifold {
    let mut m = Manifold { normal, count: 1, ..Manifold::default() };
    m.points[0] = ManifoldPoint { point, separation, id };
    m
}

fn flip(mut m: Manifold) -> Manifold {
    m.normal = -m.normal;
    m
}

/// Contact manifold of two shapes when at least one is a capsule.
pub(crate) fn collide_capsule(sa: &Shape, xa: &Xf, sb: &Shape, xb: &Xf, margin: FP) -> Manifold {
    match (sa.kind, sb.kind) {
        (SHAPE_CAPSULE, SHAPE_CAPSULE) => capsule_capsule(&world_cap(sa, xa), &world_cap(sb, xb), margin),
        (SHAPE_CAPSULE, SHAPE_CIRCLE) => capsule_circle(&world_cap(sa, xa), xb.p, sb.radius, margin),
        (SHAPE_CIRCLE, SHAPE_CAPSULE) => flip(capsule_circle(&world_cap(sb, xb), xa.p, sa.radius, margin)),
        (SHAPE_CAPSULE, _) => capsule_polygon(&world_cap(sa, xa), sb, xb, margin),
        _ => flip(capsule_polygon(&world_cap(sb, xb), sa, xa, margin)),
    }
}

/// Two discs: `pa`, `pb` centers. `fallback` is the normal for coincident
/// centers.
fn round_pair(ra: FP, pa: FPVec2, rb: FP, pb: FPVec2, margin: FP, fallback: FPVec2, id: u32) -> Manifold {
    let d = pb - pa;
    let rsum = ra + rb;
    let reach = rsum + margin;
    let d2 = d.length_sq();
    if d2 > reach * reach {
        return Manifold::default();
    }
    let dist = fastmath::sqrt(d2);
    let normal = if dist == FP::ZERO { fallback } else { vdiv(d, dist) };
    let sep = dist - rsum;
    single(normal, sep, pa + normal * (ra + sep / 2), id)
}

fn capsule_circle(cap: &Cap, c: FPVec2, rc: FP, margin: FP) -> Manifold {
    let cp = closest_on_segment(c, cap.a, cap.b);
    round_pair(cap.r, cp, rc, c, margin, perp(cap.u), 0)
}

fn capsule_capsule(ca: &Cap, cb: &Cap, margin: FP) -> Manifold {
    let (c1, c2) = seg_seg_closest(ca.a, ca.b, cb.a, cb.b);
    let rsum = ca.r + cb.r;
    let d = c2 - c1;
    let reach = rsum + margin;
    let d2 = d.length_sq();
    if d2 > reach * reach {
        return Manifold::default();
    }
    let dist = fastmath::sqrt(d2);
    let side = perp(ca.u);
    let normal = if dist > FP::ZERO {
        vdiv(d, dist)
    } else {
        // Axes cross: push apart sideways to A's axis.
        let mid = (cb.a + cb.b) * FP::HALF - (ca.a + ca.b) * FP::HALF;
        if side.dotf(mid) >= FP::ZERO {
            side
        } else {
            -side
        }
    };
    let sep = dist - rsum;
    let one = single(normal, sep, c1 + normal * (ca.r + sep / 2), ID_ONE);
    if dist == FP::ZERO || ca.u.perp_dot(cb.u).abs() > PARALLEL_TOL {
        return one;
    }

    // Nearly parallel: clip B's axis to the range it shares with A's axis.
    let n = if side.dotf(normal) >= FP::ZERO { side } else { -side };
    if n.dotf(normal) < FACE_DOT {
        return one;
    }
    let ta = ca.u.dotf(cb.a - ca.a);
    let tb = ca.u.dotf(cb.b - ca.a);
    if ta == tb {
        return one;
    }
    let lo = ta.min(tb).max(FP::ZERO);
    let hi = ta.max(tb).min(ca.len);
    if hi <= lo {
        return one;
    }
    let mut m = Manifold { normal: n, ..Manifold::default() };
    for (k, s) in [lo, hi].into_iter().enumerate() {
        let q = point_at(cb.a, cb.b, ta, tb, s);
        let p = ca.a + ca.u * s;
        let h = n.dotf(q - p);
        let sk = h - rsum;
        if h < FP::ZERO || sk > margin {
            return one;
        }
        m.points[k] = ManifoldPoint { point: p + n * (ca.r + sk / 2), separation: sk, id: ID_CAPSULE_PAIR + k as u32 };
    }
    m.count = 2;
    m
}

/// Clips the segment `a`-`b` to the side planes of polygon edge `i` and
/// returns the two end points (ascending along the edge) with their
/// heights above the edge line. `None` if the segment is perpendicular to
/// the edge or does not overlap it.
fn clip_to_edge(poly: &Shape, i: usize, a: FPVec2, b: FPVec2) -> Option<[(FPVec2, FP); 2]> {
    let n = poly.count as usize;
    let nrm = poly.normals[i];
    let v1 = poly.verts[i];
    let v2 = poly.verts[(i + 1) % n];
    let et = perp(nrm);
    let (tv1, tv2) = (et.dotf(v1), et.dotf(v2));
    let (ta, tb) = (et.dotf(a), et.dotf(b));
    if ta == tb {
        return None;
    }
    let lo = ta.min(tb).max(tv1);
    let hi = ta.max(tb).min(tv2);
    if hi <= lo {
        return None;
    }
    let mut out = [(FPVec2::ZERO, FP::ZERO); 2];
    for (k, s) in [lo, hi].into_iter().enumerate() {
        let q = point_at(a, b, ta, tb, s);
        out[k] = (q, nrm.dotf(q - v1));
    }
    Some(out)
}

fn face_points(m: &mut Manifold, nrm: FPVec2, pts: &[(FPVec2, FP)], edge: usize, r: FP, margin: FP) {
    m.normal = -nrm;
    m.count = 0;
    for (k, &(q, h)) in pts.iter().enumerate() {
        let sep = h - r;
        if sep <= margin {
            let id = ID_FACE + ((edge as u32) << 4) + k as u32;
            m.points[m.count] = ManifoldPoint { point: q - nrm * (r + sep / 2), separation: sep, id };
            m.count += 1;
        }
    }
}

/// Capsule (A) against a polygon (B) with world transform `xp`. The normal
/// points from the capsule to the polygon.
fn capsule_polygon(cap: &Cap, poly: &Shape, xp: &Xf, margin: FP) -> Manifold {
    let a = xp.inv_rot(cap.a - xp.p);
    let b = xp.inv_rot(cap.b - xp.p);
    let u = xp.inv_rot(cap.u);
    let r = cap.r;
    let reach = r + margin;
    let n = poly.count as usize;

    // Separation along the polygon face normals (of the core segment).
    let mut sep_f = FP::MIN;
    let mut fi = 0usize;
    for i in 0..n {
        let s = poly.normals[i].dotf(a - poly.verts[i]).min(poly.normals[i].dotf(b - poly.verts[i]));
        if s > reach {
            return Manifold::default();
        }
        if s > sep_f {
            sep_f = s;
            fi = i;
        }
    }
    // Separation along the segment normal.
    let nu = perp(u);
    let mut hmin = FP::MAX;
    let mut hmax = FP::MIN;
    let (mut vmin, mut vmax) = (0usize, 0usize);
    for k in 0..n {
        let h = nu.dotf(poly.verts[k] - a);
        if h < hmin {
            hmin = h;
            vmin = k;
        }
        if h > hmax {
            hmax = h;
            vmax = k;
        }
    }
    let side_pos = hmin >= -hmax;
    let sep_s = if side_pos { hmin } else { -hmax };
    if sep_s > reach {
        return Manifold::default();
    }

    let mut m;
    if sep_f > FP::ZERO || sep_s > FP::ZERO {
        // The cores are apart: exact distance between the segment and the
        // polygon boundary.
        let mut best = FP::MAX;
        let (mut cs, mut cq) = (a, poly.verts[0]);
        for i in 0..n {
            let (c1, c2) = seg_seg_closest(a, b, poly.verts[i], poly.verts[(i + 1) % n]);
            let d2 = (c2 - c1).length_sq();
            if d2 < best {
                best = d2;
                cs = c1;
                cq = c2;
            }
        }
        if best > reach * reach {
            return Manifold::default();
        }
        let d = fastmath::sqrt(best);
        let nl = if d > FP::ZERO { vdiv(cq - cs, d) } else { -poly.normals[fi] };
        let sep = d - r;
        m = single(nl, sep, cs + nl * (r + sep / 2), ID_ONE);
        // Flat contact: the face that points at the capsule, when the
        // segment lies along it.
        if d > FP::ZERO {
            let mut bi = 0usize;
            let mut bd = FP::MIN;
            for i in 0..n {
                let dt = -poly.normals[i].dotf(nl);
                if dt > bd {
                    bd = dt;
                    bi = i;
                }
            }
            if bd >= FACE_DOT && poly.normals[bi].dotf(u).abs() <= PARALLEL_TOL {
                if let Some(pts) = clip_to_edge(poly, bi, a, b) {
                    if pts.iter().all(|&(_, h)| h >= FP::ZERO && h - r <= margin) {
                        face_points(&mut m, poly.normals[bi], &pts, bi, r, margin);
                    }
                }
            }
        }
    } else if sep_s > sep_f + FP::from_raw(66) {
        // Deep overlap, segment axis of least penetration. The polygon
        // vertex nearest to the segment line is the contact point.
        let (nn, v, hh) = if side_pos { (nu, vmin, hmin) } else { (-nu, vmax, -hmax) };
        let vp = poly.verts[v];
        let sep = hh - r;
        m = single(nn, sep, vp - nn * (sep / 2), ID_AXIS);
    } else {
        // Deep overlap, polygon face of least penetration.
        let nrm = poly.normals[fi];
        m = Manifold::default();
        let pts = match clip_to_edge(poly, fi, a, b) {
            Some(p) => p,
            None => [(a, nrm.dotf(a - poly.verts[fi])), (b, nrm.dotf(b - poly.verts[fi]))],
        };
        face_points(&mut m, nrm, &pts, fi, r, margin);
        if m.count == 0 {
            let (q, h) = if pts[0].1 <= pts[1].1 { pts[0] } else { pts[1] };
            let sep = h - r;
            m = single(-nrm, sep, q - nrm * (r + sep / 2), ID_ONE);
        }
    }
    m.normal = xp.rot(m.normal);
    for k in 0..m.count {
        m.points[k].point = xp.apply(m.points[k].point);
    }
    m
}
