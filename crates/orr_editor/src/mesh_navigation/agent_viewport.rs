use super::Panel;
use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_render::{Camera3D, RenderList3D};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewInput {
    pub pixel: Option<[f32; 2]>,
    pub clicked: bool,
    pub cancel: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Endpoint {
    Start,
    Goal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PickContext {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
    frame_checksum: u64,
}

#[derive(Default)]
pub(crate) struct State {
    pending: Option<Endpoint>,
    context: Option<PickContext>,
    status: String,
}

impl Panel {
    pub(crate) fn agent_pick_busy(&self) -> bool {
        self.agent_viewport.pending.is_some()
    }

    pub(crate) fn cancel_agent_pick(&mut self) {
        self.with_agent_viewport(|state, _panel| state.cancel());
    }

    pub(crate) fn sync_agent_picking(&mut self, editor: &Editor) {
        if !self.agent_pick_busy() {
            return;
        }
        self.with_agent_viewport(|state, panel| state.sync(panel, editor));
    }

    pub(crate) fn show_agent_picking(&mut self, ui: &mut Ui, editor: &Editor) {
        self.sync_agent_picking(editor);
        let can_begin = self.agent_viewport.can_begin(self, editor);
        let mut begin_start = false;
        let mut begin_goal = false;
        let mut cancel = false;
        ui.group(|ui| {
            ui.strong("Viewport endpoint picking");
            ui.label("Pick walkable start and goal points without changing the host until Place mesh agent.");
            if ui
                .add_enabled(
                    can_begin && !self.agent_pick_busy(),
                    egui::Button::new("Pick agent start in viewport"),
                )
                .clicked()
            {
                begin_start = true;
            }
            if ui
                .add_enabled(
                    can_begin && !self.agent_pick_busy(),
                    egui::Button::new("Pick agent goal in viewport"),
                )
                .clicked()
            {
                begin_goal = true;
            }
            if self.agent_pick_busy()
                && ui.button("Cancel endpoint picking").clicked()
            {
                cancel = true;
            }
            if let Some(endpoint) = self.agent_viewport.pending {
                ui.label(match endpoint {
                    Endpoint::Start => "Click a walkable surface for the start point.",
                    Endpoint::Goal => "Click a walkable surface for the goal point.",
                });
            }
            if !self.agent_viewport.status.is_empty() {
                ui.label(&self.agent_viewport.status);
            }
        });
        if cancel {
            self.cancel_agent_pick();
        } else if begin_start {
            self.with_agent_viewport(|state, panel| state.begin(panel, editor, Endpoint::Start));
        } else if begin_goal {
            self.with_agent_viewport(|state, panel| state.begin(panel, editor, Endpoint::Goal));
        }
    }

    pub(crate) fn agent_viewport_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        self.sync_agent_picking(editor);
        self.with_agent_viewport(|state, panel| state.input(panel, editor, camera, size, input))
    }

    pub(crate) fn paint_agent_endpoints(&self, editor: &Editor, list: &mut RenderList3D) {
        if editor.mode() != Mode::Edit
            || self
                .agent_viewport
                .pick_context(self, editor, false)
                .is_err()
            || !self.agent_viewport.should_paint(self)
        {
            return;
        }
        for (endpoint, color) in [
            (&self.start, [0.2, 1.0, 0.35, 1.0]),
            (&self.goal, [1.0, 0.35, 0.2, 1.0]),
        ] {
            let Some(position) = parse_endpoint(endpoint) else {
                continue;
            };
            let half = [0.12; 3];
            list.aabb(
                std::array::from_fn(|axis| position[axis] - half[axis]),
                std::array::from_fn(|axis| position[axis] + half[axis]),
                2.0,
                color,
            );
        }
    }

    fn with_agent_viewport<T>(&mut self, operation: impl FnOnce(&mut State, &mut Panel) -> T) -> T {
        let mut state = std::mem::take(&mut self.agent_viewport);
        let result = operation(&mut state, self);
        self.agent_viewport = state;
        result
    }
}

impl State {
    fn can_begin(&self, panel: &Panel, editor: &Editor) -> bool {
        self.pick_context(panel, editor, true).is_ok()
    }

    fn pick_context(
        &self,
        panel: &Panel,
        editor: &Editor,
        allow_prop_gesture: bool,
    ) -> Result<PickContext, String> {
        if editor.game() != EditorGame::MeshNavigation
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.in_gesture()
            || editor.previewing().is_some()
            || !editor.proposals().is_empty()
        {
            return Err("picking needs an idle editable local MeshNavigation scene".into());
        }
        if panel.props.dirty() && !(allow_prop_gesture && panel.props.gesture_busy()) {
            return Err("save or undo mesh props before picking agent endpoints".into());
        }
        if panel.art.import_busy() {
            return Err("finish the agent-art import before picking endpoints".into());
        }
        let scene = editor
            .path()
            .ok_or_else(|| "the admitted mesh scene has no local path".to_owned())?;
        let snapshot = editor
            .snapshot()
            .ok_or_else(|| "wait for a coherent mesh Frame".to_owned())?;
        if editor.checksum() != editor.sim().checksum {
            return Err("wait for the matching current mesh document Frame".into());
        }
        let view = super::mesh_from_view(snapshot.predicted())?;
        let graph_revision = view.graph.revision();
        if panel.loaded_scene_epoch != Some(editor.scene_load_epoch())
            || panel.loaded_scene.as_ref() != Some(&scene)
            || panel.loaded_graph_revision != Some(graph_revision)
            || panel.graph.as_ref().map(StaticMeshGraph::revision) != Some(graph_revision)
        {
            return Err("the loaded graph does not match the live mesh Frame".into());
        }
        Ok(PickContext {
            scene,
            epoch: editor.scene_load_epoch(),
            graph_revision,
            frame_checksum: editor.checksum(),
        })
    }

    fn begin(&mut self, panel: &mut Panel, editor: &Editor, endpoint: Endpoint) {
        if panel.props.gesture_busy() {
            panel.props.cancel_viewport_gesture();
        }
        match self.pick_context(panel, editor, false) {
            Ok(context) => {
                self.context = Some(context);
                self.pending = Some(endpoint);
                self.status = match endpoint {
                    Endpoint::Start => "Picking start endpoint; click a walkable surface.".into(),
                    Endpoint::Goal => "Picking goal endpoint; click a walkable surface.".into(),
                };
            }
            Err(reason) => self.status = format!("Cannot pick endpoint: {reason}"),
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
        let Some(endpoint) = self.pending else {
            return false;
        };
        if input.cancel {
            self.cancel();
            return true;
        }
        match self.pick_context(panel, editor, false) {
            Ok(context) if self.context.as_ref() == Some(&context) => {}
            Ok(_) => {
                self.cancel_with_status(
                    "Endpoint picking canceled because the scene or graph changed.",
                );
                return true;
            }
            Err(reason) => {
                self.cancel_with_status(&format!("Endpoint picking canceled: {reason}"));
                return true;
            }
        }
        if !input.clicked {
            return true;
        }
        let Some(pixel) = input.pixel else {
            self.status = "Endpoint click had no viewport position; click again.".into();
            return true;
        };
        if size.0 == 0 || size.1 == 0 || !pixel.iter().all(|value| value.is_finite()) {
            self.status = "Endpoint click has invalid viewport coordinates; click again.".into();
            return true;
        }
        let Some(graph) = panel.graph.as_ref() else {
            self.cancel_with_status(
                "Endpoint picking canceled because the mesh graph is unavailable.",
            );
            return true;
        };
        let max_vertical = match super::parse_fixed(&panel.max_vertical) {
            Ok(value) if value.raw() >= 0 => value,
            _ => {
                self.status =
                    "Maximum vertical projection must be a nonnegative fixed-point value.".into();
                return true;
            }
        };
        let (origin, direction) = camera.screen_ray(pixel, size);
        match crate::mesh_agent_view::pick(graph, origin, direction, max_vertical) {
            Ok(Some(hit)) => {
                let target = match endpoint {
                    Endpoint::Start => &mut panel.start,
                    Endpoint::Goal => &mut panel.goal,
                };
                *target = hit.position.map(super::format_fixed);
                panel.draft_dirty = true;
                self.pending = None;
                self.context = None;
                self.status = match endpoint {
                    Endpoint::Start => {
                        format!("Start endpoint staged on triangle {}; Place mesh agent to apply it to the host.", hit.triangle)
                    }
                    Endpoint::Goal => {
                        format!("Goal endpoint staged on triangle {}; Place mesh agent to apply it to the host.", hit.triangle)
                    }
                };
            }
            Ok(None) => {
                self.status =
                    "No admitted walkable surface was hit; click a surface to try again.".into()
            }
            Err(error) => self.status = format!("Endpoint pick failed: {error}"),
        }
        true
    }

    fn sync(&mut self, panel: &Panel, editor: &Editor) {
        match self.pick_context(panel, editor, false) {
            Ok(context) if self.context.as_ref() == Some(&context) => {}
            Ok(_) => self.cancel_with_status(
                "Endpoint picking canceled because the scene or graph changed.",
            ),
            Err(reason) => self.cancel_with_status(&format!("Endpoint picking canceled: {reason}")),
        }
    }

    fn cancel(&mut self) {
        if self.pending.take().is_some() {
            self.context = None;
            self.status = "Endpoint picking canceled; staged coordinates were preserved.".into();
        }
    }

    fn cancel_with_status(&mut self, status: &str) {
        self.cancel();
        self.status = status.into();
    }

    fn should_paint(&self, panel: &Panel) -> bool {
        self.pending.is_some()
            || panel.loaded_spec.is_some_and(|spec| {
                parse_endpoint(&panel.start).is_none_or(|point| point != spec.start.map(FP::to_f32))
                    || parse_endpoint(&panel.goal)
                        .is_none_or(|point| point != spec.goal.map(FP::to_f32))
            })
    }
}

fn parse_endpoint(values: &[String; 3]) -> Option<[f32; 3]> {
    let mut point = [0.0; 3];
    for (axis, value) in values.iter().enumerate() {
        let coordinate = super::parse_fixed(value).ok()?.to_f32();
        if !coordinate.is_finite() || coordinate.abs() > 256.0 {
            return None;
        }
        point[axis] = coordinate;
    }
    Some(point)
}

#[cfg(test)]
#[path = "agent_viewport/tests.rs"]
mod tests;
