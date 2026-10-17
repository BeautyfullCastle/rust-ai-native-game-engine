use crate::model_package_import::{self, Report, Request};
use egui::Ui;
use orr_model_bindings::model_bindings::ModelKind;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct Completion {
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
                self.status =
                    "Raw GLB import worker disconnected before returning a result.".into();
                return None;
            }
        };
        self.pending = None;
        self.license_acknowledged = false;
        let is_current = project_root == Some(completion.project_root.as_path())
            && project_generation == completion.project_generation
            && source_generation == completion.source_generation;
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
                self.status = format!("Raw GLB package import failed: {error}");
                None
            }
            Err(error) => {
                self.destination_identity = None;
                self.status = format!(
                    "Raw GLB package import for a prior project/source state failed: {error}"
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
        egui::CollapsingHeader::new("Import a raw GLB package")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Import a local self-contained GLB and a license file into a new package folder.");
                ui.label("External resources, scripts, network access and package activation are not supported.");
                ui.label("A supplied license file is metadata only; its presence is not legal review or a rights determination.");
                let fields_enabled = enabled && !self.busy();
                ui.add_enabled_ui(fields_enabled, |ui| {
                    let source_changed = path_field(ui, "Raw GLB file", &mut self.source);
                    let license_changed = path_field(ui, "License file", &mut self.license);
                    if source_changed || license_changed {
                        self.license_acknowledged = false;
                    }
                    path_field(ui, "Package destination directory", &mut self.destination);
                    text_field(ui, "Package name", &mut self.name, 128);
                    text_field(ui, "Package version", &mut self.version, 128);
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.kind, ModelKind::Static, "Static model");
                        #[cfg(feature = "room-animated-models")]
                        ui.radio_value(&mut self.kind, ModelKind::Animated, "Animated model");
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
                let request_ready = fields_enabled
                    && project_root.is_some()
                    && self.license_acknowledged
                    && !self.source.trim().is_empty()
                    && !self.license.trim().is_empty()
                    && !self.destination.trim().is_empty()
                    && !self.name.trim().is_empty()
                    && !self.version.trim().is_empty()
                    && (self.kind == ModelKind::Static
                        || cfg!(feature = "room-animated-models"));
                if ui
                    .add_enabled(request_ready, egui::Button::new("Import raw GLB package"))
                    .clicked()
                {
                    match project_root {
                        Some(project_root) => self.start_import(
                            project_root.to_path_buf(),
                            project_generation,
                            source_generation,
                        ),
                        None => {
                            self.status = "Open a Room project before importing a raw GLB package.".into();
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
        let request = Request {
            source: PathBuf::from(self.source.trim()),
            license: PathBuf::from(self.license.trim()),
            destination: PathBuf::from(self.destination.trim()),
            name: self.name.trim().to_owned(),
            version: self.version.trim().to_owned(),
            kind: self.kind,
            license_acknowledged: self.license_acknowledged,
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-model-package-import".into())
            .spawn(move || {
                let expected_destination = request_destination(&request.destination);
                let result = model_package_import::import_package(request);
                let _ = sender.send(Completion {
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
                self.status = "Importing and validating the self-contained GLB on a worker…".into();
            }
            Err(error) => self.status = format!("Cannot start raw GLB package import: {error}"),
        }
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
