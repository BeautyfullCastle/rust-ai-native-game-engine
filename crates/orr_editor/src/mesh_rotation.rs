const MAX_ANGLE_DEGREES: f32 = 360_000.0;
const MAX_QUATERNION_COMPONENT: f32 = 1_000_000.0;
const MIN_QUATERNION_SCALE: f64 = 1.0e-12;
const GIMBAL_EPSILON: f64 = 1.0e-6;

pub(crate) fn from_degrees(angles: [f32; 3]) -> Result<[f32; 4], String> {
    if angles
        .iter()
        .any(|angle| !angle.is_finite() || angle.abs() > MAX_ANGLE_DEGREES)
    {
        return Err("Euler angles must be finite and within +/-360000 degrees".into());
    }

    let half_angles = angles.map(|angle| f64::from(angle).to_radians() * 0.5);
    let (sin_x, cos_x) = half_angles[0].sin_cos();
    let (sin_y, cos_y) = half_angles[1].sin_cos();
    let (sin_z, cos_z) = half_angles[2].sin_cos();
    let rotation_x = [sin_x, 0.0, 0.0, cos_x];
    let rotation_y = [0.0, sin_y, 0.0, cos_y];
    let rotation_z = [0.0, 0.0, sin_z, cos_z];
    let rotation = multiply(multiply(rotation_z, rotation_y), rotation_x);
    validate_normalize(rotation.map(|component| component as f32))
}

pub(crate) fn to_degrees(quaternion: [f32; 4]) -> Result<[f32; 3], String> {
    let normalized = validate_normalize(quaternion)?;
    let components = normalized.map(f64::from);
    let magnitude = components
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt();
    let [rotation_x, rotation_y, rotation_z, scalar] =
        components.map(|component| component / magnitude);
    let sin_pitch = (2.0 * (scalar * rotation_y - rotation_z * rotation_x)).clamp(-1.0, 1.0);
    let yaw_numerator = 2.0 * (scalar * rotation_z + rotation_x * rotation_y);
    let yaw_denominator = 1.0 - 2.0 * (rotation_y * rotation_y + rotation_z * rotation_z);
    let cos_pitch = yaw_numerator.hypot(yaw_denominator);

    let (roll, pitch, yaw) = if cos_pitch <= GIMBAL_EPSILON {
        if sin_pitch >= 0.0 {
            (
                0.0,
                std::f64::consts::FRAC_PI_2,
                2.0 * rotation_z.atan2(scalar),
            )
        } else {
            (
                0.0,
                -std::f64::consts::FRAC_PI_2,
                2.0 * rotation_x.atan2(scalar),
            )
        }
    } else {
        let roll = (2.0 * (scalar * rotation_x + rotation_y * rotation_z))
            .atan2(1.0 - 2.0 * (rotation_x * rotation_x + rotation_y * rotation_y));
        let pitch = sin_pitch.atan2(cos_pitch);
        let yaw = yaw_numerator.atan2(yaw_denominator);
        (roll, pitch, yaw)
    };

    Ok([roll, pitch, yaw].map(|angle| {
        let degrees = angle.to_degrees() as f32;
        if degrees == 0.0 {
            0.0
        } else {
            degrees
        }
    }))
}

pub(crate) fn validate_normalize(quaternion: [f32; 4]) -> Result<[f32; 4], String> {
    if quaternion
        .iter()
        .any(|component| !component.is_finite() || component.abs() > MAX_QUATERNION_COMPONENT)
    {
        return Err("quaternion components must be finite and within +/-1000000".into());
    }

    let components = quaternion.map(f64::from);
    let scale = components
        .iter()
        .map(|component| component.abs())
        .fold(0.0_f64, f64::max);
    if scale <= MIN_QUATERNION_SCALE {
        return Err("quaternion must be nonzero".into());
    }

    let scaled = components.map(|component| component / scale);
    let magnitude = scaled
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt();
    if !magnitude.is_finite() || magnitude == 0.0 {
        return Err("quaternion cannot be normalized".into());
    }
    if scale > f64::from(MAX_QUATERNION_COMPONENT) / magnitude {
        return Err("quaternion magnitude exceeds the 1000000 limit".into());
    }
    let mut normalized = scaled.map(|component| (component / magnitude) as f32);
    if negative_canonical_sign(normalized) {
        normalized = normalized.map(|component| -component);
    }
    for component in &mut normalized {
        if *component == 0.0 {
            *component = 0.0;
        }
    }
    Ok(normalized)
}

fn multiply(left: [f64; 4], right: [f64; 4]) -> [f64; 4] {
    let [left_x, left_y, left_z, left_w] = left;
    let [right_x, right_y, right_z, right_w] = right;
    [
        left_w * right_x + left_x * right_w + left_y * right_z - left_z * right_y,
        left_w * right_y - left_x * right_z + left_y * right_w + left_z * right_x,
        left_w * right_z + left_x * right_y - left_y * right_x + left_z * right_w,
        left_w * right_w - left_x * right_x - left_y * right_y - left_z * right_z,
    ]
}

fn negative_canonical_sign(quaternion: [f32; 4]) -> bool {
    for component in quaternion.into_iter().rev() {
        if component > 0.0 {
            return false;
        }
        if component < 0.0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "mesh_rotation/tests.rs"]
mod tests;
