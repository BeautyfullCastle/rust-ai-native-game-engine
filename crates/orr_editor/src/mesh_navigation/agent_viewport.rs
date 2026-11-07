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
    #[cfg(feature = "mesh-patrol")]
    Patrol(usize),
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
    #[cfg(feature = "mesh-patrol")]
    patrol_ticket: Option<super::PatrolPickToken>,
    #[cfg(feature = "mesh-patrol")]
    owned_release: bool,
    #[cfg(feature = "mesh-patrol")]
    waiting_release: bool,
    #[cfg(feature = "mesh-patrol")]
    focused: bool,
}

impl Panel {
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn patrol_pick_busy(&self) -> bool {
        matches!(self.agent_viewport.pending, Some(Endpoint::Patrol(_)))
    }
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn patrol_pick_owns_input(&self) -> bool {
        self.patrol_pick_busy() || self.agent_viewport.owned_release
    }
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn cancel_patrol_pick(&mut self) {
        if self.patrol_pick_busy() {
            self.cancel_agent_pick();
        }
    }
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn begin_patrol_pick(&mut self, editor: &Editor, row: usize) -> Result<(), String> {
        self.with_agent_viewport(|state, panel| {
            if !state.focused || state.owned_release || state.pending.is_some() {
                return Err(
                    "Finish the current viewport gesture before picking a patrol waypoint".into(),
                );
            }
            let ticket = panel.patrol_pick_token(editor, row)?;
            state.patrol_ticket = Some(ticket);
            state.pending = Some(Endpoint::Patrol(row));
            state.status = format!(
                "Pick patrol waypoint {} on a walkable mesh surface",
                row + 1
            );
            Ok(())
        })
    }
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn sync_patrol_picking(
        &mut self,
        editor: &Editor,
        ctx: &egui::Context,
        blocked: bool,
    ) {
        self.with_agent_viewport(|state, panel| {
            let (down, released, explicit_press, explicit_release) = ctx.input(|input| {
                (
                    input.pointer.primary_down(),
                    input.pointer.primary_released(),
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::PointerButton {
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                ..
                            }
                        )
                    }),
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::PointerButton {
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            }
                        )
                    }),
                )
            });
            if (down || explicit_press)
                && (matches!(state.pending, Some(Endpoint::Patrol(_))) || state.owned_release)
            {
                state.waiting_release = true;
            }
            if explicit_release {
                state.waiting_release = false;
            }
            // Focus loss may clear egui's held state without a physical release.
            if !down && !released && !state.waiting_release {
                state.owned_release = false;
            }
            state.focused = !blocked
                && ctx.input(|input| {
                    input.focused
                        && !input
                            .events
                            .iter()
                            .any(|event| matches!(event, egui::Event::WindowFocused(false)))
                });
            if matches!(state.pending, Some(Endpoint::Patrol(_))) {
                if !state.focused
                    || blocked
                    || ctx.input(|input| input.key_pressed(egui::Key::Escape))
                {
                    state.cancel();
                } else {
                    state.sync(panel, editor);
                }
            }
        });
    }
    #[cfg(feature = "mesh-patrol")]
    pub(crate) fn patrol_viewport_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
        frame_available: bool,
    ) -> bool {
        let owned = self.patrol_pick_owns_input();
        if !owned {
            return false;
        }
        if !frame_available {
            self.cancel_patrol_pick();
            return true;
        }
        self.with_agent_viewport(|state, panel| {
            if state.pending.is_none() {
                return state.owned_release;
            }
            state.input(panel, editor, camera, size, input)
        })
    }
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
                    #[cfg(feature = "mesh-patrol")]
                    Endpoint::Patrol(_) => "Click a walkable surface for the chosen patrol waypoint.",
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
        #[cfg(feature = "mesh-patrol")]
        if self.patrol_pick_owns_input() {
            return true;
        }
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
                    #[cfg(feature = "mesh-patrol")]
                    Endpoint::Patrol(_) => "Picking patrol waypoint.".into(),
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
        #[cfg(feature = "mesh-patrol")]
        if let Endpoint::Patrol(row) = endpoint {
            if self.patrol_ticket.as_ref() != panel.patrol_pick_token(editor, row).ok().as_ref() {
                self.cancel_with_status("Patrol waypoint context changed; pick again");
                return true;
            }
        }
        #[cfg(feature = "mesh-patrol")]
        let patrol = matches!(endpoint, Endpoint::Patrol(_));
        #[cfg(not(feature = "mesh-patrol"))]
        let patrol = false;
        if !patrol {
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
                #[cfg(feature = "mesh-patrol")]
                if matches!(endpoint, Endpoint::Patrol(_)) {
                    let result = self
                        .patrol_ticket
                        .clone()
                        .ok_or("Patrol pick ticket expired".to_owned())
                        .and_then(|ticket| panel.stage_patrol_pick(editor, &ticket, hit.position));
                    self.cancel();
                    self.status = match result {
                        Ok(()) => "Patrol waypoint staged; Apply and Save remain explicit".into(),
                        Err(error) => format!("Patrol pick rejected: {error}"),
                    };
                    return true;
                }
                let target = match endpoint {
                    Endpoint::Start => &mut panel.start,
                    Endpoint::Goal => &mut panel.goal,
                    #[cfg(feature = "mesh-patrol")]
                    Endpoint::Patrol(_) => unreachable!(),
                };
                *target = hit.position.map(super::format_fixed);
                panel.draft_dirty = true;
                self.pending = None;
                self.context = None;
                self.status = match endpoint {
                    Endpoint::Start => {
                        format!(
                            "Start endpoint staged on triangle {}; Place mesh agent to apply it to the host.",
                            hit.triangle
                        )
                    }
                    Endpoint::Goal => {
                        format!(
                            "Goal endpoint staged on triangle {}; Place mesh agent to apply it to the host.",
                            hit.triangle
                        )
                    }
                    #[cfg(feature = "mesh-patrol")]
                    Endpoint::Patrol(_) => unreachable!(),
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
        #[cfg(feature = "mesh-patrol")]
        if let Some(Endpoint::Patrol(row)) = self.pending {
            if self.patrol_ticket.as_ref() != panel.patrol_pick_token(editor, row).ok().as_ref() {
                self.cancel_with_status("Patrol waypoint context changed; pick again");
            }
            return;
        }
        match self.pick_context(panel, editor, false) {
            Ok(context) if self.context.as_ref() == Some(&context) => {}
            Ok(_) => self.cancel_with_status(
                "Endpoint picking canceled because the scene or graph changed.",
            ),
            Err(reason) => self.cancel_with_status(&format!("Endpoint picking canceled: {reason}")),
        }
    }

    fn cancel(&mut self) {
        #[cfg(feature = "mesh-patrol")]
        if matches!(self.pending, Some(Endpoint::Patrol(_))) {
            self.owned_release = true;
            self.patrol_ticket = None;
        }
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

#[cfg(all(test, target_os = "linux", feature = "mesh-patrol"))]
#[path = "agent_viewport/patrol_tests.rs"]
mod patrol_tests;

#[cfg(all(test, target_os = "linux", feature = "mesh-patrol"))]
#[path = "agent_viewport/patrol_ui_tests.rs"]
mod patrol_ui_tests;
