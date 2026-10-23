//! Draft-only center placement on the already-admitted walkable surface.
use super::{Identity, Panel, current, parse_count};
use crate::editor::Editor;
use orr_fp::FP;
use orr_render::Camera3D;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewInput {
    pub pixel: Option<[f32; 2]>,
    pub clicked: bool,
    pub cancel: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct PickContext {
    identity: Identity,
    enabled: bool,
    count: String,
    spheres: [[String; 4]; super::MAX_HAZARDS],
}
#[derive(Default)]
pub(crate) struct State {
    pending: Option<usize>,
    context: Option<PickContext>,
    status: String,
}
impl Panel {
    pub(crate) fn hazard_pick_busy(&self) -> bool {
        self.viewport_picking.pending.is_some()
    }
    pub(crate) fn cancel_hazard_pick(&mut self) {
        self.viewport_picking
            .cancel("Hazard center picking canceled; existing draft fields were preserved.");
    }
    pub(crate) fn sync_hazard_picking(&mut self, editor: &Editor) {
        if let Some(index) = self.viewport_picking.pending {
            if context(self, editor, index).ok().as_ref() != self.viewport_picking.context.as_ref()
            {
                self.viewport_picking.cancel("Hazard center picking canceled because the scene, graph, document or draft changed.");
            }
        }
    }
    pub(super) fn show_hazard_picking(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        self.sync_hazard_picking(editor);
        ui.group(|ui| {
            ui.strong("Viewport hazard center picking");
            ui.weak("Click a walkable surface to stage a sphere center. Radius stays unchanged; Apply mesh hazards commits it.");
            for index in 0..parse_count(&self.count).unwrap_or(0) {
                let ready=!self.hazard_pick_busy() && context(self,editor,index).is_ok();
                if ui.add_enabled(ready,egui::Button::new(format!("Pick hazard {} center in viewport",index+1))).clicked() {
                    ui.memory_mut(|memory| { if let Some(id) = memory.focused() { memory.surrender_focus(id); } });
                    self.begin_hazard_pick(editor,index);
                }
            }
            if self.hazard_pick_busy() && ui.button("Cancel hazard center picking").clicked() { self.cancel_hazard_pick(); }
            if !self.viewport_picking.status.is_empty() { ui.label(&self.viewport_picking.status); }
        });
    }
    fn begin_hazard_pick(&mut self, editor: &Editor, index: usize) -> bool {
        if self.hazard_pick_busy() {
            return false;
        }
        match context(self, editor, index) {
            Ok(context) => {
                self.viewport_picking.context = Some(context);
                self.viewport_picking.pending = Some(index);
                self.viewport_picking.status =
                    format!("Click a walkable surface for hazard {} center.", index + 1);
                true
            }
            Err(error) => {
                self.viewport_picking.status = error;
                false
            }
        }
    }
    pub(crate) fn hazard_viewport_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        // Capture ownership before sync so a stale/canceled gesture cannot leak
        // its final click into another authoring or gameplay operation.
        let Some(index) = self.viewport_picking.pending else {
            return false;
        };
        if input.cancel {
            self.cancel_hazard_pick();
            return true;
        }
        self.sync_hazard_picking(editor);
        if !self.hazard_pick_busy() {
            return true;
        }
        if !input.clicked {
            return true;
        }
        let Some(pixel) = input.pixel else {
            self.viewport_picking.status =
                "Click a walkable surface to place the hazard center.".into();
            return true;
        };
        if size.0 == 0
            || size.1 == 0
            || pixel.iter().any(|v| !v.is_finite())
            || pixel[0] < 0.0
            || pixel[1] < 0.0
            || pixel[0] >= size.0 as f32
            || pixel[1] >= size.1 as f32
        {
            self.viewport_picking.status =
                "Viewport position is invalid; click a walkable surface.".into();
            return true;
        }
        let result = (|| {
            let (current, _) = current(editor)?;
            let (origin, direction) = camera.screen_ray(pixel, size);
            crate::mesh_agent_view::pick(
                &current.mesh.graph,
                origin,
                direction,
                current.mesh.pin.max_vertical,
            )
        })();
        match result {
            Ok(Some(hit)) => match stage_position(self, index, hit.position) {
                Ok(changed) => {
                    self.form_dirty |= changed;
                    self.error = None;
                    self.viewport_picking.pending = None;
                    self.viewport_picking.context = None;
                    self.viewport_picking.status = format!(
                        "Hazard {} center staged from triangle {}; Apply mesh hazards to commit.",
                        index + 1,
                        hit.triangle
                    );
                }
                Err(error) => self.viewport_picking.status = error,
            },
            Ok(None) => {
                self.viewport_picking.status =
                    "No walkable surface was hit; click again or cancel.".into()
            }
            Err(error) => {
                self.viewport_picking.status = format!("Hazard center pick failed: {error}")
            }
        }
        true
    }
}
impl State {
    fn cancel(&mut self, status: &str) {
        if self.pending.take().is_some() {
            self.context = None;
            self.status = status.into();
        }
    }
}
fn context(panel: &Panel, editor: &Editor, index: usize) -> Result<PickContext, String> {
    if !panel.can_edit(editor) || !panel.enabled || index >= parse_count(&panel.count)? {
        return Err("Hazard picking requires an enabled sphere in the current idle editable local hazard scene.".into());
    }
    let (context, _) = current(editor)?;
    Ok(PickContext {
        identity: Identity::from_context(&context),
        enabled: panel.enabled,
        count: panel.count.clone(),
        spheres: panel.spheres.clone(),
    })
}
fn stage_position(panel: &mut Panel, index: usize, position: [FP; 3]) -> Result<bool, String> {
    if !panel.enabled || index >= parse_count(&panel.count)? {
        return Err("Selected hazard no longer exists in this draft.".into());
    }
    if position
        .iter()
        .any(|value| !(-256 * FP::SCALE..=256 * FP::SCALE).contains(&value.raw()))
    {
        return Err("Picked hazard center exceeds +/-256 units.".into());
    }
    let values = position.map(orr_reflect::decimal::fp_to_decimal);
    let selected = &mut panel.spheres[index][..3];
    let changed = selected != values;
    selected.clone_from_slice(&values);
    Ok(changed)
}
#[cfg(test)]
#[path = "viewport_picking/tests.rs"]
mod tests;
