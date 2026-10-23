//! Opt-in, stateless geometry for authored spherical point-agent hazards.
//!
//! This foundation does not register Frame state or change mesh navigation. A caller
//! supplies the actual ordered movement vertices, including every traversed corner;
//! replacing them with the tick's endpoint chord changes the swept geometry.
//!
//! Coordinates and radii are bounded to 256 world units (Q48.16 raw magnitude
//! 2^24). Coordinate differences are at most 2^25; three-term squared lengths
//! and dot products have magnitude below 2^52. The division-free interior test
//! multiplies these quantities, remaining below 2^104, safely inside i128.
//! Every arithmetic operation is nevertheless checked. Errors must stop admission
//! or movement; they must never be interpreted as a clear path.

use orr_fp::FP;

pub const MAX_HAZARDS: usize = 8;
/// A bounded mesh route plus its current and final movement positions.
pub const MAX_MOVEMENT_POINTS: usize = super::mesh::MAX_TRIANGLES + 2;
pub const MAX_COORDINATE_RAW: i64 = 256 * 65_536;
pub const MAX_RADIUS_RAW: i64 = MAX_COORDINATE_RAW;

/// Authored order defines stable hazard indices. This is not registered Frame state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SphereHazard {
    pub center: [FP; 3],
    pub radius: FP,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HazardError {
    TooManyHazards,
    InvalidRadius,
    CoordinateOutOfBounds,
    EmptyMovement,
    TooManyMovementPoints,
    ArithmeticOverflow,
}

impl core::fmt::Display for HazardError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for HazardError {}

fn validate_point(point: [FP; 3]) -> Result<(), HazardError> {
    if point
        .iter()
        .any(|value| value.raw() < -MAX_COORDINATE_RAW || value.raw() > MAX_COORDINATE_RAW)
    {
        return Err(HazardError::CoordinateOutOfBounds);
    }
    Ok(())
}

/// Validates the entire authored list; empty lists are permitted.
pub fn validate_hazards(hazards: &[SphereHazard]) -> Result<(), HazardError> {
    if hazards.len() > MAX_HAZARDS {
        return Err(HazardError::TooManyHazards);
    }
    for hazard in hazards {
        validate_point(hazard.center)?;
        if hazard.radius.raw() <= 0 || hazard.radius.raw() > MAX_RADIUS_RAW {
            return Err(HazardError::InvalidRadius);
        }
    }
    Ok(())
}

/// Returns the lowest authored hazard index touched anywhere on the movement.
///
/// All contacts are inclusive, including tangency, segment endpoints and a point
/// already inside a hazard. A single vertex is stationary movement. Repeated
/// vertices are allowed. Empty movement is invalid. Only consecutive vertices
/// are joined, so corners never introduce a false start-to-finish chord.
///
/// This selects by authored index, **not** by earliest time of impact. It does
/// not return a contact position or truncate movement. All supplied inputs are
/// validated before any hit is returned, including vertices after an early hit.
/// On error the caller must reject the operation, not continue movement.
pub fn sweep_polyline(
    hazards: &[SphereHazard],
    movement: &[[FP; 3]],
) -> Result<Option<u32>, HazardError> {
    validate_hazards(hazards)?;
    if movement.is_empty() {
        return Err(HazardError::EmptyMovement);
    }
    if movement.len() > MAX_MOVEMENT_POINTS {
        return Err(HazardError::TooManyMovementPoints);
    }
    for &point in movement {
        validate_point(point)?;
    }
    for (index, &hazard) in hazards.iter().enumerate() {
        if movement.len() == 1 {
            if touches_segment(hazard, movement[0], movement[0])? {
                return Ok(Some(index as u32));
            }
        } else {
            for segment in movement.windows(2) {
                if touches_segment(hazard, segment[0], segment[1])? {
                    return Ok(Some(index as u32));
                }
            }
        }
    }
    Ok(None)
}

fn difference(a: [FP; 3], b: [FP; 3]) -> Result<[i128; 3], HazardError> {
    let mut result = [0; 3];
    for axis in 0..3 {
        result[axis] = i128::from(a[axis].raw())
            .checked_sub(i128::from(b[axis].raw()))
            .ok_or(HazardError::ArithmeticOverflow)?;
    }
    Ok(result)
}

fn dot(a: [i128; 3], b: [i128; 3]) -> Result<i128, HazardError> {
    let mut result = 0_i128;
    for axis in 0..3 {
        let product = a[axis]
            .checked_mul(b[axis])
            .ok_or(HazardError::ArithmeticOverflow)?;
        result = result
            .checked_add(product)
            .ok_or(HazardError::ArithmeticOverflow)?;
    }
    Ok(result)
}

fn touches_segment(
    hazard: SphereHazard,
    start: [FP; 3],
    end: [FP; 3],
) -> Result<bool, HazardError> {
    let toward_center = difference(hazard.center, start)?;
    let direction = difference(end, start)?;
    let radius = i128::from(hazard.radius.raw());
    let radius_squared = radius
        .checked_mul(radius)
        .ok_or(HazardError::ArithmeticOverflow)?;
    let start_squared = dot(toward_center, toward_center)?;
    if start_squared <= radius_squared {
        return Ok(true);
    }
    let length_squared = dot(direction, direction)?;
    let projection = dot(toward_center, direction)?;
    if length_squared == 0 || projection <= 0 {
        return Ok(false);
    }
    if projection >= length_squared {
        let from_end = difference(hazard.center, end)?;
        return Ok(dot(from_end, from_end)? <= radius_squared);
    }
    // For a projection strictly inside the segment:
    // |center-start|^2 - projection^2 / |end-start|^2 <= radius^2.
    // Cross-multiply without division, square roots or fixed-point rounding.
    let left = start_squared
        .checked_sub(radius_squared)
        .and_then(|distance| distance.checked_mul(length_squared))
        .ok_or(HazardError::ArithmeticOverflow)?;
    let right = projection
        .checked_mul(projection)
        .ok_or(HazardError::ArithmeticOverflow)?;
    Ok(left <= right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i64, y: i64, z: i64) -> [FP; 3] {
        [x, y, z].map(|value| FP::from_raw(value * 65_536))
    }

    fn sphere(x: i64, y: i64, z: i64, radius: i64) -> SphereHazard {
        SphereHazard {
            center: point(x, y, z),
            radius: FP::from_raw(radius * 65_536),
        }
    }

    #[test]
    fn high_speed_crossing_does_not_tunnel() {
        let hazard = sphere(0, 0, 0, 1);
        let start = point(-256, 0, 0);
        let end = point(256, 0, 0);
        assert_eq!(sweep_polyline(&[hazard], &[start]), Ok(None));
        assert_eq!(sweep_polyline(&[hazard], &[end]), Ok(None));
        assert_eq!(sweep_polyline(&[hazard], &[start, end]), Ok(Some(0)));
    }

    #[test]
    fn exact_tangent_and_single_raw_unit_miss() {
        let hazard = sphere(0, 0, 0, 1);
        let tangent = [point(-2, 1, 0), point(2, 1, 0)];
        assert_eq!(sweep_polyline(&[hazard], &tangent), Ok(Some(0)));
        let miss = tangent.map(|mut p| {
            p[1] = FP::from_raw(p[1].raw() + 1);
            p
        });
        assert_eq!(sweep_polyline(&[hazard], &miss), Ok(None));
        let inside = tangent.map(|mut p| {
            p[1] = FP::from_raw(p[1].raw() - 1);
            p
        });
        assert_eq!(sweep_polyline(&[hazard], &inside), Ok(Some(0)));
    }

    #[test]
    fn all_three_axes_and_fractional_projection_are_used() {
        let hazard = sphere(0, 0, 0, 1);
        // Closest point is interior at t=1/3, not a representable Q16 ratio.
        assert_eq!(
            sweep_polyline(&[hazard], &[point(-1, 0, 1), point(2, 0, 1)]),
            Ok(Some(0))
        );
        assert_eq!(
            sweep_polyline(&[hazard], &[point(-1, 1, 1), point(2, 1, 1)]),
            Ok(None)
        );
        assert_eq!(
            sweep_polyline(&[hazard], &[point(0, -2, -2), point(0, 2, 2)]),
            Ok(Some(0))
        );
    }

    #[test]
    fn oblique_tangent_is_exact_and_direction_independent() {
        let hazard = sphere(0, 0, 0, 5);
        let path = [point(-1, 7, 0), point(7, 1, 0)];
        assert_eq!(sweep_polyline(&[hazard], &path), Ok(Some(0)));
        assert_eq!(sweep_polyline(&[hazard], &[path[1], path[0]]), Ok(Some(0)));
        assert_eq!(
            sweep_polyline(&[hazard], &[path[0], point(3, 4, 0), path[1]]),
            Ok(Some(0))
        );
        let miss = path.map(|mut p| {
            p[0] = FP::from_raw(p[0].raw() + 1);
            p
        });
        assert_eq!(sweep_polyline(&[hazard], &miss), Ok(None));
    }

    #[test]
    fn stationary_and_duplicate_points_include_contact() {
        let hazard = sphere(0, 0, 0, 1);
        for p in [point(0, 0, 0), point(1, 0, 0)] {
            assert_eq!(sweep_polyline(&[hazard], &[p]), Ok(Some(0)));
            assert_eq!(sweep_polyline(&[hazard], &[p, p]), Ok(Some(0)));
        }
        let outside = point(2, 0, 0);
        assert_eq!(sweep_polyline(&[hazard], &[outside, outside]), Ok(None));
    }

    #[test]
    fn endpoint_contact_is_inclusive_but_infinite_line_is_not_used() {
        let hazard = sphere(0, 0, 0, 1);
        assert_eq!(
            sweep_polyline(&[hazard], &[point(-3, 0, 0), point(-1, 0, 0)]),
            Ok(Some(0))
        );
        assert_eq!(
            sweep_polyline(&[hazard], &[point(1, 0, 0), point(3, 0, 0)]),
            Ok(Some(0))
        );
        for path in [
            [point(2, 0, 0), point(3, 0, 0)],
            [point(-3, 0, 0), point(-2, 0, 0)],
        ] {
            assert_eq!(sweep_polyline(&[hazard], &path), Ok(None));
        }
    }

    #[test]
    fn corners_do_not_create_false_chords_or_skip_real_segments() {
        let path = [
            point(-3, 0, 0),
            point(-3, 3, 0),
            point(3, 3, 0),
            point(3, 0, 0),
        ];
        assert_eq!(sweep_polyline(&[sphere(0, 0, 0, 1)], &path), Ok(None));
        assert_eq!(sweep_polyline(&[sphere(0, 3, 0, 1)], &path), Ok(Some(0)));
        assert_eq!(sweep_polyline(&[sphere(-3, 3, 0, 1)], &path), Ok(Some(0)));
    }

    #[test]
    fn lowest_authored_index_wins_not_first_time_of_impact() {
        let hazards = [sphere(4, 0, 0, 1), sphere(-4, 0, 0, 1)];
        let path = [point(-8, 0, 0), point(0, 0, 0), point(8, 0, 0)];
        assert_eq!(sweep_polyline(&hazards, &path), Ok(Some(0)));
        assert_eq!(sweep_polyline(&hazards, &[path[0], path[1]]), Ok(Some(1)));
        assert_eq!(
            sweep_polyline(&[hazards[0]; MAX_HAZARDS], &path),
            Ok(Some(0))
        );
    }

    #[test]
    fn admission_rejects_all_invalid_inputs_before_returning_any_hit() {
        let valid = sphere(0, 0, 0, 1);
        let origin = point(0, 0, 0);
        for raw in [i64::MIN, -1, 0, MAX_RADIUS_RAW + 1, i64::MAX] {
            let invalid = SphereHazard {
                radius: FP::from_raw(raw),
                ..valid
            };
            assert_eq!(
                sweep_polyline(&[valid, invalid], &[origin]),
                Err(HazardError::InvalidRadius)
            );
        }
        for raw in [
            i64::MIN,
            -MAX_COORDINATE_RAW - 1,
            MAX_COORDINATE_RAW + 1,
            i64::MAX,
        ] {
            for axis in 0..3 {
                let mut invalid_point = origin;
                invalid_point[axis] = FP::from_raw(raw);
                let invalid = SphereHazard {
                    center: invalid_point,
                    ..valid
                };
                assert_eq!(
                    validate_hazards(&[invalid]),
                    Err(HazardError::CoordinateOutOfBounds)
                );
                assert_eq!(
                    sweep_polyline(&[valid], &[origin, invalid_point]),
                    Err(HazardError::CoordinateOutOfBounds)
                );
                assert_eq!(
                    sweep_polyline(&[], &[invalid_point]),
                    Err(HazardError::CoordinateOutOfBounds)
                );
            }
        }
        assert_eq!(
            validate_hazards(&[valid; MAX_HAZARDS + 1]),
            Err(HazardError::TooManyHazards)
        );
        assert_eq!(
            sweep_polyline(&[valid], &[]),
            Err(HazardError::EmptyMovement)
        );
        assert_eq!(
            sweep_polyline(&[valid], &[origin; MAX_MOVEMENT_POINTS + 1]),
            Err(HazardError::TooManyMovementPoints)
        );
        assert_eq!(
            sweep_polyline(&[], &[origin; MAX_MOVEMENT_POINTS]),
            Ok(None)
        );
        assert_eq!(validate_hazards(&[]), Ok(()));
    }

    #[test]
    fn admitted_extremes_and_smallest_radius_are_exact() {
        let tiny = SphereHazard {
            center: point(0, 0, 0),
            radius: FP::from_raw(1),
        };
        let diagonal = [point(-256, -256, -256), point(256, 256, 256)];
        assert_eq!(sweep_polyline(&[tiny], &diagonal), Ok(Some(0)));
        let far = sphere(256, -256, 256, 256);
        assert_eq!(sweep_polyline(&[far], &diagonal), Ok(None));
        let tangent = [point(-256, 0, 256), point(256, 0, 256)];
        assert_eq!(sweep_polyline(&[far], &tangent), Ok(Some(0)));
    }

    #[test]
    fn checked_arithmetic_fails_closed_even_if_internal_bounds_are_bypassed() {
        assert_eq!(
            dot([i128::MAX, 0, 0], [2, 0, 0]),
            Err(HazardError::ArithmeticOverflow)
        );
        assert_eq!(
            dot([i128::MAX, 1, 0], [1, 1, 0]),
            Err(HazardError::ArithmeticOverflow)
        );
        let extreme = [FP::from_raw(i64::MIN); 3];
        let opposite = [FP::from_raw(i64::MAX); 3];
        assert_eq!(
            touches_segment(sphere(0, 0, 0, 1), extreme, opposite),
            Err(HazardError::ArithmeticOverflow)
        );
    }
}
