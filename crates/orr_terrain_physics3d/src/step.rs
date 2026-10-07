//! Atomic bounded sphere/heightfield runtime adapter.
//!
//! This first profile is discrete, with an explicit no-center-crossing guard:
//! the provider path clamps every post-solve velocity before integration. For
//! largest substep h and per-axis clamp V, each displacement has L1 length at
//! most D = ceil(3 V h) + 3 raw units. Admission requires D <= r/4. At every
//! substep boundary (including the final boundary), the nearest finite-mesh
//! distance must be >= r/2. Thus a center cannot cross any finite triangle in a
//! substep: a crossing would require a path of at least r/2 from the initial
//! center, contradicting the r/4 displacement bound. Invalid/deep-overlap state
//! is refused atomically rather than accepted as collision-free. This is not
//! swept CCD and does not promise exact time of first grazing contact.

use crate::asset::{terrain_view, TerrainView};
use crate::geometry::{sphere_contacts, GeometryError};
use orr_ecs::{Entity, Frame};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{
    Body, Collider, ContactCache, PhysicsConfig, PhysicsState, Scratch, StaticContact,
    StaticContactBody, StaticContactObject, BODY_DYNAMIC, BODY_KINEMATIC, BODY_STATIC, SHAPE_BOX,
    SHAPE_CAPSULE, SHAPE_SPHERE,
};
use std::fmt;

/// Maximum ordinary physics bodies admitted by the terrain runtime profile.
pub const MAX_TERRAIN_BODIES: u32 = 64;
/// Maximum triangles visited for a sphere on one provider refresh.
pub const MAX_SPHERE_CANDIDATES: u32 = 4096;
/// Maximum contacts retained per sphere on one refresh.
pub const MAX_SPHERE_CONTACTS: usize = 64;
/// An unsupported, malformed, or unsafe frame/configuration. Frame is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainStepError(pub String);
impl fmt::Display for TerrainStepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for TerrainStepError {}
fn bad(message: &str) -> TerrainStepError {
    TerrainStepError(message.to_owned())
}
fn vec_in(v: FPVec3, lo: FP, hi: FP) -> bool {
    [v.x, v.y, v.z].into_iter().all(|x| (lo..=hi).contains(&x))
}
fn positive(v: FP, max: FP) -> bool {
    v >= FP::ZERO && v <= max
}

/// A useful supported default for radius >= 0.5: 60 Hz, eight substeps,
/// per-axis linear speed at most 16. Smaller spheres need more substeps.
pub fn terrain_config() -> PhysicsConfig {
    PhysicsConfig {
        max_linear_speed: fp!(16),
        max_angular_speed: fp!(16),
        ..PhysicsConfig::default()
    }
}
fn validate_config(c: &PhysicsConfig) -> Result<FP, TerrainStepError> {
    if c.dt <= FP::ZERO
        || c.dt > FP::from_ratio(1, 30)
        || !(1..=64).contains(&c.substeps)
        || c.dt.raw() < c.substeps as i64
        || !(1..=16).contains(&c.velocity_iterations)
        || !positive(c.baumgarte, FP::ONE)
        || !positive(c.linear_slop, fp!(0.0625))
        || !positive(c.contact_margin, fp!(0.25))
        || !positive(c.restitution_threshold, fp!(128))
        || !positive(c.max_correction_speed, fp!(16))
        || c.max_linear_speed <= FP::ZERO
        || c.max_linear_speed > fp!(32)
        || !positive(c.max_angular_speed, fp!(64))
        || !vec_in(c.gravity, fp!(-128), fp!(128))
        || !positive(c.sleep_linear_speed, fp!(1))
        || !positive(c.sleep_angular_speed, fp!(1))
        || c.sleep_ticks > 65535
        || c._pad != 0
    {
        return Err(bad("unsupported terrain physics configuration"));
    }
    let h = (c.dt.raw() + c.substeps as i64 - 1) / c.substeps as i64;
    let d = ((3i128 * c.max_linear_speed.raw() as i128 * h as i128 + 65535) / 65536) + 3;
    Ok(FP::from_raw(
        i64::try_from(d).map_err(|_| bad("substep displacement overflow"))?,
    ))
}
fn validate_body(b: &Body, c: &Collider, cfg: &PhysicsConfig) -> Result<(), TerrainStepError> {
    if !vec_in(b.pos, fp!(-1024), fp!(1024))
        || !vec_in(b.vel, -cfg.max_linear_speed, cfg.max_linear_speed)
        || !vec_in(b.omega, -cfg.max_angular_speed, cfg.max_angular_speed)
        || !vec_in(b.inv_inertia, FP::ZERO, fp!(1024))
        || !positive(b.inv_mass, fp!(64))
        || (b.kind == BODY_DYNAMIC && b.inv_mass < fp!(0.25))
        || !positive(b.linear_damping, fp!(64))
        || !positive(b.angular_damping, fp!(64))
        || ![BODY_STATIC, BODY_DYNAMIC, BODY_KINEMATIC].contains(&b.kind)
        || b._pad != 0
    {
        return Err(bad("body outside bounded terrain runtime profile"));
    }
    let q = b.rot;
    if ![q.x, q.y, q.z, q.w]
        .into_iter()
        .all(|v| (-FP::ONE..=FP::ONE).contains(&v))
    {
        return Err(bad("invalid body orientation"));
    }
    let norm = [q.x, q.y, q.z, q.w]
        .into_iter()
        .map(|v| (v.raw() as i128) * (v.raw() as i128))
        .sum::<i128>();
    if (norm - (1i128 << 32)).abs() > (1 << 24) {
        return Err(bad("body quaternion must be normalized"));
    }
    if !positive(c.restitution, FP::ONE)
        || !positive(c.friction, fp!(2))
        || c.flags != 0
        || c._pad != 0
        || c.shape._pad != 0
    {
        return Err(bad("unsupported collider material or flags"));
    }
    let s = c.shape;
    let radius_ok = (fp!(0.25)..=fp!(8)).contains(&s.radius);
    let shape_ok = match s.kind {
        SHAPE_SPHERE => radius_ok && s.half == FPVec3::ZERO,
        SHAPE_BOX => s.radius == FP::ZERO && vec_in(s.half, fp!(0.25), fp!(8)),
        SHAPE_CAPSULE => {
            radius_ok
                && s.half.x == FP::ZERO
                && s.half.z == FP::ZERO
                && positive(s.half.y, fp!(8))
                && s.half.y + s.radius <= fp!(8)
        }
        _ => false,
    };
    if !shape_ok {
        return Err(bad("unsupported collider shape dimensions"));
    }
    Ok(())
}
fn permits(view: &TerrainView<'_>, collider: &Collider) -> bool {
    view.collider.layer & collider.mask != 0 && collider.layer & view.collider.mask != 0
}
fn check_sphere(
    view: &TerrainView<'_>,
    b: &Body,
    c: &Collider,
    displacement: FP,
    margin: FP,
) -> Result<Vec<crate::TerrainContact>, GeometryError> {
    if b.kind != BODY_DYNAMIC || c.shape.kind != SHAPE_SPHERE {
        return Err(GeometryError(
            "terrain supports only filter-permitted dynamic spheres",
        ));
    }
    if displacement > c.shape.radius / 4 {
        return Err(GeometryError(
            "substep displacement exceeds radius / 4; lower speed or increase substeps",
        ));
    }
    let contacts = sphere_contacts(
        view,
        b.pos,
        c.shape.radius,
        margin.max(displacement),
        MAX_SPHERE_CANDIDATES,
    )?;
    if contacts.len() > MAX_SPHERE_CONTACTS {
        return Err(GeometryError("sphere contact budget"));
    }
    // Closest-point rounding is <= two raw distance units; an eight-unit
    // guard keeps the geometric r/2 argument conservative at Q16 boundaries.
    if contacts
        .iter()
        .any(|p| p.separation < -(c.shape.radius / 2) + FP::from_raw(8))
    {
        return Err(GeometryError(
            "sphere begins or ends too deeply inside terrain",
        ));
    }
    Ok(contacts)
}

fn validate_physics_registration(frame: &Frame) -> Result<(), TerrainStepError> {
    if frame.registry().singleton_id::<PhysicsState>().is_none()
        || frame.registry().component_id::<Body>().is_none()
        || frame.registry().component_id::<Collider>().is_none()
        || frame.registry().list_id::<ContactCache>().is_none()
    {
        return Err(bad("physics types are not registered"));
    }
    Ok(())
}

/// Validates present terrain physics without changing frame state. Empty
/// optional terrain uses the original physics numerical contract. Unsupported
/// shapes are rejected whenever their filters permit terrain, even far away,
/// asleep, or not currently overlapping its triangles.
pub fn validate_frame(frame: &Frame) -> Result<(), TerrainStepError> {
    validate_physics_registration(frame)?;
    let view = terrain_view(frame).map_err(|e| bad(&e.to_string()))?;
    if view.is_none() {
        return Ok(());
    }
    let state = frame.singleton::<PhysicsState>();
    let cfg = state.config;
    let displacement = validate_config(&cfg)?;
    let cache = frame.list(state.contacts);
    if !frame.list_is_alive(state.contacts) || cache.len() > 16384 {
        return Err(bad("invalid physics contact cache"));
    }
    if view.as_ref().is_some_and(|v| v.collider.friction > fp!(2)) {
        return Err(bad("terrain friction exceeds bounded runtime profile"));
    }
    let (entities, bodies) = frame.dense::<Body>();
    if entities.len() > MAX_TERRAIN_BODIES as usize
        || frame.dense::<Collider>().0.len() != entities.len()
    {
        return Err(bad("terrain body count or unmatched collider"));
    }
    for (&e, b) in entities.iter().zip(bodies) {
        let c = frame
            .get::<Collider>(e)
            .ok_or_else(|| bad("body without collider"))?;
        validate_body(b, c, &cfg)?;
        if let Some(view) = &view {
            if permits(view, c) {
                let contacts = check_sphere(view, b, c, displacement, cfg.contact_margin)
                    .map_err(|e| bad(&e.to_string()))?;
                let cached_support = cache.iter().any(|p| {
                    p.normal_impulse > FP::ZERO
                        && ((p.a == e.index && p.b == view.asset.entity.index)
                            || (p.b == e.index && p.a == view.asset.entity.index))
                });
                if b.sleep & orr_physics3d::SLEEP_FLAG != 0 && cached_support && contacts.is_empty()
                {
                    return Err(bad(
                        "sleeping sphere has stale terrain support; wake after teleporting",
                    ));
                }
            }
        }
    }
    let mut previous = None;
    for c in cache {
        let key = (c.a, c.b, c.id);
        if c.a >= c.b || previous.is_some_and(|p| p >= key) {
            return Err(bad("invalid cached contact ordering"));
        }
        previous = Some(key);
        if !positive(c.normal_impulse, fp!(1024))
            || !vec_in(c.tangent_impulse, fp!(-1024), fp!(1024))
            || c._pad != 0
        {
            return Err(bad("unsafe cached contact impulse"));
        }
    }
    let view = view.as_ref().expect("present terrain checked above");
    let objects = [StaticContactObject {
        entity: view.asset.entity,
        friction: view.collider.friction,
        restitution: view.collider.restitution,
        layer: view.collider.layer,
        mask: view.collider.mask,
    }];
    orr_physics3d::validate_static_sleeping_support(frame, &objects, &mut |bodies, margin, out| {
        provide(view, displacement, &cfg, bodies, margin, out)
    })
    .map_err(|e| bad(&format!("terrain sleeping support: {e:?}")))?;
    Ok(())
}

fn provide(
    view: &TerrainView<'_>,
    displacement: FP,
    cfg: &PhysicsConfig,
    bodies: &[StaticContactBody],
    margin: FP,
    out: &mut Vec<StaticContact>,
) -> Result<(), GeometryError> {
    for b in bodies {
        validate_body(&b.body, &b.collider, cfg)
            .map_err(|_| GeometryError("substep body outside runtime profile"))?;
        if !permits(view, &b.collider) {
            continue;
        }
        for p in check_sphere(view, &b.body, &b.collider, displacement, margin)? {
            out.push(StaticContact {
                body: b.entity,
                static_body: view.asset.entity,
                point: p.point,
                normal: p.normal,
                separation: p.separation,
                feature_id: p.feature_id,
            });
        }
    }
    Ok(())
}

/// Advances bounded physics with actual terrain contacts refreshed every
/// substep and after final integration. Any failure leaves the Frame unchanged.
/// Runtime geometry is read solely from a Frame snapshot; no host asset or
/// pointer-keyed cache participates. Scratch may safely be reused after errors.
/// With no admitted terrain, this invokes the original ordinary physics step
/// unchanged, with its original numerical contract rather than this profile.
pub fn terrain_step(frame: &mut Frame, scratch: &mut Scratch) -> Result<(), TerrainStepError> {
    validate_physics_registration(frame)?;
    if terrain_view(frame)
        .map_err(|e| bad(&e.to_string()))?
        .is_none()
    {
        let state = frame.singleton::<PhysicsState>();
        if state.config.dt.raw() <= 0
            || state.config.dt.raw() < i64::from(state.config.substeps.max(1))
        {
            return Err(bad("invalid ordinary physics timestep"));
        }
        if !frame.list_is_alive(state.contacts) {
            return Err(bad("invalid physics contact cache"));
        }
        orr_physics3d::step(frame, scratch);
        return Ok(());
    }
    validate_frame(frame)?;
    let source = frame.clone();
    let cfg = source.singleton::<PhysicsState>().config;
    let displacement = validate_config(&cfg)?;
    let view = terrain_view(&source)
        .map_err(|e| bad(&e.to_string()))?
        .ok_or_else(|| bad("terrain disappeared from snapshot"))?;
    let objects = [StaticContactObject {
        entity: view.asset.entity,
        friction: view.collider.friction,
        restitution: view.collider.restitution,
        layer: view.collider.layer,
        mask: view.collider.mask,
    }];
    let result = orr_physics3d::step_with_static_contacts(
        frame,
        scratch,
        &objects,
        &mut |bodies, margin, out| provide(&view, displacement, &cfg, bodies, margin, out),
    );
    result.map_err(|e| bad(&format!("terrain physics step: {e:?}")))?;
    // Validate the newly produced cache as well. Keep rejection atomic even if
    // another ordinary collider made a solver impulse exceed the profile.
    if let Err(e) = validate_frame(frame) {
        frame.copy_from(&source);
        return Err(e);
    }
    Ok(())
}

/// Returns the admitted terrain entity, for diagnostics.
pub fn terrain_entity(frame: &Frame) -> Result<Option<Entity>, TerrainStepError> {
    Ok(terrain_view(frame)
        .map_err(|e| bad(&e.to_string()))?
        .map(|v| v.asset.entity))
}
