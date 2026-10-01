//! Scene queries: ray casts against every shape and an exact sphere cast.
//!
//! A sphere cast is a ray against the Minkowski sum of the target and the
//! sphere: a sphere or capsule target just grows its radius, a box becomes
//! three boxes grown along one axis each plus twelve capsules for the
//! edges (the corners are the capsule ends). The first hit of the union is
//! the earliest hit of any piece, so there is no iteration and no tunnelling
//! at any speed. Touching counts as a hit; a start that already overlaps
//! reports distance 0 and the normal `-dir`. Ties in distance go to the
//! lower entity index.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec3, FP};

use crate::geom::{dotr, ratio_q16, segment_of, Xf};
use crate::types::{Body, Collider, SHAPE_BOX, SHAPE_SPHERE};

/// Which colliders a query considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryFilter {
    /// Only colliders whose `layer` intersects this mask are tested.
    pub mask: u32,
}

impl Default for QueryFilter {
    fn default() -> Self {
        QueryFilter { mask: u32::MAX }
    }
}

/// Result of a cast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The entity that was hit.
    pub entity: Entity,
    /// Contact point on the hit shape's surface.
    pub point: FPVec3,
    /// Surface normal facing the caster. For a start that already overlaps
    /// the shape: `-dir`.
    pub normal: FPVec3,
    /// Travel distance along the (normalized) direction. 0 if the cast
    /// starts inside the shape.
    pub distance: FP,
}

type Hit = Option<(FP, FPVec3)>;

fn ray_sphere(center: FPVec3, r: FP, o: FPVec3, d: FPVec3, max: FP) -> Hit {
    let m = o - center;
    let c = m.dot(m) - r * r;
    if c <= FP::ZERO {
        return Some((FP::ZERO, -d));
    }
    let b = m.dot(d);
    if b > FP::ZERO {
        return None;
    }
    let disc = b * b - c;
    if disc < FP::ZERO {
        return None;
    }
    let t = -b - disc.sqrt();
    if t > max {
        return None;
    }
    let t = t.max(FP::ZERO);
    let n = (o + d * t - center) / r;
    Some((t, n))
}

/// Ray against the box `|x_j| <= h_j` in its own frame.
fn ray_box_local(o: FPVec3, d: FPVec3, h: FPVec3, max: FP) -> Hit {
    let (mut tmin, mut tmax) = (i64::MIN / 4, i64::MAX / 4);
    let mut axis = 0usize;
    let mut sign = FP::ONE;
    let mut inside = true;
    for j in 0..3 {
        let (oj, dj, hj) = (o.get(j).raw(), d.get(j).raw(), h.get(j).raw());
        if oj.abs() > hj {
            inside = false;
        }
        if dj == 0 {
            if oj.abs() > hj {
                return None;
            }
            continue;
        }
        let mut t1 = ratio_q16((-hj - oj) as i128, dj as i128);
        let mut t2 = ratio_q16((hj - oj) as i128, dj as i128);
        // Entering through the face the ray points against.
        let s = if dj > 0 { FP::MINUS_ONE } else { FP::ONE };
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        if t1 > tmin {
            tmin = t1;
            axis = j;
            sign = s;
        }
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    if inside {
        return Some((FP::ZERO, -d));
    }
    if tmax < 0 || tmin > max.raw() {
        return None;
    }
    let mut n = FPVec3::ZERO;
    n.set(axis, sign);
    Some((FP::from_raw(tmin.max(0)), n))
}

/// Ray against the capsule of segment `p0..p1` and radius `r`.
fn ray_capsule(p0: FPVec3, p1: FPVec3, r: FP, o: FPVec3, d: FPVec3, max: FP) -> Hit {
    let a = p1 - p0;
    let len2 = dotr(a, a);
    let mut best: Hit = None;
    let mut consider = |h: Hit| {
        if let Some((t, n)) = h {
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, n));
            }
        }
    };
    if len2 > 1 << 10 {
        let u = a / a.length();
        let m = o - p0;
        let mu = m.dot(u);
        let du = d.dot(u);
        let m2 = m - u * mu;
        let d2 = d - u * du;
        let qa = d2.dot(d2);
        let qc = m2.dot(m2) - r * r;
        if qc <= FP::ZERO && mu >= FP::ZERO && mu <= a.length() {
            return Some((FP::ZERO, -d));
        }
        if qa.raw() >= 64 {
            let qb = m2.dot(d2);
            let disc = qb * qb - qa * qc;
            if disc >= FP::ZERO && (qb < FP::ZERO || qc <= FP::ZERO) {
                let t = (-qb - disc.sqrt()) / qa;
                if t >= FP::ZERO && t <= max {
                    let y = mu + du * t;
                    if y >= FP::ZERO && y <= a.length() {
                        let p = o + d * t;
                        let n = (p - (p0 + u * y)) / r;
                        consider(Some((t, n)));
                    }
                }
            }
        }
    }
    consider(ray_sphere(p0, r, o, d, max));
    consider(ray_sphere(p1, r, o, d, max));
    best
}

/// Ray against a box grown by the radius `rc` (rounded edges and corners),
/// in the box frame.
fn ray_rounded_box_local(o: FPVec3, d: FPVec3, h: FPVec3, rc: FP, max: FP) -> Hit {
    if rc == FP::ZERO {
        return ray_box_local(o, d, h, max);
    }
    let mut best: Hit = None;
    let mut consider = |hit: Hit| {
        if let Some((t, n)) = hit {
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, n));
            }
        }
    };
    for i in 0..3 {
        let mut g = h;
        g.set(i, h.get(i) + rc);
        consider(ray_box_local(o, d, g, max));
    }
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
                consider(ray_capsule(e0, e1, rc, o, d, max));
            }
        }
    }
    best
}

fn better(best: &Option<RayHit>, t: FP, e: Entity) -> bool {
    match best {
        None => true,
        Some(b) => t < b.distance || (t == b.distance && e.index < b.entity.index),
    }
}

fn cast(frame: &mut Frame, origin: FPVec3, dir: FPVec3, max_distance: FP, radius: FP, filter: QueryFilter) -> Option<RayHit> {
    let d = dir.normalize_or_zero();
    if d == FPVec3::ZERO {
        return None;
    }
    let mut best: Option<RayHit> = None;
    for (e, (b, c)) in frame.query::<(&Body, &Collider)>() {
        if c.layer & filter.mask == 0 {
            continue;
        }
        let xf = Xf { p: b.pos, r: orr_fp::FPMat3::from_quat(b.rot) };
        let sh = &c.shape;
        let hit = if sh.kind == SHAPE_SPHERE {
            ray_sphere(b.pos, sh.radius + radius, origin, d, max_distance)
        } else if sh.kind == SHAPE_BOX {
            let ol = xf.to_local(origin);
            let dl = xf.r.tmul_vec(d);
            ray_rounded_box_local(ol, dl, sh.half, radius, max_distance).map(|(t, n)| (t, if t == FP::ZERO { -d } else { xf.r.mul_vec(n) }))
        } else {
            let (p0, p1) = segment_of(sh, &xf);
            ray_capsule(p0, p1, sh.radius + radius, origin, d, max_distance)
        };
        if let Some((t, normal)) = hit {
            if t <= max_distance && better(&best, t, e) {
                let point = origin + d * t - normal * radius;
                best = Some(RayHit { entity: e, point, normal, distance: t });
            }
        }
    }
    best
}

/// Casts a ray and returns the first collider it hits. `dir` need not be
/// normalized. A ray that starts inside a shape hits it at distance 0.
pub fn raycast(frame: &mut Frame, origin: FPVec3, dir: FPVec3, max_distance: FP, filter: QueryFilter) -> Option<RayHit> {
    cast(frame, origin, dir, max_distance, FP::ZERO, filter)
}

/// Sweeps a sphere of `radius` from `origin` along `dir` and returns the
/// first collider it touches (exact, see the module docs). `point` is the
/// contact point on the target's surface.
pub fn sphere_cast(
    frame: &mut Frame,
    origin: FPVec3,
    dir: FPVec3,
    max_distance: FP,
    radius: FP,
    filter: QueryFilter,
) -> Option<RayHit> {
    cast(frame, origin, dir, max_distance, radius, filter)
}
