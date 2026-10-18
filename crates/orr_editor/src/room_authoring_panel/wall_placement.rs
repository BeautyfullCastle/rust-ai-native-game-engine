//! One-click selected-wall X/Z draft, with retained freshness through explicit Apply.
use super::*;
use orr_render::Camera3D;

#[derive(Clone, PartialEq, Eq)]
struct Pin {
    stamp: (Option<PathBuf>, u64, u64),
    guid: String,
    original: RoomWall,
    fields: [String; 7],
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pin>,
    staged: Option<Pin>,
}
impl Panel {
    fn wall_pin(&self, editor: &Editor) -> Option<Pin> {
        let stamp = (
            editor.path(),
            editor.scene_load_epoch(),
            editor.sim().checksum,
        );
        if !self.walls_current
            || self.wall_source_stamp.as_ref() != Some(&stamp)
            || !editor.game().is_room()
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || editor.previewing().is_some()
            || !editor.can_mutate()
            || editor.in_gesture()
        {
            return None;
        }
        let guid = self.selected_wall_guid.clone()?;
        let original = self.walls.iter().find(|wall| wall.guid == guid)?.clone();
        Some(Pin {
            stamp,
            guid,
            original,
            fields: [
                self.selected_wall_name.clone(),
                self.selected_wall_center_y.clone(),
                self.selected_wall_half_height.clone(),
                self.selected_wall_center[0].clone(),
                self.selected_wall_center[1].clone(),
                self.selected_wall_half_extents[0].clone(),
                self.selected_wall_half_extents[1].clone(),
            ],
        })
    }
    pub(crate) fn wall_placement_busy(&self) -> bool {
        self.wall_placement.pending.is_some()
    }
    pub(crate) fn cancel_wall_placement(&mut self) {
        if self.wall_placement.pending.take().is_some() {
            self.status = "Wall placement canceled; selected wall fields preserved.".into();
        }
    }
    pub(crate) fn sync_wall_placement(&mut self, editor: &Editor) {
        let current = self.wall_pin(editor);
        if self
            .wall_placement
            .staged
            .as_ref()
            .is_some_and(|pin| Some(pin) != current.as_ref())
        {
            self.wall_placement = Default::default();
            self.walls_current = false;
            self.status = "Wall placement is stale. Read Room walls again before Apply.".into();
        }
        if self
            .wall_placement
            .pending
            .as_ref()
            .is_some_and(|pin| Some(pin) != current.as_ref())
            || !self.open
            || !editor.room_camera_capture_coherent()
            || self.actor_placement_busy()
            || self.wall_footprint_busy()
        {
            self.cancel_wall_placement();
        }
    }
    pub(super) fn wall_apply_ready(&self, editor: &Editor) -> bool {
        self.wall_pin(editor).is_some()
            && !self.wall_placement_busy()
            && !self.actor_placement_busy()
            && !self.wall_footprint_busy()
            && self
                .wall_placement
                .staged
                .as_ref()
                .is_none_or(|pin| Some(pin) == self.wall_pin(editor).as_ref())
    }
    pub(super) fn apply_selected_wall(&mut self, editor: &mut Editor) -> Result<(), String> {
        self.sync_wall_placement(editor);
        if !self.wall_apply_ready(editor) {
            return Err(
                "Read the current selected Room wall and finish placement before Apply.".into(),
            );
        }
        let pin = self.wall_pin(editor).ok_or("Read Room walls again")?;
        let center = parse_position(&self.selected_wall_center)?;
        let half = parse_position(&self.selected_wall_half_extents)?;
        let name = if self.wall_placement.staged.is_some() {
            pin.original.name.as_str()
        } else {
            self.selected_wall_name.trim()
        };
        self.walls_current = false;
        self.wall_placement = Default::default();
        editor.update_room_wall_checked(&pin.guid, name, center, half, pin.stamp.2, &pin.original)
    }
    fn placement_fields_clean(&self, editor: &Editor) -> bool {
        self.wall_pin(editor).is_some_and(|pin| {
            self.selected_wall_name == pin.original.name
                && parse_position(&self.selected_wall_half_extents).ok()
                    == Some(pin.original.half_extents)
                && parse_fixed(&self.selected_wall_center_y).ok() == Some(pin.original.center_y)
                && parse_fixed(&self.selected_wall_half_height).ok()
                    == Some(pin.original.half_height)
        })
    }
    fn begin_wall_placement(&mut self, editor: &Editor) -> bool {
        self.sync_wall_placement(editor);
        if !self.open
            || !self.wall_apply_ready(editor)
            || !self.placement_fields_clean(editor)
            || !editor.room_camera_capture_coherent()
        {
            return false;
        }
        self.wall_placement.pending = self.wall_pin(editor);
        self.wall_placement_busy()
    }
    pub(super) fn show_wall_placement(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        self.sync_wall_placement(editor);
        ui.small("Ground-plane center placement; grid Off. Apply or reread other wall edits first; only X/Z are staged.");
        if ui
            .add_enabled(
                self.wall_apply_ready(editor) && self.placement_fields_clean(editor),
                egui::Button::new("Place selected wall in viewport"),
            )
            .clicked()
        {
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            self.begin_wall_placement(editor);
        }
        if self.wall_placement_busy() && ui.button("Cancel wall placement").clicked() {
            self.cancel_wall_placement();
        }
    }
    pub(crate) fn wall_placement_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        if !self.wall_placement_busy() {
            return false;
        }
        if input.cancel {
            self.cancel_wall_placement();
            return true;
        }
        self.sync_wall_placement(editor);
        if !self.wall_placement_busy() || !input.clicked {
            return true;
        }
        match ground_position(camera, size, input.pixel) {
            Ok(position) => {
                self.wall_placement.pending = None;
                self.selected_wall_center = position.map(format_fixed);
                self.wall_placement.staged = self.wall_pin(editor);
                self.status = "Selected wall center staged. Apply Room wall commits; no other fields changed.".into();
            }
            Err(error) => self.status = error,
        }
        true
    }
}
fn ground_position(
    camera: &Camera3D,
    size: (u32, u32),
    pixel: Option<[f32; 2]>,
) -> Result<[FP; 2], String> {
    let pixel = pixel.ok_or("Click inside the Room viewport.")?;
    if !(1..=16384).contains(&size.0)
        || !(1..=16384).contains(&size.1)
        || pixel.iter().any(|value| !value.is_finite())
        || pixel[0] < 0.0
        || pixel[1] < 0.0
        || pixel[0] >= size.0 as f32
        || pixel[1] >= size.1 as f32
    {
        return Err("Invalid Room viewport position.".into());
    }
    let point = camera
        .screen_to_plane_y(pixel, size, 0.0)
        .ok_or("The view ray does not reach the ground plane.")?;
    if point.iter().any(|value| !value.is_finite()) {
        return Err("Ground-plane intersection is not finite.".into());
    }
    let bound = orr_sample::room_game::CENTER_BOUND.to_f64();
    let mut result = [FP::ZERO; 2];
    for (axis, value) in [point[0], point[2]].into_iter().enumerate() {
        let value = f64::from(value);
        if value < -bound || value > bound {
            return Err("Room wall X/Z must be within -16..=16; no clamping is applied.".into());
        }
        result[axis] = FP::from_raw((value * FP::SCALE as f64).trunc() as i64);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
