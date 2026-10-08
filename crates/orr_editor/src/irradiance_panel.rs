//! Local Yard3D irradiance authoring, independent from scene state and ERP undo.
//! Static sun-bounce baking runs on an immutable CPU snapshot. Publication is
//! guarded and remains independent from scene state, ERP, and package files.
use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    irradiance_bake::{BakeJob, BakeSceneSnapshot, BakeValidationJob, VerifiedBakeSources},
    irradiance_bindings::{self, IrradianceBindings},
    model::Mode,
    model_panel::ModelPanel,
};
use egui::Ui;
use orr_render::irradiance::{constant_irradiance, IrradianceGrid, IrradianceProvenance};
use orr_render::irradiance_bake::BakeSettings;
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
struct PreflightKey {
    participant_key: Result<String, String>,
    revision: u64,
    generation: u64,
    mode: Mode,
    directions: u32,
    triangle_tests: u64,
    node_visits: u64,
    duration: std::time::Duration,
}

struct PendingValidation {
    job: BakeValidationJob,
    fingerprint: String,
    source_identity: String,
    scene_path: PathBuf,
    generation: u64,
    input_key: PreflightKey,
}

struct PendingBake {
    job: BakeJob,
    sidecar: PathBuf,
    revision: u64,
    generation: u64,
    fingerprint: String,
    source_identity: String,
    settings: BakeSettings,
    input_key: PreflightKey,
    rejection: Option<String>,
    cancelled: bool,
}

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
    /// CPU integration quality. Work limits remain enforced by the worker.
    pub bake_settings: BakeSettings,
    pending_bake: Option<PendingBake>,
    pending_validation: Option<PendingValidation>,
    verified_sources: Option<(String, String, VerifiedBakeSources)>,
    validation_failed_fingerprint: Option<(String, String)>,
    binding_generation: u64,
    bake_status: Option<String>,
    bake_preflight: Option<String>,
    bake_preflight_key: Option<PreflightKey>,
    baked_fingerprint: Option<String>,
    baked_revision: Option<u64>,
    baked_checksum: Option<u64>,
    baked_stale_reason: Option<String>,
    validated_input_key: Option<PreflightKey>,
    failed_input_key: Option<(PreflightKey, String)>,
    snapshot_captures: u64,
    terrain_attached: bool,
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
            bake_settings: BakeSettings::default(),
            pending_bake: None,
            pending_validation: None,
            verified_sources: None,
            validation_failed_fingerprint: None,
            binding_generation: 0,
            bake_status: None,
            bake_preflight: None,
            bake_preflight_key: None,
            baked_fingerprint: None,
            baked_revision: None,
            baked_checksum: None,
            baked_stale_reason: None,
            validated_input_key: None,
            failed_input_key: None,
            snapshot_captures: 0,
            terrain_attached: false,
        }
    }
}
impl IrradiancePanel {
    /// Terrain is outside the bounded bake snapshot contract. Preserve the
    /// document but prevent both new bakes and application of saved receipts.
    pub fn set_terrain_attached(&mut self, attached: bool) {
        if self.terrain_attached == attached { return; }
        self.terrain_attached = attached;
        // A cancelled source job must never cache its cancellation as a source
        // failure after a quick attach/detach of otherwise unchanged terrain.
        self.binding_generation = self.binding_generation.wrapping_add(1);
        self.bake_preflight = None;
        self.bake_preflight_key = None;
        self.baked_fingerprint = None;
        self.baked_revision = None;
        self.baked_checksum = None;
        self.cancel_validation();
        if attached { self.cancel_bake(); }
    }
    pub const TERRAIN_BAKE_UNSUPPORTED: &'static str = "Scene terrain is not included in static bake snapshots. New bakes and baked irradiance are suspended while terrain is attached; authored/imported manual probes remain available";

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
        let expected: &std::path::Path = match editor.spec() {
            HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
            #[cfg(feature = "sprites")]
            HostSpec::PreparedArena { scene, .. } => scene.path(),
            #[cfg(feature = "collect-dodge")]
            HostSpec::PreparedCollect { .. } => return Err("CollectDodge does not support local 3D asset authoring".into()),
            #[cfg(feature = "room-project")]
            HostSpec::PreparedRoom { scene, .. } => scene,
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
        if reported.as_path() != expected {
            return Err("Wait for the local scene path to synchronize".into());
        }
        Ok(expected.to_path_buf())
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
            self.cancel_bake();
            self.cancel_validation();
            self.binding_generation = self.binding_generation.wrapping_add(1);
            self.baked_fingerprint = None;
            self.baked_revision = None;
            self.baked_checksum = None;
            self.baked_stale_reason = None;
            self.bake_preflight = None;
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
        let bindings = self.bindings.as_ref()?;
        let grid = &bindings.document().grid;
        if grid.provenance == IrradianceProvenance::Baked {
            if self.terrain_attached { return None; }
            let receipt = bindings.document().bake_receipt.as_ref()?;
            if self.baked_fingerprint.as_ref() != Some(&receipt.fingerprint)
                || self.baked_revision != Some(bindings.revision())
                || self.baked_checksum != Some(editor.checksum())
                || editor.previewing().is_some()
            {
                return None;
            }
        }
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
    /// Analyze exactly the participants that the CPU worker will receive. This
    /// does not modify the accepted irradiance or any document/history.
    fn preflight_key(&self, editor: &Editor, models: &ModelPanel) -> PreflightKey {
        PreflightKey {
            participant_key: self
                .bindings
                .as_ref()
                .ok_or_else(|| "Open irradiance first".to_string())
                .and_then(|bindings| {
                    BakeSceneSnapshot::participating_input_key(
                        editor,
                        models,
                        &bindings.document().grid,
                        &self.bake_settings,
                    )
                }),
            revision: self
                .bindings
                .as_ref()
                .map_or(0, IrradianceBindings::revision),
            generation: self.binding_generation,
            mode: editor.mode(),
            directions: self.bake_settings.rays_per_probe,
            triangle_tests: self.bake_settings.max_triangle_tests,
            node_visits: self.bake_settings.max_node_visits,
            duration: self.bake_settings.max_duration,
        }
    }
    /// Diagnostic count of full geometry/texture snapshots, including failed
    /// attempts. Cheap per-frame participant-key checks do not increment it.
    pub fn bake_snapshot_captures(&self) -> u64 {
        self.snapshot_captures
    }
    pub fn preflight_bake(&mut self, editor: &Editor, models: &ModelPanel) -> Result<(), String> {
        let result = (|| {
            if self.terrain_attached { return Err(Self::TERRAIN_BAKE_UNSUPPORTED.into()); }
            self.require_editable(editor)?;
            let grid = &self
                .bindings
                .as_ref()
                .ok_or("Open irradiance first")?
                .document()
                .grid;
            self.snapshot_captures = self.snapshot_captures.saturating_add(1);
            let snapshot = BakeSceneSnapshot::capture(editor, models, grid, &self.bake_settings)?;
            let cost = &snapshot.cost;
            self.bake_preflight = Some(format!(
                "{} static participants · {} excluded · {} triangles\n{} probes × {} directions = {} primary rays\nSnapshot {} KiB · limits: 64 probes, 2,048 directions, 8,192 triangles",
                cost.participants, cost.excluded, cost.triangles,
                cost.probes, cost.directions, cost.max_primary_rays,
                cost.snapshot_bytes.div_ceil(1024)
            ));
            Ok(())
        })();
        self.bake_preflight_key = Some(self.preflight_key(editor, models));
        if let Err(error) = &result {
            self.bake_preflight = Some(format!("Cannot bake: {error}"));
        }
        result
    }
    pub fn is_validating_bake(&self) -> bool {
        self.pending_validation.is_some()
    }
    fn cancel_validation(&mut self) {
        if let Some(pending) = &self.pending_validation {
            pending.job.cancel();
        }
        self.verified_sources = None;
        self.validation_failed_fingerprint = None;
        self.validated_input_key = None;
        self.failed_input_key = None;
    }
    pub fn is_baking(&self) -> bool {
        self.pending_bake.is_some()
    }
    pub fn bake_status(&self) -> Option<&str> {
        self.bake_status.as_deref()
    }
    pub fn baked_stale_reason(&self) -> Option<&str> {
        self.baked_stale_reason.as_deref()
    }
    pub fn bake_progress(&self) -> Option<(u32, u32)> {
        self.pending_bake
            .as_ref()
            .map(|pending| pending.job.progress())
    }
    pub fn start_bake(&mut self, editor: &Editor, models: &ModelPanel) -> Result<(), String> {
        let result = (|| {
            if self.terrain_attached { return Err(Self::TERRAIN_BAKE_UNSUPPORTED.into()); }
            if self.pending_bake.is_some() {
                return Err("Wait for the current bake to finish or cancel".into());
            }
            self.require_editable(editor)?;
            if let Some(pending) = &self.pending_validation {
                pending.job.cancel();
                return Err("Cancelling the source check; retry bake when it finishes".into());
            }
            let bindings = self.bindings.as_ref().ok_or("Open irradiance first")?;
            self.snapshot_captures = self.snapshot_captures.saturating_add(1);
            let snapshot = BakeSceneSnapshot::capture(
                editor,
                models,
                &bindings.document().grid,
                &self.bake_settings,
            )?;
            let fingerprint = snapshot.fingerprint.clone();
            let source_identity = snapshot.source_identity.clone();
            let sidecar = bindings.path.clone();
            let revision = bindings.revision();
            let job = BakeJob::start(snapshot)?;
            self.pending_bake = Some(PendingBake {
                job,
                sidecar,
                revision,
                fingerprint,
                source_identity,
                generation: self.binding_generation,
                settings: self.bake_settings.clone(),
                input_key: self.preflight_key(editor, models),
                rejection: None,
                cancelled: false,
            });
            self.bake_status =
                Some("Baking static sun bounce; previous irradiance retained".into());
            Ok(())
        })();
        self.report(result)
    }
    /// Cancellation never edits the accepted document. The handle remains
    /// tracked until the bounded worker exits, preventing overlapping jobs.
    pub fn cancel_bake(&mut self) {
        if let Some(pending) = &mut self.pending_bake {
            pending.cancelled = true;
            pending.job.cancel();
            self.bake_status = Some("Cancelling bake; previous irradiance retained".into());
        }
    }
    /// Called before rendering regardless of whether the inspector is open.
    /// Check the *current* snapshot before publishing, not just the snapshot at
    /// launch. Selection/camera/exposure and excluded dynamic geometry are not
    /// members of the semantic participant fingerprint.
    pub fn sync_bake_for_editor(&mut self, editor: &Editor, models: &ModelPanel) {
        if self.pending_bake.is_some() {
            let eligibility = self.require_editable(editor);
            let current_key = self.preflight_key(editor, models);
            let pending = self.pending_bake.as_mut().expect("checked pending bake");
            if !pending.cancelled && pending.rejection.is_none() {
                let guard = eligibility.and_then(|()| {
                    let bindings = self.bindings.as_ref().ok_or("Irradiance was closed")?;
                    if self.binding_generation != pending.generation
                        || bindings.path != pending.sidecar
                        || bindings.revision() != pending.revision
                    {
                        return Err("Irradiance document changed while baking".into());
                    }
                    if current_key.directions != pending.input_key.directions
                        || current_key.triangle_tests != pending.input_key.triangle_tests
                        || current_key.node_visits != pending.input_key.node_visits
                        || current_key.duration != pending.input_key.duration
                    {
                        return Err("Bake settings changed while baking".into());
                    }
                    if current_key != pending.input_key {
                        self.snapshot_captures = self.snapshot_captures.saturating_add(1);
                        let fresh = BakeSceneSnapshot::capture(
                            editor,
                            models,
                            &bindings.document().grid,
                            &pending.settings,
                        )?;
                        if fresh.fingerprint != pending.fingerprint {
                            return Err("Static participants changed while baking".into());
                        }
                        if fresh.source_identity != pending.source_identity {
                            return Err("Static model source location changed while baking".into());
                        }
                        pending.input_key = current_key;
                    }
                    Ok(())
                });
                if let Err(error) = guard {
                    pending.rejection = Some(error);
                    pending.job.cancel();
                }
            }
            if let Some(outcome) = pending.job.poll() {
                let pending = self.pending_bake.take().expect("completed bake");
                if pending.cancelled {
                    self.bake_status = Some("Bake cancelled; previous irradiance retained".into());
                } else if let Some(reason) = pending.rejection {
                    self.bake_status = Some(format!(
                        "Bake discarded: {reason}. Previous irradiance retained"
                    ));
                } else {
                    let result = outcome.and_then(|output| {
                        self.require_editable(editor)?;
                        output.verify_sources_for_commit()?;
                        let scene = Self::local_scene(editor)?
                            .canonicalize()
                            .map_err(|e| e.to_string())?;
                        let bindings = self.bindings.as_mut().ok_or("Irradiance was closed")?;
                        if self.binding_generation != pending.generation
                            || bindings.path != pending.sidecar
                            || scene != output.scene_path
                            || output.fingerprint != pending.fingerprint
                            || output.source_identity != pending.source_identity
                        {
                            return Err("Bake completion no longer matches this scene".into());
                        }
                        bindings.commit_baked(pending.revision, output.grid, output.receipt)?;
                        self.verified_sources =
                            Some((output.fingerprint, output.source_identity, output.sources));
                        self.validation_failed_fingerprint = None;
                        self.failed_input_key = None;
                        Ok(())
                    });
                    match result {
                        Ok(()) => {
                            self.refresh_geometry();
                            self.bake_status = Some(
                                "Static sun bounce ready; save irradiance to keep this bake".into(),
                            );
                            self.error = None;
                        }
                        Err(error) => {
                            self.bake_status = Some(format!(
                                "Bake failed: {error}. Previous irradiance retained"
                            ));
                            self.error = Some(error);
                        }
                    }
                }
            }
        }
        self.refresh_baked_validity(editor, models);
    }
    /// Reopening a sidecar never trusts a receipt until the actual local
    /// participants have matched it. Failure disables only its application;
    /// coefficients, receipt, history, dirty state, and file remain intact.
    pub fn refresh_baked_validity(&mut self, editor: &Editor, models: &ModelPanel) {
        self.baked_fingerprint = None;
        self.baked_revision = None;
        self.baked_checksum = None;
        self.baked_stale_reason = None;
        if self.terrain_attached {
            self.cancel_validation();
            // Drain cancelled work so detaching can start fresh validation.
            if self.pending_validation.as_mut().and_then(|p| p.job.poll()).is_some() {
                self.pending_validation = None;
            }
            if self.bindings.as_ref().is_some_and(|b| b.document().grid.provenance == IrradianceProvenance::Baked) {
                self.baked_stale_reason = Some(Self::TERRAIN_BAKE_UNSUPPORTED.into());
            }
            return;
        }
        if let Some(outcome) = self.pending_validation.as_mut().and_then(|p| p.job.poll()) {
            let pending = self
                .pending_validation
                .take()
                .expect("completed source validation");
            if pending.generation == self.binding_generation
                && pending.input_key == self.preflight_key(editor, models)
            {
                match outcome {
                    Ok(output)
                        if output.fingerprint == pending.fingerprint
                            && output.scene_path == pending.scene_path
                            && output.source_identity == pending.source_identity =>
                    {
                        self.verified_sources =
                            Some((output.fingerprint, output.source_identity, output.sources));
                        self.validation_failed_fingerprint = None;
                        self.failed_input_key = None;
                        self.validated_input_key = Some(pending.input_key);
                    }
                    outcome => {
                        let error = outcome
                            .err()
                            .unwrap_or_else(|| "Source verification returned another scene".into());
                        self.validation_failed_fingerprint =
                            Some((pending.fingerprint, pending.source_identity));
                        self.failed_input_key = Some((pending.input_key, error));
                    }
                }
            }
        }
        let Some(bindings) = &self.bindings else {
            self.cancel_validation();
            return;
        };
        if bindings.document().grid.provenance != IrradianceProvenance::Baked {
            self.cancel_validation();
            return;
        }
        if !self.scene_matches(editor)
            || !editor.yard_rows_coherent()
            || editor.previewing().is_some()
        {
            if let Some(pending) = &self.pending_validation {
                pending.job.cancel();
            }
            self.baked_stale_reason =
                Some("Wait for the matching coherent local scene without a preview".into());
            return;
        }
        let input_key = self.preflight_key(editor, models);
        if let Err(error) = &input_key.participant_key {
            self.baked_stale_reason = Some(error.clone());
            return;
        }
        if let Some((failed_key, reason)) = &self.failed_input_key {
            if failed_key == &input_key {
                self.baked_stale_reason = Some(reason.clone());
                return;
            }
        }
        if let Some(pending) = &self.pending_validation {
            if pending.input_key != input_key {
                pending.job.cancel();
            }
            // Retain the bounded worker until it drains. In particular, a
            // root retarget must not copy a new snapshot on every wait frame.
            return;
        }
        // Check file identities every frame, but do not copy an unchanged
        // immutable model's texture/triangle buffers merely to redraw Edit.
        if self.validated_input_key.as_ref() == Some(&input_key)
            && self.scene_matches(editor)
            && editor.yard_rows_coherent()
            && editor.previewing().is_none()
        {
            if let Some((fingerprint, source_identity, sources)) = &self.verified_sources {
                if bindings
                    .document()
                    .bake_receipt
                    .as_ref()
                    .is_some_and(|r| &r.fingerprint == fingerprint)
                {
                    match sources.verify_current() {
                        Ok(()) => {
                            self.baked_fingerprint = Some(fingerprint.clone());
                            self.baked_revision = Some(bindings.revision());
                            self.baked_checksum = Some(editor.checksum());
                            return;
                        }
                        Err(error) => {
                            self.validation_failed_fingerprint =
                                Some((fingerprint.clone(), source_identity.clone()));
                            self.baked_stale_reason = Some(error.clone());
                            self.failed_input_key = Some((input_key, error));
                            self.verified_sources = None;
                            self.validated_input_key = None;
                            return;
                        }
                    }
                }
            }
        }
        self.snapshot_captures = self.snapshot_captures.saturating_add(1);
        let result: Result<BakeSceneSnapshot, String> = (|| {
            if !self.scene_matches(editor) {
                return Err("The local scene changed".into());
            }
            let receipt = bindings
                .document()
                .bake_receipt
                .as_ref()
                .ok_or("Bake receipt is missing")?;
            let settings = BakeSettings {
                rays_per_probe: receipt.directions,
                max_triangle_tests: receipt.max_triangle_tests,
                max_node_visits: receipt.max_node_visits,
                max_duration: std::time::Duration::from_nanos(receipt.max_duration_nanos),
            };
            let snapshot = BakeSceneSnapshot::capture_for_validation(
                editor,
                models,
                &bindings.document().grid,
                &settings,
            )?;
            if snapshot.fingerprint != receipt.fingerprint {
                return Err("Static geometry, material, sun, or probe layout changed".into());
            }
            Ok(snapshot)
        })();
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(pending) = &self.pending_validation {
                    pending.job.cancel();
                }
                self.baked_stale_reason = Some(error.clone());
                self.failed_input_key = Some((input_key, error));
                return;
            }
        };
        if let Some((fingerprint, source_identity, sources)) = &self.verified_sources {
            if fingerprint == &snapshot.fingerprint && source_identity == &snapshot.source_identity
            {
                match sources.verify_current() {
                    Ok(()) => {
                        self.baked_fingerprint = Some(fingerprint.clone());
                        self.baked_revision = Some(bindings.revision());
                        self.baked_checksum = Some(editor.checksum());
                        self.validated_input_key = Some(input_key);
                        return;
                    }
                    Err(error) => {
                        self.validation_failed_fingerprint = Some((
                            snapshot.fingerprint.clone(),
                            snapshot.source_identity.clone(),
                        ));
                        self.baked_stale_reason = Some(error.clone());
                        self.failed_input_key = Some((input_key, error));
                        self.verified_sources = None;
                        return;
                    }
                }
            }
        }
        if self.validation_failed_fingerprint.as_ref().is_some_and(
            |(fingerprint, source_identity)| {
                fingerprint == &snapshot.fingerprint && source_identity == &snapshot.source_identity
            },
        ) {
            let error = "Source verification failed; reopen repaired model bindings and irradiance, or rebake".to_string();
            self.baked_stale_reason = Some(error.clone());
            self.failed_input_key = Some((input_key, error));
            return;
        }
        if self.pending_validation.is_none() && self.pending_bake.is_none() {
            let fingerprint = snapshot.fingerprint.clone();
            let source_identity = snapshot.source_identity.clone();
            let scene_path = snapshot.scene_path.clone();
            match BakeValidationJob::start(snapshot) {
                Ok(job) => {
                    self.pending_validation = Some(PendingValidation {
                        job,
                        fingerprint,
                        source_identity,
                        scene_path,
                        generation: self.binding_generation,
                        input_key,
                    })
                }
                Err(error) => {
                    self.baked_stale_reason = Some(error);
                }
            }
        }
    }
    fn show_bake(&mut self, ui: &mut Ui, editor: &Editor, models: &ModelPanel) {
        ui.separator();
        ui.strong("Static sun bounce");
        if self.terrain_attached { ui.colored_label(egui::Color32::YELLOW, Self::TERRAIN_BAKE_UNSUPPORTED); }
        ui.weak("One diffuse bounce from opaque static surfaces under the Yard sun. Dynamic/kinematic bodies, animated models, point lights and emission do not contribute. Unsupported static input is rejected.");
        if self.is_validating_bake() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking baked source files before enabling irradiance");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if let Some(reason) = &self.baked_stale_reason {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Stale bake disabled: {reason}. Rebake to refresh"),
            );
        }
        if let Some((complete, total)) = self.bake_progress() {
            ui.add(
                egui::ProgressBar::new(complete as f32 / total.max(1) as f32)
                    .text(format!("Static bounce: {complete}/{total}")),
            );
            if ui.button("Cancel static sun bounce").clicked() {
                self.cancel_bake();
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            ui.add_enabled_ui(self.editable(editor) && !self.is_validating_bake() && !self.terrain_attached, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Directions per probe (even)");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.bake_settings.rays_per_probe)
                                .range(64..=2048)
                                .speed(2.0),
                        )
                        .changed()
                    {
                        self.bake_preflight = None;
                    }
                });
                if self.bake_preflight.is_none()
                    || self.bake_preflight_key.as_ref() != Some(&self.preflight_key(editor, models))
                {
                    let _ = self.preflight_bake(editor, models);
                }
                if let Some(preflight) = &self.bake_preflight {
                    ui.label(preflight);
                }
                ui.weak(format!(
                    "Worker limits: {} triangle tests · {} BVH visits · {:.0}s",
                    self.bake_settings.max_triangle_tests,
                    self.bake_settings.max_node_visits,
                    self.bake_settings.max_duration.as_secs_f32()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Analyze static participants").clicked() {
                        let _ = self.preflight_bake(editor, models);
                    }
                    if ui.button("Bake static sun bounce").clicked() {
                        let _ = self.start_bake(editor, models);
                    }
                });
            });
        }
        if let Some(status) = &self.bake_status {
            ui.label(status);
        }
        ui.separator();
    }
    pub fn discard(&mut self) {
        self.cancel_bake();
        self.cancel_validation();
        self.binding_generation = self.binding_generation.wrapping_add(1);
        self.baked_fingerprint = None;
        self.baked_revision = None;
        self.baked_checksum = None;
        self.baked_stale_reason = None;
        self.bake_preflight = None;
        self.bindings = None;
        self.confirm_discard = false;
        self.error = None;
    }
    /// Legacy authored/imported controls without model context. Baking is only
    /// exposed by `show_with_models`, which receives the actual viewport assets.
    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        self.show_inner(ui, editor, None);
    }
    pub fn show_with_models(&mut self, ui: &mut Ui, editor: &Editor, models: &ModelPanel) {
        self.show_inner(ui, editor, Some(models));
    }
    fn show_inner(&mut self, ui: &mut Ui, editor: &Editor, models: Option<&ModelPanel>) {
        if editor.game() != EditorGame::Yard3D {
            return;
        }
        self.sync_for_editor(editor);
        ui.collapsing("Irradiance probes", |ui| {
            ui.weak("Diffuse SH9 · separate sidecar save and undo");
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
                if let Some(models) = models {
                    self.show_bake(ui, editor, models);
                }
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
