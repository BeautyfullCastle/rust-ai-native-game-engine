//! Explicit, source-pinned X/Z staging. No host, history, or filesystem writes.
use super::{Document, Editor, Panel};
use orr_render::{Camera3D, Projection};
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pin>,
    busy: bool,
}
struct Pin {
    raw: Vec<String>,
    document: Document,
    generation: u64,
    scene_epoch: u64,
    host_epoch: u64,
    checksum: u64,
}
impl Panel {
    pub(crate) fn set_viewport_pick_busy(&mut self, busy: bool) {
        self.viewport_pick.busy = busy;
    }
    pub(super) fn viewport_pick_busy(&self) -> bool {
        self.viewport_pick.busy
    }
    fn verify_pick_sidecar(&self) -> Result<(), String> {
        let scene = self.scene.as_deref().ok_or("Missing mesh scene")?;
        if crate::mesh_lighting_store::sidecar_stamp(scene)? != self.sidecar_sha {
            return Err("Saved lighting changed; reload before picking".into());
        }
        Ok(())
    }
    pub(crate) fn pick_pending(&self) -> bool {
        self.viewport_pick.pending.is_some()
    }
    pub(crate) fn cancel_pick(&mut self) {
        self.viewport_pick.pending = None;
    }
    pub(super) fn invalidate_viewport_pick(&mut self) {
        self.cancel_pick();
        self.pick_generation = self.pick_generation.saturating_add(1);
    }
    fn pick_raw(&self) -> Vec<String> {
        let mut raw = Vec::new();
        for values in [
            &self.direction,
            &self.color,
            &self.sky,
            &self.ground,
            &self.shadow_center,
        ] {
            raw.extend(values.iter().cloned());
        }
        raw.extend([
            self.intensity.clone(),
            self.ambient.clone(),
            self.shadow_radius.clone(),
            self.exposure.clone(),
            format!(
                "{:?}",
                (
                    self.shadows,
                    self.tonemap,
                    self.post_process,
                    self.post_process_authored,
                    self.point_light
                )
            ),
            format!(
                "{:?}",
                (
                    &self.profile,
                    &self.scene,
                    self.scene_epoch,
                    self.graph_revision,
                    &self.sidecar_sha,
                    &self.sidecar_error,
                    self.document_pending,
                    self.context_stale
                )
            ),
            format!(
                "{:?}",
                (self.cursor, self.next_id, self.saved_id, self.form_dirty)
            ),
        ]);
        for entry in &self.history {
            raw.push(format!("{}:{:?}", entry.id, entry.document));
        }
        raw
    }
    fn pick_document(&self, editor: &Editor) -> Result<Document, String> {
        if !self.can_edit(editor) || self.pick_generation == u64::MAX {
            return Err("Light picking requires the matching idle local Edit scene".into());
        }
        let doc = self.parse_document(
            self.scene.as_deref().ok_or("Missing scene")?,
            self.graph_revision.ok_or("Missing graph")?,
        )?;
        if doc.point_light.is_none() {
            return Err("Enable a point light before picking".into());
        }
        Ok(doc)
    }
    pub(crate) fn begin_viewport_pick(&mut self, editor: &Editor) -> Result<(), String> {
        if self.viewport_pick.busy {
            return Err("Finish releasing viewport input before picking".into());
        }
        self.cancel_pick();
        self.verify_pick_sidecar()?;
        let document = self.pick_document(editor)?;
        let checksum = editor
            .snapshot()
            .ok_or("Missing authoritative Frame")?
            .predicted()
            .checksum();
        self.viewport_pick.pending = Some(Pin {
            raw: self.pick_raw(),
            document,
            generation: self.pick_generation,
            scene_epoch: editor.scene_load_epoch(),
            host_epoch: editor.sim().epoch,
            checksum,
        });
        Ok(())
    }
    pub(crate) fn sync_viewport_pick(&mut self, editor: &Editor) -> bool {
        if !self.pick_pending() {
            return false;
        }
        let valid = self.pick_document(editor).ok().is_some_and(|doc| {
            self.viewport_pick.pending.as_ref().is_some_and(|pin| {
                pin.matches(
                    &self.pick_raw(),
                    &doc,
                    self.pick_generation,
                    (
                        editor.scene_load_epoch(),
                        editor.sim().epoch,
                        editor.snapshot().map(|s| s.predicted().checksum()),
                    ),
                )
            })
        });
        if !valid {
            self.cancel_pick();
        }
        valid
    }
    pub(crate) fn handle_viewport_pick(
        &mut self,
        editor: &Editor,
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
        if !self.sync_viewport_pick(editor) {
            return clicked;
        }
        if !clicked {
            return false;
        }
        let Some(point) = pixel.and_then(|p| ground_point(camera, size, p)) else {
            return true;
        };
        if let Err(error) = self.verify_pick_sidecar() {
            self.cancel_pick();
            self.error = Some(error);
            return true;
        }
        let next = self
            .viewport_pick
            .pending
            .as_ref()
            .and_then(|pin| stage_point(&pin.document, point).ok());
        if let Some(next) = next {
            self.point_light = next.point_light;
            self.invalidate_viewport_pick();
            self.form_dirty = true;
            self.error = None;
            self.status = "Point light X/Z staged; Apply and Save remain explicit.".into();
        }
        true
    }
}
impl Pin {
    fn matches(
        &self,
        raw: &[String],
        document: &Document,
        generation: u64,
        identity: (u64, u64, Option<u64>),
    ) -> bool {
        self.raw == raw
            && self.document == *document
            && self.generation == generation
            && self.scene_epoch == identity.0
            && self.host_epoch == identity.1
            && identity.2 == Some(self.checksum)
    }
}
fn stage_point(candidate: &Document, point: [f32; 2]) -> Result<Document, String> {
    let mut next = candidate.clone();
    let light = next.point_light.as_mut().ok_or("Missing point light")?;
    light.position[0] = point[0];
    light.position[2] = point[1];
    next.point_light_settings()?;
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
        .all(|v| v.is_finite() && v.abs() <= 1e9)
        .then_some(point)
}

#[cfg(test)]
#[path = "viewport_pick_tests.rs"]
mod tests;
