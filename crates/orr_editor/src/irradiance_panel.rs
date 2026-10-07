//! Local Yard3D irradiance authoring, independent from scene state and ERP undo.
//! This panel deliberately offers no bake, capture, or reflection controls.
use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    irradiance_bindings::{self, IrradianceBindings},
    model::Mode,
};
use egui::Ui;
use orr_render::irradiance::{constant_irradiance, IrradianceGrid, IrradianceProvenance};
use std::path::PathBuf;

pub struct IrradiancePanel {
    pub bindings: Option<IrradianceBindings>,
    pub project: String,
    pub package: String,
    pub asset: String,
    pub import_path: String,
    pub dimensions: [u32; 3],
    pub origin: [f32; 3],
    pub spacing: [f32; 3],
    pub selected_node: [u32; 3],
    /// Linear RGB; the color widget displays these authoring values directly.
    pub color: [f32; 3],
    pub intensity: f32,
    seen_scene: Option<String>,
    confirm_discard: bool,
    error: Option<String>,
}
impl Default for IrradiancePanel {
    fn default() -> Self {
        let grid = IrradianceGrid::default();
        Self {
            bindings: None,
            project: ".".into(),
            package: String::new(),
            asset: String::new(),
            import_path: String::new(),
            dimensions: grid.dimensions,
            origin: grid.origin,
            spacing: grid.spacing,
            selected_node: [0; 3],
            color: [1.0; 3],
            intensity: 1.0,
            seen_scene: None,
            confirm_discard: false,
            error: None,
        }
    }
}
impl IrradiancePanel {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn report<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        self.error = result.as_ref().err().cloned();
        result
    }
    fn local_scene(editor: &Editor) -> Result<PathBuf, String> {
        if editor.game() != EditorGame::Yard3D || !editor.spec().is_local() {
            return Err("Irradiance authoring requires a local Yard3D scene".into());
        }
        let expected = match editor.spec() {
            HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
            HostSpec::Remote { .. } => {
                return Err("Remote paths are not local asset authority".into())
            }
        };
        let reported = editor
            .sim()
            .scene_path
            .as_ref()
            .map(PathBuf::from)
            .ok_or("Save the local scene first")?;
        if &reported != expected {
            return Err("Wait for the local scene path to synchronize".into());
        }
        Ok(expected.clone())
    }
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        Self::local_scene(editor).ok().is_some_and(|scene| {
            self.bindings
                .as_ref()
                .is_some_and(|b| scene.to_str().is_some_and(|s| b.matches_scene(s)))
        })
    }
    fn editable(&self, editor: &Editor) -> bool {
        self.scene_matches(editor)
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.yard_rows_coherent()
    }
    fn require_editable(&self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Irradiance edits require the matching local scene in Edit mode without a preview"
                    .into(),
            );
        }
        Ok(())
    }
    fn refresh_geometry(&mut self) {
        if let Some(bindings) = &self.bindings {
            let grid = &bindings.document().grid;
            self.dimensions = grid.dimensions;
            self.origin = grid.origin;
            self.spacing = grid.spacing;
            for axis in 0..3 {
                self.selected_node[axis] = self.selected_node[axis].min(grid.dimensions[axis] - 1);
            }
        }
    }
    /// Can be called before drawing the viewport, independently of panel visibility.
    pub fn sync_for_editor(&mut self, editor: &Editor) {
        // Leave a new scene pending until it can actually be opened; stopping
        // Play or leaving a preview must not require a second scene change.
        if editor.mode() != Mode::Edit || editor.previewing().is_some() || !editor.can_mutate() {
            return;
        }
        let Ok(scene) = Self::local_scene(editor) else {
            return;
        };
        let key = scene.to_string_lossy().into_owned();
        if self.seen_scene.as_ref() == Some(&key) {
            return;
        }
        self.seen_scene = Some(key);
        if self
            .bindings
            .as_ref()
            .is_some_and(IrradianceBindings::dirty)
        {
            if !self.scene_matches(editor) {
                self.error = Some("Unsaved irradiance retained for the previous scene; its lighting is disabled here. Save or discard it before opening this scene's sidecar".into());
            }
            return;
        }
        if IrradianceBindings::sidecar_path(&scene).exists() {
            let _ = self.open_for_editor(editor, false);
        }
    }
    pub fn open_for_editor(&mut self, editor: &Editor, create: bool) -> Result<(), String> {
        let result = (|| {
            let scene = Self::local_scene(editor)?;
            if editor.mode() != Mode::Edit || !editor.can_mutate() || editor.previewing().is_some()
            {
                return Err("Stop Play and leave previews before opening irradiance".into());
            }
            if self
                .bindings
                .as_ref()
                .is_some_and(IrradianceBindings::dirty)
            {
                return Err(
                    "Save or explicitly discard irradiance changes before opening another sidecar"
                        .into(),
                );
            }
            let path = IrradianceBindings::sidecar_path(&scene);
            let candidate = if create {
                let name = scene
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("Invalid local scene filename")?;
                IrradianceBindings::create(path, name.into())?
            } else {
                IrradianceBindings::open(path)?
            };
            if !scene.to_str().is_some_and(|s| candidate.matches_scene(s)) {
                return Err("Irradiance sidecar names another scene".into());
            }
            self.bindings = Some(candidate);
            self.confirm_discard = false;
            self.refresh_geometry();
            Ok(())
        })();
        self.report(result)
    }
    pub fn grid_for_editor(&self, editor: &Editor) -> Option<&IrradianceGrid> {
        if !self.scene_matches(editor) || !editor.yard_rows_coherent() {
            return None;
        }
        let grid = &self.bindings.as_ref()?.document().grid;
        grid.enabled.then_some(grid)
    }
    pub fn apply_grid_for_editor(
        &mut self,
        editor: &Editor,
        grid: IrradianceGrid,
    ) -> Result<(), String> {
        let result = self.require_editable(editor).and_then(|()| {
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .replace_grid(grid)
        });
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn import_json_for_editor(&mut self, editor: &Editor, bytes: &[u8]) -> Result<(), String> {
        let result = self.require_editable(editor).and_then(|()| {
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .import_json(bytes)
        });
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn import_package_for_editor(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_editable(editor)?;
            let bindings = self.bindings.as_ref().ok_or("Open irradiance first")?;
            let root = bindings.base().join(&self.project);
            let grid = irradiance_bindings::load_package_grid(&root, &self.package, &self.asset)?;
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .replace_grid(grid)
        })();
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn save(&mut self) -> Result<(), String> {
        let result = self
            .bindings
            .as_mut()
            .ok_or_else(|| "Open irradiance first".into())
            .and_then(IrradianceBindings::save);
        self.report(result)
    }
    pub fn reload(&mut self, editor: &Editor) -> Result<(), String> {
        let result = self.require_editable(editor).and_then(|()| {
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .reload()
        });
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn undo(&mut self, editor: &Editor) -> Result<(), String> {
        let result = self.require_editable(editor).map(|()| {
            if let Some(bindings) = &mut self.bindings {
                bindings.undo();
            }
        });
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn redo(&mut self, editor: &Editor) -> Result<(), String> {
        let result = self.require_editable(editor).map(|()| {
            if let Some(bindings) = &mut self.bindings {
                bindings.redo();
            }
        });
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn apply_geometry(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_editable(editor)?;
            if !self.dimensions.iter().all(|d| (2..=4).contains(d)) {
                return Err("Each irradiance dimension must be 2..=4".into());
            }
            let old = &self
                .bindings
                .as_ref()
                .ok_or("Open irradiance first")?
                .document()
                .grid;
            let mut grid = old.clone();
            grid.dimensions = self.dimensions;
            grid.origin = self.origin;
            grid.spacing = self.spacing;
            grid.provenance = IrradianceProvenance::Authored;
            grid.coefficients.clear();
            // Preserve authored nodes; newly added edge nodes copy their nearest old edge.
            for z in 0..grid.dimensions[2] {
                for y in 0..grid.dimensions[1] {
                    for x in 0..grid.dimensions[0] {
                        let node = [
                            x.min(old.dimensions[0] - 1),
                            y.min(old.dimensions[1] - 1),
                            z.min(old.dimensions[2] - 1),
                        ];
                        grid.coefficients.push(
                            old.coefficients[old.node_index(node).ok_or("Invalid source node")?],
                        );
                    }
                }
            }
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .replace_grid(grid)
        })();
        if result.is_ok() {
            self.refresh_geometry();
        }
        self.report(result)
    }
    pub fn set_node_constant(&mut self, editor: &Editor, all_nodes: bool) -> Result<(), String> {
        let result = (|| {
            self.require_editable(editor)?;
            if !self.intensity.is_finite() || self.intensity < 0.0 {
                return Err("Irradiance intensity must be finite and nonnegative".into());
            }
            if !self.color.iter().all(|v| v.is_finite() && *v >= 0.0) {
                return Err("Irradiance color must be finite and nonnegative".into());
            }
            let coefficients = constant_irradiance(self.color.map(|c| c * self.intensity))?;
            let mut grid = self
                .bindings
                .as_ref()
                .ok_or("Open irradiance first")?
                .document()
                .grid
                .clone();
            if all_nodes {
                grid.coefficients.fill(coefficients);
            } else {
                let index = grid
                    .node_index(self.selected_node)
                    .ok_or("Selected irradiance node is outside the grid")?;
                grid.coefficients[index] = coefficients;
            }
            grid.provenance = IrradianceProvenance::Authored;
            self.bindings
                .as_mut()
                .ok_or("Open irradiance first")?
                .replace_grid(grid)
        })();
        self.report(result)
    }
    pub fn discard(&mut self) {
        self.bindings = None;
        self.confirm_discard = false;
        self.error = None;
    }
    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::Yard3D {
            return;
        }
        self.sync_for_editor(editor);
        ui.collapsing("Irradiance probes", |ui| {
            ui.weak("Authored/imported diffuse SH9 · separate sidecar save and undo");
            if !editor.spec().is_local() {
                ui.label("Remote host paths cannot open local irradiance assets");
            }
            ui.add_enabled_ui(
                editor.spec().is_local() && editor.mode() == Mode::Edit && editor.can_mutate(),
                |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Create irradiance").clicked() {
                            let _ = self.open_for_editor(editor, true);
                        }
                        if ui.button("Open irradiance").clicked() {
                            let _ = self.open_for_editor(editor, false);
                        }
                    });
                },
            );
            if self.bindings.is_some() {
                if !self.scene_matches(editor) {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Different scene: irradiance disabled here; previous changes retained",
                    );
                }
                let editable = self.editable(editor);
                ui.add_enabled_ui(editable, |ui| {
                    let mut enabled = self
                        .bindings
                        .as_ref()
                        .is_some_and(|b| b.document().grid.enabled);
                    if ui
                        .checkbox(&mut enabled, "Enable irradiance probes")
                        .changed()
                    {
                        if let Some(bindings) = &self.bindings {
                            let mut grid = bindings.document().grid.clone();
                            grid.enabled = enabled;
                            let _ = self.apply_grid_for_editor(editor, grid);
                        }
                    }
                    if let Some(bindings) = &self.bindings {
                        ui.weak(format!(
                            "{:?} · {} nodes · linear irradiance, one-cell boundary fade",
                            bindings.document().grid.provenance,
                            bindings.document().grid.coefficients.len()
                        ));
                    }
                    ui.horizontal(|ui| {
                        ui.label("Dimensions xyz");
                        for d in &mut self.dimensions {
                            ui.add(egui::DragValue::new(d).range(2..=4));
                        }
                    });
                    for (label, values) in [
                        ("Grid origin", &mut self.origin),
                        ("Cell spacing", &mut self.spacing),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            for v in values {
                                ui.add(egui::DragValue::new(v).speed(0.1));
                            }
                        });
                    }
                    if ui.button("Apply grid geometry").clicked() {
                        let _ = self.apply_geometry(editor);
                    }
                    ui.horizontal(|ui| {
                        ui.label("Node xyz");
                        for axis in 0..3 {
                            let maximum = self
                                .bindings
                                .as_ref()
                                .map_or(1, |b| b.document().grid.dimensions[axis] - 1);
                            ui.add(
                                egui::DragValue::new(&mut self.selected_node[axis])
                                    .range(0..=maximum),
                            );
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Linear RGB");
                        for c in &mut self.color {
                            ui.add(egui::DragValue::new(c).speed(0.01).range(0.0..=10000.0));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Intensity");
                        ui.add(
                            egui::DragValue::new(&mut self.intensity)
                                .speed(0.05)
                                .range(0.0..=10000.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Set node constant").clicked() {
                            let _ = self.set_node_constant(editor, false);
                        }
                        if ui.button("Set all nodes constant").clicked() {
                            let _ = self.set_node_constant(editor, true);
                        }
                    });
                    ui.collapsing("Selected SH9 coefficients", |ui| {
                        if let Some(grid) = self.bindings.as_ref().map(|b| &b.document().grid) {
                            if let Some(index) = grid.node_index(self.selected_node) {
                                for (i, c) in grid.coefficients[index].iter().enumerate() {
                                    ui.monospace(format!(
                                        "{i}: [{:.5}, {:.5}, {:.5}]",
                                        c[0], c[1], c[2]
                                    ));
                                }
                            }
                        }
                        ui.weak("Import full signed SH9 JSON to retain directional irradiance");
                    });
                    ui.collapsing("Import full SH9", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("JSON file");
                            ui.text_edit_singleline(&mut self.import_path);
                        });
                        if ui.button("Import irradiance JSON").clicked() {
                            let result = (|| {
                                let base = self
                                    .bindings
                                    .as_ref()
                                    .ok_or("Open irradiance first")?
                                    .base();
                                let bytes = irradiance_bindings::read_bounded(
                                    &base.join(&self.import_path),
                                )?;
                                self.import_json_for_editor(editor, &bytes)
                            })();
                            let _ = self.report(result);
                        }
                        for (label, value) in [
                            ("Project", &mut self.project),
                            ("Package", &mut self.package),
                            ("Asset", &mut self.asset),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.text_edit_singleline(value);
                            });
                        }
                        if ui.button("Import verified irradiance package").clicked() {
                            let _ = self.import_package_for_editor(editor);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Undo irradiance").clicked() {
                            let _ = self.undo(editor);
                        }
                        if ui.button("Redo irradiance").clicked() {
                            let _ = self.redo(editor);
                        }
                        if ui.button("Reload irradiance").clicked() {
                            let _ = self.reload(editor);
                        }
                    });
                });
                // Original sidecar remains saveable after scene Save As.
                if ui.button("Save irradiance").clicked() {
                    let _ = self.save();
                }
                if self
                    .bindings
                    .as_ref()
                    .is_some_and(IrradianceBindings::dirty)
                {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Unsaved irradiance changes; scene Save does not save these",
                    );
                }
                if self.confirm_discard {
                    ui.label("Discard unsaved irradiance and close?");
                    ui.horizontal(|ui| {
                        if ui.button("Discard irradiance changes and close").clicked() {
                            self.discard();
                        }
                        if ui.button("Keep irradiance open").clicked() {
                            self.confirm_discard = false;
                        }
                    });
                } else if ui.button("Close irradiance").clicked() {
                    if self
                        .bindings
                        .as_ref()
                        .is_some_and(IrradianceBindings::dirty)
                    {
                        self.confirm_discard = true;
                    } else {
                        self.discard();
                    }
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    fn scene(path: &Path) {
        std::fs::write(
            path,
            include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
        )
        .unwrap();
    }
    fn settle(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            editor.sync();
            if editor.yard_rows_coherent()
                && editor
                    .snapshot()
                    .is_some_and(|s| s.timeline().is_some() == (editor.mode() == Mode::Play))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "matching Yard snapshot did not arrive"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn enabled_panel(editor: &Editor) -> IrradiancePanel {
        let mut panel = IrradiancePanel::default();
        panel.open_for_editor(editor, true).unwrap();
        let mut grid = panel.bindings.as_ref().unwrap().document().grid.clone();
        grid.enabled = true;
        grid.coefficients
            .fill(constant_irradiance([2.0, 1.0, 0.5]).unwrap());
        panel.apply_grid_for_editor(editor, grid).unwrap();
        assert!(panel.grid_for_editor(editor).is_some());
        panel
    }

    #[test]
    fn real_scene_switch_retains_dirty_document_and_disables_application() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original.yaml");
        let next = temp.path().join("next.yaml");
        scene(&original);
        scene(&next);
        let mut editor = Editor::open_game(&original, EditorGame::Yard3D).unwrap();
        settle(&mut editor);
        let mut panel = enabled_panel(&editor);
        let accepted = panel.bindings.as_ref().unwrap().document().clone();
        let path = panel.bindings.as_ref().unwrap().path.clone();
        assert!(editor.open_path(&next));
        settle(&mut editor);
        panel.sync_for_editor(&editor);
        assert!(panel.grid_for_editor(&editor).is_none());
        assert!(!panel.scene_matches(&editor));
        assert!(panel.bindings.as_ref().unwrap().dirty());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &accepted);
        assert_eq!(panel.bindings.as_ref().unwrap().path, path);
        assert!(panel.error().is_some());
        assert!(panel
            .apply_grid_for_editor(&editor, IrradianceGrid::default())
            .is_err());
        assert!(panel.open_for_editor(&editor, true).is_err());
        panel.save().unwrap();
        assert!(!IrradianceBindings::sidecar_path(&next).exists());
        assert_eq!(
            IrradianceBindings::open(path).unwrap().document(),
            &accepted
        );
        assert!(editor.open_path(&original));
        settle(&mut editor);
        panel.sync_for_editor(&editor);
        assert!(panel.grid_for_editor(&editor).is_some());
    }

    #[test]
    fn real_save_as_keeps_previous_dirty_sidecar_saveable_and_does_not_retarget() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original.yaml");
        scene(&original);
        let next = temp.path().join("saved-as.yaml");
        let mut editor = Editor::open_game(&original, EditorGame::Yard3D).unwrap();
        settle(&mut editor);
        let mut panel = enabled_panel(&editor);
        let accepted = panel.bindings.as_ref().unwrap().document().clone();
        assert!(editor.save_as(&next));
        settle(&mut editor);
        panel.sync_for_editor(&editor);
        assert!(panel.bindings.as_ref().unwrap().dirty());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &accepted);
        assert!(panel.grid_for_editor(&editor).is_none());
        panel.save().unwrap();
        assert!(!panel.bindings.as_ref().unwrap().dirty());
        assert!(IrradianceBindings::sidecar_path(&original).exists());
        assert!(!IrradianceBindings::sidecar_path(&next).exists());
    }

    #[test]
    fn remote_host_reported_local_path_is_never_local_asset_authority() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("yard.yaml");
        scene(&path);
        let spec = HostSpec::local_game(&path, EditorGame::Yard3D)
            .with_listener(orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth));
        let mut local = Editor::start(&spec).unwrap();
        settle(&mut local);
        let mut panel = enabled_panel(&local);
        panel.save().unwrap();
        let accepted = panel.bindings.as_ref().unwrap().document().clone();
        let url = local.erp_status().unwrap().0;
        let mut remote = Editor::attach(&url, None).unwrap();
        settle(&mut remote);
        assert_eq!(remote.game(), EditorGame::Yard3D);
        assert_eq!(remote.sim().scene_path, local.sim().scene_path);
        assert!(IrradiancePanel::local_scene(&remote).is_err());
        assert!(panel.grid_for_editor(&remote).is_none());
        assert!(panel.open_for_editor(&remote, false).is_err());
        assert!(panel
            .import_json_for_editor(&remote, &accepted.grid.to_json().unwrap())
            .is_err());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &accepted);
        assert!(panel.error().is_some());
        let mut unopened = IrradiancePanel::default();
        unopened.sync_for_editor(&remote);
        assert!(unopened.bindings.is_none());
    }

    #[test]
    fn play_and_proposal_preview_reject_authoring_without_changing_history() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("yard.yaml");
        scene(&path);
        let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
        settle(&mut editor);
        let mut panel = enabled_panel(&editor);
        panel.save().unwrap();
        let accepted = panel.bindings.as_ref().unwrap().document().clone();
        let history = editor.history().entries.len();
        let checksum = editor.checksum();
        assert!(editor.start_play());
        settle(&mut editor);
        assert!(panel
            .apply_grid_for_editor(&editor, IrradianceGrid::default())
            .is_err());
        assert!(panel.set_node_constant(&editor, false).is_err());
        assert!(panel.undo(&editor).is_err());
        assert!(panel.redo(&editor).is_err());
        assert!(panel.open_for_editor(&editor, false).is_err());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &accepted);
        assert!(!panel.bindings.as_ref().unwrap().dirty());
        editor.stop();
        settle(&mut editor);
        let agent = editor.agent_client("irradiance-preview-test").unwrap();
        let id = agent
            .call(
                "proposal.begin",
                serde_json::json!({"label":"irradiance preview guard"}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        editor.sync();
        assert!(editor.set_preview(Some(id)));
        editor.sync();
        assert!(editor.previewing().is_some());
        assert!(panel
            .import_json_for_editor(&editor, &accepted.grid.to_json().unwrap())
            .is_err());
        assert!(panel.apply_geometry(&editor).is_err());
        assert!(panel.undo(&editor).is_err());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &accepted);
        assert_eq!(editor.history().entries.len(), history);
        assert_eq!(editor.checksum(), checksum);
        assert!(editor.set_preview(None));
        settle(&mut editor);
        panel.undo(&editor).unwrap();
        assert!(panel.bindings.as_ref().unwrap().dirty());
    }
}
