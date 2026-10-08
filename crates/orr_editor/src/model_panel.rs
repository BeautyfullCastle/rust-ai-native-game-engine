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
    reserved_scene_models: usize,
    room_identity: Option<(PathBuf, String)>,
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
            reserved_scene_models: 0,
            room_identity: None,
        }
    }
}
impl ModelPanel {
    /// Install an admitted immutable room presentation without reopening files.
    #[cfg(feature = "room-project")]
    pub fn install_room(
        &mut self,
        prepared: orr_sample::room_project::PreparedModels,
    ) -> Result<(), String> {
        let mut loaded = BTreeMap::new();
        for (guid, binding) in &prepared.document.bindings {
            let asset = prepared
                .assets
                .get(&(binding.package.clone(), binding.asset.clone()))
                .ok_or("admitted model missing")?;
            binding.validate(asset)?;
            loaded.insert(guid.clone(), asset.clone());
        }
        orr_sample::room_project::validate_decoded_model_budget(loaded.values())?;
        if prepared.document.project != "." {
            return Err("Room sidecar project must be exactly '.'".into());
        }
        let identity = (prepared.path.clone(), prepared.document.scene.clone());
        let bindings = Bindings::from_document(prepared.path, prepared.document)?;
        self.room_identity = Some(identity);
        self.project = bindings
            .path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .display()
            .to_string();
        self.bindings = Some(bindings);
        self.loaded = loaded;
        self.error = None;
        Ok(())
    }
    /// Reserve the bounded scene-owned terrain slot before model mutations.
    pub fn reserve_scene_models(&mut self, count: usize) {
        self.reserved_scene_models = count;
    }
    pub fn terrain_admission_error(&self) -> Option<String> {
        let bindings = self.bindings.as_ref()?;
        let mut identities = std::collections::BTreeSet::new();
        let mut draws = 7usize; // Five procedural kinds plus terrain and query marker.
        for (guid, binding) in &bindings.document().bindings {
            identities.insert((
                binding.kind == ModelKind::Animated,
                &binding.package,
                &binding.asset,
                &binding.package_digest,
                &binding.source_hash,
            ));
            let Some(asset) = self.loaded.get(guid) else {
                return Some("Wait for verified model assets before editing terrain".into());
            };
            let count = match binding.kind {
                ModelKind::Static => asset
                    .static_model()
                    .map_or(0, |model| model.source().primitives.len()),
                ModelKind::Animated => {
                    #[cfg(feature = "animated-models")]
                    {
                        asset
                            .animated_model()
                            .map_or(0, |model| model.source().primitives.len())
                    }
                    #[cfg(not(feature = "animated-models"))]
                    {
                        0
                    }
                }
            };
            draws = draws.saturating_add(count);
        }
        if bindings.document().bindings.len() >= 256
            || identities.len() >= 8
            || draws > orr_render::imported_scene::MAX_IMPORTED_DRAWS
        {
            Some("Terrain needs one shared viewport asset/instance slot and up to two draws; remove an entity model binding first".into())
        } else {
            None
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn scene_matches(&self, editor: &Editor) -> bool {
        editor.spec().is_local()
            && (editor.game() == EditorGame::Yard3D || editor.game().is_room())
            && self.bindings.as_ref().is_some_and(|b| {
                let local_path: &std::path::Path = match editor.spec() {
                    crate::backend::HostSpec::Local { scene, .. }
                    | crate::backend::HostSpec::LocalGame { scene, .. } => scene,
                    #[cfg(feature = "sprites")]
                    crate::backend::HostSpec::PreparedArena { scene, .. } => scene.path(),
                    #[cfg(feature = "collect-dodge")]
                    crate::backend::HostSpec::PreparedCollect { .. } => return false,
                    #[cfg(feature = "room-project")]
                    crate::backend::HostSpec::PreparedRoom { scene, .. } => scene,
        #[cfg(feature = "navigation-project")]
        crate::backend::HostSpec::PreparedNavigation { scene, .. } => scene,
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
        if !editor.spec().is_local()
            || (editor.game() != EditorGame::Yard3D && !editor.game().is_room())
        {
            return Err("Model bindings require a local Yard3D scene".into());
        }
        let expected: &std::path::Path = match editor.spec() {
            crate::backend::HostSpec::Local { scene, .. }
            | crate::backend::HostSpec::LocalGame { scene, .. } => scene,
            #[cfg(feature = "sprites")]
            crate::backend::HostSpec::PreparedArena { scene, .. } => scene.path(),
            #[cfg(feature = "collect-dodge")]
            crate::backend::HostSpec::PreparedCollect { .. } => {
                return Err("CollectDodge does not support local 3D asset authoring".into())
            }
            #[cfg(feature = "room-project")]
            crate::backend::HostSpec::PreparedRoom { scene, .. } => scene,
        #[cfg(feature = "navigation-project")]
        crate::backend::HostSpec::PreparedNavigation { scene, .. } => scene,
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
        if reported.as_path() != expected {
            return Err(
                "Wait for the local scene path to synchronize before opening bindings".into(),
            );
        }
        Ok(expected.to_path_buf())
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
        let path = if editor.game().is_room() {
            if create {
                return Err(
                    "Room model sidecar is declared by its project; edit the admitted bindings"
                        .into(),
                );
            }
            self.room_identity
                .as_ref()
                .ok_or("Open the room project first")?
                .0
                .clone()
        } else {
            Self::sidecar(&scene)
        };
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
        let loaded = Self::load_all(&candidate, self.room_identity.as_ref())?;
        Self::validate_candidate(
            editor,
            candidate.document(),
            &loaded,
            self.reserved_scene_models,
        )?;
        self.bindings = Some(candidate);
        self.loaded = loaded;
        self.candidate = None;
        self.error = None;
        Ok(())
    }
    fn validate_room_identity(
        bindings: &Bindings,
        identity: &(PathBuf, String),
    ) -> Result<(), String> {
        if bindings.path != identity.0
            || bindings.document().project != "."
            || bindings.document().scene != identity.1
        {
            return Err(
                "Room model sidecar must name the exact admitted project and entry scene".into(),
            );
        }
        Ok(())
    }
    fn load_all(
        bindings: &Bindings,
        room_identity: Option<&(PathBuf, String)>,
    ) -> Result<BTreeMap<String, LoadedAsset>, String> {
        #[cfg(feature = "room-project")]
        if let Some(identity) = room_identity {
            Self::validate_room_identity(bindings, identity)?;
            let root = identity.0.parent().ok_or("Room project root missing")?;
            let project =
                orr_package::Project::open(root, orr_sample::room_project::compiled_runtime())
                    .map_err(|e| e.to_string())?;
            let before = project.verify().map_err(|e| e.to_string())?;
            let mut out = BTreeMap::new();
            let mut unique = BTreeMap::new();
            for (guid, binding) in &bindings.document().bindings {
                if binding.kind != ModelKind::Static || binding.animation.is_some() {
                    return Err("Room supports static bindings only".into());
                }
                let key = (binding.package.clone(), binding.asset.clone());
                if !unique.contains_key(&key) {
                    let asset = model_bindings::load_binding_from_project(&project, binding)?;
                    unique.insert(key.clone(), asset);
                    orr_sample::room_project::validate_decoded_model_budget(unique.values())?;
                }
                binding.validate(&unique[&key])?;
                out.insert(guid.clone(), unique[&key].clone());
            }
            if before != project.verify().map_err(|e| e.to_string())? {
                return Err("Room active lock changed during model load; retry".into());
            }
            return Ok(out);
        }
        #[cfg(not(feature = "room-project"))]
        let _ = room_identity;
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
        let loaded = Self::load_all(bindings, self.room_identity.as_ref())?;
        Self::validate_candidate(
            editor,
            bindings.document(),
            &loaded,
            self.reserved_scene_models,
        )?;
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
        if editor.game().is_room() && self.kind != ModelKind::Static {
            return Err("Room v1 supports static models only".into());
        }
        let loaded = self.load_requested()?;
        self.candidate = Some((self.package.clone(), self.asset.clone(), self.kind, loaded));
        Ok(())
    }
    fn load_requested(&self) -> Result<LoadedAsset, String> {
        let bindings = self.bindings.as_ref().ok_or("Open model bindings first")?;
        #[cfg(feature = "room-project")]
        if let Some(identity) = &self.room_identity {
            Self::validate_room_identity(bindings, identity)?;
            if self.kind != ModelKind::Static {
                return Err("Room supports static bindings only".into());
            }
            let project = orr_package::Project::open(
                identity.0.parent().ok_or("Room root missing")?,
                orr_sample::room_project::compiled_runtime(),
            )
            .map_err(|e| e.to_string())?;
            let before = project.verify().map_err(|e| e.to_string())?;
            // Validate all retained identities against this same consuming project/lock.
            for binding in bindings.document().bindings.values() {
                model_bindings::load_binding_from_project(&project, binding)?;
            }
            let loaded = model_bindings::load_asset_from_project(
                &project,
                &self.package,
                &self.asset,
                self.kind,
            )?;
            if before != project.verify().map_err(|e| e.to_string())? {
                return Err("Room active lock changed during assignment; retry".into());
            }
            return Ok(loaded);
        }
        model_bindings::load_asset_for_kind(
            &bindings.project_root()?,
            &self.package,
            &self.asset,
            self.kind,
        )
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
        let loaded = self.load_requested()?;
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
        Self::validate_candidate(
            editor,
            &candidate,
            &candidate_loaded,
            self.reserved_scene_models,
        )?;
        bindings.assign_validated(std::slice::from_ref(&guid), &binding, &loaded)?;
        self.loaded = candidate_loaded;
        Ok(())
    }
    /// Validate the complete proposed state before touching document or history.
    pub fn remove(&mut self, editor: &Editor) -> Result<(), String> {
        if !self.editable(editor) {
            return Err("Model removal requires local Edit mode".into());
        }
        let guid = editor.selected_guid().ok_or("Select a model binding")?;
        let bindings = self.bindings.as_mut().ok_or("Open model bindings first")?;
        let mut candidate = bindings.document().clone();
        candidate.bindings.remove(&guid.to_string());
        let mut loaded = self.loaded.clone();
        loaded.retain(|key, _| candidate.bindings.contains_key(key));
        if editor.game().is_room() {
            Self::validate_candidate(editor, &candidate, &loaded, self.reserved_scene_models)?;
        }
        bindings.remove(&[guid])?;
        self.loaded = loaded;
        Ok(())
    }
    pub fn save(&mut self, editor: &Editor) -> Result<(), String> {
        if editor.game().is_room() {
            if !self.editable(editor) {
                return Err("Room model save requires the admitted scene in Edit mode".into());
            }
            let bindings = self.bindings.as_ref().ok_or("Open model bindings first")?;
            let identity = self
                .room_identity
                .as_ref()
                .ok_or("Open the room project first")?;
            Self::validate_room_identity(bindings, identity)?;
            let loaded = Self::load_all(bindings, Some(identity))?;
            Self::validate_candidate(
                editor,
                bindings.document(),
                &loaded,
                self.reserved_scene_models,
            )?;
        }
        self.bindings
            .as_mut()
            .ok_or("Open model bindings first")?
            .save()
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
        match Self::load_all(bindings, self.room_identity.as_ref()).and_then(|loaded| {
            Self::validate_candidate(
                editor,
                bindings.document(),
                &loaded,
                self.reserved_scene_models,
            )?;
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
        match Self::load_all(bindings, self.room_identity.as_ref()).and_then(|loaded| {
            Self::validate_candidate(
                editor,
                bindings.document(),
                &loaded,
                self.reserved_scene_models,
            )?;
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
        reserved_scene_models: usize,
    ) -> Result<(), String> {
        if !editor.yard_rows_coherent() {
            return Err("Wait for the current host GUID map before changing model bindings".into());
        }
        if document
            .bindings
            .len()
            .saturating_add(reserved_scene_models)
            > 256
        {
            return Err("Viewport model binding limit reached".into());
        }
        if editor.game().is_room()
            && (document.bindings.is_empty()
                || document.bindings.len() > 68
                || document.bindings.values().any(|binding| {
                    binding.kind != ModelKind::Static || binding.animation.is_some()
                }))
        {
            return Err("Room v1 requires1..=68 static model bindings".into());
        }
        #[cfg(feature = "room-project")]
        if editor.game().is_room() {
            orr_sample::room_project::validate_decoded_model_budget(loaded.values())?;
        }
        let mut identities = std::collections::BTreeSet::new();
        let mut draws = 5_usize.saturating_add(reserved_scene_models.saturating_mul(2));
        for (guid, binding) in &document.bindings {
            identities.insert((
                binding.kind == ModelKind::Animated,
                &binding.package,
                &binding.asset,
                &binding.package_digest,
                &binding.source_hash,
            ));
            if identities.len().saturating_add(reserved_scene_models) > 8 {
                return Err(
                    "Viewport supports at most eight distinct verified model identities".into(),
                );
            }
            let asset = loaded
                .get(guid)
                .ok_or("Missing verified model cache entry")?;
            binding.validate(asset)?;
            let local = binding.transform;
            let row = editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid));
            if editor.game().is_room() {
                let row = row.ok_or("Room model binding GUID is absent from the current scene")?;
                let frame = editor
                    .snapshot()
                    .ok_or("Room snapshot missing")?
                    .predicted();
                // Presentation may hide a collected key. Admission instead checks
                // the coherent GUID map against the actual entity generation/body.
                if !frame.exists(row.entity)
                    || !row
                        .components
                        .iter()
                        .any(|component| component == "orr_physics3d::Body")
                {
                    return Err("Room model binding has no current scene body".into());
                }
            }
            let row_pose = row.and_then(|row| {
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
                    if editor.game().is_room() {
                        for x in [-16.0, 16.0] {
                            for y in [-4.0, 4.0] {
                                for z in [-16.0, 16.0] {
                                    let pose = orr_view::Transform3 {
                                        pos: orr_view::Vec3::new(x, y, z),
                                        rot: orr_view::Quat::IDENTITY,
                                    };
                                    Self::instance(pose, local).validate_for(model).map_err(
                                        |e| format!("room reachable model placement: {e}"),
                                    )?;
                                }
                            }
                        }
                    }
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
        if editor.game() != EditorGame::Yard3D && !editor.game().is_room() {
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
                            if !editor.game().is_room() { ui.selectable_value(&mut self.kind, ModelKind::Animated, "Animated"); }
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
                                self.error = self.remove(editor).err();
                            }
                            if ui.button("Undo model").clicked() { self.error = self.undo(editor).err(); }
                            if ui.button("Redo model").clicked() { self.error = self.redo(editor).err(); }
                            if ui.button("Reload verified models").clicked() { self.error = self.reload(editor).err(); }
                        });
                    });
                    // An old sidecar remains saveable after Save As changes scene identity.
                    if ui.button("Save model bindings").clicked() { self.error = self.save(editor).err(); }
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
