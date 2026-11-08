//! Session-only framing of an already inspected terrain patrol polyline.
use super::{Editor, Mode};
use orr_render::OrbitCamera;
const MAX_POINTS: usize = 9 * (orr_remote::navigation_yard3d::patrol::MAX_ROUTE_TRIANGLES + 1) + 9;
fn fit_route(
    orbit: OrbitCamera,
    points: &[[f32; 3]],
    size: (u32, u32),
) -> Result<OrbitCamera, String> {
    super::frame_route_points::fit_route(orbit, points, size, MAX_POINTS, 256., |candidate| {
        Ok(candidate.camera())
    })
}
impl Editor {
    pub(crate) fn frame_inspected_terrain_patrol(
        &mut self,
        points: &[[f32; 3]],
        size: (u32, u32),
    ) -> Result<(), String> {
        if self.game() != crate::game::EditorGame::NavigationYard3D
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
            return Err(
                "Patrol framing requires a coherent local Terrain Navigation Edit scene".into(),
            );
        }

        let snapshot = self
            .snapshot()
            .ok_or("Missing admitted Terrain Navigation Frame")?;
        orr_remote::navigation_yard3d::navigation_from_view(snapshot.predicted())?;
        let candidate = fit_route(self.camera3d, points, size)?;
        self.camera3d.target = candidate.target;
        self.camera3d.distance = candidate.distance;
        Ok(())
    }
}
#[cfg(test)]
#[path = "frame_terrain_patrol_tests.rs"]
mod tests;
