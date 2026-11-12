//! Transient framing of every point in a current, inspected Mesh patrol route.
#[cfg(test)]
use super::frame_selection::math;
use super::{Editor, Mode};
#[cfg(test)]
use orr_render::Camera3D;
use orr_render::OrbitCamera;
const MAX_POINTS: usize = 9 * (orr_navigation::MAX_TRIANGLES as usize + 1) + 9;
#[cfg(test)]
fn enclosing(points: &[[f32; 3]]) -> Result<([f32; 3], f32), String> {
    super::frame_route_points::enclosing(points, MAX_POINTS, 256.)
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
    super::frame_route_points::fit_route(orbit, points, size, MAX_POINTS, 256., |candidate| {
        document.map_or_else(|| Ok(candidate.camera()), |d| d.camera(&candidate, size))
    })
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
