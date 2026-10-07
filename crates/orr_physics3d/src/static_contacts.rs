//! Fallible adapter boundary for immutable, non-convex static geometry.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec3, FP};

use crate::{Body, Collider, PhysicsConfig, Scratch};

/// A real frame entity backed by external static geometry, with its material.
///
/// The entity must exist and must not have a [`Body`] or [`Collider`]. Its
/// geometry is supplied by the provider, never approximated by a convex shape.
/// Adapters must keep these values and their geometry in rollback-safe state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticContactObject {
    /// Existing frame entity representing this static object.
    pub entity: Entity,
    /// Coulomb friction in `[0, 100]`, mixed with the dynamic collider.
    pub friction: FP,
    /// Restitution in `[0, 1]`, mixed with the dynamic collider.
    pub restitution: FP,
    /// Collision layer bits.
    pub layer: u32,
    /// Collision mask bits.
    pub mask: u32,
}

/// A dynamic collider at its current pose. Read-only sleep preflight also
/// includes sleeping bodies; solver-substep refreshes include awake bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticContactBody {
    /// The body's existing frame entity.
    pub entity: Entity,
    /// Current workspace position, orientation and velocities.
    pub body: Body,
    /// Shape, material and filter from the frame.
    pub collider: Collider,
}

/// One independent static contact constraint. Normals need not agree across
/// contacts belonging to the same pair (for example two sides of a crease).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticContact {
    /// Dynamic body from the provider input, including sleepers in preflight.
    pub body: Entity,
    /// Entity of one of the supplied [`StaticContactObject`]s.
    pub static_body: Entity,
    /// World-space impulse application point.
    pub point: FPVec3,
    /// Unit normal pointing from the static surface toward the dynamic body.
    pub normal: FPVec3,
    /// Signed separation along the normal, negative for penetration.
    pub separation: FP,
    /// Stable, unique feature identity within this entity pair.
    pub feature_id: u32,
}

/// A rejected static-geometry step. The frame is unchanged on every error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaticContactError<E> {
    /// The caller's geometry or admission validation rejected the step.
    Provider(E),
    /// A nonpositive or zero-duration-substep configuration was supplied.
    InvalidConfig,
    /// A static entity is stale, missing, duplicated, or has ordinary physics
    /// components; or its material is outside the documented range.
    InvalidStaticObject(Entity),
    /// A returned contact has invalid entities, a non-unit normal, or an
    /// out-of-range position/separation.
    InvalidContact,
    /// Two contacts reused the same pair and feature identity.
    DuplicateFeature,
    /// A solver arithmetic intermediate exceeds the safe fixed-point range.
    NumericOverflow,
    /// A sleeping dynamic body lacks current, cache-matched grounded support
    /// under nonzero gravity. Wake it explicitly after changing its support.
    UnsupportedSleepingSupport(Entity),
}

/// Advances one tick with additional contacts refreshed at every substep.
///
/// Before stepping, sleepers under nonzero gravity receive read-only support
/// validation; that extra provider call includes current sleeping poses.
/// `provider` otherwise receives awake dynamic bodies in ascending entity order, the
/// configured contact margin, and an empty output vector. It runs before each
/// substep's gravity/solve and once after final integration to validate and
/// retain the endpoint's contact ownership for caching and sleep. It may return
/// independent normals for one pair;
/// output order does not affect the solve. Materials, filters, restitution,
/// friction, warm starting, sleep and integration use the ordinary solver.
/// Velocities are additionally clamped before each substep integration.
/// Under nonzero gravity, sleep requires an island connected by endpoint
/// contacts with positive normal impulses to an immovable contact whose normal
/// opposes gravity. Refreshed contacts must be within `linear_slop`; ordinary
/// convex contacts retain their solver's `contact_margin` skin. The expanded
/// external query margin and friction-only wall support do not ground an
/// island. Zero gravity permits ordinary free-space sleeping.
///
/// All work precedes frame write-back: provider errors, including the final
/// validation, leave the frame byte-for-byte unchanged. Provider-owned state
/// cannot be rolled back by this function; providers must derive contacts only
/// from their immutable, rollback-bound geometry and these current bodies.
/// `Scratch` may be reused after errors or rollback. Empty `objects` invokes
/// ordinary [`crate::step`] without calling the provider.
///
/// This is an adapter seam, not continuous collision detection or a general
/// mutable mesh API. The adapter is responsible for admitting supported shapes,
/// bounding displacement, validating its geometry and invalidating old caches
/// if its immutable asset binding changes.
pub fn step_with_static_contacts<E>(
    frame: &mut Frame,
    scratch: &mut Scratch,
    objects: &[StaticContactObject],
    provider: &mut impl FnMut(&[StaticContactBody], FP, &mut Vec<StaticContact>) -> Result<(), E>,
) -> Result<(), StaticContactError<E>> {
    if objects.is_empty() {
        crate::step(frame, scratch);
        return Ok(());
    }
    let cfg = frame.singleton::<crate::PhysicsState>().config;
    if !valid_config(&cfg) {
        return Err(StaticContactError::InvalidConfig);
    }
    for (i, object) in objects.iter().enumerate() {
        if !frame.exists(object.entity)
            || frame.has::<Body>(object.entity)
            || frame.has::<Collider>(object.entity)
            || objects[..i]
                .iter()
                .any(|previous| previous.entity == object.entity)
            || object.friction < FP::ZERO
            || object.friction > FP::from_int(100)
            || object.restitution < FP::ZERO
            || object.restitution > FP::ONE
        {
            return Err(StaticContactError::InvalidStaticObject(object.entity));
        }
    }
    crate::step::validate_static_sleeping_support(frame, objects, provider)?;
    crate::step::step_static(frame, scratch, objects, provider)
}

/// Checks current support for sleeping dynamic islands without modifying the
/// frame. The provider may receive all current dynamic poses in one additional
/// read-only call, including sleepers. Matching cached feature impulses and
/// actual endpoint contacts must connect each sleeper to support against
/// nonzero gravity. Ordinary orphan/explicit wake rules are respected. Zero
/// gravity and an empty external-object slice need no support preflight.
///
/// Callers must use registered physics state and valid static objects, as for
/// [`step_with_static_contacts`]. Errors leave the frame byte-for-byte unchanged.
pub fn validate_static_sleeping_support<E>(
    frame: &Frame,
    objects: &[StaticContactObject],
    provider: &mut impl FnMut(&[StaticContactBody], FP, &mut Vec<StaticContact>) -> Result<(), E>,
) -> Result<(), StaticContactError<E>> {
    crate::step::validate_static_sleeping_support(frame, objects, provider)
}

fn valid_config(cfg: &PhysicsConfig) -> bool {
    cfg.dt.raw() > 0
        && cfg.dt.raw() >= i64::from(cfg.substeps.max(1))
        && cfg.contact_margin >= FP::ZERO
        && cfg.max_linear_speed >= FP::ZERO
        && cfg.max_angular_speed >= FP::ZERO
        && cfg.linear_slop >= FP::ZERO
        && cfg.baumgarte >= FP::ZERO
        && cfg.max_correction_speed >= FP::ZERO
        && cfg.restitution_threshold >= FP::ZERO
}
