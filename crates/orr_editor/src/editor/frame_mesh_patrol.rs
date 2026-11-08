//! Transient framing of every point in a current, inspected Mesh patrol route.
use super::frame_selection::math;
use super::{Editor, Mode};
use orr_render::{Camera3D, OrbitCamera};
const MAX_POINTS: usize = 9 * (orr_navigation::MAX_TRIANGLES as usize + 1) + 9;
fn enclosing(points: &[[f32; 3]]) -> Result<([f32; 3], f32), String> {
    if points.is_empty()
        || points.len() > MAX_POINTS
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 256.)
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
fn fit_route(
    orbit: OrbitCamera,
    document: Option<&crate::mesh_camera::Document>,
    points: &[[f32; 3]],
    size: (u32, u32),
) -> Result<OrbitCamera, String> {
    if document.is_some_and(|d| d.follow_offset.is_some()) {
        return Err("Turn off Follow mesh agent before framing a patrol route".into());
    }
    let (center, radius) = enclosing(points)?;
    let camera = render_camera(orbit, document, size)?;
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
    let result = render_camera(candidate, document, size)?;
    math::validate_fit(&result, center, radius, size)?;
    validate_points(&result, points, size)?;
    Ok(candidate)
}
fn render_camera(
    orbit: OrbitCamera,
    document: Option<&crate::mesh_camera::Document>,
    size: (u32, u32),
) -> Result<Camera3D, String> {
    document.map_or_else(|| Ok(orbit.camera()), |d| d.camera(&orbit, size))
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
impl Editor {
    pub(crate) fn mesh_camera_follows_agent(&self) -> bool {
        self.has_mesh_camera()
            && self
                .mesh_camera
                .as_ref()
                .is_some_and(|camera| camera.document.follow_offset.is_some())
    }
    pub(crate) fn frame_inspected_mesh_patrol(
        &mut self,
        points: &[[f32; 3]],
        size: (u32, u32),
    ) -> Result<(), String> {
        if !self.game().has_mesh_view()
            || !self.spec().is_local()
            || self.mode() != Mode::Edit
            || !self.can_mutate()
            || self.down().is_some()
            || self.in_gesture()
            || self.previewing().is_some()
            || !self.proposals().is_empty()
            || self.batch_uncertain
            || self.refit
            || self.refit_checksum.is_some()
            || self.initial_camera_fit
            || self.dirty.rows
            || self.dirty.state
            || self.dirty.history
            || self.input_phase() != super::input::Phase::Off
            || self.pending.values().any(|pending| {
                matches!(
                    &pending.kind,
                    super::Pend::Input(..) | super::Pend::Gesture(..)
                )
            })
            || self.snapshot().is_none_or(|s| {
                s.timeline().is_some() || s.predicted().checksum() != self.sim.checksum
            })
            || self.checksum != self.sim.checksum
        {
            return Err("Patrol framing requires a coherent local Mesh Edit scene".into());
        }
        let snapshot = self.snapshot().ok_or("Missing admitted Mesh Frame")?;
        orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted())?;
        let document = if self.has_mesh_camera() {
            self.mesh_camera.as_ref().map(|c| &c.document)
        } else {
            None
        };
        let candidate = fit_route(self.camera3d, document, points, size)?;
        self.camera3d.target = candidate.target;
        self.camera3d.distance = candidate.distance;
        Ok(())
    }
}
#[cfg(test)]
#[path = "frame_mesh_patrol_tests.rs"]
mod tests;
