#[path="model_package_import_panel/static_planar_ui.rs"]
mod static_planar_ui;
#[path="model_package_import_panel/pose_bake_ui.rs"]
mod pose_bake_ui;
#[path="model_package_import_panel/static_recolor_ui.rs"]
mod static_recolor_ui;
#[path="model_package_import_panel/static_sampler_ui.rs"]
mod static_sampler_ui;
#[path="model_package_import_panel/static_pivot_ui.rs"]
mod static_pivot_ui;
#[path="model_package_import_panel/glb_subtree_ui.rs"]
mod glb_subtree_ui;
#[path="model_package_import_panel/static_assembly_ui.rs"]
mod static_assembly_ui;
#[path="model_package_import_panel/static_texture_ui.rs"]
mod static_texture_ui;
#[path="model_package_import_panel/static_uv_ui.rs"]
mod static_uv_ui;
#[path="model_package_import_panel/uv_ui.rs"]
mod uv_ui;
#[path="model_package_import_panel/sequence_ui.rs"]
mod sequence_ui;
#[path="model_package_import_panel/sampler_ui.rs"]
mod sampler_ui;
#[path="model_package_import_panel/trim_ui.rs"]
mod trim_ui;
#[cfg(feature = "room-animated-models")]
#[path = "model_package_import_panel/variant_preview.rs"]
mod variant_preview;
#[cfg(feature = "room-animated-models")]
pub(crate) use variant_preview::PreviewFrame;
#[path = "model_package_import_panel/source_inspection.rs"]
mod source_inspection;
use crate::model_package_import::{
    self, ColorVariant, Report, Request, SourceFormat, TextureVariant,
};
use egui::Ui;
use orr_model_bindings::model_bindings::ModelKind;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct Completion {
    source_format: SourceFormat,
    project_root: PathBuf,
    expected_destination: Result<PathBuf, String>,
    project_generation: u64,
    source_generation: u64,
    result: Result<Report, String>,
}

pub struct Panel {
    source: String,
    license: String,
    destination: String,
    name: String,
    version: String,
    kind: ModelKind,
    source_format: SourceFormat,
    license_acknowledged: bool,
    pending: Option<Receiver<Completion>>,
    destination_identity: Option<PathBuf>,
    status: String,
    trim: trim_ui::Controls,
    sampler: sampler_ui::Controls,
    sequence: sequence_ui::Controls,
    uv: uv_ui::Controls,
    static_uv: static_uv_ui::Controls,
    submitted_static_uv: Option<Option<static_uv_ui::Snapshot>>,
    static_texture: static_texture_ui::Controls,
    submitted_static_texture: Option<Option<static_texture_ui::Snapshot>>,
    static_assembly: static_assembly_ui::Controls,
    submitted_static_assembly: Option<Option<static_assembly_ui::Snapshot>>,
    glb_subtree: glb_subtree_ui::Controls,
    submitted_glb_subtree: Option<Option<glb_subtree_ui::Snapshot>>,
    static_pivot: static_pivot_ui::Controls,
    submitted_static_pivot: Option<Option<static_pivot_ui::Snapshot>>,
    static_sampler: static_sampler_ui::Controls,
    submitted_static_sampler: Option<Option<static_sampler_ui::Snapshot>>,
    static_recolor: static_recolor_ui::Controls,
    submitted_static_recolor: Option<Option<static_recolor_ui::Snapshot>>,
    pose_bake: pose_bake_ui::Controls,
    submitted_pose_bake: Option<Option<pose_bake_ui::Snapshot>>,
    static_planar: static_planar_ui::Controls,
    submitted_static_planar: Option<Option<static_planar_ui::Snapshot>>,
    submitted_uv: Option<Option<model_package_import::UvVariant>>,
    submitted_sequence: Option<Option<model_package_import::SequenceVariant>>,
    submitted_sampler: Option<Option<model_package_import::SamplerVariant>>,
    submitted_trim: Option<Option<model_package_import::TrimVariant>>,
    authoring_generation: u64,
    submitted_authoring_generation: Option<u64>,
    color_variant_enabled: bool,
    color_variant: ColorVariant,
    submitted_color: Option<Option<ColorVariant>>,
    texture_variant_enabled: bool,
    texture_variant: TextureVariant,
    submitted_texture: Option<Option<TextureVariant>>,
    inspection: source_inspection::State,
    inspected_hash: Option<String>,
    submitted_inspected_hash: Option<Option<String>>,
    #[cfg(feature = "room-animated-models")]
    variant_preview: variant_preview::State,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            source: String::new(),
            license: String::new(),
            destination: String::new(),
            name: String::new(),
            version: String::new(),
            kind: ModelKind::Static,
            source_format: SourceFormat::default(),
            license_acknowledged: false,
            pending: None,
            destination_identity: None,
            status: String::new(),
            trim: Default::default(),
            sampler: Default::default(),
            sequence: Default::default(),
            uv: Default::default(),
            static_uv: Default::default(),
            submitted_static_uv: None,
            static_texture: Default::default(),
            submitted_static_texture: None,
            static_assembly: Default::default(),
            submitted_static_assembly: None,
            glb_subtree: Default::default(),
            submitted_glb_subtree: None,
            static_pivot: Default::default(),
            submitted_static_pivot: None,
            static_sampler: Default::default(),
            submitted_static_sampler: None,
            static_recolor: Default::default(),
            submitted_static_recolor: None,
            pose_bake: Default::default(),
            submitted_pose_bake: None,
            static_planar: Default::default(),
            submitted_static_planar: None,
            submitted_uv: None,
            submitted_sequence: None,
            submitted_sampler: None,
            submitted_trim: None,
            authoring_generation: 0,
            submitted_authoring_generation: None,
            color_variant_enabled: false,
            color_variant: default_color_variant(),
            submitted_color: None,
            texture_variant_enabled: false,
            texture_variant: default_texture_variant(),
            submitted_texture: None,
            inspection: Default::default(),
            inspected_hash: None,
            submitted_inspected_hash: None,
            #[cfg(feature = "room-animated-models")]
            variant_preview: Default::default(),
        }
    }
}

impl Panel {
    pub fn busy(&self) -> bool {
        let busy = self.pending.is_some() || self.inspection.busy();
        #[cfg(feature = "room-animated-models")]
        {
            busy || self.variant_preview.busy()
        }
        #[cfg(not(feature = "room-animated-models"))]
        {
            busy
        }
    }

    pub fn poll(
        &mut self,
        project_root: Option<&Path>,
        project_generation: u64,
        source_generation: u64,
    ) -> Option<Report> {
        self.sync_inspection(project_root, project_generation, source_generation);
        #[cfg(feature = "room-animated-models")]
        self.service_variant_preview(project_root, project_generation, source_generation);
        let Some(receiver) = &self.pending else {
            return None;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.submitted_color = None;
                self.submitted_texture = None;
                self.submitted_trim = None;
                self.submitted_sampler = None;
                self.submitted_sequence = None;
                self.submitted_uv = None;
                self.submitted_static_uv = None;
                self.submitted_static_texture = None;
                self.submitted_static_assembly = None;
                self.submitted_glb_subtree = None;
                self.submitted_static_pivot = None;
                self.submitted_static_sampler = None;
                self.submitted_static_recolor = None;
                self.submitted_pose_bake = None;
                self.submitted_static_planar = None;
                self.submitted_authoring_generation = None;
                self.submitted_inspected_hash = None;
                self.license_acknowledged = false;
                self.destination_identity = None;
                self.status = "Model import worker disconnected before returning a result. Publication may have completed; inspect the destination before retrying. No automatic retry.".into();
                return None;
            }
        };
        self.pending = None;
        self.license_acknowledged = false;
        let submitted_static_uv = self.submitted_static_uv.take().unwrap_or(None);
        let static_uv_is_current = project_root.is_some_and(|root| self.static_uv_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_uv));
        let submitted_static_texture = self.submitted_static_texture.take().unwrap_or(None);
        let static_texture_is_current = project_root.is_some_and(|root| self.static_texture_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_texture));
        let submitted_static_assembly = self.submitted_static_assembly.take().unwrap_or(None);
        let static_assembly_is_current = project_root.is_some_and(|root| self.static_assembly_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_assembly));
        let submitted_glb_subtree = self.submitted_glb_subtree.take().unwrap_or(None);
        let glb_subtree_is_current = project_root.is_some_and(|root| self.glb_subtree_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_glb_subtree));
        let submitted_static_pivot = self.submitted_static_pivot.take().unwrap_or(None);
        let static_pivot_is_current = project_root.is_some_and(|root| self.static_pivot_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_pivot));
        let submitted_static_sampler = self.submitted_static_sampler.take().unwrap_or(None);
        let static_sampler_is_current = project_root.is_some_and(|root| self.static_sampler_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_sampler));
        let submitted_static_recolor = self.submitted_static_recolor.take().unwrap_or(None);
        let static_recolor_is_current = project_root.is_some_and(|root| self.static_recolor_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_recolor));
        let submitted_pose_bake = self.submitted_pose_bake.take().unwrap_or(None);
        let pose_bake_is_current = project_root.is_some_and(|root| self.pose_bake_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_pose_bake));
        let submitted_static_planar = self.submitted_static_planar.take().unwrap_or(None);
        let static_planar_is_current = project_root.is_some_and(|root| self.static_planar_snapshot(root, project_generation, source_generation).is_ok_and(|snapshot| snapshot == submitted_static_planar));
        let submitted_color = self.submitted_color.take().unwrap_or(None);
        let color_is_current = self
            .color_request()
            .ok()
            .is_some_and(|color| color == submitted_color);
        let submitted_texture = self.submitted_texture.take().unwrap_or(None);
        let texture_is_current = self
            .texture_request()
            .ok()
            .is_some_and(|texture| texture == submitted_texture);
        let submitted_uv=self.submitted_uv.take().unwrap_or(None);
        let uv_is_current=self.uv_request().ok().is_some_and(|value|value==submitted_uv);
        let submitted_sequence=self.submitted_sequence.take().unwrap_or(None);
        let sequence_is_current=self.sequence_request().ok().is_some_and(|value|value==submitted_sequence);
        let submitted_sampler=self.submitted_sampler.take().unwrap_or(None);
        let sampler_is_current=self.sampler_request().ok().is_some_and(|value|value==submitted_sampler);
        let submitted_trim = self.submitted_trim.take().unwrap_or(None);
        let trim_is_current = self.trim_request().ok().is_some_and(|trim| trim == submitted_trim);
        let generation_is_current = self.submitted_authoring_generation.take().is_none_or(|generation| generation == self.authoring_generation);
        let inspected_hash_is_current =
            self.submitted_inspected_hash.take().unwrap_or(None) == self.inspected_hash;
        let is_current = static_planar_is_current && pose_bake_is_current && static_recolor_is_current && static_sampler_is_current && static_pivot_is_current && glb_subtree_is_current && static_assembly_is_current && static_texture_is_current && static_uv_is_current && uv_is_current && sequence_is_current && sampler_is_current && trim_is_current && generation_is_current && inspected_hash_is_current
            && texture_is_current
            && color_is_current
            && project_root == Some(completion.project_root.as_path())
            && project_generation == completion.project_generation
            && source_generation == completion.source_generation
            && self.source_format == completion.source_format;
        match completion.result {
            Ok(report)
                if is_current && completion.expected_destination.as_ref() == Ok(&report.root) =>
            {
                self.destination_identity = Some(report.root.clone());
                self.status = format!(
                    "Imported {}/{} ({:?}). Asset SHA-256: {}; license SHA-256: {}. The package is not installed or assigned; preview and add it explicitly.",
                    report.package,
                    report.asset,
                    report.kind,
                    report.source_sha256,
                    report.license_sha256
                );
                Some(report)
            }
            Ok(report) if is_current => {
                self.destination_identity = None;
                self.status = format!(
                    "Importer returned unexpected package root {}; expected {}. It was not selected. Inspect the project before continuing.",
                    report.root.display(),
                    completion.expected_destination.as_ref().map_or_else(
                        |error| format!("an unavailable canonical destination ({error})"),
                        |path| path.display().to_string()
                    )
                );
                None
            }
            Ok(report) => {
                self.destination_identity = None;
                self.status = format!(
                    "Imported package {} for a prior project/source state; it was not selected into the current project.",
                    report.root.display()
                );
                None
            }
            Err(error) if is_current => {
                self.destination_identity = None;
                self.status = format!(
                    "{} package import failed: {error}",
                    format_label(completion.source_format)
                );
                None
            }
            Err(error) => {
                self.destination_identity = None;
                self.status = format!(
                    "{} package import for a prior project/source state failed: {error}",
                    format_label(completion.source_format)
                );
                None
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        enabled: bool,
        project_root: Option<&Path>,
        project_generation: u64,
        source_generation: u64,
    ) {
        self.sync_inspection(project_root, project_generation, source_generation);
        #[cfg(feature = "room-animated-models")]
        self.service_variant_preview(project_root, project_generation, source_generation);
        egui::CollapsingHeader::new("Import a model package")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Import a local self-contained GLB, cooked static model or cooked animated model and a license file into a new package folder.");
                ui.label("Cooked import preserves exact bytes unless you explicitly create a static UV variant, static planar UV variant, static PNG skin, static two-copy assembly, static pivot variant, static sampler variant, static color variant or an animated color variant, PNG skin, trimmed clip, sampler variant, clip sequence or UV variant. Historical dependencies are not loaded or added to the package.");
                ui.label("External resources, scripts, network access and package activation are not supported.");
                ui.label("A supplied license file is metadata only; its presence is not legal review or a rights determination.");
                let fields_enabled = enabled && !self.busy();
                ui.add_enabled_ui(fields_enabled, |ui| {
                    let mut source_format = self.source_format;
                    ui.horizontal(|ui| {
                        ui.label("Source format");
                        ui.radio_value(&mut source_format, SourceFormat::RawGlb, "Raw GLB");
                        ui.radio_value(&mut source_format, SourceFormat::CookedStatic, "Cooked static model");
                        ui.add_enabled_ui(cfg!(feature = "room-animated-models"), |ui| {
                            ui.radio_value(&mut source_format, SourceFormat::CookedAnimated, "Cooked animated model");
                        });
                    });
                    self.set_source_format(source_format);
                    let source_label = match self.source_format {
                        SourceFormat::RawGlb => "Raw GLB file",
                        SourceFormat::CookedStatic => "Cooked static model file (.model.json)",
                        SourceFormat::CookedAnimated => "Cooked animated model file (.animated.json)",
                    };
                    let source_changed = path_field(ui, source_label, &mut self.source);
                    let license_changed = path_field(ui, "License file", &mut self.license);
                    if source_changed { self.reset_color_variant(); }
                    if source_changed || license_changed {
                        self.source_inputs_changed();
                    }
                    let old_package = (self.destination.clone(), self.name.clone(), self.version.clone());
                    path_field(ui, "Package destination directory", &mut self.destination);
                    text_field(ui, "Package name", &mut self.name, 128);
                    text_field(ui, "Package version", &mut self.version, 128);
                    if (self.static_uv.enabled || self.static_texture.enabled || self.static_assembly.enabled || self.glb_subtree.enabled || self.static_pivot.enabled || self.static_sampler.enabled || self.static_recolor.enabled || self.pose_bake.enabled || self.static_planar.enabled) && old_package != (self.destination.clone(), self.name.clone(), self.version.clone()) { self.source_inputs_changed(); }
                    let old_kind = self.kind;
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(self.source_format != SourceFormat::CookedAnimated, |ui| {
                            ui.radio_value(&mut self.kind, ModelKind::Static, "Static model");
                        });
                        #[cfg(feature = "room-animated-models")]
                        ui.add_enabled_ui(self.source_format != SourceFormat::CookedStatic, |ui| {
                            ui.radio_value(&mut self.kind, ModelKind::Animated, "Animated model");
                        });
                        #[cfg(not(feature = "room-animated-models"))]
                        ui.add_enabled(false, egui::RadioButton::new(false, "Animated model"));
                    });
                    if old_kind != self.kind { self.reset_color_variant(); self.source_inputs_changed(); }
                    if cfg!(feature = "room-animated-models") && self.source_format == SourceFormat::CookedAnimated && self.kind == ModelKind::Animated {
                        let old = (self.color_variant_enabled, self.color_variant.clone(), self.texture_variant_enabled, self.texture_variant.clone());
                        if ui.checkbox(&mut self.color_variant_enabled, "Create animated color variant").changed() && self.color_variant_enabled { self.texture_variant_enabled=false; self.trim.enabled=false; self.sampler.enabled=false; self.sequence.enabled=false; self.uv.enabled=false; self.pose_bake.enabled=false; }
                        if ui.checkbox(&mut self.texture_variant_enabled, "Create animated PNG skin").changed() && self.texture_variant_enabled { self.color_variant_enabled=false; self.trim.enabled=false; self.sampler.enabled=false; self.sequence.enabled=false; self.uv.enabled=false; self.pose_bake.enabled=false; }
                        if self.color_variant_enabled {
                            ui.weak("Replace one material's linear-light RGB factor (0..1). Texture, alpha and animation stay unchanged; this is not an sRGB color picker.");
                            ui.horizontal(|ui| { let label=ui.label("Variant material index");
                                ui.add(egui::DragValue::new(&mut self.color_variant.material)).labelled_by(label.id); });
                            // Stable labels let keyboard/accessibility editing use the actual numeric fields.
                            for (index,label) in ["Variant linear R (0..1)","Variant linear G (0..1)","Variant linear B (0..1)"].into_iter().enumerate() {
                                ui.horizontal(|ui| { let label=ui.label(label); ui.add(egui::DragValue::new(&mut self.color_variant.linear_rgb[index]).speed(0.01).range(0.0..=1.0)).labelled_by(label.id); });
                            }
                            text_field(ui,"Variant asset ID",&mut self.color_variant.asset_id,240);
                        }
                        if self.texture_variant_enabled {
                            let mut texture = self.texture_variant.texture.to_string_lossy().into_owned();
                            path_field(ui,"Variant PNG file",&mut texture);
                            self.texture_variant.texture=PathBuf::from(texture);
                            ui.horizontal(|ui| { let label=ui.label("Variant material index");ui.add(egui::DragValue::new(&mut self.texture_variant.material)).labelled_by(label.id); });
                            text_field(ui,"Variant asset ID",&mut self.texture_variant.asset_id,240);
                            ui.weak("Replace one material's embedded PNG image. The rights acknowledgement below covers this PNG as well as the model and license.");
                        }
                        if old.1.material != self.color_variant.material || old.3.material != self.texture_variant.material { self.inspected_hash=None; }
                        if old != (self.color_variant_enabled,self.color_variant.clone(),self.texture_variant_enabled,self.texture_variant.clone()) { self.source_inputs_changed(); }
                    }
                    self.show_trim_controls(ui);
                    self.show_sampler_controls(ui);
                    self.show_sequence_controls(ui);
                    self.show_uv_controls(ui);
                    self.show_static_uv_controls(ui);
                    self.show_static_texture_controls(ui);
                    self.show_static_assembly_controls(ui);
                    self.show_glb_subtree_controls(ui);
                    self.show_static_pivot_controls(ui);
                    self.show_static_sampler_controls(ui);
                    self.show_static_recolor_controls(ui);
                    self.show_pose_bake_controls(ui);
                    self.show_static_planar_controls(ui);
                    self.show_source_inspection(ui,project_root,project_generation,source_generation);
                    ui.checkbox(
                        &mut self.license_acknowledged,
                        "I have reviewed rights to distribute this content",
                    );
                });
                #[cfg(feature="room-animated-models")]
                self.show_variant_preview(ui,enabled,project_root,project_generation,source_generation);
                #[cfg(not(feature = "room-animated-models"))]
                ui.small("Animated import requires the room-animated-models feature; it never falls back to static.");
                if self.source_format == SourceFormat::CookedStatic {
                    ui.small("Cooked import requires Static model mode and never falls back to GLB parsing.");
                }
                if self.source_format == SourceFormat::CookedAnimated {
                    ui.small("Cooked animated import requires Animated model mode and room-animated-models support; it never falls back to GLB or static parsing.");
                }
                let compatible_kind = match self.source_format {
                    SourceFormat::RawGlb => self.kind == ModelKind::Static || cfg!(feature = "room-animated-models"),
                    SourceFormat::CookedStatic => self.kind == ModelKind::Static,
                    SourceFormat::CookedAnimated => self.kind == ModelKind::Animated && cfg!(feature = "room-animated-models"),
                };
                let request_ready = fields_enabled
                    && project_root.is_some()
                    && self.license_acknowledged
                    && !self.source.trim().is_empty()
                    && !self.license.trim().is_empty()
                    && !self.destination.trim().is_empty()
                    && !self.name.trim().is_empty()
                    && !self.version.trim().is_empty()
                    && compatible_kind
                    && self.color_request().is_ok()
                    && self.texture_request().is_ok()
                    && self.trim_request().is_ok()
                    && self.sampler_request().is_ok()
                    && self.sequence_request().is_ok()
                    && self.uv_request().is_ok()
                    && self.static_uv_request().is_ok()
                    && self.static_texture_request().is_ok()
                    && self.static_assembly_request().is_ok()
                    && self.glb_subtree_request().is_ok()
                    && self.static_pivot_request().is_ok()
                    && self.static_sampler_request().is_ok()
                    && self.static_recolor_request().is_ok()
                    && self.pose_bake_request().is_ok()
                    && self.static_planar_request().is_ok();
                let import_label = match self.source_format {
                    SourceFormat::RawGlb if self.glb_subtree.enabled => "Import GLB subtree package",
                    SourceFormat::RawGlb => "Import raw GLB package",
                    SourceFormat::CookedStatic => "Import cooked static package",
                    SourceFormat::CookedAnimated if self.pose_bake.enabled => "Import baked static pose package",
                    SourceFormat::CookedAnimated => "Import cooked animated package",
                };
                if ui
                    .add_enabled(request_ready, egui::Button::new(import_label))
                    .clicked()
                {
                    match project_root {
                        Some(project_root) => self.start_import(
                            project_root.to_path_buf(),
                            project_generation,
                            source_generation,
                        ),
                        None => {
                            self.status = "Open a Room project before importing a model package.".into();
                        }
                    }
                }
                if self.busy() {
                    ui.spinner();
                }
                if let Some(destination) = &self.destination_identity {
                    ui.label(format!("Last imported package root: {}", destination.display()));
                }
            });
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    fn start_import(
        &mut self,
        project_root: PathBuf,
        project_generation: u64,
        source_generation: u64,
    ) {
        if self.busy() {
            return;
        }
        if self.authoring_generation == u64::MAX { self.status="Authoring generation exhausted; reopen the panel".into(); return; }
        self.sync_inspection(Some(&project_root), project_generation, source_generation);
        let static_uv = match self.static_uv_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_uv = match self.static_uv_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_texture = match self.static_texture_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_texture = match self.static_texture_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_assembly = match self.static_assembly_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_assembly = match self.static_assembly_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let glb_subtree = match self.glb_subtree_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_glb_subtree = match self.glb_subtree_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_pivot = match self.static_pivot_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_pivot = match self.static_pivot_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_sampler = match self.static_sampler_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_sampler = match self.static_sampler_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_recolor = match self.static_recolor_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_recolor = match self.static_recolor_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let pose_bake = match self.pose_bake_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_pose_bake = match self.pose_bake_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let static_planar = match self.static_planar_request() { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let submitted_static_planar = match self.static_planar_snapshot(&project_root, project_generation, source_generation) { Ok(value) => value, Err(error) => { self.status=error; return; } };
        let trim = match self.trim_request() { Ok(trim) => trim, Err(error) => { self.status=error; return; } };
        let submitted_trim = trim.clone();
        let sampler=match self.sampler_request(){Ok(value)=>value,Err(error)=>{self.status=error;return;}};
        let submitted_sampler=sampler.clone();
        let sequence=match self.sequence_request(){Ok(value)=>value,Err(error)=>{self.status=error;return;}};
        let submitted_sequence=sequence.clone();
        let uv=match self.uv_request(){Ok(value)=>value,Err(error)=>{self.status=error;return;}};
        let submitted_uv=uv.clone();
        let color = match self.color_request() {
            Ok(color) => color,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let texture = match self.texture_request() {
            Ok(texture) => texture,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let expected_hash = self.inspected_hash.clone();
        let submitted_inspected_hash = expected_hash.clone();
        let submitted_texture = texture.clone();
        let submitted_color = color.clone();
        let request = self.request();
        let source_format = request.source_format;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-model-package-import".into())
            .spawn(move || {
                let expected_destination = request_destination(&request.destination);
                let result = match (glb_subtree, static_pivot, static_sampler, static_recolor, pose_bake, static_planar) {
                    (Some(spec), None, None, None, None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_glb_subtree_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_glb_subtree_variant(request, spec),
                    },
                    (None, Some(spec), None, None, None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_static_pivot_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_static_pivot_variant(request, spec),
                    },
                    (None, None, Some(spec), None, None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_static_sampler_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_static_sampler_variant(request, spec),
                    },
                    (None, None, None, Some(spec), None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_static_color_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_static_color_variant(request, spec),
                    },
                    (None, None, None, None, Some(spec), None) => match expected_hash {
                        Some(hash) => model_package_import::import_pose_bake_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_pose_bake_variant(request, spec),
                    },
                    (None, None, None, None, None, Some(spec)) => match expected_hash {
                        Some(hash) => model_package_import::import_static_planar_uv_variant_checked(request, spec, Some(hash)),
                        None => model_package_import::import_static_planar_uv_variant(request, spec),
                    },
                    (None, None, None, None, None, None) => match (color, texture, trim, sampler, sequence, uv, static_uv, static_texture, static_assembly) {
                    (Some(color), None, None, None, None, None, None, None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_color_variant_checked(
                            request,
                            color,
                            Some(hash),
                        ),
                        None => model_package_import::import_color_variant(request, color),
                    },
                    (None, Some(texture), None, None, None, None, None, None, None) => match expected_hash {
                        Some(hash) => model_package_import::import_texture_variant_checked(
                            request,
                            texture,
                            Some(hash),
                        ),
                        None => model_package_import::import_texture_variant(request, texture),
                    },
                    (None, None, Some(trim), None, None, None, None, None, None) => match expected_hash { Some(hash) => model_package_import::import_trim_variant_checked(request, trim, Some(hash)), None => model_package_import::import_trim_variant(request, trim) },
                    (None, None, None, Some(sampler), None, None, None, None, None) => match expected_hash {Some(hash)=>model_package_import::import_sampler_variant_checked(request,sampler,Some(hash)),None=>model_package_import::import_sampler_variant(request,sampler)},
                    (None,None,None,None,Some(sequence),None,None,None,None) => match expected_hash {Some(hash)=>model_package_import::import_sequence_variant_checked(request,sequence,Some(hash)),None=>model_package_import::import_sequence_variant(request,sequence)},
                    (None,None,None,None,None,Some(uv),None,None,None)=>match expected_hash{Some(hash)=>model_package_import::import_uv_variant_checked(request,uv,Some(hash)),None=>model_package_import::import_uv_variant(request,uv)},
                    (None,None,None,None,None,None,Some(static_uv),None,None) => match expected_hash {Some(hash)=>model_package_import::import_static_uv_variant_checked(request,static_uv,Some(hash)),None=>model_package_import::import_static_uv_variant(request,static_uv)},
                    (None,None,None,None,None,None,None,Some(static_texture),None) => match expected_hash {Some(hash)=>model_package_import::import_static_texture_variant_checked(request,static_texture,Some(hash)),None=>model_package_import::import_static_texture_variant(request,static_texture)},
                    (None,None,None,None,None,None,None,None,Some(static_assembly)) => match expected_hash {Some(hash)=>model_package_import::import_static_assembly_variant_checked(request,static_assembly,Some(hash)),None=>model_package_import::import_static_assembly_variant(request,static_assembly)},
                    (None, None, None, None, None, None, None, None, None) => model_package_import::import_package(request),
                    _ => Err("Select only one authoring operation".into()),
                    },
                    _ => Err("Select only one authoring operation".into()),
                };
                let _ = sender.send(Completion {
                    source_format,
                    project_root,
                    expected_destination,
                    project_generation,
                    source_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.destination_identity = None;
                self.pending = Some(receiver);
                self.submitted_trim = Some(submitted_trim);
                self.submitted_sampler = Some(submitted_sampler);
                self.submitted_sequence = Some(submitted_sequence);
                self.submitted_uv = Some(submitted_uv);
                self.submitted_static_uv = Some(submitted_static_uv);
                self.submitted_static_texture = Some(submitted_static_texture);
                self.submitted_static_assembly = Some(submitted_static_assembly);
                self.submitted_glb_subtree = Some(submitted_glb_subtree);
                self.submitted_static_pivot = Some(submitted_static_pivot);
                self.submitted_static_sampler = Some(submitted_static_sampler);
                self.submitted_static_recolor = Some(submitted_static_recolor);
                self.submitted_pose_bake = Some(submitted_pose_bake);
                self.submitted_static_planar = Some(submitted_static_planar);
                self.submitted_authoring_generation = Some(self.authoring_generation);
                self.submitted_color = Some(submitted_color);
                self.submitted_texture = Some(submitted_texture);
                self.submitted_inspected_hash = Some(submitted_inspected_hash);
                self.status = match source_format {
                    SourceFormat::RawGlb => {
                        "Importing and validating the self-contained GLB on a worker…"
                    }
                    SourceFormat::CookedAnimated => {
                        "Importing and validating the cooked animated model on a worker..."
                    }
                    SourceFormat::CookedStatic => {
                        "Importing and validating the cooked static model on a worker…"
                    }
                }
                .into();
            }
            Err(error) => {
                self.status = format!(
                    "Cannot start {} package import: {error}",
                    format_label(source_format)
                )
            }
        }
    }

    fn set_source_format(&mut self, source_format: SourceFormat) {
        if self.source_format != source_format {
            self.source_format = source_format;
            match source_format {
                SourceFormat::CookedStatic => self.kind = ModelKind::Static,
                SourceFormat::CookedAnimated => self.kind = ModelKind::Animated,
                SourceFormat::RawGlb => {}
            }
            self.reset_color_variant();
            self.source_inputs_changed();
        }
    }

    fn source_inputs_changed(&mut self) {
        self.authoring_generation = self.authoring_generation.saturating_add(1);
        #[cfg(feature="room-animated-models")]
        self.close_variant_preview();
        self.license_acknowledged = false;
        self.destination_identity = None;
    }

    fn reset_color_variant(&mut self) {
        self.inspected_hash = None;
        #[cfg(feature = "room-animated-models")]
        self.close_variant_preview();
        self.inspection.invalidate();
        self.trim = Default::default();
        self.sampler = Default::default();
        self.sequence = Default::default();
        self.uv = Default::default();
        self.static_uv = Default::default();
        self.static_texture = Default::default();
        self.static_assembly = Default::default();
        self.glb_subtree = Default::default();
        self.static_pivot = Default::default();
        self.static_sampler = Default::default();
        self.static_recolor = Default::default();
        self.pose_bake = Default::default();
        self.static_planar = Default::default();
        self.color_variant_enabled = false;
        self.color_variant = default_color_variant();
        self.texture_variant_enabled = false;
        self.texture_variant = default_texture_variant();
    }

    fn texture_request(&self) -> Result<Option<TextureVariant>, String> {
        if !self.texture_variant_enabled {
            return Ok(None);
        }
        if self.color_variant_enabled || self.trim.enabled || self.sampler.enabled || self.sequence.enabled || self.uv.enabled || self.pose_bake.enabled
            || !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return Err(
                "PNG skins require only Cooked animated PNG mode and room-animated-models support"
                    .into(),
            );
        }
        if self.texture_variant.texture.as_os_str().is_empty()
            || self.texture_variant.asset_id.trim().is_empty()
        {
            return Err("Provide a variant PNG file and asset ID".into());
        }
        Ok(Some(self.texture_variant.clone()))
    }

    fn color_request(&self) -> Result<Option<ColorVariant>, String> {
        if self.color_variant_enabled && (self.texture_variant_enabled || self.trim.enabled || self.sampler.enabled || self.sequence.enabled || self.uv.enabled || self.pose_bake.enabled) {
            return Err("Select only one animated authoring operation".into());
        }
        if !self.color_variant_enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return Err("Animated color variants require Cooked animated model mode and room-animated-models support".into());
        }
        if self.color_variant.asset_id.trim().is_empty()
            || !self
                .color_variant
                .linear_rgb
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        {
            return Err("Provide a variant asset ID and finite linear RGB values in 0..1".into());
        }
        Ok(Some(self.color_variant.clone()))
    }

    fn request(&self) -> Request {
        Request {
            source: PathBuf::from(self.source.trim()),
            license: PathBuf::from(self.license.trim()),
            destination: PathBuf::from(self.destination.trim()),
            name: self.name.trim().to_owned(),
            version: self.version.trim().to_owned(),
            kind: self.kind,
            source_format: self.source_format,
            license_acknowledged: self.license_acknowledged,
        }
    }
}

fn format_label(source_format: SourceFormat) -> &'static str {
    match source_format {
        SourceFormat::RawGlb => "Raw GLB",
        SourceFormat::CookedStatic => "Cooked static model",
        SourceFormat::CookedAnimated => "Cooked animated model",
    }
}

fn request_destination(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "package destination must have an existing parent directory".to_owned())?;
    let parent = crate::package_manager::checked_directory(parent, "package destination parent")?;
    let name = path
        .file_name()
        .ok_or_else(|| "package destination must name a new directory".to_owned())?;
    Ok(parent.join(name))
}

fn path_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(4096),
    )
    .labelled_by(response.id)
    .changed()
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(limit),
    )
    .labelled_by(response.id);
}

#[cfg(test)]
#[path = "model_package_import_panel/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model_package_import_panel/cooked_animated_tests.rs"]
mod cooked_animated_tests;

fn default_color_variant() -> ColorVariant {
    ColorVariant {
        material: 0,
        linear_rgb: [1.0; 3],
        asset_id: "variants/color.recolor.json".into(),
    }
}

#[cfg(test)]
#[path = "model_package_import_panel/animated_color_tests.rs"]
mod animated_color_tests;

fn default_texture_variant() -> TextureVariant {
    TextureVariant {
        material: 0,
        texture: PathBuf::new(),
        asset_id: "variants/skin.retexture.json".into(),
    }
}
#[cfg(test)]
#[path = "model_package_import_panel/animated_texture_tests.rs"]
mod animated_texture_tests;

#[cfg(all(test, not(feature = "room-animated-models")))]
#[path = "model_package_import_panel/variant_preview_tests.rs"]
mod variant_preview_disabled_tests;
