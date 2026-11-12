//! Explicit viewport input stages only the fixed-light draft, never host or files.
use super::{Document, LightingEditor, Panel};
use orr_render::{Camera3D, Projection};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    pending: Option<Pin>,
    status: String,
}
struct Pin {
    source: Arc<()>,
    identity: (u64, u64, u64),
    candidate: Document,
}
impl Pin {
    fn matches(&self, source: &Arc<()>, identity: (u64, u64, u64), candidate: &Document) -> bool {
        Arc::ptr_eq(&self.source, source)
            && self.identity == identity
            && self.candidate == *candidate
    }
}
impl Panel {
    pub(crate) fn pick_pending(&self) -> bool {
        self.viewport_pick.pending.is_some()
    }
    pub(super) fn cancel_active_pick(&mut self) {
        if self.pick_pending() {
            self.cancel_pick();
        }
    }
    pub(crate) fn cancel_pick(&mut self) {
        self.viewport_pick.pending = None;
        self.viewport_pick.status = "Light pick cancelled; draft unchanged.".into();
    }
    fn pick_identity(&mut self, editor: &dyn LightingEditor) -> Option<(Arc<()>, (u64, u64, u64))> {
        if !editor.lighting_authoring_editable()
            || self.candidate.point_light.is_none()
            || self.candidate.follow_player.is_some()
            || self.candidate.validate().is_err()
            || self.ensure_source(editor).is_err()
        {
            return None;
        }
        Some((
            editor.room_lighting_source_token(),
            editor.lighting_pick_identity()?,
        ))
    }
    pub(super) fn sync_viewport_pick(&mut self, editor: Option<&dyn LightingEditor>) -> bool {
        if !self.pick_pending() {
            return false;
        }
        let valid = editor
            .and_then(|e| self.pick_identity(e))
            .is_some_and(|(source, identity)| {
                self.viewport_pick
                    .pending
                    .as_ref()
                    .is_some_and(|pin| pin.matches(&source, identity, &self.candidate))
            });
        if !valid {
            self.cancel_pick();
            self.viewport_pick.status =
                "Light pick cancelled because the source, frame or lighting draft changed.".into();
        }
        valid
    }
    fn begin_viewport_pick(&mut self, editor: &dyn LightingEditor) -> Result<(), String> {
        self.validate_candidate(editor, &self.candidate.clone())?;
        let (source, identity) = self
            .pick_identity(editor)
            .ok_or("Light picking requires a valid fixed light and coherent local Edit frame")?;
        self.viewport_pick.pending = Some(Pin {
            source,
            identity,
            candidate: self.candidate.clone(),
        });
        self.viewport_pick.status="Click the viewport ground plane to stage fixed light X/Z; height is preserved. Apply and Save remain explicit.".into();
        Ok(())
    }
    pub(super) fn show_viewport_pick(
        &mut self,
        ui: &mut egui::Ui,
        editor: Option<&dyn LightingEditor>,
    ) {
        let enabled = editor.is_some_and(|editor| editor.lighting_pick_identity().is_some())
            && self.candidate.point_light.is_some()
            && self.candidate.follow_player.is_none()
            && self.candidate.validate().is_ok();
        if ui
            .add_enabled(
                enabled && !self.pick_pending(),
                egui::Button::new("Pick fixed light X/Z"),
            )
            .clicked()
        {
            if let Some(editor) = editor {
                self.error = self.begin_viewport_pick(editor).err();
            }
        }
        if self.pick_pending() && ui.button("Cancel light pick").clicked() {
            self.cancel_pick();
        }
        if !self.viewport_pick.status.is_empty() {
            ui.weak(&self.viewport_pick.status);
        }
    }
    /// Returns true when the pending pick owns a click or cancellation event.
    /// Calls without input still discard stale source/frame/draft pins.
    pub(crate) fn handle_viewport_pick(
        &mut self,
        editor: &crate::editor::Editor,
        camera: &Camera3D,
        size: (u32, u32),
        pixel: Option<[f32; 2]>,
        clicked: bool,
        cancel: bool,
    ) -> bool {
        if !self.pick_pending() {
            return false;
        }
        if cancel {
            self.cancel_pick();
            return true;
        }
        if !self.sync_viewport_pick(Some(editor)) {
            return clicked;
        }
        if !clicked {
            return false;
        }
        let Some(point) = pixel.and_then(|p| ground_point(camera, size, p)) else {
            self.viewport_pick.status =
                "No bounded ground-plane hit; click inside the viewport again or cancel.".into();
            return true;
        };
        let next = match stage_point(&self.candidate, point) {
            Ok(next) => next,
            Err(error) => {
                self.error = Some(error);
                return true;
            }
        };
        if let Err(error) = self.validate_candidate(editor, &next) {
            self.cancel_pick();
            self.error = Some(error);
            return true;
        }
        if !self.sync_viewport_pick(Some(editor)) {
            return true;
        }
        self.candidate = next;
        self.viewport_pick.pending = None;
        self.viewport_pick.status =
            "Fixed light X/Z staged; Apply and Save remain explicit.".into();
        self.error = None;
        true
    }
}
fn stage_point(candidate: &Document, point: [f32; 2]) -> Result<Document, String> {
    if candidate.follow_player.is_some() || !point.iter().all(|v| v.is_finite() && v.abs() <= 32.0)
    {
        return Err("Fixed light pick must lie within X/Z -32..32".into());
    }
    let mut next = candidate.clone();
    let light = next
        .point_light
        .as_mut()
        .ok_or("Enable a fixed light before picking")?;
    light.position[0] = point[0];
    light.position[2] = point[1];
    next.validate()?;
    Ok(next)
}
fn ground_point(camera: &Camera3D, size: (u32, u32), pixel: [f32; 2]) -> Option<[f32; 2]> {
    if size.0 == 0
        || size.1 == 0
        || !pixel.iter().all(|v| v.is_finite())
        || pixel[0] < 0.0
        || pixel[1] < 0.0
        || pixel[0] >= size.0 as f32
        || pixel[1] >= size.1 as f32
        || !camera
            .eye
            .iter()
            .chain(&camera.target)
            .chain(&camera.up)
            .all(|v| v.is_finite())
    {
        return None;
    }
    match camera.projection {
        Projection::Perspective { fov_y, near, far }
            if fov_y.is_finite()
                && fov_y > 0.0
                && fov_y < std::f32::consts::PI
                && near.is_finite()
                && far.is_finite()
                && near > 0.0
                && far > near => {}
        Projection::Orthographic {
            half_height,
            near,
            far,
        } if half_height.is_finite()
            && half_height > 0.0
            && near.is_finite()
            && far.is_finite()
            && far > near => {}
        _ => return None,
    }
    let forward = camera.forward();
    for axis in [forward, camera.right(), camera.camera_up()] {
        if !axis.iter().all(|v| v.is_finite())
            || axis.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() < 0.5
        {
            return None;
        }
    }
    let (origin, direction) = camera.screen_ray(pixel, size);
    if !origin.iter().chain(&direction).all(|v| v.is_finite()) || direction[1].abs() < 1e-6 {
        return None;
    }
    let distance = -origin[1] / direction[1];
    if !distance.is_finite() || distance <= 0.0 {
        return None;
    }
    let point = [
        origin[0] + direction[0] * distance,
        origin[2] + direction[2] * distance,
    ];
    point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 32.0)
        .then_some(point)
}
#[cfg(test)]
#[path = "viewport_pick_tests.rs"]
mod tests;
