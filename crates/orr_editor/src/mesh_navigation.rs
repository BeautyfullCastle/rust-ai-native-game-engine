//! Scene-owned authoring controls for the optional static-mesh navigation host.
use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::{fp, FP};
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, SearchBudget};
use orr_remote::mesh_navigation::{
    decode_asset, mesh_from_view, AgentSpec, MeshScenePin, PIN_NAME,
};
use serde_json::json;
use std::{fs, fs::OpenOptions, io::Read, path::PathBuf, sync::Arc};

#[path = "mesh_model.rs"]
mod mesh_model;
#[path = "mesh_navigation/agent_viewport.rs"]
mod agent_viewport;
#[path = "mesh_navigation/geometry.rs"]
mod geometry;
#[path = "mesh_navigation/geometry_panel.rs"]
mod geometry_panel;
#[path = "mesh_navigation/geometry_preview.rs"]
mod geometry_preview;
#[path = "mesh_navigation/geometry_history.rs"]
mod geometry_history;
#[path = "mesh_navigation/geometry_vertex_view.rs"]
mod geometry_vertex_view;
#[path = "mesh_navigation/geometry_vertex_viewport.rs"]
mod geometry_vertex_viewport;
#[cfg(feature = "mesh-mission")]
#[path = "mesh_navigation/mission_view.rs"]
mod mission_view;
#[cfg(feature = "mesh-mission")]
#[path = "mesh_navigation/mission_geometry.rs"]
mod mission_geometry;
pub(crate) use geometry_vertex_viewport::ViewInput as GeometryVertexViewInput;
pub(crate) use agent_viewport::ViewInput as AgentViewInput;

const MAX_ASSET_BYTES: usize = 8192;
const MAX_ASSET_PATH_BYTES: usize = 4096;
const MAX_VIEW_COORDINATE_RAW: i64 = 256 * 65_536;
const VIEW_ASSET_ID: &str = "mesh/navigation/view.orrmodel";

struct GeometryOrigin {
    dirty: bool,
    asset_path: String,
    model_path: String,
    asset_source: Option<String>,
    model_source: Option<String>,
    settings: [String; 9],
}

pub struct Panel {
    pub start: [String; 3],
    pub goal: [String; 3],
    pub max_slope: String,
    pub max_vertical: String,
    pub distance_per_tick: String,
    pub asset_path: String,
    pub model_path: String,
    pub mesh_output_path: String,
    pub error: Option<String>,
    loaded_scene: Option<PathBuf>,
    loaded_scene_epoch: Option<u64>,
    loaded_graph_revision: Option<[u8; 32]>,
    loaded_spec: Option<AgentSpec>,
    loaded_asset_source: Option<String>,
    loaded_model_source: Option<String>,
    draft_dirty: bool,
    last_frame: Option<(u64, u64)>,
    host_status: Option<orr_navigation::NavigationStatus>,
    host_position: Option<[FP; 3]>,
    host_triangle_count: usize,
    graph: Option<StaticMeshGraph>,
    view_model: Option<Arc<StaticModel>>,
    art: crate::mesh_art_panel::Panel,
    props: crate::mesh_props_panel::Panel,
    agent_viewport: agent_viewport::State,
    geometry: geometry_panel::Panel,
    geometry_staged: bool,
    geometry_host_context: Option<(u64, u64)>,
    geometry_presentation_pending: bool,
    geometry_history: geometry_history::History,
    geometry_origin: Option<GeometryOrigin>,
    geometry_preview: Option<Arc<StaticModel>>,
    geometry_preview_visible: bool,
    geometry_vertex: geometry_vertex_viewport::State,
    #[cfg(feature = "mesh-mission")]
    mission_bound: bool,
    #[cfg(feature = "mesh-mission")]
    mission_geometry: mission_geometry::State,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            start: ["1".into(), "0".into(), "1".into()],
            goal: ["3".into(), "2".into(), "1".into()],
            max_slope: "4".into(),
            max_vertical: "2".into(),
            distance_per_tick: "0.03125".into(),
            asset_path: String::new(),
            model_path: String::new(),
            mesh_output_path: String::new(),
            error: None,
            loaded_scene: None,
            loaded_scene_epoch: None,
            loaded_graph_revision: None,
            loaded_spec: None,
            loaded_asset_source: None,
            loaded_model_source: None,
            draft_dirty: false,
            last_frame: None,
            host_status: None,
            host_position: None,
            host_triangle_count: 0,
            graph: None,
            view_model: None,
            art: crate::mesh_art_panel::Panel::default(),
            props: crate::mesh_props_panel::Panel::default(),
            agent_viewport: agent_viewport::State::default(),
            geometry: geometry_panel::Panel::default(),
            geometry_staged: false,
            geometry_host_context: None,
            geometry_presentation_pending: false,
            geometry_history: geometry_history::History::default(),
            geometry_origin: None,
            geometry_preview: None,
            geometry_preview_visible: false,
            geometry_vertex: geometry_vertex_viewport::State::default(),
            #[cfg(feature = "mesh-mission")]
            mission_bound: false,
            #[cfg(feature = "mesh-mission")]
            mission_geometry: mission_geometry::State::default(),
        }
    }
}

impl Panel {
    pub fn sync(&mut self, editor: &Editor) {
        self.sync_geometry_vertex(editor);
        self.geometry_preview_visible = self.geometry_staged
            && editor.game().is_mesh_navigation()
            && editor.mode() == Mode::Edit
            && self.loaded_scene_epoch == Some(editor.scene_load_epoch())
            && editor.snapshot().is_some_and(|snapshot| {
                let frame = snapshot.predicted();
                self.geometry_host_context == Some((frame.tick(), frame.checksum()))
            });
        self.sync_agent_picking(editor);
        self.art.sync(editor);
        self.props.sync(editor);
        if editor.game() != EditorGame::MeshNavigation {
            self.clear_host_view();
            return;
        }
        let scene = editor.path();
        let scene_epoch = editor.scene_load_epoch();
        let Some(snapshot) = editor.snapshot() else {
            self.clear_host_view();
            return;
        };
        let frame = snapshot.predicted();
        #[cfg(feature = "mesh-mission")]
        {
            self.mission_bound = !frame
                .singleton::<orr_remote::mesh_navigation::mission::MeshMission>()
                .disabled();
        }
        let frame_identity = (frame.tick(), frame.checksum());
        let new_document = self.loaded_scene_epoch != Some(scene_epoch);
        let scene_path_changed = self.loaded_scene != scene;
        if !new_document && scene_path_changed {
            self.loaded_scene = scene.clone();
        }
        if !new_document && !scene_path_changed && self.last_frame == Some(frame_identity) {
            return;
        }
        self.last_frame = Some(frame_identity);
        let pin = *frame.singleton::<MeshScenePin>();
        if new_document {
            self.clear_geometry_history();
            self.loaded_scene = scene;
            self.loaded_scene_epoch = Some(scene_epoch);
            self.loaded_graph_revision = None;
            self.loaded_spec = None;
            self.start = ["1".into(), "0".into(), "1".into()];
            self.goal = ["3".into(), "2".into(), "1".into()];
            self.max_slope = "4".into();
            self.max_vertical = "2".into();
            self.distance_per_tick = "0.03125".into();
            self.asset_path.clear();
            self.model_path.clear();
            self.loaded_asset_source = None;
            self.loaded_model_source = None;
            self.draft_dirty = false;
            self.graph = None;
            self.view_model = None;
            self.host_status = None;
            self.host_position = None;
            self.host_triangle_count = 0;
            self.error = None;
        }
        if pin.is_empty() {
            if self.loaded_graph_revision.is_some() {
                if self.draft_dirty || self.geometry.dirty() {
                    self.loaded_graph_revision = None;
                    self.loaded_spec = None;
                } else {
                    self.reset_to_empty_pin();
                }
            }
            self.view_model = None;
            self.host_status = None;
            self.host_position = None;
            self.host_triangle_count = 0;
            return;
        }
        match mesh_from_view(frame) {
            Ok(view) => {
                let graph_revision = view.graph.revision();
                let spec = view.agent.spec;
                self.host_status = Some(view.agent.status);
                self.host_position = Some(view.agent.position);
                self.host_triangle_count = view.graph.triangles().len();
                if self.loaded_graph_revision != Some(graph_revision)
                    || self.loaded_spec != Some(spec)
                {
                    self.loaded_graph_revision = Some(graph_revision);
                    self.loaded_spec = Some(spec);
                    if !self.draft_dirty {
                        self.start = spec.start.map(format_fixed);
                        self.goal = spec.goal.map(format_fixed);
                        self.max_slope = format_fixed(view.graph.profile().max_slope);
                        self.max_vertical = format_fixed(spec.max_vertical);
                        self.distance_per_tick = format_fixed(spec.distance_per_tick);
                        self.asset_path.clear();
                        self.model_path.clear();
                        self.loaded_asset_source = None;
                        self.loaded_model_source = None;
                        self.graph = Some(view.graph.clone());
                        self.error = None;
                    }
                }
                #[cfg(feature = "mesh-mission")]
                let model = orr_remote::mesh_navigation::mission::from_view(frame)
                    .and_then(|mission| mission_view::build(&view, mission.as_ref()));
                #[cfg(not(feature = "mesh-mission"))]
                let model = build_view_model(&view.graph, &view.agent.path, view.agent.position);
                self.view_model = model
                        .map(Arc::new)
                        .map_err(|error| self.error = Some(error))
                        .ok();
            }
            Err(error) => {
                self.clear_host_view();
                self.loaded_graph_revision = None;
                self.loaded_spec = None;
                self.loaded_asset_source = None;
                self.loaded_model_source = None;
                if !self.draft_dirty {
                    self.graph = None;
                }
                self.error = Some(error);
            }
        }
    }

    pub fn observe(&mut self, editor: &Editor) {
        self.sync(editor);
    }

    pub fn model(&self) -> Option<Arc<StaticModel>> {
        if let Some(model) = self.geometry_vertex.preview_model() { return Some(model); }
        if self.geometry_preview_active() {
            return self.geometry_preview.clone();
        }
        self.view_model.clone()
    }

    pub(crate) fn geometry_preview_active(&self) -> bool {
        self.geometry_vertex.preview_active() || (self.geometry_preview_visible && self.geometry_preview.is_some())
    }

    pub(crate) fn set_geometry_presentation_pending(&mut self, pending: bool) {
        self.geometry_presentation_pending = pending;
    }

    pub(crate) fn has_unplaced_draft(&self) -> bool {
        self.geometry_vertex_busy() || self.draft_dirty || self.geometry.dirty() || self.art_dirty()
    }

    pub(crate) fn art_model(&self, editor: &Editor) -> Option<crate::viewport3d::SceneModel> {
        self.art.placement(editor)
    }

    pub(crate) fn art_dirty(&self) -> bool {
        self.art.dirty() || self.props.dirty()
    }

    pub(crate) fn prop_models(&self, editor: &Editor) -> Vec<crate::viewport3d::SceneModel> {
        self.props.placements(editor)
    }

    pub(crate) fn prop_viewport(
        &mut self,
        editor: &Editor,
        camera: &orr_render::Camera3D,
        size: (u32, u32),
        input: crate::mesh_props_panel::ViewInput,
    ) -> bool {
        self.props.viewport(editor, camera, size, input)
    }

    pub(crate) fn selected_prop_bounds(
        &self,
        editor: &Editor,
    ) -> Option<crate::mesh_prop_view::Bounds> {
        self.props.selected_bounds(editor)
    }

    pub(crate) fn prop_gesture_busy(&self) -> bool {
        self.props.gesture_busy()
    }

    pub(crate) fn cancel_prop_gesture(&mut self) {
        self.props.cancel_viewport_gesture();
    }

    pub(crate) fn art_import_busy(&self) -> bool {
        self.art.import_busy()
    }

    pub fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if self.geometry_vertex_busy() { return Some("Finish or cancel viewport vertex editing before Play".into()); }
        if self.agent_pick_busy() {
            return Some("Finish or cancel viewport endpoint picking before Play".into());
        }
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.geometry.dirty() {
            return Some("Stage or reset mesh geometry fields before Play".into());
        }
        if let Some(reason) = self.art.play_blocker(editor) {
            return Some(reason);
        }
        if let Some(reason) = self.props.play_blocker(editor) {
            return Some(reason);
        }
        if !editor.proposals().is_empty() {
            return Some("Resolve open proposals before Play".into());
        }
        if self.loaded_graph_revision.is_none() || self.loaded_spec.is_none() {
            return Some("Place a mesh agent before Play".into());
        }
        if self.draft_is_stale() {
            return Some(
                "Mesh or agent settings changed; place the mesh agent again before Play".into(),
            );
        }
        if self.host_status.is_none() || self.view_model.is_none() {
            return Some("Wait for an admitted static-mesh Frame".into());
        }
        None
    }

    pub fn create_ramp_and_bridge(&mut self) -> Result<(), String> {
        self.geometry_replacement_blocker()?;
        let max_slope = parse_fixed(&self.max_slope)?;
        if max_slope.raw() < 0 {
            return Err("Maximum slope must be nonnegative".into());
        }
        let graph = fixture_graph(max_slope)?;
        let spec = self.parse_spec()?;
        validate_route(&graph, spec)?;
        validate_display_bounds(&graph, &spec)?;
        self.clear_geometry_history();
        self.graph = Some(graph);
        self.draft_dirty = true;
        self.asset_path.clear();
        self.model_path.clear();
        self.loaded_asset_source = None;
        self.loaded_model_source = None;
        self.error = None;
        Ok(())
    }

    pub fn open_mesh_asset(&mut self) -> Result<(), String> {
        self.geometry_replacement_blocker()?;
        if self.asset_path.len() > MAX_ASSET_PATH_BYTES {
            return Err("Mesh asset path exceeds the 4096-byte limit".into());
        }
        let path = PathBuf::from(self.asset_path.trim());
        if !path.is_absolute() {
            return Err("Mesh asset path must be absolute".into());
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("orrmesh") {
            return Err("Mesh asset must use the .orrmesh extension".into());
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Mesh asset must be a regular non-symlink file".into());
        }
        if metadata.len() > MAX_ASSET_BYTES as u64 {
            return Err("Mesh asset exceeds the 8192-byte limit".into());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(MAX_ASSET_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err("Mesh asset exceeds the 8192-byte limit".into());
        }
        let graph = decode_asset(&bytes).map_err(|error| error.to_string())?;
        let spec = self.parse_spec()?;
        validate_route(&graph, spec)?;
        validate_display_bounds(&graph, &spec)?;
        self.max_slope = format_fixed(graph.profile().max_slope);
        self.clear_geometry_history();
        self.graph = Some(graph);
        self.draft_dirty = true;
        self.model_path.clear();
        self.loaded_model_source = None;
        self.asset_path = path.display().to_string();
        self.loaded_asset_source = Some(self.asset_path.clone());
        self.error = None;
        Ok(())
    }

    pub fn open_model(&mut self) -> Result<(), String> {
        self.geometry_replacement_blocker()?;
        if self.model_path.len() > MAX_ASSET_PATH_BYTES {
            return Err("Model path exceeds the 4096-byte limit".into());
        }
        let path = PathBuf::from(self.model_path.trim());
        if !path.is_absolute() {
            return Err("Model path must be absolute".into());
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or("Model asset must use .glb, .gltf, or .orrmodel")?;
        if !matches!(extension.as_str(), "glb" | "gltf" | "orrmodel") {
            return Err("Model asset must use .glb, .gltf, or .orrmodel".into());
        }
        let max_slope = parse_fixed(&self.max_slope)?;
        if max_slope.raw() < 0 {
            return Err("Maximum slope must be nonnegative".into());
        }
        let profile = AgentProfile {
            max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        };
        let graph = mesh_model::import_model(&path, profile)?;
        let spec = self.parse_spec()?;
        validate_route(&graph, spec)?;
        validate_display_bounds(&graph, &spec)?;
        self.clear_geometry_history();
        self.graph = Some(graph);
        self.draft_dirty = true;
        self.model_path = path.display().to_string();
        self.loaded_model_source = Some(self.model_path.clone());
        self.asset_path.clear();
        self.loaded_asset_source = None;
        self.error = None;
        Ok(())
    }

    pub fn save_mesh_asset(&mut self) -> Result<(), String> {
        if self.mesh_output_path.len() > MAX_ASSET_PATH_BYTES {
            return Err("Mesh output path exceeds the 4096-byte limit".into());
        }
        let path = PathBuf::from(self.mesh_output_path.trim());
        if !path.is_absolute() {
            return Err("Mesh output path must be absolute".into());
        }
        let graph = self.graph.as_ref().ok_or("Create or open a mesh first")?;
        mesh_model::save_graph(&path, graph)?;
        self.mesh_output_path = path.display().to_string();
        self.error = None;
        Ok(())
    }

    pub fn place_agent(&mut self, editor: &mut Editor) -> Result<(), String> {
        let result = (|| {
            if self.geometry_vertex_busy() { return Err("Finish or cancel viewport vertex editing before placing an agent".into()); }
            if self.geometry.dirty() {
                return Err("Stage or reset mesh geometry fields before placing a mesh agent".into());
            }
            if self.geometry_staged {
                self.geometry_bindings_blocker(editor, false)?;
            }
            if self.agent_pick_busy() || self.props.gesture_busy() {
                return Err("Finish or cancel active viewport picking or prop movement before placing an agent".into());
            }
            if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
                return Err("Mesh authoring requires a local MeshNavigation host".into());
            }
            if editor.mode() != Mode::Edit || !editor.can_mutate() || editor.previewing().is_some()
            {
                return Err("Stop Play and clear previews before placing a mesh agent".into());
            }
            if editor.snapshot().is_none() {
                return Err("Wait for a mesh host Frame before placing an agent".into());
            }
            if !editor.proposals().is_empty() {
                return Err("Resolve open proposals before placing a mesh agent".into());
            }
            let graph = self
                .graph
                .as_ref()
                .ok_or("Create or open a mesh asset first")?;
            let spec = self.parse_spec()?;
            if graph.profile().max_slope != parse_fixed(&self.max_slope)? {
                return Err(
                    "Maximum slope differs from the open graph; create or reopen the mesh".into(),
                );
            }
            validate_route(graph, spec)?;
            validate_display_bounds(graph, &spec)?;
            #[cfg(feature = "mesh-mission")]
            if let Some(snapshot) = editor.snapshot() {
                if let Some(mission) = orr_remote::mesh_navigation::mission::from_view(snapshot.predicted())? {
                    if mission.config.graph_revision != graph.revision() {
                        return Err("Remove the mesh mission before replacing its graph; mission points are not silently rebound".into());
                    }
                }
            }
            let pin = MeshScenePin::new(graph, spec)?;
            editor.host_call("world.singleton.patch", pin_patch(&pin))?;
            editor.mesh_navigation_mark_edited();
            editor.sync();
            if !editor.has_mesh_camera() {
                editor.fit_camera();
            }
            self.draft_dirty = false;
            self.agent_viewport = agent_viewport::State::default();
            self.clear_geometry_history();
            self.error = None;
            self.sync(editor);
            Ok(())
        })();
        self.error = result.as_ref().err().cloned();
        result
    }

    pub fn show(&mut self, ui: &mut Ui, editor: &mut Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Static-mesh navigation", |ui| {
            self.show_geometry_vertex(ui, editor);
            let vertex_busy = self.geometry_vertex_busy();
            ui.add_enabled_ui(!vertex_busy, |ui| {
            ui.weak(
                "Point agent · canonical open-sky triangles · no physics or collision controller",
            );
            ui.horizontal(|ui| {
                if ui.button("Create ramp and bridge").clicked() {
                    if let Err(error) = self.create_ramp_and_bridge() {
                        self.error = Some(error);
                    }
                }
                if ui
                    .add_enabled(
                        (self.draft_dirty || self.geometry.dirty()) && editor.mode() == Mode::Edit,
                        egui::Button::new("Discard unplaced changes"),
                    )
                    .clicked()
                {
                    let art = std::mem::take(&mut self.art);
                    let props = std::mem::take(&mut self.props);
                    *self = Self {
                        art,
                        props,
                        ..Self::default()
                    };
                    self.sync(editor);
                }
            });
            let geometry_blocker = self.geometry_blocker(editor);
            if let Some(reason) = &geometry_blocker {
                ui.weak(reason);
            }
            let geometry_action = self.geometry_profile().ok().and_then(|profile| {
                ui.add_enabled_ui(geometry_blocker.is_none(), |ui| {
                    self.geometry.show(ui, self.graph.as_ref(), profile)
                }).inner
            });
            if let Some(action) = geometry_action {
                if let Err(error) = self.stage_geometry(action, editor) {
                    self.error = Some(error);
                }
            }
            if self.geometry_staged {
                if self.geometry_preview_active() {
                    ui.strong("PENDING GEOMETRY PREVIEW · host unchanged");
                    ui.weak("Surface-only preview; Place mesh agent commits geometry and the agent together.");
                } else {
                    ui.colored_label(egui::Color32::YELLOW, "STAGED GEOMETRY PREVIEW UNAVAILABLE · host changed or Edit mode required");
                    ui.weak("The local draft is preserved. Discard unplaced changes before editing the newer host.");
                }
            }
            let history_ready = geometry_blocker.is_none() && !self.geometry.dirty();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(history_ready && self.geometry_history.can_undo(), egui::Button::new("Undo staged geometry")).clicked() {
                    if let Err(error) = self.change_geometry_history(editor, false) { self.error = Some(error); }
                }
                if ui.add_enabled(history_ready && self.geometry_history.can_redo(), egui::Button::new("Redo staged geometry")).clicked() {
                    if let Err(error) = self.change_geometry_history(editor, true) { self.error = Some(error); }
                }
                if ui.add_enabled(geometry_blocker.is_none() && self.geometry_staged, egui::Button::new("Frame staged geometry")).clicked() {
                    if let Err(error) = self.frame_staged_geometry(editor) { self.error = Some(error); }
                }
            });
            let asset_label = ui.label("Mesh asset (.orrmesh)");
            ui.horizontal(|ui| {
                let input_width = (ui.available_width() - 100.0).max(40.0);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.asset_path).desired_width(input_width),
                    )
                    .labelled_by(asset_label.id)
                    .changed()
                {
                    self.draft_dirty = true;
                }
                if ui.button("Open mesh asset").clicked() {
                    if let Err(error) = self.open_mesh_asset() {
                        self.error = Some(error);
                    }
                }
            });
            let model_label = ui.label("Model source (.glb, .gltf, .orrmodel)");
            ui.horizontal(|ui| {
                let input_width = (ui.available_width() - 100.0).max(40.0);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.model_path).desired_width(input_width),
                    )
                    .labelled_by(model_label.id)
                    .changed()
                {
                    self.draft_dirty = true;
                }
                if ui.button("Open model").clicked() {
                    if let Err(error) = self.open_model() {
                        self.error = Some(error);
                    }
                }
            });
            let output_label = ui.label("Save mesh asset (.orrmesh)");
            ui.horizontal(|ui| {
                let input_width = (ui.available_width() - 100.0).max(40.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.mesh_output_path)
                        .desired_width(input_width),
                )
                .labelled_by(output_label.id);
                if ui.button("Save mesh asset").clicked() {
                    if let Err(error) = self.save_mesh_asset() {
                        self.error = Some(error);
                    }
                }
            });
            let mut coordinate_changed = false;
            for (label, values) in [("Start", &mut self.start), ("Goal", &mut self.goal)] {
                for (axis_label, value) in ["X", "Y", "Z"].into_iter().zip(values) {
                    coordinate_changed |= field(ui, &format!("{label} {axis_label}"), value);
                }
            }
            self.draft_dirty |= coordinate_changed;
            self.draft_dirty |= field(ui, "Maximum slope", &mut self.max_slope);
            self.draft_dirty |= field(ui, "Maximum vertical projection", &mut self.max_vertical);
            self.draft_dirty |= field(ui, "Distance per tick", &mut self.distance_per_tick);
            self.show_agent_picking(ui, editor);
            if let Some(reason) = self.authoring_blocker(editor) {
                ui.label(reason);
            }
            if ui
                .add_enabled(
                    self.authoring_blocker(editor).is_none(),
                    egui::Button::new("Place mesh agent"),
                )
                .clicked()
            {
                let _ = self.place_agent(editor);
            }
            if let (Some(status), Some(position)) = (self.host_status, self.host_position) {
                ui.label(format!(
                    "Frame mesh · {} triangles · {status:?}",
                    self.host_triangle_count
                ));
                ui.label(format!(
                    "Position [{}, {}, {}]",
                    format_fixed(position[0]),
                    format_fixed(position[1]),
                    format_fixed(position[2])
                ));
            }
            if let Some(error) = self.error.as_ref() {
                ui.colored_label(egui::Color32::RED, error);
            }
            });
        });
        ui.add_enabled_ui(!self.geometry_vertex_busy(), |ui| {
            self.art.show(ui, editor);
            let agent_pick_busy = self.agent_pick_busy();
            self.props.show(ui, editor, agent_pick_busy);
        });
        #[cfg(feature = "mesh-mission")]
        self.show_mission_geometry(ui, editor);
    }

    #[cfg(feature = "mesh-mission")]
    fn show_mission_geometry(&mut self, ui: &mut Ui, editor: &mut Editor) {
        if !self.mission_bound && !self.mission_geometry.is_enabled() { return; }
        let context = mission_geometry::Context::capture(self, editor);
        if !self.mission_geometry.show(ui, context) { return; }
        let result = mission_geometry::Context::capture(self, editor)
            .and_then(|context| self.mission_geometry.apply(editor, context));
        match result {
            Ok(applied) => {
                if let Some(status) = self.mission_geometry.status() { editor.info(status); }
                self.graph = Some(applied.graph);
                self.loaded_graph_revision = self.graph.as_ref().map(StaticMeshGraph::revision);
                self.loaded_spec = Some(applied.spec);
                self.start = applied.spec.start.map(format_fixed);
                self.goal = applied.spec.goal.map(format_fixed);
                self.max_vertical = format_fixed(applied.spec.max_vertical);
                self.distance_per_tick = format_fixed(applied.spec.distance_per_tick);
                if let Some(graph) = self.graph.as_ref() { self.max_slope = format_fixed(graph.profile().max_slope); }
                self.draft_dirty = false;
                self.geometry.acknowledge();
                self.clear_geometry_history();
                self.loaded_asset_source = None;
                self.loaded_model_source = None;
                self.error = None;
                self.sync(editor);
            }
            Err(error) => self.error = Some(format!("Apply mission-preserving geometry failed: {error}")),
        }
    }

    fn mission_reprojection_enabled(&self) -> bool {
        #[cfg(feature = "mesh-mission")]
        { self.mission_bound && self.mission_geometry.is_enabled() }
        #[cfg(not(feature = "mesh-mission"))]
        { false }
    }

    fn authoring_blocker(&self, editor: &Editor) -> Option<String> {
        if self.geometry_vertex_busy() { return Some("Finish or cancel viewport vertex editing before placing an agent".into()); }
        if self.geometry.dirty() {
            return Some("Stage or reset mesh geometry fields before placing a mesh agent".into());
        }
        if self.agent_pick_busy() || self.props.gesture_busy() {
            return Some("Finish or cancel active viewport picking or prop movement before placing an agent".into());
        }
        if !editor.spec().is_local() || !editor.can_mutate() {
            return Some("Mesh placement requires a writable local host".into());
        }
        if editor.mode() != Mode::Edit || editor.previewing().is_some() {
            return Some("Stop Play and clear previews before placing a mesh agent".into());
        }
        if editor.snapshot().is_none() {
            return Some("Wait for a mesh host Frame before placing an agent".into());
        }
        if !editor.proposals().is_empty() {
            return Some("Resolve open proposals before placing a mesh agent".into());
        }
        let Some(graph) = self.graph.as_ref() else {
            return Some("Create or open a mesh asset first".into());
        };
        if self.asset_path.trim() != self.loaded_asset_source.as_deref().unwrap_or_default() {
            return Some("Open the selected mesh asset before placing its agent".into());
        }
        if self.model_path.trim() != self.loaded_model_source.as_deref().unwrap_or_default() {
            return Some("Open the selected model before placing its agent".into());
        }
        self.parse_spec()
            .and_then(|spec| {
                if graph.profile().max_slope != parse_fixed(&self.max_slope)? {
                    return Err(
                        "Maximum slope differs from the open graph; create or reopen the mesh"
                            .into(),
                    );
                }
                validate_route(graph, spec)?;
                validate_display_bounds(graph, &spec)
            })
            .err()
    }

    fn geometry_blocker(&self, editor: &Editor) -> Option<String> {
        if self.geometry_vertex_busy() { return Some("Finish viewport vertex editing before using geometry forms".into()); }
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() || !editor.can_mutate() {
            return Some("Mesh geometry requires a writable local MeshNavigation host".into());
        }
        if editor.mode() != Mode::Edit || editor.previewing().is_some() {
            return Some("Stop Play and clear previews before editing mesh geometry".into());
        }
        if self.agent_pick_busy() || self.props.gesture_busy() {
            return Some("Finish or cancel active viewport picking or prop movement before editing geometry".into());
        }
        if self.geometry_presentation_pending || self.art_import_busy() || self.art_dirty() {
            return Some("Finish importing and save or discard presentation edits before geometry".into());
        }
        if editor.snapshot().is_none() || self.loaded_scene_epoch != Some(editor.scene_load_epoch()) || self.loaded_scene != editor.path() {
            return Some("Wait for the current mesh scene Frame before editing geometry".into());
        }
        if !editor.proposals().is_empty() {
            return Some("Resolve open proposals before editing mesh geometry".into());
        }
        self.geometry_profile().err()
    }

    fn geometry_profile(&self) -> Result<AgentProfile, String> {
        let max_slope = parse_fixed(&self.max_slope)?;
        if max_slope < FP::ZERO || max_slope > FP::from_int(4) {
            return Err("Maximum slope must be within 0..=4 for mesh geometry".into());
        }
        Ok(AgentProfile { max_slope, radius: FP::ZERO, headroom: FP::ZERO, max_step: FP::ZERO })
    }

    fn geometry_bindings_blocker(&self, editor: &Editor, allow_mission_reprojection: bool) -> Result<(), String> {
        if allow_mission_reprojection && !self.mission_reprojection_enabled() {
            return Err("Explicitly enable mission-preserving geometry before staging changes to an active mission".into());
        }
        if self.geometry_presentation_pending {
            return Err("Save or discard camera, lighting and animation edits before geometry; save or discard HUD changes too".into());
        }
        #[cfg(feature = "mesh-mission")]
        if let Some(snapshot) = editor.snapshot() {
            if orr_remote::mesh_navigation::mission::from_view(snapshot.predicted())?.is_some() && !allow_mission_reprojection {
                return Err("Remove the mesh mission before changing its graph; mission points are not silently rebound".into());
            }
        }
        if self.geometry_host_context.is_some() {
            let snapshot = editor.snapshot().ok_or("Wait for the staged geometry host Frame")?;
            let frame = snapshot.predicted();
            if self.geometry_host_context != Some((frame.tick(), frame.checksum())) {
                return Err("Host mesh changed while geometry was staged; discard the unplaced changes before retrying".into());
            }
        }
        let scene = editor.path().ok_or("Save the mesh scene before editing geometry")?;
        if crate::mesh_lighting_store::has_sidecar(&scene)? || crate::mesh_hud_store::has_sidecar(&scene)? {
            return Err("Remove and save existing scene-bound art, props, camera, lighting and animation before geometry edits; remove and save HUD too; graph revisions are not silently rebound".into());
        }
        for path in [
            crate::mesh_art::sidecar_path(&scene)?,
            crate::mesh_props::sidecar_path(&scene)?,
            crate::mesh_camera_store::sidecar_path(&scene)?,
            crate::mesh_animation_store::sidecar_path(&scene)?,
        ] {
            match fs::symlink_metadata(&path) {
                Ok(_) => return Err("Remove and save existing scene-bound art, props, camera, lighting and animation before geometry edits; remove and save HUD too; graph revisions are not silently rebound".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(format!("Inspect mesh geometry presentation binding: {error}")),
            }
        }
        Ok(())
    }

    fn stage_geometry(&mut self, action: geometry_panel::Action, editor: &Editor) -> Result<(), String> {
        if let Some(reason) = self.geometry_blocker(editor) {
            return Err(reason);
        }
        self.geometry_bindings_blocker(editor, self.mission_reprojection_enabled())?;
        let profile = self.geometry_profile()?;
        if self.graph.as_ref().is_some_and(|graph| graph.profile() != profile) {
            return Err("Maximum slope differs from the open graph; create or reopen the mesh".into());
        }
        let graph = match action {
            geometry_panel::Action::AppendQuad(quad) => geometry::append_quad(self.graph.as_ref(), profile, quad)?,
            geometry_panel::Action::MoveVertex { index, position } => geometry::move_vertex(self.graph.as_ref().ok_or("Create or open a mesh first")?, index, position)?,
            geometry_panel::Action::RemoveFace(index) => geometry::remove_face(self.graph.as_ref().ok_or("Create or open a mesh first")?, index)?,
        };
        if graph.to_bytes().len() > MAX_ASSET_BYTES {
            return Err("Mesh geometry exceeds the 8192-byte scene limit".into());
        }
        for point in graph.vertices() {
            point_to_f32(*point)?;
        }
        if self.graph.as_ref().is_some_and(|current| current.revision() == graph.revision()) {
            return Err("Mesh geometry operation did not change the graph".into());
        }
        let snapshot = editor.snapshot().ok_or("Wait for the mesh geometry host Frame")?;
        let frame = snapshot.predicted();
        let host_context = (frame.tick(), frame.checksum());
        let preview = geometry_preview::build(&graph)?;
        let mut history = self.geometry_history.clone();
        let first_edit = history.current().is_none();
        if first_edit { history.reset(self.graph.as_ref())?; }
        if !history.push(Some(&graph))? { return Err("Mesh geometry operation did not change the graph".into()); }
        let origin = first_edit.then(|| GeometryOrigin {
            dirty: self.draft_dirty,
            asset_path: self.asset_path.clone(),
            model_path: self.model_path.clone(),
            asset_source: self.loaded_asset_source.clone(),
            model_source: self.loaded_model_source.clone(),
            settings: self.geometry_settings(),
        });
        self.cancel_agent_pick();
        if let Some(origin) = origin { self.geometry_origin = Some(origin); }
        self.geometry_history = history;
        self.geometry_preview = Some(preview);
        self.geometry_preview_visible = true;
        self.graph = Some(graph);
        self.draft_dirty = true;
        self.geometry_staged = true;
        self.geometry_host_context = Some(host_context);
        self.asset_path.clear();
        self.model_path.clear();
        self.loaded_asset_source = None;
        self.loaded_model_source = None;
        self.geometry.acknowledge();
        self.error = None;
        Ok(())
    }

    fn geometry_settings(&self) -> [String; 9] {
        [self.start[0].clone(), self.start[1].clone(), self.start[2].clone(),
         self.goal[0].clone(), self.goal[1].clone(), self.goal[2].clone(),
         self.max_slope.clone(), self.max_vertical.clone(), self.distance_per_tick.clone()]
    }

    fn geometry_replacement_blocker(&self) -> Result<(), String> {
        #[cfg(feature = "mesh-mission")]
        if self.mission_bound {
            return Err("Remove the mesh mission before replacing its graph; mission points are not silently rebound".into());
        }
        if self.geometry_vertex_busy() { return Err("Finish or cancel viewport vertex editing before replacing the mesh source".into()); }
        if self.geometry_staged || self.geometry.dirty() {
            return Err("Discard unplaced geometry changes before creating or opening another mesh source".into());
        }
        Ok(())
    }

    fn clear_geometry_history(&mut self) {
        #[cfg(feature = "mesh-mission")]
        { self.mission_geometry = mission_geometry::State::default(); }
        self.geometry_vertex = geometry_vertex_viewport::State::default();
        self.geometry.reset();
        self.geometry_history = geometry_history::History::default();
        self.geometry_origin = None;
        self.geometry_preview = None;
        self.geometry_preview_visible = false;
        self.geometry_staged = false;
        self.geometry_host_context = None;
    }

    fn change_geometry_history(&mut self, editor: &Editor, redo: bool) -> Result<(), String> {
        if let Some(reason) = self.geometry_blocker(editor) { return Err(reason); }
        if self.geometry.dirty() { return Err("Stage or reset geometry fields before local Undo/Redo".into()); }
        if self.asset_path.trim() != self.loaded_asset_source.as_deref().unwrap_or_default()
            || self.model_path.trim() != self.loaded_model_source.as_deref().unwrap_or_default() {
            return Err("Open or clear pending mesh source paths before local geometry Undo/Redo".into());
        }
        self.geometry_bindings_blocker(editor, self.mission_reprojection_enabled())?;
        let mut history = self.geometry_history.clone();
        let graph = if redo { history.redo() } else { history.undo() }.ok_or("No local geometry history in that direction")?;
        let staged = !history.at_origin();
        let preview = if staged { Some(geometry_preview::build(graph.as_ref().ok_or("Staged geometry has no graph")?)?) } else { None };
        let origin = self.geometry_origin.as_ref().ok_or("Geometry history has no origin")?;
        let settings_changed = origin.settings != self.geometry_settings();
        let dirty = origin.dirty || staged || settings_changed;
        let sources = if staged {
            (String::new(), String::new(), None, None)
        } else {
            (origin.asset_path.clone(), origin.model_path.clone(), origin.asset_source.clone(), origin.model_source.clone())
        };
        self.cancel_agent_pick();
        self.geometry_history = history;
        self.graph = graph;
        self.geometry_preview = preview;
        self.geometry_staged = staged;
        self.geometry_preview_visible = staged;
        self.draft_dirty = dirty;
        (self.asset_path, self.model_path, self.loaded_asset_source, self.loaded_model_source) = sources;
        self.geometry.acknowledge();
        self.error = None;
        Ok(())
    }

    fn frame_staged_geometry(&self, editor: &mut Editor) -> Result<(), String> {
        if let Some(reason) = self.geometry_blocker(editor) { return Err(reason); }
        self.geometry_bindings_blocker(editor, self.mission_reprojection_enabled())?;
        if !self.geometry_staged { return Err("Stage geometry before framing its preview".into()); }
        let graph = self.graph.as_ref().ok_or("Staged geometry has no graph")?;
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for point in graph.vertices() {
            let values = point_to_f32(*point)?;
            for axis in 0..3 { low[axis] = low[axis].min(values[axis]); high[axis] = high[axis].max(values[axis]); }
        }
        if !low.iter().chain(&high).all(|value| value.is_finite()) { return Err("Staged geometry has no finite bounds".into()); }
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let extent = (0..3).map(|axis| high[axis] - low[axis]).fold(0.0_f32, f32::max);
        editor.camera3d = orr_render::OrbitCamera::new(center, 0.55, 0.5, (extent * 1.8).max(2.0));
        Ok(())
    }

    fn parse_spec(&self) -> Result<AgentSpec, String> {
        let start = [
            parse_fixed(&self.start[0])?,
            parse_fixed(&self.start[1])?,
            parse_fixed(&self.start[2])?,
        ];
        let goal = [
            parse_fixed(&self.goal[0])?,
            parse_fixed(&self.goal[1])?,
            parse_fixed(&self.goal[2])?,
        ];
        let max_vertical = parse_fixed(&self.max_vertical)?;
        let distance_per_tick = parse_fixed(&self.distance_per_tick)?;
        if max_vertical.raw() < 0 || distance_per_tick <= FP::ZERO {
            return Err(
                "Maximum vertical tolerance must be nonnegative and distance per tick positive"
                    .into(),
            );
        }
        Ok(AgentSpec {
            start,
            goal,
            max_vertical,
            distance_per_tick,
        })
    }

    fn draft_is_stale(&self) -> bool {
        if self.geometry.dirty() {
            return true;
        }
        let (Some(graph), Some(loaded_revision), Some(loaded_spec)) =
            (&self.graph, self.loaded_graph_revision, self.loaded_spec)
        else {
            return true;
        };
        let Ok(spec) = self.parse_spec() else {
            return true;
        };
        self.asset_path.trim() != self.loaded_asset_source.as_deref().unwrap_or_default()
            || self.model_path.trim() != self.loaded_model_source.as_deref().unwrap_or_default()
            || graph.revision() != loaded_revision
            || graph.profile().max_slope != parse_fixed(&self.max_slope).unwrap_or(FP::from_raw(-1))
            || spec != loaded_spec
    }

    fn reset_to_empty_pin(&mut self) {
        let art = std::mem::take(&mut self.art);
        let props = std::mem::take(&mut self.props);
        *self = Self {
            loaded_scene: self.loaded_scene.clone(),
            loaded_scene_epoch: self.loaded_scene_epoch,
            last_frame: self.last_frame,
            art,
            props,
            ..Self::default()
        };
    }

    fn clear_host_view(&mut self) {
        self.last_frame = None;
        self.view_model = None;
        self.host_status = None;
        self.host_position = None;
        self.host_triangle_count = 0;
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
        .labelled_by(response.id)
        .changed()
}

fn parse_fixed(text: &str) -> Result<FP, String> {
    orr_reflect::decimal::parse_fp(text.trim())
        .map_err(|error| format!("invalid fixed-point decimal: {error:?}"))
}

fn format_fixed(value: FP) -> String {
    orr_reflect::decimal::fp_to_decimal(value)
}

fn fixture_graph(max_slope: FP) -> Result<StaticMeshGraph, String> {
    let vertices = [
        [fp!(0), fp!(0), fp!(0)],
        [fp!(4), fp!(0), fp!(0)],
        [fp!(0), fp!(0), fp!(2)],
        [fp!(4), fp!(0), fp!(2)],
        [fp!(6), fp!(2), fp!(0)],
        [fp!(6), fp!(2), fp!(2)],
        [fp!(2), fp!(2), fp!(0)],
        [fp!(2), fp!(2), fp!(2)],
    ];
    StaticMeshGraph::bake(
        &vertices,
        &[
            [0, 2, 3],
            [0, 3, 1],
            [1, 3, 5],
            [1, 5, 4],
            [6, 7, 5],
            [6, 5, 4],
        ],
        AgentProfile {
            max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .map_err(|error| error.to_string())
}

fn validate_route(graph: &StaticMeshGraph, spec: AgentSpec) -> Result<(), String> {
    if graph.triangles().is_empty() {
        return Err("Mesh graph has no walkable triangles".into());
    }
    graph
        .find_path(
            spec.start,
            spec.goal,
            spec.max_vertical,
            SearchBudget {
                max_expansions: graph.triangles().len() as u32,
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_display_bounds(graph: &StaticMeshGraph, spec: &AgentSpec) -> Result<(), String> {
    for point in graph
        .vertices()
        .iter()
        .copied()
        .chain([spec.start, spec.goal])
    {
        for coordinate in point {
            let raw = coordinate.raw();
            let display = coordinate.to_f32();
            if i128::from(raw).abs() > i128::from(MAX_VIEW_COORDINATE_RAW)
                || !display.is_finite()
                || f64::from(display) * 65_536.0 != raw as f64
            {
                return Err(
                    "Editor mesh display requires exact f32 coordinates within +/-256".into(),
                );
            }
        }
    }
    Ok(())
}

fn pin_patch(pin: &MeshScenePin) -> serde_json::Value {
    json!({"name": PIN_NAME, "value": {
        "graph_len": pin.graph_len,
        "graph_bytes": pin.graph_bytes.as_slice(),
        "graph_revision": pin.graph_revision,
        "start": pin.start.map(format_fixed),
        "goal": pin.goal.map(format_fixed),
        "max_vertical": format_fixed(pin.max_vertical),
        "distance_per_tick": format_fixed(pin.distance_per_tick),
    }})
}

fn build_view_model(
    graph: &StaticMeshGraph,
    path: &orr_navigation::mesh::MeshPath,
    position: [FP; 3],
) -> Result<StaticModel, String> {
    let graph_revision = graph.revision();
    let mut surface = Primitive {
        id: format!("{VIEW_ASSET_ID}#node=0"),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: 0,
        transform: IDENTITY,
    };
    for triangle in graph.triangles() {
        let [first, second, third] = triangle.vertices;
        add_triangle(
            &mut surface,
            [
                point_to_f32(graph.vertices()[first as usize])?,
                point_to_f32(graph.vertices()[second as usize])?,
                point_to_f32(graph.vertices()[third as usize])?,
            ],
            [0.0, 1.0, 0.0],
        );
    }
    let mut route = Primitive {
        id: format!("{VIEW_ASSET_ID}#node=1"),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: 1,
        transform: IDENTITY,
    };
    for segment in path.points.windows(2) {
        add_segment_tube(
            &mut route,
            point_to_f32(segment[0])?,
            point_to_f32(segment[1])?,
            0.035,
        );
    }
    if route.indices.is_empty() {
        add_octahedron(&mut route, point_to_f32(path.points[0])?, 0.035);
    }
    let agent_position = point_to_f32(position)?;
    let mut marker = Primitive {
        id: format!("{VIEW_ASSET_ID}#node=2"),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: 2,
        transform: IDENTITY,
    };
    add_octahedron(&mut marker, agent_position, 0.10);
    let source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: VIEW_ASSET_ID.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: hex_revision(graph_revision),
        }],
        primitives: vec![surface, route, marker],
        materials: [
            [70, 145, 205, 255],
            [255, 190, 35, 255],
            [255, 45, 210, 255],
        ]
        .into_iter()
        .map(|rgba| Material {
            base_color: rgba.map(|channel| channel as f32 / 255.0),
            image: 0,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        })
        .collect(),
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    StaticModel::new(source).map_err(|error| error.to_string())
}

fn exact_f32(value: FP) -> Result<f32, String> {
    let display = value.to_f32();
    if i128::from(value.raw()).abs() > i128::from(MAX_VIEW_COORDINATE_RAW)
        || !display.is_finite()
        || f64::from(display) * 65_536.0 != value.raw() as f64
    {
        return Err("Host mesh view contains a coordinate that cannot be displayed exactly".into());
    }
    Ok(display)
}

fn point_to_f32(point: [FP; 3]) -> Result<[f32; 3], String> {
    Ok([
        exact_f32(point[0])?,
        exact_f32(point[1])?,
        exact_f32(point[2])?,
    ])
}

fn add_triangle(mesh: &mut Primitive, points: [[f32; 3]; 3], normal: [f32; 3]) {
    for point in points {
        mesh.indices.push(mesh.vertices.len() as u32);
        mesh.vertices.push(Vertex {
            position: point,
            normal,
            uv: [0.5; 2],
        });
    }
}

fn add_segment_tube(mesh: &mut Primitive, start: [f32; 3], end: [f32; 3], radius: f32) {
    let Some(direction) = normalize(subtract(end, start)) else {
        return;
    };
    let reference = if direction[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let side = normalize(cross(direction, reference)).unwrap();
    let up = normalize(cross(direction, side)).unwrap();
    let basis = [side, up];
    let base = mesh.vertices.len() as u32;
    for (endpoint_index, endpoint) in [start, end].into_iter().enumerate() {
        for side in 0..4 {
            let angle = side as f32 * std::f32::consts::FRAC_PI_2;
            let offset = add(
                scale(basis[0], angle.cos() * radius),
                scale(basis[1], angle.sin() * radius),
            );
            let normal = normalize(offset).unwrap();
            mesh.vertices.push(Vertex {
                position: add(endpoint, offset),
                normal,
                uv: [side as f32 / 4.0, endpoint_index as f32],
            });
        }
    }
    for side in 0..4u32 {
        let next = (side + 1) % 4;
        let first = base + side;
        let second = base + next;
        let third = base + 4 + next;
        let fourth = base + 4 + side;
        mesh.indices
            .extend_from_slice(&[first, second, third, first, third, fourth]);
    }
}

fn add_octahedron(mesh: &mut Primitive, center: [f32; 3], radius: f32) {
    let points = [
        [center[0] + radius, center[1], center[2]],
        [center[0] - radius, center[1], center[2]],
        [center[0], center[1] + radius, center[2]],
        [center[0], center[1] - radius, center[2]],
        [center[0], center[1], center[2] + radius],
        [center[0], center[1], center[2] - radius],
    ];
    for [first, second, third] in [
        [2, 0, 4],
        [2, 4, 1],
        [2, 1, 5],
        [2, 5, 0],
        [3, 4, 0],
        [3, 1, 4],
        [3, 5, 1],
        [3, 0, 5],
    ] {
        let mut triangle = [points[first], points[second], points[third]];
        let mut normal = cross(
            subtract(triangle[1], triangle[0]),
            subtract(triangle[2], triangle[0]),
        );
        let face_center = scale(add(add(triangle[0], triangle[1]), triangle[2]), 1.0 / 3.0);
        let outward = subtract(face_center, center);
        if dot(normal, outward) < 0.0 {
            triangle.swap(1, 2);
            normal = scale(normal, -1.0);
        }
        let normal = normalize(normal).unwrap_or([0.0, 1.0, 0.0]);
        add_triangle(mesh, triangle, normal);
    }
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}
fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}
fn scale(vector: [f32; 3], amount: f32) -> [f32; 3] {
    [vector[0] * amount, vector[1] * amount, vector[2] * amount]
}
fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}
fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}
fn normalize(vector: [f32; 3]) -> Option<[f32; 3]> {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    (length.is_finite() && length > 1e-8).then(|| scale(vector, 1.0 / length))
}

fn hex_revision(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}
