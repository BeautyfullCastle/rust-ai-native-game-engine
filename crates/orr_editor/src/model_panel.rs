//! Yard3D static and animated model bindings. Sidecar history/save is explicitly
//! separate from the host document; animation samples only the host snapshot.
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    model_bindings::{self, Binding, Bindings, LoadedAsset, LocalTransform, ModelKind},
    viewport3d::ModelPlacement,
};
#[cfg(feature = "animated-models")]
use crate::{
    model_bindings::{AnimationDescriptor, PlaybackMode},
    viewport3d::AnimatedPlacement,
};
use egui::Ui;
use std::{collections::BTreeMap, path::PathBuf};

pub struct ModelPanel {
    pub bindings: Option<Bindings>,
    pub project: String,
    pub package: String,
    pub asset: String,
    pub transform: LocalTransform,
    /// Explicit requested model kind; animation support is selected only when
    /// the `animated-models` feature is compiled.
    pub kind: ModelKind,
    #[cfg(feature = "animated-models")]
    /// Persisted clip choice and once/loop policy for animated bindings.
    pub animation: AnimationDescriptor,
    loaded: BTreeMap<String, LoadedAsset>,
    candidate: Option<(String, String, ModelKind, LoadedAsset)>,
    confirm_discard: bool,
    seen_scene: Option<String>,
    error: Option<String>,
}
impl Default for ModelPanel {
    fn default() -> Self {
        Self {
            bindings: None,
            project: ".".into(),
            package: String::new(),
            asset: String::new(),
            transform: LocalTransform::default(),
            kind: ModelKind::Static,
            #[cfg(feature = "animated-models")]
            animation: AnimationDescriptor {
                clip_index: 0,
                playback: PlaybackMode::Once,
            },
            loaded: BTreeMap::new(),
            candidate: None,
            confirm_discard: false,
            seen_scene: None,
            error: None,
        }
    }
}
impl ModelPanel {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        editor.spec().is_local()
            && editor.game() == EditorGame::Yard3D
            && self.bindings.as_ref().is_some_and(|b| {
                let local_path = match editor.spec() {
                    crate::backend::HostSpec::Local { scene, .. }
                    | crate::backend::HostSpec::LocalGame { scene, .. } => scene,
                    crate::backend::HostSpec::Remote { .. } => return false,
                };
                if !local_path
                    .to_str()
                    .is_some_and(|path| b.matches_scene(path))
                {
                    return false;
                }

                editor
                    .sim()
                    .scene_path
                    .as_deref()
                    .is_some_and(|p| b.matches_scene(p))
            })
    }
    fn editable(&self, editor: &Editor) -> bool {
        self.scene_matches(editor)
            && editor.yard_rows_coherent()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
    }
    fn local_scene(editor: &Editor) -> Result<PathBuf, String> {
        if !editor.spec().is_local() || editor.game() != EditorGame::Yard3D {
            return Err("Model bindings require a local Yard3D scene".into());
        }
        let expected = match editor.spec() {
            crate::backend::HostSpec::Local { scene, .. }
            | crate::backend::HostSpec::LocalGame { scene, .. } => scene,
            crate::backend::HostSpec::Remote { .. } => {
                return Err("Remote scene paths are not local authority".into())
            }
        };
        let reported = editor
            .sim()
            .scene_path
            .as_ref()
            .map(PathBuf::from)
            .ok_or("Save the local scene first")?;
        if &reported != expected {
            return Err(
                "Wait for the local scene path to synchronize before opening bindings".into(),
            );
        }
        Ok(expected.clone())
    }
    fn sidecar(scene: &std::path::Path) -> PathBuf {
        let mut path = scene.as_os_str().to_os_string();
        path.push(".models.json");
        PathBuf::from(path)
    }
    /// Atomic opening: invalid sidecars/assets leave the current document/cache intact.
    pub fn open_for_editor(&mut self, editor: &Editor, create: bool) -> Result<(), String> {
        let scene = Self::local_scene(editor)?;
        if editor.mode() != Mode::Edit {
            return Err("Stop Play before opening model bindings".into());
        }
        if self.bindings.as_ref().is_some_and(Bindings::dirty) {
            return Err(
                "Save model bindings or explicitly discard them before opening another sidecar"
                    .into(),
            );
        }
        let path = Self::sidecar(&scene);
        let candidate = if create {
            if path.exists() {
                return Err("Bindings already exist; use Open model bindings".into());
            }
            let name = scene
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("Invalid local scene filename")?;
            Bindings::create(path, name.into(), self.project.clone())?
        } else {
            Bindings::open(path)?
        };
        if !candidate.matches_scene(&scene.to_string_lossy()) {
            return Err("Model sidecar names another scene".into());
        }
        let loaded = Self::load_all(&candidate)?;
        Self::validate_candidate(editor, candidate.document(), &loaded)?;
        self.bindings = Some(candidate);
        self.loaded = loaded;
        self.candidate = None;
        self.error = None;
        Ok(())
    }
    fn load_all(bindings: &Bindings) -> Result<BTreeMap<String, LoadedAsset>, String> {
        if bindings.document().bindings.len() > 256 {
            return Err("Viewport supports at most 256 model-bound entities".into());
        }
        let mut out = BTreeMap::new();
        let mut assets: BTreeMap<(bool, String, String, String, String), LoadedAsset> =
            BTreeMap::new();
        let root = bindings.project_root()?;
        for (guid, binding) in &bindings.document().bindings {
            let key = (
                binding.kind == ModelKind::Animated,
                binding.package.clone(),
                binding.asset.clone(),
                binding.package_digest.clone(),
                binding.source_hash.clone(),
            );
            if !assets.contains_key(&key) {
                if assets.len() >= 8 {
                    return Err(
                        "Viewport supports at most eight distinct verified model assets".into(),
                    );
                }
                assets.insert(key.clone(), model_bindings::load_binding(&root, binding)?);
            }
            out.insert(guid.clone(), assets[&key].clone());
        }
        Ok(out)
    }
    pub fn reload(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Reload model assets is disabled during Play, previews and remote sessions".into(),
            );
        }
        let bindings = self.bindings.as_ref().ok_or("Open model bindings first")?;
        let loaded = Self::load_all(bindings)?;
        Self::validate_candidate(editor, bindings.document(), &loaded)?;
        self.loaded = loaded;
        Ok(())
    }
    /// Loads exactly the currently selected kind/package/path into a transient
    /// candidate. A failed replacement drops the previous candidate so stale
    /// content cannot be assigned accidentally.
    pub fn load_candidate(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Loading model assets is disabled during Play, previews and remote sessions".into(),
            );
        }
        self.candidate = None;
        let root = self
            .bindings
            .as_ref()
            .ok_or("Open model bindings first")?
            .project_root()?;
        let loaded =
            model_bindings::load_asset_for_kind(&root, &self.package, &self.asset, self.kind)?;
        self.candidate = Some((self.package.clone(), self.asset.clone(), self.kind, loaded));
        Ok(())
    }
    pub fn assign(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Model assignment is disabled during Play, previews and remote sessions".into(),
            );
        }
        let guid = editor
            .selected_guid()
            .ok_or("Select a persistent body GUID")?;
        let row = editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref() == Some(&guid))
            .ok_or("Selected GUID no longer exists")?;
        if !editor
            .yard_frame()
            .poses
            .iter()
            .any(|(entity, _)| *entity == row.entity)
        {
            return Err("Selected entity has no Yard3D body pose".into());
        }
        let project_root = self
            .bindings
            .as_ref()
            .ok_or("Open model bindings first")?
            .project_root()?;
        // Re-read and verify the requested package contents at assignment
        // time. The transient candidate only drives clip UI; field edits or an
        // installed-package replacement cannot turn that preview cache into
        // assignment authority.
        let loaded = model_bindings::load_asset_for_kind(
            &project_root,
            &self.package,
            &self.asset,
            self.kind,
        )?;
        if self
            .candidate
            .as_ref()
            .is_some_and(|(package, asset, kind, candidate)| {
                package == &self.package
                    && asset == &self.asset
                    && *kind == self.kind
                    && (candidate.package_digest() != loaded.package_digest()
                        || candidate.source_hash() != loaded.source_hash())
            })
        {
            self.candidate = None;
            return Err("Installed model asset changed since it was loaded; reload it and confirm the clip selection before assigning".into());
        }
        let loaded = self
            .loaded
            .values()
            .find(|old| {
                old.kind() == loaded.kind()
                    && old.package() == loaded.package()
                    && old.asset() == loaded.asset()
                    && old.package_digest() == loaded.package_digest()
                    && old.source_hash() == loaded.source_hash()
            })
            .cloned()
            .unwrap_or(loaded);
        let binding = match self.kind {
            ModelKind::Static => Binding::from_asset(
                self.package.clone(),
                self.asset.clone(),
                &loaded,
                self.transform,
            )?,
            ModelKind::Animated => {
                #[cfg(feature = "animated-models")]
                {
                    Binding::from_animated_asset(
                        self.package.clone(),
                        self.asset.clone(),
                        &loaded,
                        self.animation,
                        self.transform,
                    )?
                }
                #[cfg(not(feature = "animated-models"))]
                {
                    return Err(
                        "Animated model bindings require the animated-models feature".into(),
                    );
                }
            }
        };
        let bindings = self.bindings.as_mut().ok_or("Open model bindings first")?;
        if bindings.document().bindings.len() >= 256
            && !bindings.document().bindings.contains_key(&guid.to_string())
        {
            return Err("Viewport model binding limit reached".into());
        }
        let mut candidate = bindings.document().clone();
        candidate.bindings.insert(guid.to_string(), binding.clone());
        let mut candidate_loaded = self.loaded.clone();
        candidate_loaded.insert(guid.to_string(), loaded.clone());
        candidate_loaded.retain(|guid, _| candidate.bindings.contains_key(guid));
        Self::validate_candidate(editor, &candidate, &candidate_loaded)?;
        bindings.assign_validated(std::slice::from_ref(&guid), &binding, &loaded)?;
        self.loaded = candidate_loaded;
        Ok(())
    }
    /// Sidecar undo/redo is independent of scene undo and preserves the previous
    /// valid binding/cache if restored content cannot be verified.
    pub fn undo(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Model binding undo is disabled during Play, previews and remote sessions".into(),
            );
        }
        let bindings = self.bindings.as_mut().ok_or("Open model bindings first")?;
        let previous = bindings.document().clone();
        bindings.undo();
        match Self::load_all(bindings).and_then(|loaded| {
            Self::validate_candidate(editor, bindings.document(), &loaded)?;
            Ok(loaded)
        }) {
            Ok(loaded) => {
                self.loaded = loaded;
                Ok(())
            }
            Err(error) => {
                if bindings.document() != &previous {
                    bindings.redo();
                }
                Err(error)
            }
        }
    }
    pub fn redo(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err(
                "Model binding redo is disabled during Play, previews and remote sessions".into(),
            );
        }
        let bindings = self.bindings.as_mut().ok_or("Open model bindings first")?;
        let previous = bindings.document().clone();
        bindings.redo();
        match Self::load_all(bindings).and_then(|loaded| {
            Self::validate_candidate(editor, bindings.document(), &loaded)?;
            Ok(loaded)
        }) {
            Ok(loaded) => {
                self.loaded = loaded;
                Ok(())
            }
            Err(error) => {
                if bindings.document() != &previous {
                    bindings.undo();
                }
                Err(error)
            }
        }
    }
    fn instance(pose: orr_view::Transform3, local: LocalTransform) -> orr_render::StaticInstance {
        let offset = pose.rot.rotate(orr_view::Vec3::new(
            local.translation[0],
            local.translation[1],
            local.translation[2],
        ));
        let q = orr_view::Quat::new(
            local.rotation[0],
            local.rotation[1],
            local.rotation[2],
            local.rotation[3],
        );
        orr_render::StaticInstance {
            translation: (pose.pos + offset).to_array(),
            rotation: (pose.rot * q).normalize().to_array(),
            scale: local.scale,
        }
    }
    #[cfg(feature = "animated-models")]
    fn animated_instance(
        body: orr_view::Transform3,
        local: LocalTransform,
    ) -> orr_model::animation::Matrix4 {
        let offset = body.rot.rotate(orr_view::Vec3::new(
            local.translation[0],
            local.translation[1],
            local.translation[2],
        ));
        let rotation = orr_view::Quat::new(
            local.rotation[0],
            local.rotation[1],
            local.rotation[2],
            local.rotation[3],
        );
        orr_model::animation::Trs {
            translation: (body.pos + offset).to_array(),
            rotation: (body.rot * rotation).normalize().to_array(),
            scale: local.scale,
        }
        .matrix()
    }
    /// Bound-content preflight also runs without a GPU. Reserve five procedural
    /// draw kinds so all accepted sidecars fit the composed viewport contract.
    fn validate_candidate(
        editor: &Editor,
        document: &model_bindings::Document,
        loaded: &BTreeMap<String, LoadedAsset>,
    ) -> Result<(), String> {
        if !editor.yard_rows_coherent() {
            return Err("Wait for the current host GUID map before changing model bindings".into());
        }
        if document.bindings.len() > 256 {
            return Err("Viewport model binding limit reached".into());
        }
        let mut identities = std::collections::BTreeSet::new();
        let mut draws = 5_usize;
        for (guid, binding) in &document.bindings {
            identities.insert((
                binding.kind == ModelKind::Animated,
                &binding.package,
                &binding.asset,
                &binding.package_digest,
                &binding.source_hash,
            ));
            if identities.len() > 8 {
                return Err(
                    "Viewport supports at most eight distinct verified model identities".into(),
                );
            }
            let asset = loaded
                .get(guid)
                .ok_or("Missing verified model cache entry")?;
            binding.validate(asset)?;
            let local = binding.transform;
            let row_pose = editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
                .and_then(|row| {
                    editor
                        .yard_frame()
                        .poses
                        .iter()
                        .find(|(e, _)| *e == row.entity)
                        .map(|(_, pose)| *pose)
                });
            let draw_count = match binding.kind {
                ModelKind::Static => {
                    let model = asset
                        .static_model()
                        .ok_or("Static binding has no verified static model")?;
                    orr_render::StaticInstance {
                        translation: local.translation,
                        rotation: local.rotation,
                        scale: local.scale,
                    }
                    .validate_for(model)
                    .map_err(|e| e.to_string())?;
                    if let Some(pose) = row_pose {
                        Self::instance(pose, local)
                            .validate_for(model)
                            .map_err(|e| e.to_string())?;
                    }
                    model.source().primitives.len()
                }
                ModelKind::Animated => {
                    #[cfg(feature = "animated-models")]
                    {
                        let model = asset
                            .animated_model()
                            .ok_or("Animated binding has no verified animated model")?;
                        let animation = binding
                            .animation
                            .ok_or("Animated binding has no clip descriptor")?;
                        let pose = model
                            .sample_clip(animation.clip_index, 0.0)
                            .map_err(|e| e.to_string())?;
                        let transform = row_pose.map_or_else(
                            || {
                                orr_model::animation::Trs {
                                    translation: local.translation,
                                    rotation: local.rotation,
                                    scale: local.scale,
                                }
                                .matrix()
                            },
                            |body| Self::animated_instance(body, local),
                        );
                        // Edit/Stop renders rest, which can differ from clip time zero.
                        // Admit both before committing the independent sidecar.
                        let rest = model.rest_pose().map_err(|e| e.to_string())?;
                        for pose in [&rest, &pose] {
                            orr_render::SkinnedInstance { pose, transform }
                                .validate_for(model)
                                .map_err(|e| e.to_string())?;
                        }
                        model.source().primitives.len()
                    }
                    #[cfg(not(feature = "animated-models"))]
                    {
                        return Err(
                            "Animated model bindings require the animated-models feature".into(),
                        );
                    }
                }
            };
            draws = draws
                .checked_add(draw_count)
                .ok_or("Model draw count overflow")?;
            if draws > orr_render::imported_scene::MAX_IMPORTED_DRAWS {
                return Err("Viewport model draw limit exceeded".into());
            }
        }
        Ok(())
    }
    pub fn placements(&self, editor: &Editor) -> Vec<ModelPlacement> {
        if !self.scene_matches(editor) || !editor.yard_rows_coherent() {
            return Vec::new();
        }
        let Some(bindings) = &self.bindings else {
            return Vec::new();
        };
        bindings
            .document()
            .bindings
            .iter()
            .filter(|(_, binding)| binding.kind == ModelKind::Static)
            .filter_map(|(guid, binding)| {
                let row = editor
                    .rows()
                    .iter()
                    .find(|r| r.guid.as_ref().is_some_and(|g| g.to_string() == *guid))?;
                let (_, pose) = editor
                    .yard_frame()
                    .poses
                    .iter()
                    .find(|(e, _)| *e == row.entity)?;
                let loaded = self.loaded.get(guid)?;
                binding.validate(loaded).ok()?;
                Some(ModelPlacement {
                    entity: row.entity,
                    model: loaded.static_model()?.clone(),
                    instance: Self::instance(*pose, binding.transform),
                })
            })
            .collect()
    }
    /// Animated bindings are sampled at an absolute time from the same
    /// immutable Yard snapshot used for body poses. Edit/Stop returns the
    /// asset's rest pose; Play tick 0 and Seek 0 sample clip time zero.
    #[cfg(feature = "animated-models")]
    pub fn animated_placements(&self, editor: &Editor) -> Result<Vec<AnimatedPlacement>, String> {
        if !self.scene_matches(editor) || !editor.yard_rows_coherent() {
            return Ok(Vec::new());
        }
        let Some(bindings) = &self.bindings else {
            return Ok(Vec::new());
        };
        let Some(snapshot) = editor.snapshot() else {
            return Ok(Vec::new());
        };
        let mut placements = Vec::new();
        for (guid, binding) in &bindings.document().bindings {
            if binding.kind != ModelKind::Animated {
                continue;
            }
            let Some(row) = editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
            else {
                continue;
            };
            let Some((_, body_pose)) = editor
                .yard_frame()
                .poses
                .iter()
                .find(|(entity, _)| *entity == row.entity)
            else {
                continue;
            };
            let Some(loaded) = self.loaded.get(guid) else {
                continue;
            };
            if binding.validate(loaded).is_err() {
                continue;
            }
            let Some(model) = loaded.animated_model().cloned() else {
                continue;
            };
            let animation = binding
                .animation
                .ok_or_else(|| format!("{guid}: animated binding has no clip descriptor"))?;
            let duration = model
                .source()
                .clips
                .get(animation.clip_index as usize)
                .ok_or_else(|| {
                    format!(
                        "{guid}: animation clip {} is unavailable",
                        animation.clip_index
                    )
                })?
                .duration();
            let absolute_time =
                crate::yard_animation::snapshot_clip_time(snapshot, duration, animation.playback)?;
            let playback_time = match (editor.mode(), absolute_time) {
                (Mode::Play, Some(time)) => Some(time),
                (Mode::Play, None) => {
                    return Err(format!("{guid}: Play mode has no host timeline"))
                }
                (Mode::Edit, Some(_)) => {
                    return Err(format!("{guid}: Edit mode has a stale Play snapshot"))
                }
                // Stopped and Edit scenes deliberately show the bind/rest pose.
                (Mode::Edit, None) => None,
            };
            let pose = match playback_time {
                Some(time) => model
                    .sample_clip(animation.clip_index, time)
                    .map_err(|e| format!("{guid}: sample animation clip: {e}"))?,
                None => model
                    .rest_pose()
                    .map_err(|e| format!("{guid}: sample rest pose: {e}"))?,
            };
            placements.push(AnimatedPlacement {
                entity: row.entity,
                instance: Self::animated_instance(*body_pose, binding.transform),
                model,
                pose,
            });
        }
        Ok(placements)
    }
    /// Loads an existing local sidecar once per scene identity, never a remote host path.
    fn sync_scene(&mut self, editor: &Editor) {
        if !editor.yard_rows_coherent() {
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
        if Self::sidecar(&scene).exists() {
            if let Err(e) = self.open_for_editor(editor, false) {
                self.error = Some(e);
            }
        }
    }
    fn discard_bindings(&mut self) {
        self.bindings = None;
        self.loaded.clear();
        self.candidate = None;
        self.confirm_discard = false;
        self.error = None;
    }

    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::Yard3D {
            return;
        }
        self.sync_scene(editor);
        ui.collapsing("Static model binding", |ui| {
            ui.weak("Presentation only · scene and model saves/undo are separate");
            if !editor.spec().is_local() { ui.label("Remote host paths are not local asset authority"); return; }
            ui.add_enabled_ui(editor.mode() == Mode::Edit && editor.can_mutate(), |ui| {
                ui.horizontal(|ui| { ui.label("Project"); ui.text_edit_singleline(&mut self.project); });
                ui.horizontal(|ui| {
                    if ui.button("Create model bindings").clicked() { self.error = self.open_for_editor(editor, true).err(); }
                    if ui.button("Open model bindings").clicked() { self.error = self.open_for_editor(editor, false).err(); }
                });
                if self.bindings.is_some() {
                    if let Some(bindings) = &self.bindings {
                        if let Some(current) = editor.selected_guid().and_then(|guid| bindings.document().bindings.get(&guid.to_string())) {
                            let label = current.animation.map_or_else(
                                || format!("Assigned static model: {}/{}", current.package, current.asset),
                                |a| format!("Assigned animated model: {}/{} · clip {} ({:?})", current.package, current.asset, a.clip_index, a.playback),
                            );
                            ui.label(label);
                        }
                        let orphaned = bindings.document().bindings.keys().filter(|guid| !editor.rows().iter().any(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == **guid))).count();
                        if orphaned > 0 { ui.colored_label(egui::Color32::YELLOW, format!("{orphaned} orphan GUID binding(s); no handle rebinding")); }
                    }
                    if !self.scene_matches(editor) { ui.colored_label(egui::Color32::YELLOW, "Bindings belong to a different scene; save/discard before opening this scene's bindings"); }
                    let editable = self.editable(editor);
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Model kind");
                            ui.selectable_value(&mut self.kind, ModelKind::Static, "Static");
                            #[cfg(feature = "animated-models")]
                            ui.selectable_value(&mut self.kind, ModelKind::Animated, "Animated");
                        });
                        ui.horizontal(|ui| { ui.label("Package"); ui.text_edit_singleline(&mut self.package); });
                        ui.horizontal(|ui| { ui.label("Asset"); ui.text_edit_singleline(&mut self.asset); });
                        if ui.button("Load model asset").clicked() { self.error = self.load_candidate(editor).err(); }
                        #[cfg(feature = "animated-models")]
                        if let Some((package, asset, kind, loaded)) = self.candidate.as_ref().filter(|(p, a, k, _)| p == &self.package && a == &self.asset && *k == self.kind) {
                            if *kind == ModelKind::Animated {
                                if let Some(model) = loaded.animated_model() {
                                    let clips = &model.source().clips;
                                    let label = clips.get(self.animation.clip_index as usize).map_or_else(
                                        || "Select a clip".to_owned(),
                                        |clip| format!("{}: {} ({:.2}s)", self.animation.clip_index, clip.name, clip.duration()),
                                    );
                                    egui::ComboBox::from_label("Animation clip").selected_text(label).show_ui(ui, |ui| {
                                        for (index, clip) in clips.iter().enumerate() {
                                            let Ok(index) = u32::try_from(index) else { continue };
                                            ui.selectable_value(&mut self.animation.clip_index, index, format!("{index}: {} ({:.2}s)", clip.name, clip.duration()));
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Playback");
                                        ui.selectable_value(&mut self.animation.playback, PlaybackMode::Once, "Once");
                                        ui.selectable_value(&mut self.animation.playback, PlaybackMode::Loop, "Loop");
                                    });
                                    ui.weak(format!("Loaded animated asset: {package}/{asset}"));
                                }
                            }
                        }
                        for (label, values) in [("Offset", &mut self.transform.translation), ("Scale", &mut self.transform.scale)] {
                            ui.horizontal(|ui| { ui.label(label); for value in values { ui.add(egui::DragValue::new(value).speed(0.05)); } });
                        }
                        ui.horizontal(|ui| { ui.label("Local quaternion xyzw"); for value in &mut self.transform.rotation { ui.add(egui::DragValue::new(value).speed(0.01)); } });
                        let assign_label = if self.kind == ModelKind::Static { "Assign static model" } else { "Assign animated model" };
                        if ui.add_enabled(editor.selected_guid().is_some(), egui::Button::new(assign_label)).clicked() { self.error = self.assign(editor).err(); }
                        ui.horizontal(|ui| {
                            if ui.button("Remove model").clicked() {
                                if let (Some(bindings), Some(guid)) = (&mut self.bindings, editor.selected_guid()) {
                                    self.error = bindings.remove(&[guid]).err();
                                    self.loaded.retain(|key, _| bindings.document().bindings.contains_key(key));
                                }
                            }
                            if ui.button("Undo model").clicked() { self.error = self.undo(editor).err(); }
                            if ui.button("Redo model").clicked() { self.error = self.redo(editor).err(); }
                            if ui.button("Reload verified models").clicked() { self.error = self.reload(editor).err(); }
                        });
                    });
                    // An old sidecar remains saveable after Save As changes scene identity.
                    if ui.button("Save model bindings").clicked() { if let Some(bindings) = &mut self.bindings { self.error = bindings.save().err(); } }
                    if self.bindings.as_ref().is_some_and(Bindings::dirty) { ui.colored_label(egui::Color32::YELLOW, "Model bindings have unsaved changes (scene Save does not save these)"); }
                    if self.confirm_discard {
                        ui.colored_label(egui::Color32::YELLOW, "Discard unsaved model bindings?");
                        ui.horizontal(|ui| {
                            if ui.button("Discard model changes and close").clicked() { self.discard_bindings(); }
                            if ui.button("Keep model bindings open").clicked() { self.confirm_discard = false; }
                        });
                    } else if ui.button("Discard model bindings").clicked() {
                        if self.bindings.as_ref().is_some_and(Bindings::dirty) { self.confirm_discard = true; }
                        else { self.discard_bindings(); }
                    }
                }
                if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
            });
        });
    }
}
