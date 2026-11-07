use crate::model_package_import::{self, ColorVariant, Report, Request, SourceFormat};
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
    color_variant_enabled: bool,
    color_variant: ColorVariant,
    submitted_color: Option<Option<ColorVariant>>,
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
            color_variant_enabled: false,
            color_variant: default_color_variant(),
            submitted_color: None,
        }
    }
}

impl Panel {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn poll(
        &mut self,
        project_root: Option<&Path>,
        project_generation: u64,
        source_generation: u64,
    ) -> Option<Report> {
        let Some(receiver) = &self.pending else {
            return None;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.submitted_color = None;
                self.license_acknowledged = false;
                self.destination_identity = None;
                self.status = "Model import worker disconnected before returning a result. Publication may have completed; inspect the destination before retrying. No automatic retry.".into();
                return None;
            }
        };
        self.pending = None;
        self.license_acknowledged = false;
        let submitted_color = self.submitted_color.take().unwrap_or(None);
        let color_is_current = self
            .color_request()
            .ok()
            .is_some_and(|color| color == submitted_color);
        let is_current = color_is_current
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
        egui::CollapsingHeader::new("Import a model package")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Import a local self-contained GLB, cooked static model or cooked animated model and a license file into a new package folder.");
                ui.label("Cooked import preserves exact bytes unless you explicitly create an animated color variant. Historical dependencies are not loaded or added to the package.");
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
                    if source_changed || license_changed {
                        self.source_inputs_changed();
                    }
                    path_field(ui, "Package destination directory", &mut self.destination);
                    text_field(ui, "Package name", &mut self.name, 128);
                    text_field(ui, "Package version", &mut self.version, 128);
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
                        let old = (self.color_variant_enabled, self.color_variant.clone());
                        ui.checkbox(&mut self.color_variant_enabled, "Create animated color variant");
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
                        if old != (self.color_variant_enabled,self.color_variant.clone()) { self.source_inputs_changed(); }
                    }
                    ui.checkbox(
                        &mut self.license_acknowledged,
                        "I have reviewed rights to distribute this content",
                    );
                });
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
                    && self.color_request().is_ok();
                let import_label = match self.source_format {
                    SourceFormat::RawGlb => "Import raw GLB package",
                    SourceFormat::CookedStatic => "Import cooked static package",
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
        let color = match self.color_request() {
            Ok(color) => color,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let submitted_color = color.clone();
        let request = self.request();
        let source_format = request.source_format;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-model-package-import".into())
            .spawn(move || {
                let expected_destination = request_destination(&request.destination);
                let result = match color {
                    Some(color) => model_package_import::import_color_variant(request, color),
                    None => model_package_import::import_package(request),
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
                self.submitted_color = Some(submitted_color);
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
        self.license_acknowledged = false;
        self.destination_identity = None;
    }

    fn reset_color_variant(&mut self) {
        self.color_variant_enabled = false;
        self.color_variant = default_color_variant();
    }

    fn color_request(&self) -> Result<Option<ColorVariant>, String> {
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
