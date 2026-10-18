use super::{geometry, geometry_panel, geometry_preview, Panel};
use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_model::StaticModel;
use orr_navigation::mesh::StaticMeshGraph;
use orr_render::{Camera3D, RenderList3D};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewInput {
    pub pixel: Option<[f32; 2]>,
    pub press_pixel: Option<[f32; 2]>,
    pub started: bool,
    pub dragging: bool,
    pub released: bool,
    pub clicked: bool,
    pub cancel: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
    tick: u64,
    checksum: u64,
}

struct DragState {
    graph: StaticMeshGraph,
    index: u32,
    drag: crate::mesh_prop_view::Drag,
    camera: Camera3D,
    viewport: (u32, u32),
    candidate_model: Option<Arc<StaticModel>>,
    final_position: Option<[FP; 3]>,
    final_valid: bool,
}

pub(crate) struct State {
    armed: bool,
    selected: Option<u32>,
    drag: Option<DragState>,
    context: Option<Context>,
    snap: String,
    status: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            armed: false,
            selected: None,
            drag: None,
            context: None,
            snap: "0.25".into(),
            status: String::new(),
        }
    }
}

impl State {
    pub(crate) fn preview_model(&self) -> Option<Arc<StaticModel>> {
        self.drag.as_ref()?.candidate_model.clone()
    }

    pub(crate) fn preview_active(&self) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|drag| drag.candidate_model.is_some())
    }

    fn busy(&self) -> bool {
        self.armed || self.drag.is_some()
    }

    fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

impl Panel {
    pub(crate) fn geometry_vertex_busy(&self) -> bool {
        self.geometry_vertex.busy()
    }

    pub(crate) fn geometry_vertex_dragging(&self) -> bool {
        self.geometry_vertex.dragging()
    }

    pub(crate) fn cancel_geometry_vertex(&mut self) {
        self.with_geometry_vertex(|state, _| {
            state.cancel("Vertex drag canceled; mesh and history are unchanged.")
        });
    }

    pub(crate) fn sync_geometry_vertex(&mut self, editor: &Editor) {
        self.with_geometry_vertex(|state, panel| state.sync(panel, editor));
    }

    pub(crate) fn show_geometry_vertex(&mut self, ui: &mut Ui, editor: &Editor) {
        self.sync_geometry_vertex(editor);
        let can_arm = self.geometry_vertex.context(self, editor).is_ok()
            && !self.geometry.dirty()
            && !self.geometry_presentation_pending;
        let mut arm = false;
        let mut finish = false;
        let mut cancel = false;
        ui.group(|ui| {
            ui.strong("Viewport vertex editing");
            if !self.geometry_vertex.busy() {
                if ui
                    .add_enabled(can_arm, egui::Button::new("Edit mesh vertices in viewport"))
                    .clicked()
                {
                    arm = true;
                }
            } else {
                ui.label("Armed; click a referenced vertex to preview a snapped drag.");
                let label = ui.label("Vertex grid snap");
                ui.add(
                    egui::TextEdit::singleline(&mut self.geometry_vertex.snap)
                        .desired_width(96.0)
                        .char_limit(16),
                )
                .labelled_by(label.id);
                if self.geometry_vertex.dragging() {
                    if ui.button("Cancel vertex drag").clicked() {
                        cancel = true;
                    }
                } else if ui.button("Finish vertex editing").clicked() {
                    finish = true;
                }
            }
            if !self.geometry_vertex.status.is_empty() {
                ui.label(&self.geometry_vertex.status);
            }
        });
        if arm {
            self.with_geometry_vertex(|state, panel| state.arm(panel, editor));
        } else if finish {
            self.with_geometry_vertex(|state, _| state.finish());
        } else if cancel {
            self.cancel_geometry_vertex();
        }
    }

    pub(crate) fn geometry_vertex_input(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        self.sync_geometry_vertex(editor);
        self.with_geometry_vertex(|state, panel| state.input(panel, editor, camera, size, input))
    }

    pub(crate) fn paint_geometry_vertices(&self, editor: &Editor, list: &mut RenderList3D) {
        if !self.geometry_vertex.armed
            || self.geometry_vertex.context(self, editor).is_err()
            || self.geometry.dirty()
            || self.geometry_presentation_pending
        {
            return;
        }
        let Some(graph) = self.graph.as_ref() else {
            return;
        };
        let Ok(faces) = geometry::faces(graph) else {
            return;
        };
        let mut referenced = std::collections::BTreeSet::new();
        for face in faces {
            referenced.extend(face);
        }
        for index in referenced {
            let Some(vertex) = graph.vertices().get(index as usize) else {
                continue;
            };
            let selected_position = self
                .geometry_vertex
                .drag
                .as_ref()
                .filter(|drag| drag.final_valid && drag.index == index)
                .and_then(|drag| drag.final_position);
            let displayed = selected_position
                .unwrap_or(*vertex)
                .map(super::geometry_vertex_view::exact_display);
            let [Ok(x_value), Ok(y_value), Ok(z_value)] = displayed else {
                continue;
            };
            let radius = if self.geometry_vertex.selected == Some(index) {
                0.12
            } else {
                0.07
            };
            let color = if self.geometry_vertex.selected == Some(index) {
                [1.0, 0.9, 0.1, 1.0]
            } else {
                [0.2, 0.95, 1.0, 0.9]
            };
            let center = [x_value, y_value, z_value];
            list.aabb(
                center.map(|coordinate| coordinate - radius),
                center.map(|coordinate| coordinate + radius),
                2.0,
                color,
            );
        }
    }

    fn with_geometry_vertex<T>(
        &mut self,
        operation: impl FnOnce(&mut State, &mut Panel) -> T,
    ) -> T {
        let mut state = std::mem::take(&mut self.geometry_vertex);
        let result = operation(&mut state, self);
        self.geometry_vertex = state;
        result
    }
}

impl State {
    fn context(&self, panel: &Panel, editor: &Editor) -> Result<Context, String> {
        if editor.game() != EditorGame::MeshNavigation
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.in_gesture()
            || editor.previewing().is_some()
            || !editor.proposals().is_empty()
            || panel.agent_pick_busy()
            || panel.props.dirty()
            || panel.props.gesture_busy()
            || panel.art.import_busy()
            || panel.art.dirty()
            || panel.geometry.dirty()
            || panel.geometry_presentation_pending
            || panel.asset_path.trim() != panel.loaded_asset_source.as_deref().unwrap_or_default()
            || panel.model_path.trim() != panel.loaded_model_source.as_deref().unwrap_or_default()
        {
            return Err("vertex editing needs an idle local editable MeshNavigation scene".into());
        }
        panel.geometry_bindings_blocker(editor)?;
        let scene = editor
            .path()
            .ok_or_else(|| "the admitted mesh scene has no local path".to_owned())?;
        let snapshot = editor
            .snapshot()
            .ok_or_else(|| "wait for a coherent mesh Frame".to_owned())?;
        let frame = snapshot.predicted();
        if editor.checksum() != frame.checksum() {
            return Err("wait for the matching current mesh document Frame".into());
        }
        let graph = panel
            .graph
            .as_ref()
            .ok_or_else(|| "create or open a mesh before editing vertices".to_owned())?;
        let revision = graph.revision();
        if panel.loaded_scene_epoch != Some(editor.scene_load_epoch())
            || panel.loaded_scene.as_ref() != Some(&scene)
        {
            return Err("the editor graph does not match the admitted scene".into());
        }
        if panel.geometry_staged {
            if panel.geometry_host_context != Some((frame.tick(), frame.checksum())) {
                return Err("staged geometry no longer matches the admitted host Frame".into());
            }
            let pin = *frame.singleton::<super::MeshScenePin>();
            if pin.is_empty() {
                if panel.loaded_graph_revision.is_some() {
                    return Err("staged geometry host mesh pin was cleared".into());
                }
            } else {
                let view = super::mesh_from_view(frame)?;
                if panel.loaded_graph_revision != Some(view.graph.revision()) {
                    return Err("the staged graph was based on a different host mesh".into());
                }
            }
        } else {
            let view = super::mesh_from_view(frame)?;
            if view.graph.revision() != revision || panel.loaded_graph_revision != Some(revision) {
                return Err("the admitted host graph differs from the editor graph".into());
            }
        }
        Ok(Context {
            scene,
            epoch: editor.scene_load_epoch(),
            graph_revision: revision,
            tick: frame.tick(),
            checksum: frame.checksum(),
        })
    }

    fn arm(&mut self, panel: &Panel, editor: &Editor) {
        match self.context(panel, editor) {
            Ok(context) => {
                self.context = Some(context);
                self.armed = true;
                self.status = "Click a referenced mesh vertex to drag it.".into();
            }
            Err(reason) => self.status = format!("Cannot edit mesh vertices: {reason}"),
        }
    }

    fn sync(&mut self, panel: &Panel, editor: &Editor) {
        if !self.busy() {
            return;
        }
        match self.context(panel, editor) {
            Ok(context) if self.context.as_ref() == Some(&context) => {}
            Ok(_) => self.cancel("Vertex editing canceled because the scene or graph changed."),
            Err(reason) => self.cancel(&format!("Vertex editing canceled: {reason}")),
        }
    }

    fn input(
        &mut self,
        panel: &mut Panel,
        editor: &Editor,
        camera: &Camera3D,
        viewport: (u32, u32),
        input: ViewInput,
    ) -> bool {
        if !self.armed {
            return false;
        }
        if input.cancel {
            self.cancel("Vertex editing canceled; mesh and history are unchanged.");
            return true;
        }
        let Ok(context) = self.context(panel, editor) else {
            self.cancel("Vertex editing canceled because its context is no longer valid.");
            return true;
        };
        if self.context.as_ref() != Some(&context) {
            self.cancel("Vertex editing canceled because the scene or graph changed.");
            return true;
        }
        if let Some(drag) = self.drag.as_ref() {
            if drag.camera != *camera || drag.viewport != viewport {
                self.cancel("Vertex drag canceled because the camera or viewport changed.");
                return true;
            }
            if input.released {
                self.update_drag(panel, editor, camera, viewport, input.pixel, true);
                return true;
            }
            if input.dragging {
                self.update_drag(panel, editor, camera, viewport, input.pixel, false);
            }
            return true;
        }
        if !input.started && !input.clicked {
            return true;
        }
        let Some(pixel) = input.press_pixel.or(input.pixel) else {
            self.status = "Vertex pick has no viewport position.".into();
            return true;
        };
        let Some(graph) = panel.graph.as_ref() else {
            self.cancel("Vertex editing canceled because the mesh graph is unavailable.");
            return true;
        };
        let picked = super::geometry_vertex_view::pick_vertex(graph, camera, pixel, viewport);
        if !input.started {
            match picked {
                Ok(Some(index)) => {
                    self.selected = Some(index);
                    self.status =
                        format!("Selected canonical vertex {index}; click and drag to move it.");
                }
                Ok(None) => {
                    self.status = "No mesh vertex within 14 pixels; host is unchanged.".into()
                }
                Err(error) => self.status = format!("Vertex pick failed: {error}"),
            }
            return true;
        }
        match picked {
            Ok(Some(index)) => {
                let vertex = graph.vertices()[index as usize];
                let position = match vertex.map(super::geometry_vertex_view::exact_display) {
                    [Ok(x_value), Ok(y_value), Ok(z_value)] => [x_value, y_value, z_value],
                    _ => {
                        self.status = "Selected vertex cannot be dragged exactly.".into();
                        return true;
                    }
                };
                let Some(drag) =
                    crate::mesh_prop_view::Drag::new(camera, pixel, viewport, position)
                else {
                    self.status = "Could not initialize a horizontal vertex drag.".into();
                    return true;
                };
                self.selected = Some(index);
                self.status = format!("Dragging canonical vertex {index}; host remains unchanged.");
                self.drag = Some(DragState {
                    graph: graph.clone(),
                    index,
                    drag,
                    camera: *camera,
                    viewport,
                    candidate_model: None,
                    final_position: None,
                    final_valid: false,
                });
                if input.released {
                    self.cancel(
                        "Vertex drag ended without movement; mesh and history are unchanged.",
                    );
                }
            }
            Ok(None) => self.status = "No mesh vertex within 14 pixels; host is unchanged.".into(),
            Err(error) => self.status = format!("Vertex pick failed: {error}"),
        }
        true
    }

    fn update_drag(
        &mut self,
        panel: &mut Panel,
        editor: &Editor,
        camera: &Camera3D,
        viewport: (u32, u32),
        pixel: Option<[f32; 2]>,
        released: bool,
    ) {
        let Some(pixel) = pixel else {
            if released {
                self.cancel(
                    "Vertex drag ended without a final pointer position; host is unchanged.",
                );
            }
            return;
        };
        let Some(active) = self.drag.as_mut() else {
            return;
        };
        let snap = super::parse_fixed(&self.snap)
            .map_err(|error| format!("invalid vertex grid snap: {error}"))
            .and_then(|value| {
                let display = value.to_f32();
                if value <= FP::ZERO
                    || value > FP::from_int(16)
                    || !display.is_finite()
                    || f64::from(display) * FP::SCALE as f64 != value.raw() as f64
                {
                    Err("vertex grid snap must be exactly representable within (0, 16]".into())
                } else {
                    Ok(display)
                }
            });
        let candidate = snap
            .and_then(|snap| {
                active
                    .drag
                    .position(camera, pixel, viewport, Some(snap))
                    .ok_or_else(|| "pointer is outside the valid vertex drag plane".to_owned())
            })
            .and_then(super::geometry_vertex_view::quantize_snapped)
            .and_then(|position| {
                geometry::move_vertex(&active.graph, active.index, position)
                    .map(|graph| (position, graph))
            });
        match candidate {
            Ok((position, graph)) => match geometry_preview::build(&graph) {
                Ok(model) => {
                    active.candidate_model = Some(model);
                    active.final_position = Some(position);
                    active.final_valid = true;
                    self.status = format!(
                        "STATUS: VERTEX DRAG PREVIEW · host unchanged · vertex {}",
                        active.index
                    );
                }
                Err(error) => {
                    active.candidate_model = None;
                    active.final_position = None;
                    active.final_valid = false;
                    self.status = format!("Invalid vertex preview: {error}");
                }
            },
            Err(error) => {
                active.candidate_model = None;
                active.final_position = None;
                active.final_valid = false;
                self.status = format!("Invalid vertex drag preview: {error}");
            }
        }
        if released {
            let Some(active) = self.drag.take() else {
                return;
            };
            if !active.final_valid {
                self.status = "Invalid final vertex position; no mesh change was staged.".into();
                return;
            }
            let Some(position) = active.final_position else {
                self.status =
                    "Vertex preview lost its canonical vertex; no change was staged.".into();
                return;
            };
            let staged = panel.stage_geometry(
                geometry_panel::Action::MoveVertex {
                    index: active.index,
                    position,
                },
                editor,
            );
            let succeeded = staged.is_ok();
            self.status = match staged {
                Ok(()) => format!(
                    "Moved vertex {}; one local geometry history entry staged.",
                    active.index
                ),
                Err(error) => format!("Vertex move was not staged: {error}"),
            };
            if succeeded {
                self.selected = panel
                    .graph
                    .as_ref()
                    .and_then(|graph| {
                        graph
                            .vertices()
                            .iter()
                            .position(|vertex| *vertex == position)
                    })
                    .and_then(|index| u32::try_from(index).ok());
                self.context = self.context(panel, editor).ok();
            } else {
                self.drag = None;
                self.armed = false;
                self.context = None;
            }
        }
    }

    fn cancel(&mut self, status: &str) {
        self.drag = None;
        self.armed = false;
        self.context = None;
        self.status = status.into();
    }

    fn finish(&mut self) {
        self.drag = None;
        self.armed = false;
        self.context = None;
        self.status = "Vertex editing finished; staged mesh and host are unchanged.".into();
    }
}

#[cfg(test)]
#[path = "geometry_vertex_viewport/tests.rs"]
mod tests;
