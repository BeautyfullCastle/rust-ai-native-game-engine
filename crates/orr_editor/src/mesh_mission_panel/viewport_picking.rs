use super::{current_identity, format_fixed, parse_count, Panel, MAX_COORDINATE_RAW};
use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_render::Camera3D;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Collectible(usize),
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PickContext {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
    frame_checksum: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewInput {
    pub pixel: Option<[f32; 2]>,
    pub clicked: bool,
    pub cancel: bool,
}

#[derive(Default)]
pub(crate) struct State {
    pending: Option<Target>,
    context: Option<PickContext>,
    status: String,
}

impl Panel {
    pub(crate) fn mission_pick_busy(&self) -> bool {
        self.viewport_picking.pending.is_some()
    }

    pub(crate) fn cancel_mission_pick(&mut self) {
        self.with_viewport_picking(|state, _panel| state.cancel());
    }

    pub(crate) fn sync_mission_picking(&mut self, editor: &Editor) {
        if !self.mission_pick_busy() {
            return;
        }
        self.with_viewport_picking(|state, panel| state.sync(panel, editor));
    }

    pub(crate) fn show_mission_picking(&mut self, ui: &mut Ui, editor: &Editor) {
        self.sync_mission_picking(editor);
        let can_begin = self.viewport_picking.can_begin(self, editor);
        let count = parse_count(&self.count).unwrap_or(0);
        let mut target = None;
        let mut cancel = false;
        ui.group(|ui| {
            ui.strong("Viewport mission point picking");
            ui.label("Pick collectible or exit positions on the admitted walkable mesh. Apply mesh mission commits the staged fields.");
            for index in 0..count {
                if ui
                    .add_enabled(
                        can_begin && !self.mission_pick_busy(),
                        egui::Button::new(format!("Pick collectible {} in viewport", index + 1)),
                    )
                    .clicked()
                {
                    target = Some(Target::Collectible(index));
                }
            }
            if ui
                .add_enabled(
                    can_begin && !self.mission_pick_busy(),
                    egui::Button::new("Pick mission exit in viewport"),
                )
                .clicked()
            {
                target = Some(Target::Exit);
            }
            if self.mission_pick_busy() && ui.button("Cancel mission point picking").clicked() {
                cancel = true;
            }
            if let Some(pending) = self.viewport_picking.pending {
                ui.label(match pending {
                    Target::Collectible(index) => {
                        format!("Click an admitted walkable surface for collectible {}.", index + 1)
                    }
                    Target::Exit => "Click an admitted walkable surface for the mission exit.".into(),
                });
            }
            if !self.viewport_picking.status.is_empty() {
                ui.label(&self.viewport_picking.status);
            }
        });
        if cancel {
            self.cancel_mission_pick();
        } else if let Some(target) = target {
            self.with_viewport_picking(|state, panel| state.begin(panel, editor, target));
        }
    }

    pub(crate) fn mission_viewport_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        self.sync_mission_picking(editor);
        self.with_viewport_picking(|state, panel| state.input(panel, editor, camera, size, input))
    }

    fn with_viewport_picking<T>(
        &mut self,
        operation: impl FnOnce(&mut State, &mut Panel) -> T,
    ) -> T {
        let mut state = std::mem::take(&mut self.viewport_picking);
        let result = operation(&mut state, self);
        self.viewport_picking = state;
        result
    }
}

impl State {
    fn can_begin(&self, panel: &Panel, editor: &Editor) -> bool {
        context(panel, editor).is_ok()
    }

    fn begin(&mut self, panel: &mut Panel, editor: &Editor, target: Target) {
        match context(panel, editor) {
            Ok(context) => {
                self.context = Some(context);
                self.pending = Some(target);
                self.status = match target {
                    Target::Collectible(index) => format!(
                        "Picking collectible {}; click a walkable surface.",
                        index + 1
                    ),
                    Target::Exit => "Picking mission exit; click a walkable surface.".into(),
                };
            }
            Err(reason) => self.status = format!("Cannot pick mission point: {reason}"),
        }
    }

    fn input(
        &mut self,
        panel: &mut Panel,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        let Some(target) = self.pending else {
            return false;
        };
        if input.cancel {
            self.cancel();
            return true;
        }
        match context(panel, editor) {
            Ok(current) if self.context.as_ref() == Some(&current) => {}
            Ok(_) => {
                self.cancel_with_status(
                    "Mission point picking canceled because the scene or graph changed.",
                );
                return true;
            }
            Err(reason) => {
                self.cancel_with_status(&format!("Mission point picking canceled: {reason}"));
                return true;
            }
        }
        if !input.clicked {
            return true;
        }
        let Some(pixel) = input.pixel else {
            self.status =
                "Viewport click had no position; click a walkable surface to try again.".into();
            return true;
        };
        if size.0 == 0 || size.1 == 0 || pixel.iter().any(|coordinate| !coordinate.is_finite()) {
            self.status = "Viewport click coordinates are invalid; click again.".into();
            return true;
        }
        let Some(snapshot) = editor.snapshot() else {
            self.cancel_with_status(
                "Mission point picking canceled because the mesh Frame is unavailable.",
            );
            return true;
        };
        let view = match orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted()) {
            Ok(view) => view,
            Err(error) => {
                self.status =
                    format!("Mission point pick could not read the admitted graph: {error}");
                return true;
            }
        };
        let (origin, direction) = camera.screen_ray(pixel, size);
        match crate::mesh_agent_view::pick(&view.graph, origin, direction, view.pin.max_vertical) {
            Ok(Some(hit)) => match stage_position(panel, target, hit.position) {
                Ok(changed) => {
                    if changed {
                        panel.form_dirty = true;
                        panel.error = None;
                    }
                    self.pending = None;
                    self.context = None;
                    self.status = match target {
                            Target::Collectible(index) => format!(
                                "Collectible {} position staged from triangle {}; Apply mesh mission to commit it.",
                                index + 1,
                                hit.triangle
                            ),
                            Target::Exit => format!(
                                "Mission exit position staged from triangle {}; Apply mesh mission to commit it.",
                                hit.triangle
                            ),
                        };
                }
                Err(error) => self.status = error,
            },
            Ok(None) => {
                self.status =
                    "No admitted walkable surface was hit; click a surface to try again.".into()
            }
            Err(error) => self.status = format!("Mission point pick failed: {error}"),
        }
        true
    }

    fn sync(&mut self, panel: &Panel, editor: &Editor) {
        match context(panel, editor) {
            Ok(current) if self.context.as_ref() == Some(&current) => {}
            Ok(_) => self.cancel_with_status(
                "Mission point picking canceled because the scene or graph changed.",
            ),
            Err(reason) => {
                self.cancel_with_status(&format!("Mission point picking canceled: {reason}"))
            }
        }
    }

    fn cancel(&mut self) {
        if self.pending.take().is_some() {
            self.context = None;
            self.status =
                "Mission point picking canceled; staged coordinates were preserved.".into();
        }
    }

    fn cancel_with_status(&mut self, status: &str) {
        self.cancel();
        self.status = status.into();
    }
}

fn context(panel: &Panel, editor: &Editor) -> Result<PickContext, String> {
    if editor.game() != EditorGame::MeshNavigation
        || !editor.spec().is_local()
        || editor.mode() != Mode::Edit
        || !editor.can_mutate()
        || editor.in_gesture()
        || editor.previewing().is_some()
        || !editor.proposals().is_empty()
        || panel.document_pending
        || panel.context_stale
    {
        return Err("picking needs the matching idle editable local MeshNavigation scene".into());
    }
    let Some((scene, graph_revision, frame_checksum, _)) = current_identity(editor) else {
        return Err("wait for a coherent admitted mesh scene".into());
    };
    if panel.scene.as_ref() != Some(&scene)
        || panel.scene_epoch != Some(editor.scene_load_epoch())
        || panel.graph_revision != Some(graph_revision)
        || panel.observed_checksum != Some(frame_checksum)
    {
        return Err("the mission panel context does not match the current scene".into());
    }
    Ok(PickContext {
        scene,
        epoch: editor.scene_load_epoch(),
        graph_revision,
        frame_checksum,
    })
}

fn stage_position(panel: &mut Panel, target: Target, position: [FP; 3]) -> Result<bool, String> {
    if position
        .iter()
        .any(|coordinate| !(-MAX_COORDINATE_RAW..=MAX_COORDINATE_RAW).contains(&coordinate.raw()))
    {
        return Err("Picked mission position exceeds the +/-256 unit bound.".into());
    }
    let coordinates = position.map(format_fixed);
    let values = match target {
        Target::Collectible(index) => {
            let count = parse_count(&panel.count)?;
            if index >= count {
                return Err(
                    "The selected collectible no longer exists in the mission form.".into(),
                );
            }
            &mut panel.points[index][..3]
        }
        Target::Exit => &mut panel.exit[..3],
    };
    let changed = values
        .iter()
        .zip(coordinates.iter())
        .any(|(current, picked)| current != picked);
    values.clone_from_slice(&coordinates);
    Ok(changed)
}

#[cfg(test)]
#[path = "viewport_picking/tests.rs"]
mod tests;
