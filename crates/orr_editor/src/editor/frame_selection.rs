//! Session-only framing of admitted collider bounds, never imported model bounds.
use super::{Editor, Mode, Target};
use crate::game::EditorGame;
use orr_render::OrbitCamera;
use orr_view::Shape3;
#[path = "frame_selection/math.rs"]
mod math;

impl Editor {
    pub(crate) fn frame_selected_collider(&mut self, size: (u32, u32)) -> Result<(), String> {
        if !(self.game() == EditorGame::Yard3D || self.game().is_room())
            || !self.spec().is_local()
            || self.mode() != Mode::Edit
            || !self.can_mutate()
            || self.previewing().is_some()
            || self.in_gesture()
            || self.batch_uncertain
            || !self.yard_rows_coherent()
            || self.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.timeline().is_some()
                    || snapshot.predicted().checksum() != self.sim.checksum
            })
            || self.checksum != self.sim.checksum
        {
            return Err("Framing requires a coherent local Room or Yard3D in Edit mode.".into());
        }
        let target = self.selection().ok_or("Select a collider first")?;
        if !matches!(target, Target::Guid(_)) {
            return Err("Framing requires a current authored GUID selection.".into());
        }
        let row = self
            .row_of(target)
            .ok_or("Selection is not in the current entity map")?;
        let mut items = self
            .yard_frame()
            .items
            .iter()
            .filter(|item| item.entity == row.entity);
        let item = items
            .next()
            .ok_or("Selected entity has no admitted collider bounds")?;
        if items.next().is_some() {
            return Err("Selected collider bounds are ambiguous.".into());
        }
        let center = item.transform.pos.to_array();
        let radius = radius(item.style.shape)?;
        let current = self.presentation_camera3d(size)?;
        let (min, max) = if self.game().is_room() {
            (1.0, 128.0)
        } else {
            (OrbitCamera::MIN_DISTANCE, OrbitCamera::MAX_DISTANCE)
        };
        let candidate = math::fit(self.camera3d, current, center, radius, size, min, max)?;
        #[cfg(feature = "room-project")]
        let camera = if let Some(document) = &self.room_camera {
            document.camera(&candidate, size)?
        } else {
            candidate.camera()
        };
        #[cfg(not(feature = "room-project"))]
        let camera = candidate.camera();
        math::validate_fit(&camera, center, radius, size)?;
        self.camera3d = candidate;
        Ok(())
    }
}
fn radius(shape: Shape3) -> Result<f32, String> {
    let half = match shape {
        Shape3::Box { half } => half,
        Shape3::Sphere { radius } => {
            if radius.is_finite() && radius > 0.0 {
                return Ok(radius);
            }
            return Err("Invalid collider radius.".into());
        }
        Shape3::Capsule {
            half_length,
            radius,
        } => {
            if !half_length.is_finite() || half_length < 0.0 || !radius.is_finite() || radius <= 0.0
            {
                return Err("Invalid capsule bounds.".into());
            }
            let result = half_length + radius;
            return result
                .is_finite()
                .then_some(result)
                .ok_or("Capsule bounds overflow.".into());
        }
        Shape3::Plane { half_x, half_z } => [half_x, 0.025, half_z],
    };
    if half.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err("Invalid collider extents.".into());
    }
    // A sphere enclosing the local box also encloses every rotated box.
    let result = half
        .into_iter()
        .map(|value| f64::from(value).powi(2))
        .sum::<f64>()
        .sqrt();
    if !result.is_finite() || result > f64::from(f32::MAX) {
        return Err("Collider bounds overflow.".into());
    }
    Ok(result as f32)
}
#[cfg(test)]
#[path = "frame_selection/tests.rs"]
mod tests;
