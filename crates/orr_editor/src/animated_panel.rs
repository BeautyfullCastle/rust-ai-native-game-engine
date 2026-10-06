//! Local, presentation-only animated-model authoring. This never changes the
//! Arena viewport, collider, simulation state, host timeline or scene save.
use crate::{
    animated_bindings::{self, Binding, Bindings, LoadedAsset, PlaybackMode, PlaybackSettings},
    animated_preview::{GpuAnimatedPreview, PreviewPlayers},
    editor::Editor,
    game::EditorGame,
    model::Mode,
};
use egui::{Color32, Ui};
use orr_reflect::Guid;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const MAX_LOADED: usize = 8;
type AssetKey = (String, String);

/// All document state belongs to the separate sidecar; playback is transient.
pub struct AnimatedPanel {
    pub bindings: Option<Bindings>,
    path: String,
    scene: String,
    project: String,
    package: String,
    asset: String,
    clip_index: u32,
    playback: PlaybackSettings,
    loaded: BTreeMap<AssetKey, LoadedAsset>,
    players: PreviewPlayers,
    gpu: Option<GpuAnimatedPreview>,
    error: Option<String>,
    last_time: Option<f64>,
    confirm_discard: bool,
}

impl Default for AnimatedPanel {
    fn default() -> Self {
        Self {
            bindings: None,
            path: String::new(),
            scene: String::new(),
            project: ".".into(),
            package: String::new(),
            asset: String::new(),
            clip_index: 0,
            playback: PlaybackSettings {
                mode: PlaybackMode::Loop,
                speed: 1.0,
            },
            loaded: BTreeMap::new(),
            players: PreviewPlayers::default(),
            gpu: None,
            error: None,
            last_time: None,
            confirm_discard: false,
        }
    }
}

impl AnimatedPanel {
    /// A remote host's path is never used as local filesystem authority.
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        editor.spec().is_local()
            && editor.game() == EditorGame::Arena
            && self.bindings.as_ref().is_some_and(|b| {
                editor
                    .sim()
                    .scene_path
                    .as_ref()
                    .is_some_and(|p| b.matches_scene(p))
            })
    }
    fn editable(&self, editor: &Editor) -> bool {
        self.scene_matches(editor)
            && editor.mode() == Mode::Edit
            && !editor.is_viewer()
            && editor.previewing().is_none()
            && editor.down().is_none()
    }
    fn project_root(&self) -> Result<PathBuf, String> {
        let b = self
            .bindings
            .as_ref()
            .ok_or("Open animation bindings first")?;
        animated_bindings::resolve_project(b.base(), &b.document().project)
    }
    /// Release transient playback and native textures while retaining unsaved bindings.
    pub fn suspend_preview(&mut self) {
        self.reset_preview();
    }
    fn reset_preview(&mut self) {
        self.players = PreviewPlayers::default();
        self.gpu = None;
        self.last_time = None;
    }
    fn close(&mut self) {
        self.bindings = None;
        self.loaded.clear();
        self.reset_preview();
        self.error = None;
        self.confirm_discard = false;
    }
    fn report(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }

    /// Explicitly refresh every referenced asset. Failure removes old cached
    /// models and players, so missing/replaced content cannot keep previewing.
    pub fn reload(&mut self) -> Result<(), String> {
        self.loaded.clear();
        self.reset_preview();
        let keys: BTreeSet<_> = self
            .bindings
            .as_ref()
            .into_iter()
            .flat_map(|b| b.document().bindings.values())
            .map(|b| (b.package.clone(), b.asset.clone()))
            .collect();
        if keys.len() > MAX_LOADED {
            return Err("At most eight animated assets can be previewed".into());
        }
        if keys.is_empty() {
            return Ok(());
        }
        let root = self.project_root()?;
        let mut errors = Vec::new();
        for (package, asset) in keys {
            match animated_bindings::load_asset(&root, &package, &asset) {
                Ok(loaded) => {
                    self.loaded.insert((package, asset), loaded);
                }
                Err(e) => errors.push(format!("{package}/{asset}: {e}")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
    fn load_candidate(&mut self) -> Result<(), String> {
        let key = (self.package.clone(), self.asset.clone());
        if !self.loaded.contains_key(&key) && self.loaded.len() >= MAX_LOADED {
            return Err(
                "Eight-asset preview limit reached; close and reopen to release unused assets"
                    .into(),
            );
        }
        // Fail closed on an explicit failed reload of the current candidate.
        self.loaded.remove(&key);
        self.reset_preview();
        let loaded =
            animated_bindings::load_asset(&self.project_root()?, &self.package, &self.asset)?;
        self.clip_index = 0;
        self.loaded.insert(key, loaded);
        Ok(())
    }
    /// Human-readable diagnostics retain orphan GUIDs without rebinding handles.
    pub fn diagnostics(&self, editor: &Editor) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(b) = &self.bindings {
            for (guid, binding) in &b.document().bindings {
                if !editor
                    .rows()
                    .iter()
                    .any(|r| r.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
                {
                    errors.push(format!("Orphan or unavailable scene GUID: {guid}"));
                }
                match self
                    .loaded
                    .get(&(binding.package.clone(), binding.asset.clone()))
                {
                    None => errors.push(format!(
                        "{guid}: animated asset unavailable; Reload animated assets for details"
                    )),
                    Some(asset) => {
                        if let Err(e) = binding.validate(asset) {
                            errors.push(format!("{guid}: {e}"));
                        }
                    }
                }
            }
        }
        errors
    }
    fn synchronize_players(&mut self, editor: &Editor) -> Result<(), String> {
        let mut keep = BTreeSet::new();
        if self.editable(editor) {
            if let Some(b) = &self.bindings {
                for row in editor.rows() {
                    let Some(guid) = &row.guid else {
                        continue;
                    };
                    let Some(binding) = b.document().bindings.get(&guid.to_string()) else {
                        continue;
                    };
                    let Some(asset) = self
                        .loaded
                        .get(&(binding.package.clone(), binding.asset.clone()))
                    else {
                        continue;
                    };
                    if binding.validate(asset).is_err() {
                        continue;
                    }
                    let mode = match binding.playback.mode {
                        PlaybackMode::Once => orr_model::animation::PlaybackMode::Once,
                        PlaybackMode::Loop => orr_model::animation::PlaybackMode::Loop,
                    };
                    self.players.bind(
                        guid,
                        asset.model().clone(),
                        binding.clip_index,
                        mode,
                        binding.playback.speed,
                    )?;
                    keep.insert(guid.clone());
                }
            }
        }
        self.players.retain(&keep);
        if keep.is_empty() {
            self.gpu = None;
        }
        Ok(())
    }
    /// Exposed for integrations and regression tests, never serialized.
    pub fn player(&self, guid: &Guid) -> Option<&orr_model::animation::AnimationPlayer> {
        self.players.player(guid)
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        if editor.game() != EditorGame::Arena {
            self.reset_preview();
            return;
        }
        let sync = self.synchronize_players(editor);
        if let Err(e) = sync {
            self.error = Some(e);
        }
        let now = ui.input(|i| i.time);
        let dt = self
            .last_time
            .replace(now)
            .map_or(0.0, |last| (now - last).clamp(0.0, 0.1) as f32);
        if let Err(e) = self.players.tick(dt) {
            self.error = Some(e);
        }
        if self.players.is_playing() {
            ui.ctx().request_repaint();
        }
        egui::CollapsingHeader::new("Animated model bindings (presentation only)")
            .default_open(true).show(ui, |ui| {
            ui.weak("Separate binding save/undo. Dedicated animation package project. Preview does not move Arena bodies or change colliders.");
            if !editor.spec().is_local() {
                ui.colored_label(Color32::YELLOW, "Local Arena scene required. Remote host paths cannot authorize local file access.");
                return;
            }
            if self.bindings.is_none() { self.open_controls(ui); } else {
                self.document_controls(ui);
                if self.bindings.is_none() { return; }
                let matches = self.scene_matches(editor);
                if !matches { ui.colored_label(Color32::YELLOW, "Scene changed or unavailable. Unsaved bindings retained; assignment/preview disabled. Save As does not retarget this sidecar."); }
                ui.add_enabled_ui(matches, |ui| self.assignment_controls(ui, editor));
                for error in self.diagnostics(editor) { ui.colored_label(Color32::YELLOW, error); }
                if self.editable(editor) { self.preview_controls(ui, editor, render_state); }
            }
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
        });
    }
    fn open_controls(&mut self, ui: &mut Ui) {
        field(ui, "Animation sidecar path", &mut self.path);
        field(ui, "Animation scene relative path", &mut self.scene);
        field(ui, "Animation project relative path", &mut self.project);
        ui.horizontal(|ui| {
            if ui.button("Open animation bindings").clicked() {
                match Bindings::open(PathBuf::from(&self.path)) {
                    Ok(b) => {
                        self.bindings = Some(b);
                        let r = self.reload();
                        self.report(r);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if ui.button("Create animation bindings").clicked() {
                match Bindings::create(
                    PathBuf::from(&self.path),
                    self.scene.clone(),
                    self.project.clone(),
                ) {
                    Ok(b) => {
                        self.bindings = Some(b);
                        self.loaded.clear();
                        self.reset_preview();
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        });
    }
    fn document_controls(&mut self, ui: &mut Ui) {
        let b = self.bindings.as_ref().unwrap();
        ui.label(format!(
            "{}{}",
            b.path.display(),
            if b.dirty() {
                " · UNSAVED animation bindings"
            } else {
                " · saved"
            }
        ));
        ui.label(format!(
            "Scene: {} · Project: {}",
            b.document().scene,
            b.document().project
        ));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save animation bindings").clicked() {
                let r = self.bindings.as_mut().unwrap().save();
                self.report(r);
            }
            if ui.button("Undo animation binding").clicked() {
                self.bindings.as_mut().unwrap().undo();
            }
            if ui.button("Redo animation binding").clicked() {
                self.bindings.as_mut().unwrap().redo();
            }
            if ui.button("Reload animated assets").clicked() {
                let r = self.reload();
                self.report(r);
            }
            if ui.button("Close animation bindings").clicked() {
                if self.bindings.as_ref().unwrap().dirty() {
                    self.confirm_discard = true;
                } else {
                    self.close();
                }
            }
        });
        if self.confirm_discard {
            ui.colored_label(Color32::YELLOW, "Discard unsaved animation bindings?");
            ui.horizontal(|ui| {
                if ui.button("Discard animation changes and close").clicked() {
                    self.close();
                }
                if ui.button("Keep animation bindings open").clicked() {
                    self.confirm_discard = false;
                }
            });
        }
    }
    fn assignment_controls(&mut self, ui: &mut Ui, editor: &Editor) {
        ui.separator();
        field(ui, "Animated package name", &mut self.package);
        field(ui, "Animated asset path in package", &mut self.asset);
        if ui.button("Load animated model").clicked() {
            let r = self.load_candidate();
            self.report(r);
        }
        let key = (self.package.clone(), self.asset.clone());
        let editable = self.editable(editor) && !editor.selected_guids().is_empty();
        if let Some(loaded) = self.loaded.get(&key) {
            let label = loaded
                .clips()
                .iter()
                .find(|c| c.index == self.clip_index)
                .map_or_else(
                    || "Select a clip".into(),
                    |c| format!("{}: {}", c.index, c.name),
                );
            egui::ComboBox::from_label("Animation clip")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for clip in loaded.clips() {
                        ui.selectable_value(
                            &mut self.clip_index,
                            clip.index,
                            format!("{}: {} ({:.2}s)", clip.index, clip.name, clip.duration),
                        );
                    }
                });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.playback.mode, PlaybackMode::Once, "Once");
                ui.selectable_value(&mut self.playback.mode, PlaybackMode::Loop, "Loop");
            });
            ui.add(egui::Slider::new(&mut self.playback.speed, 0.05..=4.0).text("Preview speed"));
            if ui
                .add_enabled(
                    editable,
                    egui::Button::new("Assign animated clip to selection"),
                )
                .clicked()
            {
                let result = Binding::from_asset(
                    self.package.clone(),
                    self.asset.clone(),
                    loaded,
                    self.clip_index,
                    self.playback,
                )
                .and_then(|binding| {
                    self.bindings.as_mut().unwrap().assign_validated(
                        editor.selected_guids(),
                        &binding,
                        loaded,
                    )
                });
                self.report(result);
            }
        }
        if ui
            .add_enabled(
                editable,
                egui::Button::new("Remove selected animation bindings"),
            )
            .clicked()
        {
            let r = self
                .bindings
                .as_mut()
                .unwrap()
                .remove(editor.selected_guids());
            self.report(r);
        }
        if let Some(b) = &self.bindings {
            for guid in editor.selected_guids() {
                if let Some(binding) = b.document().bindings.get(&guid.to_string()) {
                    ui.label(format!(
                        "{guid}: {}/{} · clip {} · {:?}",
                        binding.package, binding.asset, binding.clip_index, binding.playback.mode
                    ));
                }
            }
        }
    }
    fn preview_controls(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        ui.separator();
        ui.label("3D animation preview · presentation only");
        let Some(guid) = editor.selected_guids().first() else {
            ui.weak("Select a scene entity with an animation binding");
            return;
        };
        let Some(binding) = self
            .bindings
            .as_ref()
            .and_then(|b| b.document().bindings.get(&guid.to_string()))
        else {
            return;
        };
        let Some(asset) = self
            .loaded
            .get(&(binding.package.clone(), binding.asset.clone()))
        else {
            return;
        };
        if binding.validate(asset).is_err() || self.players.player(guid).is_none() {
            return;
        }
        let model = asset.model().clone();
        let duration = asset
            .clips()
            .iter()
            .find(|c| c.index == binding.clip_index)
            .map_or(0.0, |c| c.duration);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Play animation preview").clicked() {
                let r = self.players.play(guid);
                self.report(r);
            }
            if ui.button("Pause animation preview").clicked() {
                let r = self.players.pause(guid);
                self.report(r);
            }
            if ui.button("Stop animation preview").clicked() {
                let r = self.players.stop(guid);
                self.report(r);
            }
        });
        if let Some(player) = self.players.player(guid) {
            let mut time = player.time();
            let playing = player.state() == orr_model::animation::PlaybackState::Playing;
            ui.label(format!("Preview state: {:?}", player.state()));
            if ui
                .add(egui::Slider::new(&mut time, 0.0..=duration).text("Preview time (seconds)"))
                .changed()
            {
                let r = self.players.seek(guid, time);
                self.report(r);
            }
            if playing {
                ui.ctx().request_repaint();
            }
        }
        let Some(state) = render_state else {
            ui.weak("GPU preview unavailable in this headless UI; bindings remain editable");
            return;
        };
        if self.gpu.is_none() {
            self.gpu = Some(GpuAnimatedPreview::new(state));
        }
        let width = ui.available_width().clamp(64.0, 512.0);
        let desired = egui::vec2(width, width * 0.75);
        let ppp = ui.ctx().pixels_per_point();
        let pixels = (
            (desired.x * ppp).round().clamp(1.0, 1024.0) as u32,
            (desired.y * ppp).round().clamp(1.0, 1024.0) as u32,
        );
        match self.players.pose(guid) {
            Ok(Some(pose)) => match self.gpu.as_mut().unwrap().draw(&model, &pose, pixels) {
                Ok(id) => {
                    ui.image((id, desired));
                }
                Err(e) => self.error = Some(e.to_string()),
            },
            Ok(None) => {}
            Err(e) => self.error = Some(e),
        }
    }
}
fn field(ui: &mut Ui, label: &str, text: &mut String) {
    let response = ui.label(label);
    ui.text_edit_singleline(text).labelled_by(response.id);
}
