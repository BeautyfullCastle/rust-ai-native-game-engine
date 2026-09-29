//! Scene queries: ray casts and circle casts.
//!
//! Queries walk every collider (O(n), no acceleration structure) in a
//! fixed order and break ties on distance by the lower entity index, so a
//! result depends only on frame state. They take `&mut Frame` only because
//! `orr_ecs` queries need it; they never modify the frame.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};

use crate::collide::{collide, Xf};
use crate::types::{Body, Collider, Shape, SHAPE_CIRCLE};

/// Which colliders a query considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryFilter {
    /// Only colliders whose `layer` intersects this mask are tested.
    pub mask: u32,
    /// Also test sensor colliders.
    pub include_sensors: bool,
}

impl Default for QueryFilter {
    fn default() -> Self {
        QueryFilter { mask: u32::MAX, include_sensors: false }
    }
}

/// Result of a cast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The entity that was hit.
    pub entity: Entity,
    /// Contact point on the hit shape's surface.
    pub point: FPVec2,
    /// Surface normal facing the caster. For a start that already overlaps
    /// the shape: the direction that separates the two.
    pub normal: FPVec2,
    /// Travel distance along the (normalized) direction. 0 if the cast
    /// starts inside the shape.
    pub distance: FP,
}

fn ray_circle(center: FPVec2, r: FP, o: FPVec2, d: FPVec2, max: FP) -> Option<(FP, FPVec2)> {
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
    let q = o + d * t;
    Some((t, (q - center) / r))
}

fn ray_polygon(shape: &Shape, xf: &Xf, o: FPVec2, d: FPVec2, max: FP) -> Option<(FP, FPVec2)> {
    let o = xf.inv_rot(o - xf.p);
    let d_l = xf.inv_rot(d);
    let mut lower = FP::ZERO;
    let mut upper = max;
    let mut idx: Option<usize> = None;
    for i in 0..shape.count as usize {
        let n = shape.normals[i];
        let num = n.dot(shape.verts[i] - o);
        let den = n.dot(d_l);
        if den == FP::ZERO {
            if num < FP::ZERO {
                return None;
            }
        } else if den < FP::ZERO {
            if num < lower * den {
                lower = num / den;
                idx = Some(i);
            }
        } else if num < upper * den {
            upper = num / den;
        }
        if upper < lower {
            return None;
        }
    }
    match idx {
        Some(i) => Some((lower, xf.rot(shape.normals[i]))),
        None => Some((FP::ZERO, -d)),
    }
}

fn cast_circle_polygon(shape: &Shape, xf: &Xf, o: FPVec2, d: FPVec2, max: FP, radius: FP) -> Option<(FP, FPVec2)> {
    let o = xf.inv_rot(o - xf.p);
    let d = xf.inv_rot(d);
    let n = shape.count as usize;
    let mut best: Option<(FP, FPVec2)> = None;
    let mut consider = |t: FP, nl: FPVec2| {
        if best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, nl));
        }
    };
    for i in 0..n {
        let nrm = shape.normals[i];
        let v = shape.verts[i];
        let den = nrm.dot(d);
        if den < FP::ZERO {
            let t = (nrm.dot(v) + radius - nrm.dot(o)) / den;
            if t >= FP::ZERO && t <= max {
                let q = o + d * t;
                let e = shape.verts[(i + 1) % n] - v;
                let s = (q - v).dot(e);
                if s >= FP::ZERO && s <= e.dot(e) {
                    consider(t, nrm);
                }
            }
        }
        if let Some((t, nl)) = ray_circle(v, radius, o, d, max) {
            consider(t, nl);
        }
    }
    best.map(|(t, nl)| (t, xf.rot(nl)))
}

/// Casts a ray. `dir` need not be normalized; a zero `dir` hits nothing.
pub fn raycast(frame: &mut Frame, origin: FPVec2, dir: FPVec2, max_distance: FP, filter: QueryFilter) -> Option<RayHit> {
    let d = dir.normalize_or_zero();
    if d == FPVec2::ZERO {
        return None;
    }
    let mut best: Option<RayHit> = None;
    for (e, (b, c)) in frame.query::<(&Body, &Collider)>() {
        if c.layer & filter.mask == 0 || (c.is_sensor() && !filter.include_sensors) {
            continue;
        }
        let xf = Xf::new(b.pos, b.angle);
        let hit = if c.shape.kind == SHAPE_CIRCLE {
            ray_circle(b.pos, c.shape.radius, origin, d, max_distance)
        } else {
            ray_polygon(&c.shape, &xf, origin, d, max_distance)
        };
        if let Some((t, normal)) = hit {
            if better(&best, t, e) {
                best = Some(RayHit { entity: e, point: origin + d * t, normal, distance: t });
            }
        }
    }
    best
}

/// Sweeps a circle of `radius` from `origin` along `dir` and returns the
/// first collider it touches. Polygons are swept exactly (edge offsets plus
/// corner circles). Other swept shapes are not supported yet.
pub fn circle_cast(
    frame: &mut Frame,
    origin: FPVec2,
    dir: FPVec2,
    max_distance: FP,
    radius: FP,
    filter: QueryFilter,
) -> Option<RayHit> {
    circle_cast_ignoring(frame, origin, dir, max_distance, radius, filter, Entity::NONE)
}

/// Like [`circle_cast`], but never reports `ignore` (e.g. the caster's own
/// collider). Pass `Entity::NONE` to ignore nothing.
pub fn circle_cast_ignoring(
    frame: &mut Frame,
    origin: FPVec2,
    dir: FPVec2,
    max_distance: FP,
    radius: FP,
    filter: QueryFilter,
    ignore: Entity,
) -> Option<RayHit> {
    let d = dir.normalize_or_zero();
    if d == FPVec2::ZERO {
        return None;
    }
    let caster = Shape::circle(radius);
    let caster_xf = Xf { p: origin, c: FP::ONE, s: FP::ZERO };
    let mut best: Option<RayHit> = None;
    for (e, (b, c)) in frame.query::<(&Body, &Collider)>() {
        if e == ignore || c.layer & filter.mask == 0 || (c.is_sensor() && !filter.include_sensors) {
            continue;
        }
        let xf = Xf::new(b.pos, b.angle);
        // Already overlapping at the start: report distance 0.
        let m = collide(&caster, &caster_xf, &c.shape, &xf, FP::ZERO);
        let hit = if m.count > 0 && m.points[0].separation <= FP::ZERO {
            Some((FP::ZERO, -m.normal))
        } else if c.shape.kind == SHAPE_CIRCLE {
            ray_circle(b.pos, c.shape.radius + radius, origin, d, max_distance)
        } else {
            cast_circle_polygon(&c.shape, &xf, origin, d, max_distance, radius)
        };
        if let Some((t, normal)) = hit {
            if better(&best, t, e) {
                let center = origin + d * t;
                best = Some(RayHit { entity: e, point: center - normal * radius, normal, distance: t });
            }
        }
    }
    best
}

fn better(best: &Option<RayHit>, t: FP, e: Entity) -> bool {
    match best {
        None => true,
        Some(h) => t < h.distance || (t == h.distance && e.index < h.entity.index),
    }
}

/// Result of [`move_and_slide`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterMove {
    /// Final center position.
    pub pos: FPVec2,
    /// True if a contact normal was within the slope limit of `up`.
    pub grounded: bool,
    /// Normal of the last walkable contact (`FPVec2::ZERO` if not grounded).
    pub ground_normal: FPVec2,
    /// Number of slide iterations that hit something.
    pub hits: u32,
}

/// Settings for [`move_and_slide`].
#[derive(Clone, Copy, Debug)]
pub struct CharacterParams {
    /// Circle radius of the character.
    pub radius: FP,
    /// Direction that counts as up, unit length.
    pub up: FPVec2,
    /// A surface is walkable if `dot(normal, up) >= min_ground_dot`.
    pub min_ground_dot: FP,
    /// Gap kept between the character and surfaces.
    pub skin: FP,
    /// Query filter for what blocks the character.
    pub filter: QueryFilter,
}

impl CharacterParams {
    /// Circle character with up = +y, walkable slopes up to about 45 degrees
    /// and a 0.01 skin.
    pub fn new(radius: FP) -> Self {
        CharacterParams {
            radius,
            up: FPVec2::Y,
            min_ground_dot: orr_fp::fp!(0.7),
            skin: orr_fp::fp!(0.01),
            filter: QueryFilter::default(),
        }
    }
}

/// Kinematic character controller step for a circle. Moves from `pos` by
/// `delta`, stopping at solid colliders (except `ignore`) and sliding along
/// them, for at most 4 contacts. It only reads the frame; the caller
/// writes the resulting position back (usually into a kinematic `Body`).
pub fn move_and_slide(frame: &mut Frame, ignore: Entity, pos: FPVec2, delta: FPVec2, p: &CharacterParams) -> CharacterMove {
    let mut out = CharacterMove { pos, grounded: false, ground_normal: FPVec2::ZERO, hits: 0 };
    let mut remaining = delta;
    for _ in 0..4 {
        let len = remaining.length();
        if len <= FP::EPSILON * 4 {
            break;
        }
        let dir = remaining / len;
        let hit = circle_cast_ignoring(frame, out.pos, dir, len + p.skin, p.radius, p.filter, ignore);
        let Some(hit) = hit else {
            out.pos += remaining;
            break;
        };
        out.hits += 1;
        let travel = (hit.distance - p.skin).max(FP::ZERO).min(len);
        out.pos += dir * travel;
        if hit.normal.dot(p.up) >= p.min_ground_dot {
            out.grounded = true;
            out.ground_normal = hit.normal;
        }
        let rest = dir * (len - travel);
        remaining = rest - hit.normal * rest.dot(hit.normal).min(FP::ZERO);
    }
    out
}
