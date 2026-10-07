//! Static model bindings in the real Yard3D inspector. Sidecar history/save is
//! explicitly separate from the host document. No animation clocks or sim writes.
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    model_bindings::{self, Binding, Bindings, LoadedAsset, LocalTransform},
    viewport3d::ModelPlacement,
};
use egui::Ui;
use std::{collections::BTreeMap, path::PathBuf};

pub struct ModelPanel {
    pub bindings: Option<Bindings>,
    pub project: String,
    pub package: String,
    pub asset: String,
    pub transform: LocalTransform,
    loaded: BTreeMap<String, LoadedAsset>,
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
            loaded: BTreeMap::new(),
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
        self.error = None;
        Ok(())
    }
    fn load_all(bindings: &Bindings) -> Result<BTreeMap<String, LoadedAsset>, String> {
        if bindings.document().bindings.len() > 256 {
            return Err("Viewport supports at most 256 model-bound entities".into());
        }
        let mut out = BTreeMap::new();
        let mut assets: BTreeMap<(String, String, String, String), LoadedAsset> = BTreeMap::new();
        let root = bindings.project_root()?;
        for (guid, binding) in &bindings.document().bindings {
            let key = (
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
        let bindings = self.bindings.as_ref().ok_or("Open model bindings first")?;
        let loaded = Self::load_all(bindings)?;
        Self::validate_candidate(editor, bindings.document(), &loaded)?;
        self.loaded = loaded;
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
        let bindings = self.bindings.as_mut().ok_or("Open model bindings first")?;
        if bindings.document().bindings.len() >= 256
            && !bindings.document().bindings.contains_key(&guid.to_string())
        {
            return Err("Viewport model binding limit reached".into());
        }
        let loaded =
            model_bindings::load_asset(&bindings.project_root()?, &self.package, &self.asset)?;
        let loaded = self
            .loaded
            .values()
            .find(|old| {
                old.package() == loaded.package()
                    && old.asset() == loaded.asset()
                    && old.package_digest() == loaded.package_digest()
                    && old.source_hash() == loaded.source_hash()
            })
            .cloned()
            .unwrap_or(loaded);
        let binding = Binding::from_asset(
            self.package.clone(),
            self.asset.clone(),
            &loaded,
            self.transform,
        )?;
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
            orr_render::StaticInstance {
                translation: local.translation,
                rotation: local.rotation,
                scale: local.scale,
            }
            .validate_for(asset.model())
            .map_err(|e| e.to_string())?;
            let Some(row) = editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
            else {
                continue;
            };
            let Some((_, pose)) = editor
                .yard_frame()
                .poses
                .iter()
                .find(|(e, _)| *e == row.entity)
            else {
                continue;
            };
            Self::instance(*pose, local)
                .validate_for(asset.model())
                .map_err(|e| e.to_string())?;
            draws = draws
                .checked_add(asset.model().source().primitives.len())
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
                    model: loaded.model().clone(),
                    instance: Self::instance(*pose, binding.transform),
                })
            })
            .collect()
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
    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::Yard3D {
            return;
        }
        self.sync_scene(editor);
        ui.collapsing("Static model binding",|ui|{
            ui.weak("Presentation only · scene and model saves/undo are separate");
            if !editor.spec().is_local(){ui.label("Remote host paths are not local asset authority");return;}
            ui.add_enabled_ui(editor.mode()==Mode::Edit&&editor.can_mutate(),|ui|{
                ui.horizontal(|ui|{ui.label("Project");ui.text_edit_singleline(&mut self.project);});
                ui.horizontal(|ui|{
                    if ui.button("Create model bindings").clicked(){self.error=self.open_for_editor(editor,true).err();}
                    if ui.button("Open model bindings").clicked(){self.error=self.open_for_editor(editor,false).err();}
                });
                if self.bindings.is_some() {
                    if let Some(bindings)=&self.bindings {
                        if let Some(current)=editor.selected_guid().and_then(|guid|bindings.document().bindings.get(&guid.to_string())) {
                            ui.label(format!("Assigned static model: {}/{}",current.package,current.asset));
                        }
                        let orphaned=bindings.document().bindings.keys().filter(|guid|!editor.rows().iter().any(|row|row.guid.as_ref().is_some_and(|g|g.to_string()==**guid))).count();
                        if orphaned>0 { ui.colored_label(egui::Color32::YELLOW,format!("{orphaned} orphan GUID binding(s); no handle rebinding")); }
                    }
                    if !self.scene_matches(editor){ui.colored_label(egui::Color32::YELLOW,"Bindings belong to a different scene; save/discard before opening this scene's bindings");}
                    ui.horizontal(|ui|{ui.label("Package");ui.text_edit_singleline(&mut self.package);});
                    ui.horizontal(|ui|{ui.label("Asset");ui.text_edit_singleline(&mut self.asset);});
                    for (label,values) in [("Offset",&mut self.transform.translation),("Scale",&mut self.transform.scale)]{
                        ui.horizontal(|ui|{ui.label(label);for value in values {ui.add(egui::DragValue::new(value).speed(0.05));}});
                    }
                    ui.horizontal(|ui|{ui.label("Local quaternion xyzw");for value in &mut self.transform.rotation {ui.add(egui::DragValue::new(value).speed(0.01));}});
                    if ui.add_enabled(self.editable(editor)&&editor.selected_guid().is_some(),egui::Button::new("Assign static model")).clicked(){self.error=self.assign(editor).err();}
                    let editable=self.editable(editor);
                    ui.add_enabled_ui(editable,|ui|{
                        ui.horizontal(|ui|{
                            if ui.button("Remove model").clicked(){if let (Some(bindings),Some(guid))=(&mut self.bindings,editor.selected_guid()){self.error=bindings.remove(&[guid]).err();self.loaded.retain(|key,_|bindings.document().bindings.contains_key(key));}}
                            if ui.button("Undo model").clicked(){self.error=self.undo(editor).err();}
                            if ui.button("Redo model").clicked(){self.error=self.redo(editor).err();}
                        });
                        if ui.button("Reload verified models").clicked(){self.error=self.reload(editor).err();}
                    });
                    if ui.button("Save model bindings").clicked(){if let Some(bindings)=&mut self.bindings{self.error=bindings.save().err();}}
                    if self.bindings.as_ref().is_some_and(Bindings::dirty){ui.colored_label(egui::Color32::YELLOW,"Model bindings have unsaved changes (scene Save does not save these)");}
                    if ui.button("Discard model bindings").clicked(){self.bindings=None;self.loaded.clear();self.error=None;}
                }
            });
            if let Some(error)=&self.error{ui.colored_label(egui::Color32::RED,error);}
        });
    }
}
