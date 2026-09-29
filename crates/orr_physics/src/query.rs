//! Scene queries: ray casts, shape casts and character controllers.
//!
//! Queries walk every collider (O(n), no acceleration structure) in a
//! fixed order and break ties on distance by the lower entity index, so a
//! result depends only on frame state. They take `&mut Frame` only because
//! `orr_ecs` queries need it; they never modify the frame.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};

use crate::cast::{ray_rounded_hull, sweep, Caster};
use crate::collide::{collide, Xf};
use crate::types::{Body, Collider, Shape, SHAPE_CAPSULE, SHAPE_CIRCLE};

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
        } else if c.shape.kind == SHAPE_CAPSULE {
            let pts = [xf.apply(c.shape.verts[0]), xf.apply(c.shape.verts[1])];
            ray_rounded_hull(&pts, c.shape.radius, origin, d, max_distance)
                .map(|h| if h.inside { (FP::ZERO, -d) } else { (h.t, h.normal) })
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

/// Result of [`shape_cast`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeHit {
    /// The entity that was hit.
    pub entity: Entity,
    /// Time of impact as a fraction of the cast distance, in `[0, 1]`.
    pub fraction: FP,
    /// Travel distance to the impact (`fraction * max_distance`).
    pub distance: FP,
    /// A contact point between the two surfaces at the time of impact
    /// (the midpoint of the contact manifold; for a flat contact the
    /// middle of the touching range).
    pub point: FPVec2,
    /// Unit normal from the hit shape toward the caster. For a start that
    /// already overlaps: the direction that separates the two.
    pub normal: FPVec2,
}

/// Sweeps a circle, capsule or convex polygon along `dir` for at most
/// `max_distance` and returns the first collider it touches.
///
/// The caster sits at `pos` with rotation `angle`, which stays fixed during
/// the cast. `dir` need not be normalized; a zero `dir` hits nothing.
/// `ignore` is skipped (`Entity::NONE` skips nothing), and so are sensors
/// unless the filter includes them.
///
/// The sweep is exact, not iterated: the ray of the caster origin is cast
/// against the Minkowski sum of the target and the mirrored caster (see
/// `cast`). The distance is accurate to a few 1/65536 units. Touching
/// counts as a hit: a caster that grazes a surface reports it, one that
/// misses by more than the rounding does not. A caster that already
/// overlaps a shape reports `fraction == 0`; one that touches exactly is a
/// hit only if it moves toward the shape. Ties in distance go to the lower
/// entity index.
#[allow(clippy::too_many_arguments)]
pub fn shape_cast(
    frame: &mut Frame,
    shape: &Shape,
    pos: FPVec2,
    angle: FP,
    dir: FPVec2,
    max_distance: FP,
    filter: QueryFilter,
    ignore: Entity,
) -> Option<ShapeHit> {
    let d = dir.normalize_or_zero();
    if d == FPVec2::ZERO || max_distance < FP::ZERO {
        return None;
    }
    let caster = Caster::new(shape, angle);
    let mut best: Option<(FP, Entity, FPVec2, Body, Collider)> = None;
    for (e, (b, c)) in frame.query::<(&Body, &Collider)>() {
        if e == ignore || c.layer & filter.mask == 0 || (c.is_sensor() && !filter.include_sensors) {
            continue;
        }
        let xf = Xf::new(b.pos, b.angle);
        let Some(h) = sweep(&caster, &c.shape, &xf, pos, d, max_distance) else { continue };
        let wins = match &best {
            None => true,
            Some((bt, be, ..)) => h.t < *bt || (h.t == *bt && e.index < be.index),
        };
        if wins {
            best = Some((h.t, e, h.normal, *b, *c));
        }
    }
    let (t, entity, normal, b, c) = best?;
    let at = Xf::new(pos + d * t, angle);
    let xf = Xf::new(b.pos, b.angle);
    let m = collide(shape, &at, &c.shape, &xf, orr_fp::fp!(0.05));
    let point = if m.count > 0 {
        let mut sum = FPVec2::ZERO;
        for k in 0..m.count {
            sum += m.points[k].point;
        }
        sum / FP::from_int(m.count as i32)
    } else {
        at.p - normal * shape.bounding_radius()
    };
    let fraction = if max_distance == FP::ZERO { FP::ZERO } else { (t / max_distance).min(FP::ONE) };
    Some(ShapeHit { entity, fraction, distance: t, point, normal })
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

/// Settings for [`move_and_slide_capsule`].
#[derive(Clone, Copy, Debug)]
pub struct CapsuleCharacterParams {
    /// Half length of the capsule segment (the body is `2 * (half_length +
    /// radius)` tall when upright).
    pub half_length: FP,
    /// Capsule radius.
    pub radius: FP,
    /// Rotation of the capsule (0 = upright, segment along +y). Fixed
    /// during the move.
    pub angle: FP,
    /// Direction that counts as up, unit length.
    pub up: FPVec2,
    /// A surface is walkable if `dot(normal, up) >= min_ground_dot`.
    pub min_ground_dot: FP,
    /// Gap kept between the character and surfaces.
    pub skin: FP,
    /// Highest ledge the character walks up in one move (0 = no stepping).
    pub step_height: FP,
    /// How far the character sticks to walkable ground when it walks down
    /// a slope or off a small ledge (0 = no snapping).
    pub snap_distance: FP,
    /// Query filter for what blocks the character.
    pub filter: QueryFilter,
}

impl CapsuleCharacterParams {
    /// Upright capsule with up = +y, walkable slopes up to about 45 degrees,
    /// a 0.01 skin, no stepping and no snapping.
    pub fn new(half_length: FP, radius: FP) -> Self {
        CapsuleCharacterParams {
            half_length,
            radius,
            angle: FP::ZERO,
            up: FPVec2::Y,
            min_ground_dot: orr_fp::fp!(0.7),
            skin: orr_fp::fp!(0.01),
            step_height: FP::ZERO,
            snap_distance: FP::ZERO,
            filter: QueryFilter::default(),
        }
    }
}

/// Slides a shape along `delta` for at most 4 contacts. With `stop_on_ground`
/// a walkable contact ends the slide (so the character does not creep down
/// a slope it stands on).
#[allow(clippy::too_many_arguments)]
fn slide(
    frame: &mut Frame,
    ignore: Entity,
    shape: &Shape,
    p: &CapsuleCharacterParams,
    pos: FPVec2,
    delta: FPVec2,
    stop_on_ground: bool,
    out: &mut CharacterMove,
) -> FPVec2 {
    let mut pos = pos;
    let mut remaining = delta;
    for _ in 0..4 {
        let len = remaining.length();
        if len <= FP::EPSILON * 4 {
            break;
        }
        let dir = remaining / len;
        let Some(hit) = shape_cast(frame, shape, pos, p.angle, dir, len + p.skin, p.filter, ignore) else {
            pos += remaining;
            break;
        };
        out.hits += 1;
        let travel = (hit.distance - p.skin).max(FP::ZERO).min(len);
        pos += dir * travel;
        if hit.normal.dot(p.up) >= p.min_ground_dot {
            out.grounded = true;
            out.ground_normal = hit.normal;
            if stop_on_ground {
                break;
            }
        }
        let rest = dir * (len - travel);
        remaining = rest - hit.normal * rest.dot(hit.normal).min(FP::ZERO);
    }
    pos
}

/// Kinematic character controller step for a capsule (see
/// [`move_and_slide`] for the circle version). Moves from `pos` by `delta`
/// and returns the final position, ground state and the ground normal. It
/// only reads the frame; the caller writes the position back.
///
/// The move has three parts, all built on [`shape_cast`]:
///
/// 1. The part of `delta` across `up` slides along blockers, at most 4
///    contacts. With `step_height > 0`, a move that a wall-like surface
///    cuts short is retried from a raised position (up by at most
///    `step_height`, across, then down again). The step is used only if it
///    starts on the ground, lands on walkable ground and gets farther than
///    the plain slide.
/// 2. The part of `delta` along `up` slides the same way. A walkable
///    contact stops it, so a character standing on a slope does not slide
///    down.
/// 3. A probe of `2 * skin` (plus `snap_distance`) below the capsule sets
///    `grounded` and `ground_normal`, so a character that stands still on
///    the ground is grounded too. With `snap_distance > 0` a character that
///    does not move up is pulled down onto the ground.
///
/// A surface is walkable when `dot(normal, up) >= min_ground_dot`. The
/// capsule keeps `skin` distance from the surfaces it touches.
pub fn move_and_slide_capsule(
    frame: &mut Frame,
    ignore: Entity,
    pos: FPVec2,
    delta: FPVec2,
    p: &CapsuleCharacterParams,
) -> CharacterMove {
    let shape = Shape::capsule(p.half_length, p.radius);
    let mut out = CharacterMove { pos, grounded: false, ground_normal: FPVec2::ZERO, hits: 0 };
    let vert = p.up * delta.dot(p.up);
    let horiz = delta - vert;

    // Part 1: across `up`.
    let mut cur = slide(frame, ignore, &shape, p, pos, horiz, false, &mut out);
    let hlen = horiz.length();
    if p.step_height > FP::ZERO && hlen > FP::EPSILON * 4 {
        let hdir = horiz / hlen;
        let progress = (cur - pos).dot(hdir);
        if progress < hlen - p.skin * 2 {
            if let Some((stepped, n)) = try_step(frame, ignore, &shape, p, pos, horiz) {
                if (stepped - pos).dot(hdir) > progress + p.skin {
                    cur = stepped;
                    out.grounded = true;
                    out.ground_normal = n;
                }
            }
        }
    }

    // Part 2: along `up`.
    cur = slide(frame, ignore, &shape, p, cur, vert, true, &mut out);

    // Part 3: ground probe and snap.
    let moving_up = delta.dot(p.up) > FP::ZERO;
    let probe = p.skin * 2 + if moving_up { FP::ZERO } else { p.snap_distance };
    let down = -p.up;
    if let Some(hit) = shape_cast(frame, &shape, cur, p.angle, down, probe, p.filter, ignore) {
        if hit.normal.dot(p.up) >= p.min_ground_dot {
            out.grounded = true;
            out.ground_normal = hit.normal;
            if !moving_up && p.snap_distance > FP::ZERO {
                cur += down * (hit.distance - p.skin).max(FP::ZERO);
            }
        }
    }
    out.pos = cur;
    out
}

/// One step attempt: up, across, down. Returns the landing position and the
/// ground normal if the character ends on walkable ground.
fn try_step(
    frame: &mut Frame,
    ignore: Entity,
    shape: &Shape,
    p: &CapsuleCharacterParams,
    pos: FPVec2,
    horiz: FPVec2,
) -> Option<(FPVec2, FPVec2)> {
    let down = -p.up;
    // Only from the ground.
    let ground = shape_cast(frame, shape, pos, p.angle, down, p.skin * 2 + p.snap_distance, p.filter, ignore)?;
    if ground.normal.dot(p.up) < p.min_ground_dot {
        return None;
    }
    let up_dist = match shape_cast(frame, shape, pos, p.angle, p.up, p.step_height + p.skin, p.filter, ignore) {
        Some(h) => (h.distance - p.skin).max(FP::ZERO),
        None => p.step_height,
    }
    .min(p.step_height);
    if up_dist <= FP::ZERO {
        return None;
    }
    let raised = pos + p.up * up_dist;
    let mut scratch = CharacterMove { pos, grounded: false, ground_normal: FPVec2::ZERO, hits: 0 };
    let across = slide(frame, ignore, shape, p, raised, horiz, false, &mut scratch);
    let land = shape_cast(frame, shape, across, p.angle, down, up_dist + p.skin * 2, p.filter, ignore)?;
    if land.normal.dot(p.up) < p.min_ground_dot {
        return None;
    }
    let drop = (land.distance - p.skin).max(FP::ZERO);
    Some((across + down * drop, land.normal))
}
