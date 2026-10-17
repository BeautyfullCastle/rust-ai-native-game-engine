use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    mesh_art::Snapshot,
    mesh_props::{self, Document, Prop, MAX_PROPS},
    model::Mode,
    viewport3d::SceneModel,
};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use orr_render::Camera3D;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 4;

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

struct PropGesture {
    original: Document,
    selected: u64,
    drag: crate::mesh_prop_view::Drag,
}

struct PropPlacement {
    prop_id: u64,
    scene: PathBuf,
    scene_epoch: u64,
    graph_revision: [u8; 32],
    checksum: u64,
    position: Option<[orr_fp::FP; 3]>,
}

struct HistoryEntry {
    id: u64,
    document: Document,
}

pub(crate) struct Panel {
    package_root: String,
    asset: String,
    license: String,
    name: String,
    position: [String; 3],
    scale: String,
    rotation: [String; 3],
    tint: [String; 4],
    surface_enabled: bool,
    surface_material: String,
    surface_uv_offset: [String; 2],
    surface_uv_scale: [String; 2],
    surface_quarter_turns: String,
    surface_pivot: [String; 3],
    license_acknowledged: bool,
    confirm_remove: bool,
    confirm_discard_invalid: bool,
    confirm_discard_changes: bool,
    recovery_target: Option<(PathBuf, u64, [u8; 32])>,
    move_enabled: bool,
    snap_to_quarter: bool,
    gesture: Option<PropGesture>,
    placement: Option<PropPlacement>,
    candidate: Option<Snapshot>,
    selected: Option<u64>,
    document: Document,
    scene: Option<PathBuf>,
    scene_epoch: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    document_pending: bool,
    context_stale: bool,
    sidecar_digest: Option<String>,
    sidecar_error: Option<String>,
    history: Vec<HistoryEntry>,
    cursor: usize,
    next_history_id: u64,
    next_prop_id: u64,
    saved_history_id: Option<u64>,
    status: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            package_root: String::new(),
            asset: String::new(),
            license: String::new(),
            name: "prop".into(),
            position: ["0".into(), "0".into(), "0".into()],
            scale: "1".into(),
            rotation: ["0".into(), "0".into(), "0".into()],
            tint: ["1".into(), "1".into(), "1".into(), "1".into()],
            surface_enabled: false,
            surface_material: "0".into(),
            surface_uv_offset: ["0".into(), "0".into()],
            surface_uv_scale: ["1".into(), "1".into()],
            surface_quarter_turns: "0".into(),
            surface_pivot: ["0".into(), "0".into(), "0".into()],
            license_acknowledged: false,
            confirm_remove: false,
            confirm_discard_invalid: false,
            confirm_discard_changes: false,
            recovery_target: None,
            move_enabled: false,
            snap_to_quarter: false,
            gesture: None,
            placement: None,
            candidate: None,
            selected: None,
            document: Document::default(),
            scene: None,
            scene_epoch: None,
            graph_revision: None,
            document_pending: true,
            context_stale: false,
            sidecar_digest: None,
            sidecar_error: None,
            history: vec![HistoryEntry {
                id: 0,
                document: Document::default(),
            }],
            cursor: 0,
            next_history_id: 1,
            next_prop_id: 1,
            saved_history_id: Some(0),
            status: String::new(),
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        if self.placement.is_some() && !self.placement_context_matches(editor) {
            self.cancel_placement();
        }
        if self.gesture.is_some() && !self.gesture_context_matches(editor) {
            self.cancel_gesture();
        }
        if editor.game() != EditorGame::MeshNavigation {
            self.document_pending = true;
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.document_pending = true;
            return;
        };
        self.document_pending = false;
        let scene_epoch = editor.scene_load_epoch();
        let requested_context = (scene.clone(), scene_epoch, graph_revision);
        if self.recovery_target.as_ref() != Some(&requested_context) {
            self.confirm_discard_changes = false;
            self.recovery_target = Some(requested_context.clone());
        }
        if self.scene.as_ref() != Some(&scene)
            || self.scene_epoch != Some(scene_epoch)
            || self.graph_revision != Some(graph_revision)
        {
            self.move_enabled = false;
        }
        if self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(scene_epoch)
            && self.graph_revision == Some(graph_revision)
        {
            return;
        }
        if self.scene.is_some()
            && self.scene_epoch == Some(scene_epoch)
            && self.graph_revision == Some(graph_revision)
        {
            self.scene = Some(scene);
            self.move_enabled = false;
            self.sidecar_digest = None;
            self.context_stale = false;
            self.sidecar_error = None;
            self.confirm_discard_invalid = false;
            if self.document.props.is_empty() && !self.dirty() {
                self.scene = None;
                self.scene_epoch = None;
                self.graph_revision = None;
                self.recovery_target = None;
                self.confirm_discard_changes = false;
                self.candidate = None;
                self.selected = None;
                self.confirm_remove = false;
                self.sync(editor);
                return;
            }
            self.recovery_target = Some(requested_context);
            self.saved_history_id = None;
            self.status = "Scene path changed; save mesh props explicitly or discard and reload the target sidecar.".into();
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.recovery_target = Some(requested_context);
            self.confirm_discard_invalid = false;
            self.status = "Unsaved mesh props belong to a different scene or graph revision; discard and reload explicitly or return to the original scene.".into();
            return;
        }

        self.scene = Some(scene.clone());
        self.scene_epoch = Some(scene_epoch);
        self.graph_revision = Some(graph_revision);
        self.context_stale = false;
        self.move_enabled = false;
        self.candidate = None;
        self.selected = None;
        self.confirm_remove = false;
        self.confirm_discard_invalid = false;
        self.confirm_discard_changes = false;
        self.recovery_target = None;
        self.error = None;
        self.sidecar_error = None;
        self.sidecar_digest = None;
        self.reset_history(Document::default(), Some(0));
        match mesh_props::load(&scene, graph_revision) {
            Ok(Some((document, digest))) => {
                self.sidecar_digest = Some(digest);
                self.selected = document.props.first().map(|prop| prop.id);
                self.document = document.clone();
                self.copy_selection_fields();
                self.reset_history(document, Some(0));
                self.recompute_next_prop_id();
                self.status = "Loaded saved mesh props for this graph revision.".into();
            }
            Ok(None) => {
                self.document = Document::default();
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
                self.status = "No saved mesh props for this graph revision.".into();
            }
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.sidecar_digest = mesh_props::sidecar_stamp(&scene).ok().flatten();
                self.error = Some(error.clone());
                self.status = format!("Saved mesh props could not be admitted: {error}");
            }
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &Editor, agent_pick_busy: bool) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh props", |ui| {
            ui.label("Static cosmetic props are saved in a separate scene sidecar and do not affect navigation physics.");
            ui.weak("Up to six props reserve the remaining viewport asset slots for the graph and agent art.");
            ui.add_enabled_ui(
                self.matching_edit_context(editor) && editor.spec().is_local(),
                |ui| {
                    ui.checkbox(&mut self.move_enabled, "Move props in viewport");
                    if self.move_enabled {
                        ui.checkbox(&mut self.snap_to_quarter, "Snap movement to 0.25 units");
                        ui.weak("Drag a prop on its horizontal plane. This changes cosmetic presentation only, not ECS entities or physics.");
                    }
                },
            );
            if !self.move_enabled && self.gesture.is_some() {
                self.cancel_gesture();
            }
            let editable = self.can_edit(editor) && self.gesture.is_none();
            ui.add_enabled_ui(editable, |ui| {
                let package_changed = field(ui, "Prop package source directory", &mut self.package_root);
                let asset_changed = field(ui, "Prop model asset", &mut self.asset);
                let license_changed = field(ui, "Prop license asset", &mut self.license);
                if package_changed || asset_changed || license_changed {
                    self.candidate = None;
                    self.license_acknowledged = false;
                    self.error = None;
                    self.status = "Prop source changed; load the package again before adding or updating props.".into();
                }
                if ui
                    .checkbox(
                        &mut self.license_acknowledged,
                        "I have reviewed rights to use these props",
                    )
                    .changed()
                {
                    self.candidate = None;
                }
                if ui
                    .add_enabled(
                        self.license_acknowledged
                            && !self.package_root.trim().is_empty()
                            && !self.asset.trim().is_empty()
                            && !self.license.trim().is_empty(),
                        egui::Button::new("Load prop candidate"),
                    )
                    .clicked()
                {
                    self.load_candidate();
                }
            });

            ui.add_enabled_ui(editable, |ui| {
                field(ui, "Prop name", &mut self.name);
                field(ui, "Prop position X", &mut self.position[0]);
                field(ui, "Prop position Y", &mut self.position[1]);
                field(ui, "Prop position Z", &mut self.position[2]);
                field(ui, "Prop scale", &mut self.scale);
                field(ui, "Prop rotation X (degrees)", &mut self.rotation[0]);
                field(ui, "Prop rotation Y (degrees)", &mut self.rotation[1]);
                field(ui, "Prop rotation Z (degrees)", &mut self.rotation[2]);
                for (channel, label) in ["Prop tint R", "Prop tint G", "Prop tint B", "Prop tint A"]
                    .into_iter()
                    .enumerate()
                {
                    field(ui, label, &mut self.tint[channel]);
                }
                ui.checkbox(&mut self.surface_enabled, "Edit prop UV and pivot");
                if self.surface_enabled {
                    field(ui, "Prop UV material index", &mut self.surface_material);
                    if let Some(snapshot) = self.candidate.as_ref().or_else(|| self.selected_prop().map(|prop| &prop.art)) {
                        let choices = snapshot
                            .material_choices()
                            .into_iter()
                            .map(|index| index.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        ui.label(format!("UV materials used by this model: {choices}"));
                    }
                    field(ui, "Prop UV offset U", &mut self.surface_uv_offset[0]);
                    field(ui, "Prop UV offset V", &mut self.surface_uv_offset[1]);
                    field(ui, "Prop UV scale U", &mut self.surface_uv_scale[0]);
                    field(ui, "Prop UV scale V", &mut self.surface_uv_scale[1]);
                    field(ui, "Prop UV quarter turns", &mut self.surface_quarter_turns);
                    for (axis, label) in ["Prop pivot X", "Prop pivot Y", "Prop pivot Z"].into_iter().enumerate() {
                        field(ui, label, &mut self.surface_pivot[axis]);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        editable
                            && self.candidate.is_some()
                            && self.document.props.len() < MAX_PROPS,
                        egui::Button::new("Add prop"),
                    )
                    .clicked()
                {
                    self.add_prop();
                }
                if ui
                    .add_enabled(editable && self.selected_prop().is_some(), egui::Button::new("Update selected prop"))
                    .clicked()
                {
                    self.update_selected();
                }
                if ui
                    .add_enabled(
                        editable
                            && self.selected_prop().is_some()
                            && self.document.props.len() < MAX_PROPS,
                        egui::Button::new("Duplicate selected prop"),
                    )
                    .clicked()
                {
                    self.duplicate_selected_prop();
                }
                if ui
                    .add_enabled(
                        editable && self.selected_prop().is_some() && !agent_pick_busy,
                        egui::Button::new("Place selected prop on mesh"),
                    )
                    .clicked()
                {
                    self.begin_placement(editor, agent_pick_busy);
                }
            });
            let staged_position = self.placement.as_ref().and_then(|placement| placement.position);
            let can_apply_placement =
                staged_position.is_some() && self.placement_context_matches(editor);
            if self.placement.is_some() {
                ui.label(match staged_position {
                    Some(position) => {
                        let coordinates = position.map(orr_fp::FP::to_f32).map(format_number);
                        format!(
                            "Staged mesh placement at [{}, {}, {}]; apply explicitly to create one history entry.",
                            coordinates[0], coordinates[1], coordinates[2]
                        )
                    }
                    None => "Click an admitted walkable mesh triangle to stage this prop's placement.".into(),
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can_apply_placement,
                            egui::Button::new("Apply prop placement"),
                        )
                        .clicked()
                    {
                        self.apply_placement(editor);
                    }
                    if ui.button("Cancel prop placement").clicked() {
                        self.cancel_placement();
                    }
                });
            }

            ui.label(format!("Props: {}/{}", self.document.props.len(), MAX_PROPS));
            let mut selected_prop = None;
            ui.add_enabled_ui(self.placement.is_none(), |ui| {
                for prop in &self.document.props {
                    let label = format!("{} · prop-{:016x}", prop.name, prop.id);
                    if ui
                        .selectable_label(self.selected == Some(prop.id), label)
                        .clicked()
                    {
                        selected_prop = Some(prop.id);
                    }
                }
            });
            if let Some(identifier) = selected_prop {
                self.selected = Some(identifier);
                self.copy_selection_fields();
                self.confirm_remove = false;
            }
            ui.add_enabled_ui(editable, |ui| {
                ui.checkbox(&mut self.confirm_remove, "I confirm removing this prop");
            });
            if ui
                .add_enabled(
                    editable && self.confirm_remove && self.selected_prop().is_some(),
                    egui::Button::new("Remove selected prop"),
                )
                .clicked()
            {
                self.remove_selected();
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(
                    &mut self.confirm_discard_invalid,
                    "I confirm discarding invalid mesh props",
                );
                if ui
                    .add_enabled(
                        self.can_discard_invalid(editor),
                        egui::Button::new("Discard invalid props sidecar"),
                    )
                    .clicked()
                {
                    self.discard_invalid_sidecar(editor);
                }
            }
            if self.dirty() || self.context_stale {
                ui.checkbox(
                    &mut self.confirm_discard_changes,
                    "I confirm discarding unsaved prop changes",
                );
                if ui
                    .add_enabled(
                        self.can_discard_changes(editor),
                        egui::Button::new("Discard unsaved props and reload"),
                    )
                    .clicked()
                {
                    self.discard_changes_and_reload(editor);
                }
            }
            ui.horizontal(|ui| {
                let history_enabled = self.can_change_history(editor);
                if ui
                    .add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo prop edit"))
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        history_enabled && self.cursor + 1 < self.history.len(),
                        egui::Button::new("Redo prop edit"),
                    )
                    .clicked()
                {
                    self.redo();
                }
                if ui
                    .add_enabled(editable && self.dirty(), egui::Button::new("Save props"))
                    .clicked()
                {
                    self.save(editor);
                }
                if ui
                    .add_enabled(self.can_reload(editor), egui::Button::new("Reload props"))
                    .clicked()
                {
                    self.reload(editor);
                }
            });
            if self.document_pending {
                ui.label("Waiting for an admitted local mesh scene and coherent Frame.");
            }
            if self.context_stale {
                ui.colored_label(egui::Color32::RED, "Mesh props belong to an earlier scene or graph revision.");
            }
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved mesh props changes block Play.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn placements(&self, editor: &Editor) -> Vec<SceneModel> {
        if self.document_pending
            || self.context_stale
            || self.sidecar_error.is_some()
            || (self.placement.is_some() && !self.placement_context_matches(editor))
            || !current_identity(editor).is_some_and(|(scene, revision)| {
                self.scene.as_ref() == Some(&scene)
                    && self.graph_revision == Some(revision)
                    && self.scene_epoch == Some(editor.scene_load_epoch())
            })
        {
            return Vec::new();
        }
        if self
            .placement
            .as_ref()
            .and_then(|placement| placement.position)
            .is_none()
        {
            return self.document.placements().unwrap_or_default();
        }
        self.presentation_document()
            .and_then(|document| document.placements())
            .unwrap_or_default()
    }

    pub(crate) fn selected_bounds(&self, editor: &Editor) -> Option<crate::mesh_prop_view::Bounds> {
        if !self.move_enabled || !self.matching_edit_context(editor) {
            return None;
        }
        let selected = self.selected?;
        let scene_models = self.placements(editor);
        let selected_index = self
            .document
            .props
            .iter()
            .position(|prop| prop.id == selected)?;
        let model = scene_models.get(selected_index)?;
        if scene_models.len() != self.document.props.len() {
            return None;
        }
        crate::mesh_prop_view::bounds(model).ok()
    }

    pub(crate) fn viewport(
        &mut self,
        editor: &Editor,
        camera: &Camera3D,
        size: (u32, u32),
        input: ViewInput,
    ) -> bool {
        if self.placement.is_some() && !self.placement_context_matches(editor) {
            self.cancel_placement();
            return true;
        }
        if self.gesture.is_some() && !self.gesture_context_matches(editor) {
            self.cancel_gesture();
            return true;
        }
        if input.cancel {
            if self.placement.is_some() {
                self.cancel_placement();
                return true;
            }
            let active = self.gesture.is_some();
            self.cancel_gesture();
            return active;
        }
        if self.placement.is_some() {
            if input.clicked {
                let Some(pixel) = input.pixel else {
                    return true;
                };
                let Some(frame) = editor.snapshot().map(|snapshot| snapshot.predicted()) else {
                    self.cancel_placement();
                    return true;
                };
                let view = match mesh_from_view(frame) {
                    Ok(view) => view,
                    Err(error) => {
                        self.error = Some(format!("Mesh placement is unavailable: {error}"));
                        return true;
                    }
                };
                let (origin, direction) = camera.screen_ray(pixel, size);
                match crate::mesh_agent_view::pick(
                    &view.graph,
                    origin,
                    direction,
                    view.agent.spec.max_vertical,
                ) {
                    Ok(Some(hit)) => {
                        if let Some(placement) = &mut self.placement {
                            placement.position = Some(hit.position);
                        }
                        self.error = None;
                        self.status =
                            "Mesh prop position staged; apply explicitly or cancel.".into();
                    }
                    Ok(None) => {
                        self.error =
                            Some("The click did not hit an admitted walkable mesh triangle".into());
                    }
                    Err(error) => self.error = Some(format!("Mesh prop placement failed: {error}")),
                }
            }
            return true;
        }
        if self.gesture.is_some() && !self.move_enabled {
            self.cancel_gesture();
            return true;
        }
        if let Some((gesture_selected, drag)) = self
            .gesture
            .as_ref()
            .map(|gesture| (gesture.selected, gesture.drag))
        {
            if let Some(pixel) = input.pixel.filter(|_| input.dragging || input.released) {
                let snap = self.snap_to_quarter.then_some(0.25);
                if let Some(position) = drag.position(camera, pixel, size, snap) {
                    let changed = if let Some(prop) = self
                        .document
                        .props
                        .iter_mut()
                        .find(|prop| prop.id == gesture_selected)
                    {
                        let mut offset = prop.art.offset();
                        offset[0] = position[0];
                        offset[2] = position[2];
                        if let Ok(art) = prop.art.clone().with_transform(offset, prop.art.scale()) {
                            prop.art = art;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if changed {
                        self.copy_selection_fields();
                    }
                }
            }
            if input.released {
                self.finish_gesture();
            }
            return true;
        }
        if !self.move_enabled || !self.can_edit(editor) || editor.mode() != Mode::Edit {
            return false;
        }
        if input.started {
            let Some(pixel) = input.press_pixel else {
                return false;
            };
            let (origin, direction) = camera.screen_ray(pixel, size);
            let models = match self.viewport_models() {
                Ok(models) => models,
                Err(error) => {
                    self.error = Some(error);
                    return true;
                }
            };
            let selected = match crate::mesh_prop_view::pick(&models, origin, direction) {
                Ok(selected) => selected,
                Err(error) => {
                    self.error = Some(error);
                    return true;
                }
            };
            let Some(selected) = selected else {
                self.selected = None;
                self.confirm_remove = false;
                return true;
            };
            self.selected = Some(selected);
            self.copy_selection_fields();
            let Some(position) = self.selected_prop().map(|prop| prop.art.offset()) else {
                return true;
            };
            let Some(drag) = crate::mesh_prop_view::Drag::new(camera, pixel, size, position) else {
                return true;
            };
            self.gesture = Some(PropGesture {
                original: self.document.clone(),
                selected,
                drag,
            });
            self.confirm_remove = false;
            if input.released {
                self.finish_gesture();
            }
            return true;
        }
        if input.clicked {
            let Some(pixel) = input.pixel else {
                return false;
            };
            let (origin, direction) = camera.screen_ray(pixel, size);
            let models = match self.viewport_models() {
                Ok(models) => models,
                Err(error) => {
                    self.error = Some(error);
                    return true;
                }
            };
            match crate::mesh_prop_view::pick(&models, origin, direction) {
                Ok(selected) => {
                    self.selected = selected;
                    self.copy_selection_fields();
                    self.confirm_remove = false;
                }
                Err(error) => self.error = Some(error),
            }
            return true;
        }
        false
    }

    pub(crate) fn gesture_busy(&self) -> bool {
        self.gesture.is_some() || self.placement.is_some()
    }

    fn viewport_models(&self) -> Result<Vec<(u64, SceneModel)>, String> {
        self.document
            .props
            .iter()
            .map(|prop| {
                prop.art
                    .placement([orr_fp::FP::ZERO; 3])
                    .map(|model| (prop.id, model))
            })
            .collect()
    }

    fn matching_edit_context(&self, editor: &Editor) -> bool {
        let Some((scene, graph_revision)) = current_identity(editor) else {
            return false;
        };
        editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.screenshot_gesture_busy()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && !self.context_stale
            && self.sidecar_error.is_none()
            && self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(editor.scene_load_epoch())
            && self.graph_revision == Some(graph_revision)
    }

    fn gesture_context_matches(&self, editor: &Editor) -> bool {
        editor.spec().is_local() && self.matching_edit_context(editor)
    }

    fn cancel_gesture(&mut self) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        self.document = gesture.original;
        self.selected = Some(gesture.selected);
        self.copy_selection_fields();
        self.status =
            "Viewport prop movement cancelled; the original placement was restored.".into();
    }

    pub(crate) fn cancel_viewport_gesture(&mut self) {
        self.cancel_gesture();
        self.cancel_placement();
    }

    fn finish_gesture(&mut self) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        let updated = self.document.clone();
        self.document = gesture.original.clone();
        self.selected = Some(gesture.selected);
        self.copy_selection_fields();
        if same_document_placements(&gesture.original, &updated) {
            return;
        }
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some(
                "Mesh graph changed before the prop move completed; original placement restored."
                    .into(),
            );
            return;
        };
        if let Err(error) = validate_document(&updated, graph_revision) {
            self.error = Some(format!(
                "Mesh prop move was rejected; original placement restored: {error}"
            ));
            return;
        }
        self.push_document(updated);
        self.selected = Some(gesture.selected);
        self.copy_selection_fields();
        self.status = "Moved mesh prop in viewport; save explicitly to persist.".into();
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.gesture.is_some() {
            return Some("Finish or cancel the active mesh prop move before Play".into());
        }
        if self.placement.is_some() {
            return Some("Apply or cancel the staged mesh prop placement before Play".into());
        }
        if self.document_pending
            || current_identity(editor).is_none_or(|(scene, revision)| {
                self.scene.as_ref() != Some(&scene)
                    || self.graph_revision != Some(revision)
                    || self.scene_epoch != Some(editor.scene_load_epoch())
            })
        {
            return Some("Wait for the requested admitted mesh scene and coherent Frame".into());
        }
        if self.context_stale {
            return Some(
                "Resolve unsaved mesh props from the prior scene or graph revision".into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved mesh props are invalid: {error}"));
        }
        if self.dirty() {
            return Some("Save or undo mesh prop changes before Play".into());
        }
        if self.document.placements().is_err() {
            return Some("Mesh props cannot be presented for the current graph".into());
        }
        None
    }

    pub(crate) fn dirty(&self) -> bool {
        self.gesture.is_some()
            || self.placement.is_some()
            || self.current_history_id() != self.saved_history_id
    }

    fn load_candidate(&mut self) {
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before loading a prop".into());
            return;
        };
        match Snapshot::from_package(
            Path::new(self.package_root.trim()),
            self.asset.trim(),
            self.license.trim(),
            graph_revision,
        ) {
            Ok(snapshot) => {
                self.copy_snapshot_fields(&snapshot);
                self.candidate = Some(snapshot);
                self.error = None;
                self.status = "Loaded prop candidate; add or update a prop explicitly.".into();
                self.license_acknowledged = false;
            }
            Err(error) => {
                self.candidate = None;
                self.error = Some(format!("Prop package load failed: {error}"));
            }
        }
    }

    fn duplicate_selected_prop(&mut self) {
        let Some(selected) = self.selected else {
            self.error = Some("Select a mesh prop before duplicating it".into());
            return;
        };
        let Some(identifier) = self.available_prop_id() else {
            self.error = Some("No more stable prop identifiers are available".into());
            return;
        };
        let document = match duplicate_prop_document(&self.document, selected, identifier) {
            Ok(document) => document,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before duplicating a prop".into());
            return;
        };
        if let Err(error) = validate_document(&document, graph_revision) {
            self.error = Some(error);
            return;
        }
        self.push_document(document);
        self.next_prop_id = identifier.checked_add(1).unwrap_or(1);
        self.selected = Some(identifier);
        self.confirm_remove = false;
        self.copy_selection_fields();
        self.error = None;
        self.status = "Duplicated mesh prop in memory; save explicitly to persist.".into();
    }

    fn begin_placement(&mut self, editor: &Editor, agent_pick_busy: bool) {
        if agent_pick_busy {
            self.error = Some("Finish or cancel endpoint picking before placing a prop".into());
            return;
        }
        if !self.can_edit(editor) {
            self.error =
                Some("Prop placement requires the matching editable local mesh scene".into());
            return;
        }
        let Some(prop_id) = self.selected.filter(|identifier| {
            self.document
                .props
                .iter()
                .any(|prop| prop.id == *identifier)
        }) else {
            self.error = Some("Select a mesh prop before placing it on the mesh".into());
            return;
        };
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("Wait for an admitted mesh graph before placing a prop".into());
            return;
        };
        self.placement = Some(PropPlacement {
            prop_id,
            scene,
            scene_epoch: editor.scene_load_epoch(),
            graph_revision,
            checksum: editor.sim().checksum,
            position: None,
        });
        self.error = None;
        self.status = "Click an admitted walkable mesh triangle to stage the prop position.".into();
    }

    fn apply_placement(&mut self, editor: &Editor) {
        if !self.placement_context_matches(editor) {
            self.cancel_placement();
            self.error = Some("Mesh placement context changed; the prop was not moved".into());
            return;
        }
        let Some(placement) = self.placement.as_ref() else {
            return;
        };
        let (prop_id, graph_revision, Some(position)) = (
            placement.prop_id,
            placement.graph_revision,
            placement.position,
        ) else {
            self.error = Some("Click a walkable mesh triangle before applying placement".into());
            return;
        };
        let updated =
            match relocate_prop_document(&self.document, prop_id, position, graph_revision) {
                Ok(document) => document,
                Err(error) => {
                    self.error = Some(format!("Mesh prop placement rejected: {error}"));
                    return;
                }
            };
        self.placement = None;
        if same_document_placements(&self.document, &updated) {
            self.error = None;
            self.status =
                "Prop is already at the selected mesh position; no history entry was added.".into();
            return;
        }
        self.push_document(updated);
        self.selected = Some(prop_id);
        self.copy_selection_fields();
        self.error = None;
        self.status =
            "Placed mesh prop on the walkable surface; save explicitly to persist.".into();
    }

    fn cancel_placement(&mut self) {
        if self.placement.take().is_some() {
            self.status = "Mesh prop placement cancelled; the document was unchanged.".into();
        }
    }

    fn placement_context_matches(&self, editor: &Editor) -> bool {
        let Some(placement) = &self.placement else {
            return false;
        };
        current_identity(editor).is_some_and(|(scene, graph_revision)| {
            scene == placement.scene
                && editor.scene_load_epoch() == placement.scene_epoch
                && graph_revision == placement.graph_revision
                && editor.sim().checksum == placement.checksum
                && self.matching_edit_context(editor)
        })
    }

    fn presentation_document(&self) -> Result<Document, String> {
        let Some(placement) = self.placement.as_ref() else {
            return Ok(self.document.clone());
        };
        let Some(position) = placement.position else {
            return Ok(self.document.clone());
        };
        relocate_prop_document(
            &self.document,
            placement.prop_id,
            position,
            placement.graph_revision,
        )
    }

    fn add_prop(&mut self) {
        let Some(candidate) = self.candidate.clone() else {
            self.error = Some("Load a verified package before adding a prop".into());
            return;
        };
        if self.document.props.len() >= MAX_PROPS {
            self.error = Some(format!("A mesh scene supports at most {MAX_PROPS} props"));
            return;
        }
        let name = match validate_name(&self.name) {
            Ok(name) => name,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if self.document.props.iter().any(|prop| prop.name == name) {
            self.error = Some("Prop names must be unique in the mesh scene".into());
            return;
        }
        let Some(id) = self.available_prop_id() else {
            self.error = Some("No more stable prop identifiers are available".into());
            return;
        };
        let art = match self.transformed(candidate) {
            Ok(art) => art,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let mut document = self.document.clone();
        document.props.push(Prop { id, name, art });
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before adding a prop".into());
            return;
        };
        if let Err(error) = validate_document(&document, graph_revision) {
            self.error = Some(error);
            return;
        }
        self.push_document(document);
        self.next_prop_id = id.checked_add(1).unwrap_or(1);
        self.selected = Some(id);
        self.confirm_remove = false;
        self.error = None;
        self.status = "Added mesh prop in memory; save explicitly to persist.".into();
    }

    fn update_selected(&mut self) {
        let Some(selected) = self.selected else {
            self.error = Some("Select a mesh prop before updating it".into());
            return;
        };
        let Some(index) = self
            .document
            .props
            .iter()
            .position(|prop| prop.id == selected)
        else {
            self.error = Some("The selected mesh prop is no longer present".into());
            return;
        };
        let name = match validate_name(&self.name) {
            Ok(name) => name,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if self
            .document
            .props
            .iter()
            .enumerate()
            .any(|(prop_index, prop)| prop_index != index && prop.name == name)
        {
            self.error = Some("Prop names must be unique in the mesh scene".into());
            return;
        }
        let base = self
            .candidate
            .as_ref()
            .unwrap_or(&self.document.props[index].art)
            .clone();
        let art = match self.transformed(base) {
            Ok(art) => art,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let mut document = self.document.clone();
        document.props[index].name = name;
        document.props[index].art = art;
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before updating a prop".into());
            return;
        };
        if let Err(error) = validate_document(&document, graph_revision) {
            self.error = Some(error);
            return;
        }
        self.push_document(document);
        self.error = None;
        self.status = "Updated mesh prop in memory; save explicitly to persist.".into();
    }

    fn remove_selected(&mut self) {
        let Some(selected) = self.selected else {
            self.error = Some("Select a mesh prop before removing it".into());
            return;
        };
        let mut document = self.document.clone();
        let original_count = document.props.len();
        document.props.retain(|prop| prop.id != selected);
        if document.props.len() == original_count {
            self.error = Some("The selected mesh prop is no longer present".into());
            return;
        }
        self.push_document(document);
        self.selected = self.document.props.first().map(|prop| prop.id);
        self.confirm_remove = false;
        self.copy_selection_fields();
        self.error = None;
        self.status = "Removed mesh prop in memory; save explicitly to persist.".into();
    }

    fn save(&mut self, editor: &Editor) {
        let Some((scene, revision)) = current_identity(editor) else {
            self.error = Some("Save props only for the matching admitted local mesh scene".into());
            return;
        };
        if !self.can_edit(editor) || self.scene.as_ref() != Some(&scene) {
            self.error = Some("Save props only for the matching editable local mesh scene".into());
            return;
        }
        if let Err(error) = validate_document(&self.document, revision) {
            self.error = Some(error);
            return;
        }
        let document = (!self.document.props.is_empty()).then_some(&self.document);
        match mesh_props::save(&scene, document, self.sidecar_digest.as_deref(), revision) {
            Ok(digest) => {
                self.sidecar_digest = digest;
                self.saved_history_id = self.current_history_id();
                self.sidecar_error = None;
                self.context_stale = false;
                self.confirm_discard_invalid = false;
                self.confirm_discard_changes = false;
                self.recovery_target = None;
                self.error = None;
                self.status = "Mesh props saved for this scene.".into();
            }
            Err(error) => self.error = Some(format!("Save mesh props failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &Editor) {
        if self.dirty() {
            self.error = Some("Save or undo mesh prop changes before reloading".into());
            return;
        }
        let Some((scene, revision)) = current_identity(editor) else {
            self.error =
                Some("Wait for an admitted local mesh scene before reloading props".into());
            return;
        };
        self.scene = Some(scene.clone());
        self.scene_epoch = Some(editor.scene_load_epoch());
        self.graph_revision = Some(revision);
        self.sidecar_digest = None;
        self.sidecar_error = None;
        self.confirm_discard_invalid = false;
        self.confirm_discard_changes = false;
        self.recovery_target = None;
        self.error = None;
        match mesh_props::load(&scene, revision) {
            Ok(Some((document, digest))) => {
                self.sidecar_digest = Some(digest);
                self.selected = document.props.first().map(|prop| prop.id);
                self.document = document.clone();
                self.copy_selection_fields();
                self.reset_history(document, Some(0));
                self.recompute_next_prop_id();
                self.context_stale = false;
                self.confirm_discard_changes = false;
                self.recovery_target = None;
                self.status = "Reloaded saved mesh props.".into();
            }
            Ok(None) => {
                self.document = Document::default();
                self.selected = None;
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
                self.status = "No saved mesh props for this graph revision.".into();
            }
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.sidecar_digest = mesh_props::sidecar_stamp(&scene).ok().flatten();
                self.error = Some(error.clone());
                self.status = format!("Saved mesh props could not be admitted: {error}");
            }
        }
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        let current_scene = editor.path();
        let scene_matches = current_identity(editor).is_some_and(|(scene, revision)| {
            Some(scene.clone()) == current_scene
                && self.scene.as_ref() == Some(&scene)
                && self.scene_epoch == Some(editor.scene_load_epoch())
                && self.graph_revision == Some(revision)
        });
        discard_invalid_ready(
            self.confirm_discard_invalid,
            editor.game() == EditorGame::MeshNavigation
                && editor.spec().is_local()
                && editor.mode() == Mode::Edit
                && editor.can_mutate()
                && editor.previewing().is_none()
                && editor.proposals().is_empty()
                && !self.document_pending,
            self.sidecar_error.is_some(),
            self.sidecar_digest.is_some(),
            !self.dirty(),
            scene_matches,
        )
    }

    fn can_discard_changes(&self, editor: &Editor) -> bool {
        let editable_context = editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.screenshot_gesture_busy()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && self.gesture.is_none()
            && self.placement.is_none()
            && !self.document_pending
            && current_identity(editor).is_some();
        discard_changes_ready(
            self.confirm_discard_changes,
            editable_context,
            self.dirty() || self.context_stale,
        )
    }

    fn discard_changes_and_reload(&mut self, editor: &Editor) {
        if !self.can_discard_changes(editor) {
            self.confirm_discard_changes = false;
            self.error = Some("Discard is available only for confirmed unsaved props in an idle admitted local mesh scene".into());
            return;
        }
        self.document = Document::default();
        self.candidate = None;
        self.selected = None;
        self.scene = None;
        self.scene_epoch = None;
        self.graph_revision = None;
        self.document_pending = false;
        self.context_stale = false;
        self.sidecar_digest = None;
        self.sidecar_error = None;
        self.confirm_remove = false;
        self.confirm_discard_invalid = false;
        self.confirm_discard_changes = false;
        self.recovery_target = None;
        self.move_enabled = false;
        self.error = None;
        self.reset_history(Document::default(), Some(0));
        self.next_prop_id = 1;
        self.status =
            "Unsaved props discarded in memory; reloading the target scene sidecar.".into();
        self.sync(editor);
    }

    fn discard_invalid_sidecar(&mut self, editor: &Editor) {
        if !self.can_discard_invalid(editor) {
            self.error = Some(
                "Discard is available only for the unchanged invalid sidecar of this editable local mesh scene".into(),
            );
            return;
        }
        let (Some(scene), Some(revision), Some(expected_digest)) = (
            self.scene.as_deref(),
            self.graph_revision,
            self.sidecar_digest.as_deref(),
        ) else {
            self.error = Some("No safely observed invalid mesh props sidecar is available".into());
            return;
        };
        match mesh_props::save(scene, None, Some(expected_digest), revision) {
            Ok(_) => {
                self.document = Document::default();
                self.selected = None;
                self.candidate = None;
                self.sidecar_digest = None;
                self.sidecar_error = None;
                self.context_stale = false;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.status =
                    "Invalid mesh props sidecar discarded; load or add props again explicitly."
                        .into();
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
            }
            Err(error) => {
                self.confirm_discard_invalid = false;
                self.error = Some(format!(
                    "Discard invalid mesh props sidecar failed: {error}"
                ));
            }
        }
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        self.gesture.is_none()
            && self.placement.is_none()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && self.matching_edit_context(editor)
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        self.gesture.is_none()
            && self.placement.is_none()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && document_frame_ready(
                editor.game(),
                editor.spec(),
                editor.path().as_deref(),
                editor.mode(),
                editor.snapshot().is_some(),
                editor.checksum(),
                editor.sim().checksum,
            )
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        self.gesture.is_none()
            && self.placement.is_none()
            && !self.dirty()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && current_identity(editor).is_some()
    }

    fn transformed(&self, snapshot: Snapshot) -> Result<Snapshot, String> {
        let (position, scale, rotation) =
            parse_transform(&self.position, &self.scale, &self.rotation)?;
        let tint = parse_tint(&self.tint)?;
        snapshot
            .with_transform(position, scale)?
            .with_rotation(crate::mesh_rotation::from_degrees(rotation)?)?
            .with_tint(tint)?
            .with_surface(parse_surface(
                self.surface_enabled,
                &self.surface_material,
                &self.surface_uv_offset,
                &self.surface_uv_scale,
                &self.surface_quarter_turns,
                &self.surface_pivot,
            )?)
    }

    fn selected_prop(&self) -> Option<&Prop> {
        self.document
            .props
            .iter()
            .find(|prop| Some(prop.id) == self.selected)
    }

    fn available_prop_id(&self) -> Option<u64> {
        let mut candidate = self.next_prop_id.max(1);
        for _ in 0..=MAX_PROPS {
            if self.document.props.iter().all(|prop| prop.id != candidate) {
                return Some(candidate);
            }
            candidate = candidate.checked_add(1).unwrap_or(1);
        }
        None
    }

    fn push_document(&mut self, document: Document) {
        self.history.truncate(self.cursor + 1);
        let id = self.next_history_id;
        self.next_history_id = self.next_history_id.wrapping_add(1).max(1);
        self.history.push(HistoryEntry {
            id,
            document: document.clone(),
        });
        self.cursor = self.history.len() - 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.document = document;
    }

    fn undo(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.document = self.history[self.cursor].document.clone();
            self.selected = self
                .selected
                .filter(|selected| self.document.props.iter().any(|prop| prop.id == *selected))
                .or_else(|| self.document.props.first().map(|prop| prop.id));
            self.copy_selection_fields();
            self.status = "Undid mesh prop edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn redo(&mut self) {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            self.document = self.history[self.cursor].document.clone();
            self.selected = self
                .selected
                .filter(|selected| self.document.props.iter().any(|prop| prop.id == *selected))
                .or_else(|| self.document.props.first().map(|prop| prop.id));
            self.copy_selection_fields();
            self.status = "Redid mesh prop edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn reset_history(&mut self, document: Document, saved_id: Option<u64>) {
        self.document = document.clone();
        self.history.clear();
        self.history.push(HistoryEntry { id: 0, document });
        self.cursor = 0;
        self.next_history_id = 1;
        self.saved_history_id = saved_id;
    }

    fn current_history_id(&self) -> Option<u64> {
        self.history.get(self.cursor).map(|entry| entry.id)
    }

    fn recompute_next_prop_id(&mut self) {
        self.next_prop_id = self
            .document
            .props
            .iter()
            .map(|prop| prop.id)
            .max()
            .and_then(|id| id.checked_add(1))
            .unwrap_or(1);
    }

    fn copy_selection_fields(&mut self) {
        if let Some((name, snapshot)) = self
            .selected_prop()
            .map(|prop| (prop.name.clone(), prop.art.clone()))
        {
            self.name = name;
            self.copy_snapshot_fields(&snapshot);
        }
    }

    fn copy_snapshot_fields(&mut self, snapshot: &Snapshot) {
        self.position = snapshot.offset().map(format_number);
        self.scale = format_number(snapshot.scale());
        if let Ok(rotation) = crate::mesh_rotation::to_degrees(snapshot.rotation()) {
            self.rotation = rotation.map(format_number);
        }
        self.tint = snapshot.tint().map(format_number);
        if let Some(settings) = snapshot.surface() {
            self.surface_enabled = true;
            self.surface_material = settings.material.to_string();
            self.surface_uv_offset = settings.uv_offset.map(format_number);
            self.surface_uv_scale = settings.uv_scale.map(format_number);
            self.surface_quarter_turns = settings.uv_quarter_turns.to_string();
            self.surface_pivot = settings.pivot.map(format_number);
        } else {
            self.surface_enabled = false;
            self.surface_material = "0".into();
            self.surface_uv_offset = ["0".into(), "0".into()];
            self.surface_uv_scale = ["1".into(), "1".into()];
            self.surface_quarter_turns = "0".into();
            self.surface_pivot = ["0".into(), "0".into(), "0".into()];
        }
    }
}

fn parse_tint(values: &[String; 4]) -> Result<[f32; 4], String> {
    let mut tint = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        tint[index] = value
            .trim()
            .parse::<f32>()
            .map_err(|_| "Prop tint values must be finite RGBA multipliers in 0..=1")?;
    }
    crate::mesh_material::validate(tint)
}

fn parse_surface(
    enabled: bool,
    material: &str,
    uv_offset: &[String; 2],
    uv_scale: &[String; 2],
    uv_quarter_turns: &str,
    pivot: &[String; 3],
) -> Result<Option<crate::mesh_surface::Settings>, String> {
    if !enabled {
        return Ok(None);
    }
    let material = material
        .trim()
        .parse::<u32>()
        .map_err(|_| "UV material index must be a nonnegative integer")?;
    let parse_pair = |values: &[String; 2], name: &str| -> Result<[f32; 2], String> {
        Ok([
            values[0]
                .trim()
                .parse::<f32>()
                .map_err(|_| format!("{name} U must be a number"))?,
            values[1]
                .trim()
                .parse::<f32>()
                .map_err(|_| format!("{name} V must be a number"))?,
        ])
    };
    let uv_offset = parse_pair(uv_offset, "UV offset")?;
    let uv_scale = parse_pair(uv_scale, "UV scale")?;
    let uv_quarter_turns = uv_quarter_turns
        .trim()
        .parse::<u32>()
        .map_err(|_| "UV quarter turns must be an integer from 0 through 3")?;
    let pivot = [
        pivot[0]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot X must be a number")?,
        pivot[1]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot Y must be a number")?,
        pivot[2]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot Z must be a number")?,
    ];
    if uv_offset
        .iter()
        .chain(uv_scale.iter())
        .chain(pivot.iter())
        .any(|value| !value.is_finite())
    {
        return Err("UV and pivot values must be finite".into());
    }
    if uv_quarter_turns > 3 {
        return Err("UV quarter turns must be from 0 through 3".into());
    }
    Ok(Some(crate::mesh_surface::Settings {
        material,
        uv_offset,
        uv_scale,
        uv_quarter_turns,
        pivot,
    }))
}

fn current_identity(editor: &Editor) -> Option<(PathBuf, [u8; 32])> {
    if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
        return None;
    }
    let scene = editor.path()?;
    if !document_frame_ready(
        editor.game(),
        editor.spec(),
        Some(&scene),
        editor.mode(),
        editor.snapshot().is_some(),
        editor.checksum(),
        editor.sim().checksum,
    ) {
        return None;
    }
    let view = mesh_from_view(editor.snapshot()?.predicted()).ok()?;
    Some((scene, view.graph.revision()))
}

fn duplicate_prop_document(
    document: &Document,
    selected: u64,
    new_id: u64,
) -> Result<Document, String> {
    if document.props.len() >= MAX_PROPS {
        return Err(format!("A mesh scene supports at most {MAX_PROPS} props"));
    }
    if new_id == 0 || document.props.iter().any(|prop| prop.id == new_id) {
        return Err("A duplicate needs a fresh nonzero prop identifier".into());
    }
    let source = document
        .props
        .iter()
        .find(|prop| prop.id == selected)
        .ok_or("The selected mesh prop is no longer present")?;
    let preferred = format!("{} copy", source.name);
    let name = if preferred.len() <= 64 && document.props.iter().all(|prop| prop.name != preferred)
    {
        preferred
    } else {
        let fallback = format!("Prop {new_id}");
        if document.props.iter().all(|prop| prop.name != fallback) {
            fallback
        } else {
            (1..=MAX_PROPS)
                .map(|suffix| format!("Prop {new_id}-{suffix}"))
                .find(|candidate| document.props.iter().all(|prop| prop.name != *candidate))
                .ok_or("No unique bounded name is available for the duplicate")?
        }
    };
    let mut duplicated = document.clone();
    duplicated.props.push(Prop {
        id: new_id,
        name: validate_name(&name)?,
        art: source.art.clone(),
    });
    Ok(duplicated)
}

fn relocate_prop_document(
    document: &Document,
    selected: u64,
    position: [orr_fp::FP; 3],
    graph_revision: [u8; 32],
) -> Result<Document, String> {
    let mut relocated = document.clone();
    let prop = relocated
        .props
        .iter_mut()
        .find(|prop| prop.id == selected)
        .ok_or("The selected mesh prop is no longer present")?;
    prop.art = prop
        .art
        .clone()
        .with_transform(position.map(orr_fp::FP::to_f32), prop.art.scale())?;
    validate_document(&relocated, graph_revision)?;
    Ok(relocated)
}

fn discard_invalid_ready(
    confirmed: bool,
    editable_local_scene: bool,
    invalid_sidecar: bool,
    expected_digest_present: bool,
    clean_document: bool,
    matching_scene_epoch_and_revision: bool,
) -> bool {
    confirmed
        && editable_local_scene
        && invalid_sidecar
        && expected_digest_present
        && clean_document
        && matching_scene_epoch_and_revision
}

fn discard_changes_ready(confirmed: bool, editable_context: bool, has_changes: bool) -> bool {
    confirmed && editable_context && has_changes
}

fn reported_scene_matches_request(
    game: EditorGame,
    spec: &HostSpec,
    reported: Option<&Path>,
) -> bool {
    if game != EditorGame::MeshNavigation {
        return false;
    }
    let requested = match spec {
        HostSpec::Local { scene, .. } => scene.as_path(),
        HostSpec::LocalGame {
            scene,
            game: requested_game,
            ..
        } if *requested_game == EditorGame::MeshNavigation => scene.as_path(),
        _ => return false,
    };
    let Some(reported) = reported else {
        return false;
    };
    if requested == reported {
        return true;
    }
    match (
        std::fs::canonicalize(requested),
        std::fs::canonicalize(reported),
    ) {
        (Ok(requested), Ok(reported)) => requested == reported,
        _ => false,
    }
}

fn document_frame_ready(
    game: EditorGame,
    spec: &HostSpec,
    reported: Option<&Path>,
    mode: Mode,
    has_snapshot: bool,
    frame_checksum: u64,
    state_checksum: u64,
) -> bool {
    reported_scene_matches_request(game, spec, reported)
        && has_snapshot
        && (mode == Mode::Play || frame_checksum == state_checksum)
}

fn parse_transform(
    position: &[String; 3],
    scale: &str,
    rotation: &[String; 3],
) -> Result<([f32; 3], f32, [f32; 3]), String> {
    let parsed_position = [
        position[0]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position X must be a number")?,
        position[1]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position Y must be a number")?,
        position[2]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position Z must be a number")?,
    ];
    let parsed_scale = scale
        .trim()
        .parse::<f32>()
        .map_err(|_| "prop scale must be a number")?;
    let parsed_rotation = [
        rotation[0]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop rotation X must be a number")?,
        rotation[1]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop rotation Y must be a number")?,
        rotation[2]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop rotation Z must be a number")?,
    ];
    if parsed_position.iter().any(|value| !value.is_finite())
        || !parsed_scale.is_finite()
        || parsed_rotation.iter().any(|value| !value.is_finite())
    {
        return Err("prop transform values must be finite".into());
    }
    Ok((parsed_position, parsed_scale, parsed_rotation))
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return Err("prop name must be 1-64 printable UTF-8 bytes".into());
    }
    Ok(name.to_owned())
}

fn validate_document(document: &Document, graph_revision: [u8; 32]) -> Result<(), String> {
    if document.props.len() > MAX_PROPS {
        return Err(format!("A mesh scene supports at most {MAX_PROPS} props"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for prop in &document.props {
        if prop.id == 0 || !ids.insert(prop.id) {
            return Err("mesh prop identifiers must be nonzero and unique".into());
        }
        let name = validate_name(&prop.name)?;
        if name != prop.name || !names.insert(name) {
            return Err("mesh prop names must be trimmed and unique".into());
        }
        if prop.art.graph_revision() != graph_revision {
            return Err("mesh prop art belongs to a different graph revision".into());
        }
        prop.art.placement([orr_fp::FP::ZERO; 3])?;
    }
    Ok(())
}

fn field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
        .labelled_by(response.id)
        .changed()
}

fn format_number(value: f32) -> String {
    value.to_string()
}

fn same_document_placements(left: &Document, right: &Document) -> bool {
    left.props.len() == right.props.len()
        && left
            .props
            .iter()
            .zip(&right.props)
            .all(|(left_prop, right_prop)| {
                left_prop.id == right_prop.id
                    && left_prop.name == right_prop.name
                    && left_prop.art.scale() == right_prop.art.scale()
                    && left_prop
                        .art
                        .rotation()
                        .iter()
                        .zip(right_prop.art.rotation())
                        .all(|(left_value, right_value)| {
                            (*left_value - right_value).abs() <= 1.0e-5
                        })
                    && left_prop
                        .art
                        .offset()
                        .iter()
                        .zip(right_prop.art.offset())
                        .all(|(left_value, right_value)| {
                            (*left_value - right_value).abs() <= 1.0e-5
                        })
            })
}

#[cfg(test)]
#[path = "mesh_props_panel/tests.rs"]
mod tests;
