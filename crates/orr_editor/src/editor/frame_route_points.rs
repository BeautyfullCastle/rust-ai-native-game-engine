//! Shared bounded point-cloud framing, using the established collider fit policy.
use super::frame_selection::math;
use orr_render::{Camera3D, OrbitCamera};
pub(super) fn enclosing(
    points: &[[f32; 3]],
    max_points: usize,
    max_coordinate: f32,
) -> Result<([f32; 3], f32), String> {
    if points.is_empty()
        || points.len() > max_points
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > max_coordinate)
    {
        return Err("Patrol framing requires bounded, finite admitted route points".into());
    }
    let mut low = points[0];
    let mut high = low;
    for point in points {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let center =
        std::array::from_fn(|axis| ((f64::from(low[axis]) + f64::from(high[axis])) * 0.5) as f32);
    let radius = points
        .iter()
        .map(|point| {
            (0..3)
                .map(|axis| (f64::from(point[axis]) - f64::from(center[axis])).powi(2))
                .sum::<f64>()
        })
        .fold(0.0, f64::max)
        .sqrt();
    if !radius.is_finite() {
        return Err("Patrol route framing bounds overflow".into());
    }
    // Round outward so narrowing never excludes a source point.
    let mut narrowed = radius as f32;
    if f64::from(narrowed) < radius {
        narrowed = f32::from_bits(narrowed.to_bits() + 1);
    }
    Ok((center, narrowed))
}
pub(super) fn fit_route(
    orbit: OrbitCamera,
    points: &[[f32; 3]],
    size: (u32, u32),
    max_points: usize,
    max_coordinate: f32,
    mut render: impl FnMut(OrbitCamera) -> Result<Camera3D, String>,
) -> Result<OrbitCamera, String> {
    let (center, radius) = enclosing(points, max_points, max_coordinate)?;
    let camera = render(orbit)?;
    let candidate = if radius == 0.0 {
        if !orbit.distance.is_finite()
            || !(OrbitCamera::MIN_DISTANCE..=OrbitCamera::MAX_DISTANCE).contains(&orbit.distance)
        {
            return Err("Stationary patrol framing requires an admitted camera distance".into());
        }
        let mut candidate = orbit;
        candidate.target = center;
        candidate
    } else {
        math::fit(
            orbit,
            camera,
            center,
            radius,
            size,
            OrbitCamera::MIN_DISTANCE,
            OrbitCamera::MAX_DISTANCE,
        )?
    };
    let result = render(candidate)?;
    math::validate_fit(&result, center, radius, size)?;
    validate_points(&result, points, size)?;
    Ok(candidate)
}
fn validate_points(camera: &Camera3D, points: &[[f32; 3]], size: (u32, u32)) -> Result<(), String> {
    let matrix = camera.view_proj(size.0 as f32 / size.1 as f32);
    for point in points {
        let clip = matrix.transform_point4(*point);
        if clip.iter().any(|v| !v.is_finite()) || clip[3] <= 0.0 {
            return Err("Patrol route is behind the resulting camera".into());
        }
        let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
        if ndc[0].abs() > 1.0 || ndc[1].abs() > 1.0 || !(0.0..=1.0).contains(&ndc[2]) {
            return Err("Patrol route exceeds the resulting viewport or clipping range".into());
        }
    }
    Ok(())
}
