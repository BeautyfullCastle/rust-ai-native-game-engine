//! View-only ground-plane placement. Only the existing Create action writes a wall.
use super::{Editor, FP, Mode, Panel, PathBuf, format_fixed};
use orr_render::Camera3D;

#[derive(Clone, Copy, Default)]
pub(crate) struct ViewInput {
    pub pixel: Option<[f32; 2]>,
    pub clicked: bool,
    pub cancel: bool,
}
#[derive(Clone, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
    name: String,
    center: [String; 2],
    half: [String; 2],
}
#[derive(Default)]
pub(super) struct State {
    context: Option<Context>,
    first: Option<[f64; 2]>,
}
impl Panel {
    pub(crate) fn wall_footprint_busy(&self) -> bool {
        self.footprint.context.is_some()
    }
    pub(crate) fn cancel_wall_footprint(&mut self) {
        if !self.wall_footprint_busy() {
            return;
        }
        self.footprint = State::default();
        self.status = "Wall footprint canceled; draft fields preserved.".into();
    }
    pub(crate) fn sync_wall_footprint(&mut self, editor: &Editor) {
        if self.wall_footprint_busy()
            && context(self, editor).as_ref() != self.footprint.context.as_ref()
        {
            self.cancel_wall_footprint();
        }
    }
    fn begin_wall_footprint(&mut self, editor: &Editor) {
        if self.wall_footprint_busy() {
            return;
        }
        if let Some(context) = context(self, editor) {
            self.footprint = State {
                context: Some(context),
                first: None,
            };
            self.status = "Click the first wall corner on the ground plane (Y=0).".into();
        }
    }
    pub(super) fn show_wall_footprint(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        self.sync_wall_footprint(editor);
        if ui
            .add_enabled(
                !self.wall_footprint_busy() && context(self, editor).is_some(),
                egui::Button::new("Draw Room wall footprint"),
            )
            .clicked()
        {
            ui.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            self.begin_wall_footprint(editor);
        }
        if self.wall_footprint_busy() && ui.button("Cancel wall footprint").clicked() {
            self.cancel_wall_footprint();
        }
        ui.small("Two opposite ground-plane corners stage X/Z fields. Name and wall height stay unchanged. Create Room wall commits.");
    }
    pub(crate) fn wall_footprint_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        if !self.wall_footprint_busy() {
            return false;
        }
        if input.cancel {
            self.cancel_wall_footprint();
            return true;
        }
        self.sync_wall_footprint(editor);
        if !self.wall_footprint_busy() || !input.clicked {
            return true;
        }
        let point = ground_point(camera, size, input.pixel);
        match point.and_then(|point| self.accept_corner(point)) {
            Ok(()) => {}
            Err(error) => self.status = error,
        }
        true
    }
    fn accept_corner(&mut self, point: [f64; 2]) -> Result<(), String> {
        for value in point {
            checked_fixed(value)?;
        }
        if let Some(first) = self.footprint.first {
            let (center, half) = rectangle(first, point)?;
            self.wall_center = center.map(format_fixed);
            self.wall_half_extents = half.map(format_fixed);
            self.footprint = State::default();
            self.status = "Wall footprint staged. Create Room wall applies it as one edit.".into();
        } else {
            self.footprint.first = Some(point);
            self.status =
                "Click the opposite wall corner. No scene changes until Create Room wall.".into();
        }
        Ok(())
    }
}
fn context(panel: &Panel, editor: &Editor) -> Option<Context> {
    if !panel.open
        || !editor.game().is_room()
        || !editor.spec().is_local()
        || editor.mode() != Mode::Edit
        || !editor.can_mutate()
        || editor.previewing().is_some()
        || editor.in_gesture()
        || !editor.room_camera_capture_coherent()
    {
        return None;
    }
    Some(Context {
        scene: editor.path()?,
        epoch: editor.scene_load_epoch(),
        checksum: editor.sim().checksum,
        name: panel.wall_name.clone(),
        center: panel.wall_center.clone(),
        half: panel.wall_half_extents.clone(),
    })
}
fn ground_point(
    camera: &Camera3D,
    size: (u32, u32),
    pixel: Option<[f32; 2]>,
) -> Result<[f64; 2], String> {
    let pixel = pixel.ok_or("Click inside the Room viewport.")?;
    if !(1..=16384).contains(&size.0)
        || !(1..=16384).contains(&size.1)
        || pixel.iter().any(|v| !v.is_finite())
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
    if point.iter().any(|v| !v.is_finite()) {
        return Err("Ground-plane intersection is not finite.".into());
    }
    Ok([f64::from(point[0]), f64::from(point[2])])
}
fn checked_fixed(value: f64) -> Result<FP, String> {
    let raw = value * FP::SCALE as f64;
    // i64::MAX rounds upward in f64, so the upper endpoint is exclusive.
    if !raw.is_finite() || raw < i64::MIN as f64 || raw >= -(i64::MIN as f64) {
        return Err("Wall footprint exceeds fixed-point bounds.".into());
    }
    Ok(FP::from_raw(raw.trunc() as i64))
}
fn rectangle(a: [f64; 2], b: [f64; 2]) -> Result<([FP; 2], [FP; 2]), String> {
    let mut center = [FP::ZERO; 2];
    let mut half = center;
    for axis in 0..2 {
        checked_fixed(a[axis])?;
        checked_fixed(b[axis])?;
        center[axis] = checked_fixed(a[axis] * 0.5 + b[axis] * 0.5)?;
        half[axis] = checked_fixed((a[axis] * 0.5 - b[axis] * 0.5).abs())?;
        if half[axis] <= FP::ZERO {
            return Err(
                "Wall footprint needs positive width and depth after fixed-point conversion."
                    .into(),
            );
        }
    }
    Ok((center, half))
}
#[cfg(test)]
mod tests;
