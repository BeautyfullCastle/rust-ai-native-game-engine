//! Draft-only placement of one existing Room role; Apply remains one layout transaction.
use super::{Editor, FP, Mode, Panel, PathBuf, ViewInput, format_fixed};
use orr_render::Camera3D;
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
}
#[derive(Clone, PartialEq, Eq)]
struct Pending {
    identity: Identity,
    role: usize,
    fields: [[String; 2]; 3],
}
#[derive(Default)]
pub(super) struct State {
    source: Option<Identity>,
    pending: Option<Pending>,
}
impl Panel {
    pub(crate) fn actor_placement_busy(&self) -> bool {
        self.actor_placement.pending.is_some()
    }
    pub(crate) fn cancel_actor_placement(&mut self) {
        if self.actor_placement.pending.take().is_some() {
            self.status = "Actor placement canceled; layout fields preserved.".into();
        }
    }
    fn layout_fields(&self) -> [[String; 2]; 3] {
        [self.player.clone(), self.key.clone(), self.exit.clone()]
    }
    pub(crate) fn sync_actor_placement(&mut self, editor: &Editor) {
        let current = identity(editor);
        if self.actor_placement.source.is_some() && self.actor_placement.source != current {
            self.actor_placement.source = None;
            self.cancel_actor_placement();
            self.status =
                "Room layout is stale; Read Room layout again before placing or applying.".into();
        }
        if self
            .actor_placement
            .pending
            .as_ref()
            .is_some_and(|pending| {
                Some(&pending.identity) != current.as_ref()
                    || pending.fields != self.layout_fields()
                    || !self.open
                    || !editor.room_camera_capture_coherent()
                    || self.wall_footprint_busy()
            })
        {
            self.cancel_actor_placement();
        }
    }
    pub(super) fn read_actor_layout(&mut self, editor: &mut Editor) -> Result<(), String> {
        self.cancel_actor_placement();
        self.actor_placement.source = None;
        let [player, key, exit] = editor.read_room_layout()?;
        self.player = player.map(format_fixed);
        self.key = key.map(format_fixed);
        self.exit = exit.map(format_fixed);
        self.actor_placement.source = identity(editor);
        Ok(())
    }
    pub(super) fn layout_apply_ready(&self, editor: &Editor) -> bool {
        self.actor_placement.source.is_some()
            && self.actor_placement.source == identity(editor)
            && !self.actor_placement_busy()
            && !self.wall_footprint_busy()
            && editor.room_camera_capture_coherent()
    }
    pub(super) fn apply_actor_layout(
        &mut self,
        editor: &mut Editor,
        positions: [[FP; 2]; 3],
    ) -> Result<(), String> {
        self.sync_actor_placement(editor);
        if !self.layout_apply_ready(editor) {
            return Err("Read the current Room layout and finish placement before Apply.".into());
        }
        let expected = self
            .actor_placement
            .source
            .as_ref()
            .expect("checked source")
            .checksum;
        if let Err(error) = editor.apply_room_layout_checked(positions, expected) {
            self.sync_actor_placement(editor);
            return Err(error);
        }
        // A fresh Read is required after a successful authoritative edit.
        self.actor_placement.source = None;
        Ok(())
    }
    fn begin_actor_placement(&mut self, editor: &Editor, role: usize) -> bool {
        self.sync_actor_placement(editor);
        if role >= 3 || !self.open || !self.layout_apply_ready(editor) {
            return false;
        }
        self.actor_placement.pending = Some(Pending {
            identity: identity(editor).expect("checked identity"),
            role,
            fields: self.layout_fields(),
        });
        self.status="Click the ground plane to stage this role's X/Z. Apply Room layout commits all three roles.".into();
        true
    }
    pub(super) fn show_actor_placement(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        self.sync_actor_placement(editor);
        ui.small("Read Room layout first. Placement changes only the selected role's X/Z; Apply commits all three together.");
        for (role, name) in ["Player", "Key", "Exit"].into_iter().enumerate() {
            if ui
                .add_enabled(
                    self.layout_apply_ready(editor),
                    egui::Button::new(format!("Place {name} in viewport")),
                )
                .clicked()
            {
                ui.memory_mut(|memory| {
                    if let Some(id) = memory.focused() {
                        memory.surrender_focus(id);
                    }
                });
                self.begin_actor_placement(editor, role);
            }
        }
        if self.actor_placement_busy() && ui.button("Cancel actor placement").clicked() {
            self.cancel_actor_placement();
        }
    }
    pub(crate) fn actor_placement_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        if !self.actor_placement_busy() {
            return false;
        }
        if input.cancel {
            self.cancel_actor_placement();
            return true;
        }
        self.sync_actor_placement(editor);
        if !self.actor_placement_busy() || !input.clicked {
            return true;
        }
        match ground_position(camera, size, input.pixel) {
            Ok(position) => {
                let role = self
                    .actor_placement
                    .pending
                    .take()
                    .expect("pending checked")
                    .role;
                let fields = position.map(format_fixed);
                match role {
                    0 => self.player = fields,
                    1 => self.key = fields,
                    _ => self.exit = fields,
                };
                self.status =
                    "Actor position staged. Apply Room layout commits all three roles.".into();
            }
            Err(error) => self.status = error,
        }
        true
    }
}
fn identity(editor: &Editor) -> Option<Identity> {
    if !editor.game().is_room()
        || !editor.spec().is_local()
        || editor.mode() != Mode::Edit
        || !editor.can_mutate()
        || editor.previewing().is_some()
        || editor.in_gesture()
    {
        return None;
    }
    Some(Identity {
        scene: editor.path()?,
        epoch: editor.scene_load_epoch(),
        checksum: editor.sim().checksum,
    })
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
            return Err("Room actor X/Z must be within -16..=16; no clamping is applied.".into());
        }
        result[axis] = FP::from_raw((value * FP::SCALE as f64).trunc() as i64);
    }
    Ok(result)
}
#[cfg(test)]
mod tests;
