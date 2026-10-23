use orr_render::{Camera3D, OrbitCamera, camera3d::Projection};
const MARGIN: f64 = 1.1;
fn aspect(size: (u32, u32)) -> Result<f64, String> {
    if !(1..=16384).contains(&size.0) || !(1..=16384).contains(&size.1) {
        return Err("Invalid framing viewport dimensions.".into());
    }
    Ok(f64::from(size.0) / f64::from(size.1))
}
pub(super) fn fit(
    orbit: OrbitCamera,
    camera: Camera3D,
    center: [f32; 3],
    radius: f32,
    size: (u32, u32),
    minimum: f32,
    maximum: f32,
) -> Result<OrbitCamera, String> {
    let aspect = aspect(size)?;
    if center
        .iter()
        .chain(orbit.target.iter())
        .any(|v| !v.is_finite())
        || !radius.is_finite()
        || radius <= 0.0
        || !orbit.yaw.is_finite()
        || !orbit.pitch.is_finite()
        || !orbit.distance.is_finite()
        || orbit.distance <= 0.0
        || !orbit.fov_y_degrees.is_finite()
        || !(0.0..180.0).contains(&orbit.fov_y_degrees)
        || !minimum.is_finite()
        || minimum <= 0.0
        || !maximum.is_finite()
        || maximum < minimum
    {
        return Err("Invalid camera or collider bounds.".into());
    }
    let radius = f64::from(radius);
    let required = match camera.projection {
        Projection::Perspective { fov_y, near, far } => {
            if !fov_y.is_finite()
                || !(0.0..std::f32::consts::PI).contains(&fov_y)
                || !near.is_finite()
                || !far.is_finite()
                || near <= 0.0
                || far <= near
            {
                return Err("Invalid perspective projection.".into());
            }
            let vertical = f64::from(fov_y) * 0.5;
            let horizontal = (vertical.tan() * aspect).atan();
            (radius * MARGIN / vertical.min(horizontal).sin()).max(radius + f64::from(near) * 1.1)
        }
        Projection::Orthographic {
            half_height,
            near,
            far,
        } => {
            if !half_height.is_finite()
                || half_height <= 0.0
                || !near.is_finite()
                || !far.is_finite()
                || far <= near
            {
                return Err("Invalid orthographic projection.".into());
            }
            radius * MARGIN / (f64::from(half_height) * aspect.min(1.0)) * f64::from(orbit.distance)
        }
    };
    let distance = required.max(f64::from(minimum));
    if !distance.is_finite() || distance > f64::from(maximum) {
        return Err(
            "Selected collider cannot fit within the existing camera distance limits.".into(),
        );
    }
    let mut candidate = orbit;
    candidate.target = center;
    candidate.distance = distance as f32;
    Ok(candidate)
}
pub(super) fn validate_fit(
    camera: &Camera3D,
    center: [f32; 3],
    radius: f32,
    size: (u32, u32),
) -> Result<(), String> {
    let aspect = aspect(size)? as f32;
    if camera
        .eye
        .iter()
        .chain(camera.target.iter())
        .chain(camera.up.iter())
        .any(|v| !v.is_finite())
        || camera
            .view_proj(aspect)
            .0
            .iter()
            .flatten()
            .any(|v| !v.is_finite())
    {
        return Err("Framing produced an invalid camera.".into());
    }
    let forward = camera.forward();
    let depth = (0..3)
        .map(|i| (center[i] - camera.eye[i]) * forward[i])
        .sum::<f32>();
    let (near, far) = match camera.projection {
        Projection::Perspective { near, far, .. } | Projection::Orthographic { near, far, .. } => {
            (near, far)
        }
    };
    if !depth.is_finite() || depth - radius <= near || depth + radius >= far {
        return Err("Selected collider exceeds the camera clipping range.".into());
    }
    // Verify the complete enclosing sphere fits the actual resulting projection.
    let fits = match camera.projection {
        Projection::Perspective { fov_y, .. } => {
            let angle = (fov_y * 0.5).min(((fov_y * 0.5).tan() * aspect).atan());
            radius <= depth * angle.sin() / 1.05
        }
        Projection::Orthographic { half_height, .. } => {
            radius <= half_height * aspect.min(1.0) / 1.05
        }
    };
    if !fits {
        return Err("Selected collider does not fit the resulting projection.".into());
    }
    Ok(())
}
