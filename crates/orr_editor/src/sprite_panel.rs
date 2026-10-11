//! Explicit view-sidecar workflow and egui atlas preview. Shape picking remains
//! the host game's existing collider picking; sprites do not change colliders.
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    sprite_bindings::{self, Asset, Binding, Bindings, Orientation, Source},
    sprite_playback::SpritePlayback,
};
use egui::{Color32, Pos2, Rect, TextureHandle, Ui};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

struct Loaded {
    asset: Asset,
    texture: TextureHandle,
}

pub struct SpritePanel {
    pub bindings: Option<Bindings>,
    pub playback: SpritePlayback,
    pub path: String,
    pub scene: String,
    pub project: String,
    pub package: String,
    pub document: String,
    pub source: Source,
    pub scale: f32,
    // None preserves each target; Some(None) explicitly copies identity.
    picked_orientation: Option<Option<Orientation>>,
    loaded: BTreeMap<(String, String), Loaded>,
    error: Option<String>,
    #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
    pub replacement_image: String,
    #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
    pub replacement_version: String,
    preview_playing: bool,
    preview_started: f64,
}
impl Default for SpritePanel {
    fn default() -> Self {
        Self {
            bindings: None,
            playback: SpritePlayback::default(),
            path: String::new(),
            scene: String::new(),
            project: ".".into(),
            package: String::new(),
            document: String::new(),
            source: Source::Region(0),
            scale: 0.05,
            picked_orientation: None,
            loaded: BTreeMap::new(),
            error: None,
            #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
            replacement_image: String::new(),
            #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
            replacement_version: String::new(),
            preview_playing: false,
            preview_started: 0.0,
        }
    }
}
impl SpritePanel {
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        self.bindings.as_ref().is_some_and(|b| {
            editor
                .sim()
                .scene_path
                .as_ref()
                .is_some_and(|scene| b.matches_scene(scene))
        })
    }
    fn load(&mut self, ctx: &egui::Context, package: &str, document: &str) -> Result<(), String> {
        let key = (package.to_string(), document.to_string());
        if self.loaded.contains_key(&key) {
            return Ok(());
        }
        if self.loaded.len() >= 8 {
            return Err(
                "atlas cache limit reached (8 documents); close and reopen with fewer documents"
                    .into(),
            );
        }
        let bindings = self
            .bindings
            .as_ref()
            .ok_or("open or create a view sidecar first")?;
        let asset = sprite_bindings::load_asset(
            &sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?,
            package,
            document,
        )?;
        self.cache_asset(ctx, key, asset);
        Ok(())
    }
    fn cache_asset(&mut self, ctx: &egui::Context, key: (String, String), asset: Asset) {
        let atlas = asset.document.atlas();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [atlas.width as usize, atlas.height as usize],
            &asset.rgba,
        );
        let texture = ctx.load_texture(
            format!("sprite:{}:{}", key.0, key.1),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.loaded.insert(key, Loaded { asset, texture });
    }
    /// Failed open/reload retains the last good document and atlas textures.
    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) -> Result<(), String> {
        if self.bindings.as_ref().is_some_and(Bindings::dirty) {
            return Err(
                "save or explicitly discard the current bindings before opening another sidecar"
                    .into(),
            );
        }
        let candidate = Bindings::open(path)?;
        let assets = Self::load_assets(&candidate)?;
        self.install_prepared(ctx, candidate, assets);
        Ok(())
    }
    /// Install a complete CPU-validated candidate without further filesystem I/O.
    pub(crate) fn install_prepared(
        &mut self,
        ctx: &egui::Context,
        candidate: Bindings,
        assets: BTreeMap<(String, String), Asset>,
    ) {
        self.replace_assets(ctx, assets);
        self.path = candidate.path.to_string_lossy().into_owned();
        self.scene.clone_from(&candidate.document().scene);
        self.project.clone_from(&candidate.document().project);
        self.bindings = Some(candidate);
        self.picked_orientation = None;
        self.playback.reset_document();
        self.preview_playing = false;
        self.error = None;
    }
    pub fn create(&mut self, path: PathBuf, scene: String, project: String) -> Result<(), String> {
        if self.bindings.is_some() {
            return Err("close the current bindings before creating another sidecar".into());
        }
        let candidate = Bindings::create(path, scene, project)?;
        self.path = candidate.path.to_string_lossy().into_owned();
        self.bindings = Some(candidate);
        self.picked_orientation = None;
        self.playback.reset_document();
        self.loaded.clear();
        self.preview_playing = false;
        Ok(())
    }
    fn load_assets(bindings: &Bindings) -> Result<BTreeMap<(String, String), Asset>, String> {
        // Preserve standalone empty-sidecar behavior; saved-project admission
        // always verifies the entire lock, even without bindings or a sidecar.
        if bindings.document().bindings.is_empty() {
            return Ok(BTreeMap::new());
        }
        let root = sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?;
        let project = sprite_bindings::open_project(&root)?;
        Self::load_project_assets(bindings, &project)
    }
    pub(crate) fn load_project_assets(
        bindings: &Bindings,
        project: &orr_package::Project,
    ) -> Result<BTreeMap<(String, String), Asset>, String> {
        orr_sample::project_sprites::load_project_assets(bindings.document(), project)
    }

    fn replace_assets(&mut self, ctx: &egui::Context, assets: BTreeMap<(String, String), Asset>) {
        self.loaded = Self::prepare_loaded(ctx, assets);
    }
    fn prepare_loaded(ctx: &egui::Context, assets: BTreeMap<(String, String), Asset>) -> BTreeMap<(String, String), Loaded> {
        assets.into_iter().map(|(key, asset)| {
            let atlas = asset.document.atlas();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [atlas.width as usize, atlas.height as usize], &asset.rgba,
            );
            let texture = ctx.load_texture(
                format!("sprite:{}:{}", key.0, key.1), image, egui::TextureOptions::NEAREST,
            );
            (key, Loaded { asset, texture })
        }).collect()
    }
    #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
    pub fn reimport_image(&mut self, ctx: &egui::Context, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) || editor.is_dirty() || editor.sim().in_tx
            || editor.screenshot_waiting_for().is_some() || !editor.has_local_screenshot_owner() {
            return Err("image reimport requires a clean settled local scene in Edit".into());
        }
        let bindings = self.bindings.as_ref().ok_or("open saved sprite bindings first")?;
        if bindings.dirty() { return Err("save sprite bindings before image reimport".into()); }
        let root = sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?;
        let transaction = orr_sample::image_reimport::prepare(&orr_sample::image_reimport::Options {
            project: root, package: self.package.clone(), document: self.document.clone(),
            image: PathBuf::from(&self.replacement_image), version: self.replacement_version.clone(),
            consumer: orr_sample::image_reimport::Consumer { collect: editor.game().is_collect(),
                ui: (editor.game() == EditorGame::Arena && cfg!(feature = "project-ui"))
                    || (editor.game().is_collect() && cfg!(feature = "collect-ui")) },
        })?;
        let checked_file = |path: &std::path::Path| -> Result<PathBuf, String> {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
            Ok(sprite_bindings::resolve_project(parent, ".")?.join(path.file_name().ok_or("missing local file name")?))
        };
        let scene_path = editor.sim().scene_path.as_deref().ok_or("saved scene required")?;
        if transaction.document() != bindings.document()
            || transaction.sidecar_path() != checked_file(&bindings.path)?
            || transaction.scene_path() != checked_file(std::path::Path::new(scene_path))?
            || transaction.initial_checksum() != editor.checksum() {
            return Err("saved project differs from the displayed scene or sprite bindings; reopen before reimport".into());
        }
        let loaded = Self::prepare_loaded(ctx, transaction.assets().clone());
        transaction.commit()?;
        // No filesystem reads or fallible operations after active lock publication.
        // Retain Bindings and both Undo stacks, frame, selection and playback cursor.
        self.loaded = loaded;
        Ok(())
    }
    pub fn reload(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let bindings = self.bindings.as_ref().ok_or("open a view sidecar first")?;
        let assets = Self::load_assets(bindings)?;
        self.replace_assets(ctx, assets);
        Ok(())
    }
    /// Restore authoring history only if every restored asset is still valid.
    pub fn undo(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let bindings = self.bindings.as_mut().ok_or("open a view sidecar first")?;
        let before = bindings.document().clone();
        bindings.undo();
        if bindings.document() == &before {
            return Ok(());
        }
        if let Err(error) = self.reload(ctx) {
            self.bindings.as_mut().unwrap().redo();
            return Err(error);
        }
        Ok(())
    }
    pub fn redo(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let bindings = self.bindings.as_mut().ok_or("open a view sidecar first")?;
        let before = bindings.document().clone();
        bindings.redo();
        if bindings.document() == &before {
            return Ok(());
        }
        if let Err(error) = self.reload(ctx) {
            self.bindings.as_mut().unwrap().undo();
            return Err(error);
        }
        Ok(())
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn display_coherent(editor: &Editor) -> bool {
        editor.yard_rows_coherent()
            && editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.timeline().is_some() == editor.is_playing_mode())
    }
    fn editable(&self, editor: &Editor) -> bool {
        (editor.game() == EditorGame::Arena || editor.game().is_collect())
            && self.scene_matches(editor)
            && editor.mode() == Mode::Edit
            && !editor.is_viewer()
            && editor.previewing().is_none()
            && Self::display_coherent(editor)
    }
    /// Copy the selected entity's admitted appearance into the assignment draft.
    /// Picking never refreshes packages or changes bindings/history. Assign still
    /// revalidates installed bytes before committing the draft to a selection.
    pub fn use_selected_settings(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err("sprite picking requires the matching settled scene in Edit".into());
        }
        let [guid] = editor.selected_guids() else {
            return Err("select exactly one entity to pick sprite settings".into());
        };
        let guid = guid.to_string();
        let row = binding_row(editor.rows(), &guid).ok_or("selected entity is unavailable")?;
        if !editor.bodies().iter().any(|body| body.entity == row.entity) {
            return Err("selected entity has no displayed sprite body".into());
        }
        let bindings = self.bindings.as_ref().ok_or("open a view sidecar first")?;
        let binding = bindings.document().bindings.get(&guid)
            .ok_or("selected entity has no sprite binding")?;
        let asset = self.loaded.get(&(binding.package.clone(), binding.document.clone()))
            .ok_or("selected sprite asset is unavailable; reload assets first")?;
        binding.region(&asset.asset.document, 0)?;
        // All fallible checks precede draft mutation, including both locomotion clips.
        self.package.clone_from(&binding.package);
        self.document.clone_from(&binding.document);
        self.source.clone_from(&binding.source);
        self.scale = binding.units_per_pixel;
        self.picked_orientation = Some(binding.orientation);
        self.preview_playing = false;
        Ok(())
    }
    /// Verify the currently installed bytes before committing an assignment.
    pub fn assign(&mut self, ctx: &egui::Context, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err("sprite assignment requires the matching settled scene in Edit".into());
        }
        let bindings = self.bindings.as_ref().ok_or("open a view sidecar first")?;
        let key = (self.package.clone(), self.document.clone());
        let asset = sprite_bindings::load_asset(
            &sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?,
            &self.package,
            &self.document,
        )?;
        let binding = Binding {
            package: self.package.clone(),
            document: self.document.clone(),
            source: self.source.clone(),
            units_per_pixel: self.scale,
            orientation: self.picked_orientation.flatten(),
        };
        binding.region(&asset.document, 0)?;
        // One atlas is shared by every binding of this document. A package
        // refresh must not silently invalidate an unselected entity's clip.
        for (guid, existing) in &bindings.document().bindings {
            if existing.package == self.package
                && existing.document == self.document
                && !editor.selected_guids().iter().any(|g| g.as_str() == guid)
            {
                existing.region(&asset.document, 0)?;
            }
        }
        let mut references: BTreeSet<_> = bindings
            .document()
            .bindings
            .iter()
            .filter(|(guid, _)| {
                !editor
                    .selected_guids()
                    .iter()
                    .any(|g| g.as_str() == guid.as_str())
            })
            .map(|(_, b)| (b.package.clone(), b.document.clone()))
            .collect();
        references.insert(key.clone());
        if references.len() > 8 {
            return Err("sidecar references more than 8 sprite documents".into());
        }
        let bindings = self.bindings.as_mut().unwrap();
        if self.picked_orientation.is_some() {
            bindings.assign(editor.selected_guids(), Some(binding))?;
        } else {
            bindings.assign_preserving_orientation(editor.selected_guids(), &binding)?;
        }
        self.loaded.retain(|key, _| references.contains(key));
        self.cache_asset(ctx, key, asset);
        Ok(())
    }
    fn selected_orientation(&self, editor: &Editor) -> Option<Orientation> {
        if editor.selected_guids().len() != 1 { return None; }
        self.bindings.as_ref()?.document().bindings
            .get(editor.selected_guids()[0].as_str())?.orientation
    }
    /// Immediate current-selection transaction: no retained target draft can be
    /// applied after selection, document or history has changed. Revalidate the
    /// installed source and refuse to edit a stale last-good texture preview.
    pub fn set_selected_orientation(&mut self, editor: &Editor, orientation: Option<Orientation>) -> Result<(), String> {
        if !self.editable(editor) || !editor.spec().is_local() || !editor.can_mutate()
            || editor.sim().in_tx || editor.selected_guids().len() != 1 {
            return Err("sprite orientation requires one selected binding in the matching settled local Edit scene".into());
        }
        if let Some(orientation) = orientation { orientation.validate()?; }
        let guid = &editor.selected_guids()[0];
        let row = binding_row(editor.rows(), guid.as_str()).ok_or("selected sprite entity is unavailable")?;
        if !editor.bodies().iter().any(|body| body.entity == row.entity) {
            return Err("selected sprite entity is absent from the displayed snapshot".into());
        }
        let bindings = self.bindings.as_ref().ok_or("open sprite bindings first")?;
        let binding = bindings.document().bindings.get(guid.as_str()).ok_or("selected entity has no sprite binding")?;
        let key = (binding.package.clone(), binding.document.clone());
        let current = self.loaded.get(&key).ok_or("reload verified sprite assets first")?;
        let fresh = sprite_bindings::load_asset(
            &sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?,
            &binding.package, &binding.document,
        )?;
        binding.region(&fresh.document, 0)?;
        if fresh.rgba != current.asset.rgba
            || fresh.document.to_json().map_err(|e| e.to_string())?
                != current.asset.document.to_json().map_err(|e| e.to_string())? {
            return Err("installed sprite source changed; Reload assets before editing orientation".into());
        }
        self.bindings.as_mut().unwrap().set_orientation(guid, orientation)?;
        self.picked_orientation = None;
        Ok(())
    }
    fn show_orientation(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.selected_guids().len() != 1 { return; }
        let guid = &editor.selected_guids()[0];
        let Some(binding) = self.bindings.as_ref().and_then(|b| b.document().bindings.get(guid.as_str())) else { return; };
        let saved = binding.orientation;
        let mut draft = saved.unwrap_or_default();
        ui.collapsing("Selected sprite orientation", |ui| {
            ui.weak("Mirrors use the original atlas axes, then quarter turns rotate counter-clockwise around the sprite center. Collision stays unchanged.");
            let enabled = self.editable(editor) && editor.spec().is_local() && editor.can_mutate() && !editor.sim().in_tx;
            ui.add_enabled_ui(enabled, |ui| {
                let mut changed = false;
                egui::ComboBox::from_label("Sprite quarter turns")
                    .selected_text(format!("{} degrees", u16::from(draft.quarter_turns) * 90))
                    .show_ui(ui, |ui| {
                        for turns in 0..=3 {
                            changed |= ui.selectable_value(&mut draft.quarter_turns, turns, format!("{} degrees", u16::from(turns) * 90)).changed();
                        }
                    });
                changed |= ui.checkbox(&mut draft.flip_x, "Mirror sprite X").changed();
                changed |= ui.checkbox(&mut draft.flip_y, "Mirror sprite Y").changed();
                let reset = ui.add_enabled(saved.is_some(), egui::Button::new("Reset sprite orientation")).clicked();
                if reset || changed {
                    let result = self.set_selected_orientation(editor, if reset { None } else { Some(draft) });
                    self.report(result);
                }
            });
        });
    }
    pub fn update(&mut self, editor: &mut Editor) {
        let matches = self.scene_matches(editor);
        self.playback.update(
            editor,
            self.bindings.as_ref().map(Bindings::document),
            matches,
        );
    }
    /// The same sampled region used by the production viewport paint pass.
    pub fn sampled_region(&self, editor: &Editor, guid: &str) -> Option<u32> {
        if (editor.game() != EditorGame::Arena && !editor.game().is_collect())
            || !self.scene_matches(editor)
            || editor.previewing().is_some()
            || !Self::display_coherent(editor)
        {
            return None;
        }
        let bindings = self.bindings.as_ref()?;
        let binding = bindings.document().bindings.get(guid)?;
        let row = binding_row(editor.rows(), guid)?;
        editor.bodies().iter().find(|b| b.entity == row.entity)?;
        let asset = self
            .loaded
            .get(&(binding.package.clone(), binding.document.clone()))?;
        self.sample_binding(editor, guid, binding, &asset.asset)
    }
    fn sample_binding(
        &self,
        editor: &Editor,
        guid: &str,
        binding: &Binding,
        asset: &Asset,
    ) -> Option<u32> {
        let state = self.playback.state(guid);
        let elapsed = match &binding.source {
            Source::Locomotion { .. } => state.elapsed_ms,
            _ => editor.snapshot().map_or(0, |s| {
                #[cfg(feature="collect-dodge")]
                if editor.game().is_collect() {
                    return s.timeline().map_or(0, |_| scene_elapsed_ms(Mode::Play, u64::from(s.predicted().singleton::<orr_sample::collect_game::CollectRun>().elapsed_ticks), s.tick_rate()));
                }
                s.timeline().map_or(0, |t| scene_elapsed_ms(Mode::Play, t.tick, s.tick_rate()))
            }),
        };
        binding
            .region_for_motion(&asset.document, elapsed, state.moving)
            .ok()
    }
    fn report(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::Arena && !editor.game().is_collect() {
            return;
        }
        if editor.mode() != Mode::Edit {
            self.preview_playing = false;
        }
        ui.collapsing("Sprite bindings (view only)", |ui| {
            ui.weak("Separate sidecar save and undo. Scene Save does not save bindings. Picking uses existing collider shapes.");
            if self.bindings.is_none() {
                ui.label("Local sidecar path"); ui.text_edit_singleline(&mut self.path);
                ui.label("Scene path relative to sidecar"); ui.text_edit_singleline(&mut self.scene);
                ui.label("Package project relative to sidecar"); ui.text_edit_singleline(&mut self.project);
                ui.weak("Choose local scene/project files. Remote host paths are not local filesystem authority.");
                ui.horizontal(|ui| {
                    if ui.button("Open bindings").clicked() {
                        let result = self.open(ui.ctx(), PathBuf::from(&self.path)); self.report(result);
                    }
                    if ui.button("Create bindings").clicked() {
                        let result = self.create(PathBuf::from(&self.path), self.scene.clone(), self.project.clone()); self.report(result);
                    }
                });
            } else {
                let b = self.bindings.as_ref().unwrap();
                ui.label(format!("{}{}", b.path.display(), if b.dirty() { " · UNSAVED bindings" } else { " · saved" }));
                ui.label(format!("Scene: {} · Project: {}", b.document().scene, b.document().project));
                let matches = self.scene_matches(editor);
                if !matches { ui.colored_label(Color32::YELLOW, "Scene changed or unavailable. Bindings retained; preview/assignment disabled. Save the old sidecar or explicitly discard/close. Save As does not retarget this sidecar."); }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save bindings").clicked() { let result = self.bindings.as_mut().unwrap().save(); self.report(result); }
                    if ui.button("Undo binding").clicked() { let result = self.undo(ui.ctx()); self.report(result); }
                    if ui.button("Redo binding").clicked() { let result = self.redo(ui.ctx()); self.report(result); }
                    if ui.button("Reload assets").clicked() { let result = self.reload(ui.ctx()); self.report(result); }
                });
                let dirty = self.bindings.as_ref().unwrap().dirty();
                if ui.button(if dirty { "Discard unsaved bindings and close" } else { "Close bindings" }).clicked() {
                    self.bindings = None; self.picked_orientation = None; self.loaded.clear(); self.error = None; self.preview_playing = false; self.playback.reset_document(); return;
                }
                ui.separator();
                if ui.add_enabled(self.editable(editor) && editor.selected_guids().len() == 1,
                    egui::Button::new("Use selected sprite settings")).clicked() {
                    let result = self.use_selected_settings(editor); self.report(result);
                }
                ui.weak("Pick settings into the draft, then select targets and Assign. Picking does not change saved bindings.");
                ui.label("Package name"); ui.text_edit_singleline(&mut self.package);
                ui.label("Sprite document path in package"); ui.text_edit_singleline(&mut self.document);
                if ui.button("Load sprite document").clicked() {
                    let package = self.package.clone(); let document = self.document.clone();
                    let result = self.load(ui.ctx(), &package, &document);
                    if result.is_ok() {
                        if let Some(first) = self.loaded[&(package, document)].asset.document.regions().first() { self.source = Source::Region(first.id); }
                    }
                    self.report(result);
                }
                #[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
                ui.collapsing("Explicit PNG replacement", |ui| {
                    ui.weak("Same dimensions and sprite layout only. New immutable package version; not a scene Undo operation.");
                    ui.label("Replacement PNG path"); ui.text_edit_singleline(&mut self.replacement_image);
                    ui.label("New package version"); ui.text_edit_singleline(&mut self.replacement_version);
                    if ui.button("Replace atlas with new package version").clicked() {
                        let result = self.reimport_image(ui.ctx(), editor); self.report(result);
                    }
                });
                let key = (self.package.clone(), self.document.clone());
                if let Some(loaded) = self.loaded.get(&key) {
                    egui::ComboBox::from_label("Sprite source").selected_text(format!("{:?}", self.source)).show_ui(ui, |ui| {
                        for region in loaded.asset.document.regions() { ui.selectable_value(&mut self.source, Source::Region(region.id), format!("Region {}", region.id)); }
                        for clip in loaded.asset.document.clips() { ui.selectable_value(&mut self.source, Source::Clip(clip.id().into()), format!("Clip {}", clip.id())); }
                        let idle = loaded.asset.document.clip("idle").or_else(|| loaded.asset.document.clips().first());
                        let walk = loaded.asset.document.clip("walk").or_else(|| loaded.asset.document.clips().first());
                        if let (Some(idle), Some(walk)) = (idle, walk) {
                            let pair = match &self.source { Source::Locomotion { .. } => self.source.clone(),
                                _ => Source::Locomotion { idle: idle.id().into(), walk: walk.id().into() } };
                            ui.selectable_value(&mut self.source, pair, "Idle / walk");
                        }
                    });
                    if let Source::Locomotion { idle, walk } = &mut self.source {
                        for (label, selected) in [("Idle clip", idle), ("Walk clip", walk)] {
                            egui::ComboBox::from_label(label).selected_text(selected.as_str()).show_ui(ui, |ui| {
                                for clip in loaded.asset.document.clips() { ui.selectable_value(selected, clip.id().to_string(), clip.id()); }
                            });
                        }
                        ui.weak("Movement uses displayed snapshot positions. Seek and discontinuities reset to idle; paused frames never advance.");
                    }
                    ui.add(egui::Slider::new(&mut self.scale, 0.0001..=100.0).logarithmic(true).text("world units/pixel"));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(editor.mode() == Mode::Edit, egui::Button::new("Play sprite preview")).clicked() { self.preview_playing = true; self.preview_started = ui.input(|i| i.time); }
                        if ui.button("Stop sprite preview").clicked() { self.preview_playing = false; }
                    });
                    let binding = Binding { package: self.package.clone(), document: self.document.clone(), source: self.source.clone(), units_per_pixel: self.scale, orientation: self.picked_orientation.unwrap_or_else(|| self.selected_orientation(editor)) };
                    let elapsed = if self.preview_playing { ((ui.input(|i| i.time) - self.preview_started).max(0.0) * 1000.0) as u64 } else { 0 };
                    if self.preview_playing { ui.ctx().request_repaint(); }
                    if let Ok(region) = binding.region(&loaded.asset.document, elapsed) {
                        let uv = loaded.asset.document.uv_rect(region).unwrap();
                        let region = loaded.asset.document.region(region).unwrap();
                        let size = egui::vec2(region.width as f32, region.height as f32);
                        let size = size * (128.0 / size.x.max(size.y));
                        let orientation = binding.orientation.unwrap_or_default();
                        let extent = if orientation.quarter_turns.is_multiple_of(2) { size } else { egui::vec2(size.y, size.x) };
                        let (rect, _) = ui.allocate_exact_size(extent, egui::Sense::hover());
                        paint_oriented_preview(ui, loaded.texture.id(), rect, size, uv, orientation);
                    }
                    let enabled = self.editable(editor) && !editor.selected_guids().is_empty();
                    if ui.add_enabled(enabled, egui::Button::new("Assign sprite to selection")).clicked() {
                        let result = self.assign(ui.ctx(), editor); self.report(result);
                    }
                }
                if let Some(orientation) = self.picked_orientation {
                    let orientation = orientation.unwrap_or_default();
                    ui.weak(format!("Assignment copies orientation: {} degrees, mirror X {}, mirror Y {}", u16::from(orientation.quarter_turns) * 90, orientation.flip_x, orientation.flip_y));
                    if ui.button("Keep target sprite orientations").clicked() {
                        self.picked_orientation = None;
                    }
                }
                self.show_orientation(ui, editor);
                let enabled = self.editable(editor) && !editor.selected_guids().is_empty();
                if ui.add_enabled(enabled, egui::Button::new("Remove selected sprite bindings")).clicked() {
                    let result = self.bindings.as_mut().unwrap().assign(editor.selected_guids(), None); self.report(result);
                }
                ui.separator();
                let follow = self.bindings.as_ref().unwrap().document().camera_follow.as_deref().unwrap_or("none");
                ui.label(format!("Saved camera follow: {follow}"));
                ui.weak("Following runs in Play. Manual pan/zoom suspends it until resumed. Stop restores the Edit camera.");
                if ui.add_enabled(self.editable(editor) && editor.selected_guids().len() == 1,
                    egui::Button::new("Follow selected entity")).clicked() {
                    let result = self.bindings.as_mut().unwrap().set_camera_follow(editor.selected_guid()); self.report(result);
                    self.playback.resume_follow();
                }
                if ui.add_enabled(self.editable(editor), egui::Button::new("Clear camera follow")).clicked() {
                    let result = self.bindings.as_mut().unwrap().set_camera_follow(None); self.report(result);
                }
                if self.playback.follow_suspended() && ui.button("Resume camera follow").clicked() { self.playback.resume_follow(); }
                if let Some(diagnostic) = self.playback.diagnostic() { ui.colored_label(Color32::YELLOW, diagnostic); }
                for guid in editor.selected_guids() {
                    if let Some(binding) = self.bindings.as_ref().unwrap().document().bindings.get(&guid.to_string()) { ui.label(format!("{guid}: {}/{} {:?}", binding.package, binding.document, binding.source)); }
                }
                for error in self.diagnostics(editor) { ui.colored_label(Color32::YELLOW, error); }
            }
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
        });
    }
    /// Errors remain visible until repaired; dangling GUIDs are never reassigned
    /// to an entity that happens to occupy an old frame handle.
    pub fn diagnostics(&self, editor: &Editor) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(bindings) = &self.bindings {
            if let Some(guid) = &bindings.document().camera_follow {
                if binding_row(editor.rows(), guid).is_none() {
                    errors.push(format!(
                        "Camera follow target unavailable: {guid}; holding camera position"
                    ));
                }
            }
            for (guid, binding) in &bindings.document().bindings {
                if !editor
                    .rows()
                    .iter()
                    .any(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
                {
                    errors.push(format!("Orphan or unavailable scene GUID: {guid}"));
                }
                match self
                    .loaded
                    .get(&(binding.package.clone(), binding.document.clone()))
                {
                    None => errors.push(format!(
                        "{guid}: sprite asset unavailable; Reload assets for details"
                    )),
                    Some(asset) => {
                        if let Err(e) = binding.region(&asset.asset.document, 0) {
                            errors.push(format!("{guid}: {e}"));
                        }
                    }
                }
            }
        }
        errors
    }
    pub fn paint(&self, ui: &Ui, editor: &Editor, rect: Rect, pixels: (u32, u32)) {
        if (editor.game() != EditorGame::Arena && !editor.game().is_collect())
            || !self.scene_matches(editor)
            || editor.previewing().is_some()
            || !Self::display_coherent(editor)
        {
            return;
        }
        let Some(bindings) = &self.bindings else {
            return;
        };
        let painter = ui.painter().with_clip_rect(rect);
        for (guid, binding) in &bindings.document().bindings {
            let Some(row) = binding_row(editor.rows(), guid) else {
                continue;
            };
            let Some(body) = editor
                .bodies()
                .iter()
                .find(|body| body.entity == row.entity)
            else {
                continue;
            };
            let Some(asset) = self
                .loaded
                .get(&(binding.package.clone(), binding.document.clone()))
            else {
                continue;
            };
            let Some(id) = self.sample_binding(editor, guid, binding, &asset.asset) else {
                continue;
            };
            let region = asset.asset.document.region(id).unwrap();
            let orientation = binding.orientation.unwrap_or_default();
            let uv = orientation.uv_rect(asset.asset.document.uv_rect(id).unwrap());
            let half = [
                region.width as f32 * binding.units_per_pixel * 0.5,
                region.height as f32 * binding.units_per_pixel * 0.5,
            ];
            let (sin, cos) = (body.angle + orientation.radians()).sin_cos();
            let mut mesh = egui::Mesh::with_texture(asset.texture.id());
            for ([x, y], tex) in [
                ([-half[0], half[1]], [uv[0], uv[1]]),
                ([half[0], half[1]], [uv[2], uv[1]]),
                ([half[0], -half[1]], [uv[2], uv[3]]),
                ([-half[0], -half[1]], [uv[0], uv[3]]),
            ] {
                let screen = editor.camera.world_to_screen(
                    [
                        body.pos[0] + x * cos - y * sin,
                        body.pos[1] + x * sin + y * cos,
                    ],
                    pixels,
                );
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.min
                        + egui::vec2(
                            screen[0] * rect.width() / pixels.0 as f32,
                            screen[1] * rect.height() / pixels.1 as f32,
                        ),
                    uv: Pos2::new(tex[0], tex[1]),
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}
fn paint_oriented_preview(ui: &Ui, texture: egui::TextureId, rect: Rect, size: egui::Vec2, uv: [f32; 4], orientation: Orientation) {
    let uv = orientation.uv_rect(uv);
    let half = size * 0.5;
    let (sin, cos) = orientation.radians().sin_cos();
    let mut mesh = egui::Mesh::with_texture(texture);
    for ([x, y], tex) in [
        ([-half.x, half.y], [uv[0], uv[1]]),
        ([half.x, half.y], [uv[2], uv[1]]),
        ([half.x, -half.y], [uv[2], uv[3]]),
        ([-half.x, -half.y], [uv[0], uv[3]]),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.center() + egui::vec2(x * cos - y * sin, -x * sin - y * cos),
            uv: Pos2::new(tex[0], tex[1]),
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    ui.painter().add(egui::Shape::mesh(mesh));
}
fn binding_row<'a>(
    rows: &'a [crate::model::EntityRow],
    guid: &str,
) -> Option<&'a crate::model::EntityRow> {
    rows.iter()
        .find(|row| row.guid.as_ref().is_some_and(|g| g.as_str() == guid))
}
/// Pausing/rewinding follows the host tick. Stop returns exactly to first frame.
pub fn scene_elapsed_ms(mode: Mode, tick: u64, tick_rate: u32) -> u64 {
    if mode == Mode::Edit {
        0
    } else {
        tick.saturating_mul(1000) / u64::from(tick_rate.max(1))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn play_pause_rewind_stop_use_host_time() {
        assert_eq!(scene_elapsed_ms(Mode::Play, 120, 60), 2000);
        assert_eq!(scene_elapsed_ms(Mode::Play, 120, 60), 2000);
        assert_eq!(scene_elapsed_ms(Mode::Play, 30, 60), 500);
        assert_eq!(scene_elapsed_ms(Mode::Edit, 120, 60), 0);
    }
    #[test]
    fn recycled_handle_never_steals_guid_binding() {
        let entity = orr_ecs::Entity {
            index: 3,
            version: 0,
        };
        let original = crate::model::EntityRow {
            entity,
            guid: Some(orr_reflect::Guid::from_u32(1)),
            name: None,
            components: vec![],
        };
        assert_eq!(
            binding_row(std::slice::from_ref(&original), "e_00000001")
                .unwrap()
                .entity,
            entity
        );
        let replacement = crate::model::EntityRow {
            guid: Some(orr_reflect::Guid::from_u32(2)),
            ..original.clone()
        };
        assert!(binding_row(&[replacement], "e_00000001").is_none());
        assert!(binding_row(&[], "e_00000001").is_none());
        assert!(binding_row(&[original], "e_00000001").is_some());
    }
    /// Real egui widgets and input events, CPU-only: no eframe renderer/window.
    /// The asset comes from the real installed sample package.
    #[test]
    fn headless_widgets_assign_remove_save_and_guard_dirty_scene_switch() {
        use egui_kittest::{kittest::Queryable, Harness};
        let dir = tempfile::tempdir().unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/sprite_demo")
            .canonicalize()
            .unwrap();
        orr_package::Project::open_for_install(
            dir.path(),
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[source])
        .unwrap();
        let scene = dir.path().join("arena.yaml");
        let other = dir.path().join("other.yaml");
        let scene_text = include_str!("../../../scenes/arena_blank.scene.yaml");
        std::fs::write(&scene, scene_text).unwrap();
        std::fs::write(&other, scene_text).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
        editor.sync();
        assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
        editor.sync();
        let checksum = editor.checksum();
        let history_len = editor.history().entries.len();
        let path = dir.path().join("arena.sprites.json");
        let panel = SpritePanel {
            bindings: Some(
                Bindings::create(path.clone(), "arena.yaml".into(), ".".into()).unwrap(),
            ),
            package: "sample-sprites".into(),
            document: "sprites.json".into(),
            source: Source::Region(10),
            ..Default::default()
        };
        let mut h = Harness::builder()
            .with_size([1200.0, 1400.0])
            .build_ui_state(
                |ui, (panel, editor): &mut (SpritePanel, Editor)| {
                    if panel.loaded.is_empty() && panel.bindings.is_some() {
                        panel
                            .load(ui.ctx(), "sample-sprites", "sprites.json")
                            .unwrap();
                    }
                    panel.show(ui, editor);
                },
                (panel, editor),
            );
        h.get_by_label("Sprite bindings (view only)").click();
        h.run_steps(3);
        h.get_by_label("Assign sprite to selection").click();
        h.run_steps(3);
        assert_eq!(
            h.state()
                .0
                .bindings
                .as_ref()
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        h.get_by_label("Save bindings").click();
        h.run_steps(3);
        assert!(!h.state().0.bindings.as_ref().unwrap().dirty());
        assert_eq!(
            Bindings::open(path.clone())
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        h.get_by_label("Remove selected sprite bindings").click();
        h.run_steps(3);
        assert!(h
            .state()
            .0
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .is_empty());
        h.get_by_label("Undo binding").click();
        h.run_steps(3);
        assert_eq!(
            h.state()
                .0
                .bindings
                .as_ref()
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        h.get_by_label("Redo binding").click();
        h.run_steps(3);
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history().entries.len(), history_len);
        assert_eq!(std::fs::read_to_string(&scene).unwrap(), scene_text);
        assert!(h.state_mut().1.open_path(&other));
        h.state_mut().1.sync();
        h.run_steps(3);
        assert!(!h.state().0.scene_matches(&h.state().1));
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        h.get_by_label("Assign sprite to selection").click();
        h.run_steps(3);
        assert!(h
            .state()
            .0
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .is_empty());
        assert_eq!(Bindings::open(path).unwrap().document().bindings.len(), 1);
        h.get_by_label("Discard unsaved bindings and close").click();
        h.run_steps(3);
        assert!(h.state().0.bindings.is_none());
        assert!(h.query_by_label("Create bindings").is_some());
    }
}

#[cfg(test)]
#[path = "sprite_settings_tests.rs"]
mod sprite_settings_tests;
