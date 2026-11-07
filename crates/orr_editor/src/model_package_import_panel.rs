use crate::model_package_import::{self, Report, Request, SourceFormat};
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
                self.license_acknowledged = false;
                self.destination_identity = None;
                self.status = "Model import worker disconnected before returning a result. Publication may have completed; inspect the destination before retrying. No automatic retry.".into();
                return None;
            }
        };
        self.pending = None;
        self.license_acknowledged = false;
        let is_current = project_root == Some(completion.project_root.as_path())
            && project_generation == completion.project_generation
            && source_generation == completion.source_generation
            && self.source_format == completion.source_format;
        match completion.result {
            Ok(report)
                if is_current && completion.expected_destination.as_ref() == Ok(&report.root) =>
            {
                self.destination_identity = Some(report.root.clone());
                self.status = format!(
                    "Imported {}/{} ({:?}). Source SHA-256: {}; license SHA-256: {}. The package is not installed or assigned; preview and add it explicitly.",
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
                ui.label("Cooked models retain their exact bytes and historical dependency digests; those dependencies are not loaded or added to the package.");
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
                    && compatible_kind;
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
        let request = self.request();
        let source_format = request.source_format;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-model-package-import".into())
            .spawn(move || {
                let expected_destination = request_destination(&request.destination);
                let result = model_package_import::import_package(request);
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
            self.source_inputs_changed();
        }
    }

    fn source_inputs_changed(&mut self) {
        self.license_acknowledged = false;
        self.destination_identity = None;
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
