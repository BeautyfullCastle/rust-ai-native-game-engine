//! Explicit JSON-backed sprite package authoring. Inspection and creation run
//! on workers; successful output is left for the existing preview/install flow.
use super::*;
use crate::sprite_json_authoring as backend;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Context {
    bindings_path: PathBuf,
    bindings_document: Vec<u8>,
    project: PathBuf,
    project_input: String,
    scene: String,
    scene_load_epoch: u64,
    host_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    json: String,
    png: String,
    license: String,
    name: String,
    version: String,
    destination: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            json: String::new(),
            png: String::new(),
            license: String::new(),
            name: String::new(),
            version: "1.0.0".into(),
            destination: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InspectPin {
    context: Context,
    context_ticket: u64,
    source_ticket: u64,
    json: PathBuf,
    png: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inspected {
    pin: InspectPin,
    inspection: backend::Inspection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CreationPin {
    source: InspectPin,
    draft_ticket: u64,
    draft: Draft,
    inspection: backend::Inspection,
}

enum Pending {
    Inspect {
        pin: InspectPin,
        receiver: Receiver<Result<backend::Inspection, String>>,
    },
    Create {
        pin: Box<CreationPin>,
        receiver: Receiver<Result<backend::Report, String>>,
    },
}

enum Completion {
    Inspect(InspectPin, Result<backend::Inspection, String>),
    Create(Box<CreationPin>, Result<backend::Report, String>),
    Disconnected(Option<PathBuf>),
}

#[derive(Default)]
pub(super) struct State {
    draft: Draft,
    observed_draft: Draft,
    context: Option<Context>,
    context_ticket: u64,
    source_ticket: u64,
    draft_ticket: u64,
    inspection: Option<Inspected>,
    rights: bool,
    pending: Option<Pending>,
    created: Option<backend::Report>,
    last_output: Option<PathBuf>,
    status: String,
    blocked_by_other: bool,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    fn exhausted(&self) -> bool {
        [self.context_ticket, self.source_ticket, self.draft_ticket].contains(&u64::MAX)
    }

    fn sync(&mut self, context: Option<Context>) {
        if self.context != context {
            self.context_ticket = self.context_ticket.saturating_add(1);
            self.context = context;
            self.inspection = None;
            self.rights = false;
            self.created = None;
        }
        let sources_changed = self.observed_draft.json != self.draft.json
            || self.observed_draft.png != self.draft.png;
        if sources_changed {
            self.source_ticket = self.source_ticket.saturating_add(1);
            self.inspection = None;
            self.rights = false;
            self.created = None;
        }
        if self.observed_draft != self.draft {
            self.draft_ticket = self.draft_ticket.saturating_add(1);
            self.observed_draft.clone_from(&self.draft);
            self.rights = false;
            self.created = None;
        }
        if self.exhausted() {
            self.inspection = None;
            self.rights = false;
            self.created = None;
            self.status = "Sprite JSON authoring tickets exhausted; reopen the editor before creating another package.".into();
        }
    }

    fn inspect_pin(&self) -> Option<InspectPin> {
        if self.exhausted() || self.draft.json.trim().is_empty() || self.draft.png.trim().is_empty()
        {
            return None;
        }
        Some(InspectPin {
            context: self.context.clone()?,
            context_ticket: self.context_ticket,
            source_ticket: self.source_ticket,
            json: PathBuf::from(self.draft.json.trim()),
            png: PathBuf::from(self.draft.png.trim()),
        })
    }

    fn current_inspection(&self) -> Result<&backend::Inspection, String> {
        let inspected = self
            .inspection
            .as_ref()
            .ok_or("Inspect the current sprite JSON and PNG before creating a package")?;
        if self.inspect_pin().as_ref() != Some(&inspected.pin)
            || inspected.inspection.json_sha256().is_empty()
            || inspected.inspection.png_sha256().is_empty()
        {
            return Err(
                "Inspect the current JSON and PNG in the current local bindings context again"
                    .into(),
            );
        }
        Ok(&inspected.inspection)
    }

    fn creation_pin(&self) -> Result<CreationPin, String> {
        if self.draft.license.trim().is_empty()
            || self.draft.name.trim().is_empty()
            || self.draft.version.trim().is_empty()
            || self.draft.destination.trim().is_empty()
        {
            return Err("Provide a license file, package name, version and new destination".into());
        }
        let inspection = self.current_inspection()?.clone();
        let source = self
            .inspect_pin()
            .ok_or("Matching local Edit bindings context and JSON/PNG files required")?;
        Ok(CreationPin {
            source,
            draft_ticket: self.draft_ticket,
            draft: self.draft.clone(),
            inspection,
        })
    }

    fn start_inspection(&mut self) {
        if self.busy() || self.blocked_by_other || self.exhausted() {
            return;
        }
        let Some(pin) = self.inspect_pin() else {
            self.status =
                "A local Edit bindings context, sprite JSON file and PNG file are required.".into();
            return;
        };
        let json = pin.json.clone();
        let png = pin.png.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-json-inspect".into())
            .spawn(move || {
                let _ = sender.send(backend::inspect_sources(&json, &png));
            }) {
            Ok(_) => {
                self.pending = Some(Pending::Inspect { pin, receiver });
                self.inspection = None;
                self.rights = false;
                self.created = None;
                self.status = "Inspecting the JSON document and complete PNG on a worker...".into();
            }
            Err(error) => self.status = format!("Cannot start sprite JSON inspection: {error}"),
        }
    }

    fn start_creation(&mut self) {
        if self.busy() || self.blocked_by_other || self.exhausted() || !self.rights {
            return;
        }
        let pin = match self.creation_pin() {
            Ok(pin) => pin,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let request = backend::Request {
            inspection: pin.inspection.clone(),
            license_path: PathBuf::from(pin.draft.license.trim()),
            destination: PathBuf::from(pin.draft.destination.trim()),
            name: pin.draft.name.trim().into(),
            version: pin.draft.version.trim().into(),
            license_acknowledged: self.rights,
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-json-create".into())
            .spawn(move || {
                let _ = sender.send(backend::create_package(request));
            }) {
            Ok(_) => {
                self.pending = Some(Pending::Create {
                    pin: Box::new(pin),
                    receiver,
                });
                self.rights = false;
                self.created = None;
                self.status = "Creating the sprite JSON package on a worker...".into();
            }
            Err(error) => {
                self.status = format!("Cannot start sprite JSON package creation: {error}")
            }
        }
    }

    fn creation_pin_matches(&self, pin: &CreationPin, live_context: Option<&Context>) -> bool {
        !self.exhausted()
            && live_context == Some(&pin.source.context)
            && self.context.as_ref() == Some(&pin.source.context)
            && self.context_ticket == pin.source.context_ticket
            && self.source_ticket == pin.source.source_ticket
            && self.draft_ticket == pin.draft_ticket
            && self.draft == pin.draft
            && self.observed_draft == pin.draft
            && self.inspect_pin().as_ref() == Some(&pin.source)
            && self.inspection.as_ref().is_some_and(|inspection| {
                inspection.pin == pin.source && inspection.inspection == pin.inspection
            })
    }

    fn poll(&mut self, context: Option<Context>) {
        self.sync(context.clone());
        let completion = match &self.pending {
            Some(Pending::Inspect { pin, receiver }) => match receiver.try_recv() {
                Ok(result) => Some(Completion::Inspect(pin.clone(), result)),
                Err(TryRecvError::Disconnected) => Some(Completion::Disconnected(None)),
                Err(TryRecvError::Empty) => None,
            },
            Some(Pending::Create { pin, receiver }) => match receiver.try_recv() {
                Ok(result) => Some(Completion::Create(pin.clone(), result)),
                Err(TryRecvError::Disconnected) => Some(Completion::Disconnected(Some(
                    pin.draft.destination.trim().into(),
                ))),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        let Some(completion) = completion else {
            return;
        };
        self.pending = None;
        self.rights = false;
        match completion {
            Completion::Inspect(pin, result) => {
                if self.inspect_pin().as_ref() != Some(&pin) {
                    self.status = "Discarded sprite JSON inspection from a prior editor context or source; inspect explicitly again.".into();
                    return;
                }
                match result {
                    Ok(inspection) => {
                        self.status = format!(
                            "Inspected {} and {}: {} x {}, {} regions, {} clips; JSON SHA-256 {}, PNG SHA-256 {}. Nothing was created or loaded.",
                            pin.json.display(),
                            pin.png.display(),
                            inspection.width(),
                            inspection.height(),
                            inspection.region_count(),
                            inspection.clip_count(),
                            inspection.json_sha256(),
                            inspection.png_sha256()
                        );
                        self.inspection = Some(Inspected { pin, inspection });
                    }
                    Err(error) => {
                        self.inspection = None;
                        self.status = format!("Sprite JSON inspection failed: {error}");
                    }
                }
            }
            Completion::Create(pin, result) => match result {
                Ok(report) => {
                    self.last_output = Some(report.root.clone());
                    let expected = report.root.as_path() == Path::new(pin.draft.destination.trim())
                        && report.package == pin.draft.name.trim()
                        && report.document == "sprites.json"
                        && report.image_filename == pin.inspection.image_filename()
                        && report.width == pin.inspection.width()
                        && report.height == pin.inspection.height()
                        && report.region_count == pin.inspection.region_count()
                        && report.clip_count == pin.inspection.clip_count()
                        && report
                            .json_sha256
                            .eq_ignore_ascii_case(pin.inspection.json_sha256())
                        && report
                            .png_sha256
                            .eq_ignore_ascii_case(pin.inspection.png_sha256());
                    if self.creation_pin_matches(&pin, context.as_ref()) && expected {
                        self.status = format!(
                            "Created sprite package {} with {} regions and {} clips. It is not installed, loaded or assigned. Use the existing local package preview/install flow, then Load sprite document.",
                            report.root.display(),
                            report.region_count,
                            report.clip_count
                        );
                        self.created = Some(report);
                    } else {
                        self.created = None;
                        self.status = format!(
                            "Worker completed package {} for a prior context/draft or unexpected result; the output is retained for inspection and was not adopted.",
                            report.root.display()
                        );
                    }
                }
                Err(error) => {
                    let destination = PathBuf::from(pin.draft.destination.trim());
                    self.last_output = Some(destination.clone());
                    self.created = None;
                    self.status = format!(
                        "Sprite JSON package creation failed for {}: {error}. Retain any partial output for inspection; no automatic retry.",
                        destination.display()
                    );
                }
            },
            Completion::Disconnected(destination) => {
                self.created = None;
                if let Some(destination) = destination {
                    self.last_output = Some(destination.clone());
                    self.status = format!(
                        "Sprite JSON package worker disconnected. Publication at {} may have completed or left partial output; inspect it before retrying. No automatic retry.",
                        destination.display()
                    );
                } else {
                    self.inspection = None;
                    self.status =
                        "Sprite JSON inspection worker disconnected; inspect explicitly again."
                            .into();
                }
            }
        }
    }

    fn show(&mut self, ui: &mut Ui, context: Option<Context>, blocked_by_other: bool) {
        self.sync(context.clone());
        self.blocked_by_other = blocked_by_other;
        crate::theme::header("Create sprite package from JSON")
            .default_open(false)
            .show(ui, |ui| {
                ui.weak("Inspect a sprite JSON document and its referenced PNG, then create a source package. Creation does not install, load or assign assets.");
                let enabled = context.is_some()
                    && !self.busy()
                    && !self.exhausted()
                    && !blocked_by_other;
                ui.add_enabled_ui(enabled, |ui| {
                    field(ui, "Sprite JSON file", &mut self.draft.json, 4096);
                    field(ui, "Sprite JSON PNG file", &mut self.draft.png, 4096);
                    self.sync(context.clone());
                    if ui.add_enabled(
                        self.inspect_pin().is_some(),
                        egui::Button::new("Inspect sprite JSON and PNG"),
                    ).clicked() {
                        self.start_inspection();
                    }
                    if let Ok(inspection) = self.current_inspection() {
                        ui.label(format!(
                            "Inspected {} x {}, {} regions, {} clips; image {}. JSON SHA-256 {}; PNG SHA-256 {}",
                            inspection.width(), inspection.height(), inspection.region_count(),
                            inspection.clip_count(), inspection.image_filename(),
                            inspection.json_sha256(), inspection.png_sha256()
                        ));
                    }
                    field(ui, "Sprite JSON license file", &mut self.draft.license, 4096);
                    field(ui, "Sprite JSON package name", &mut self.draft.name, 128);
                    field(ui, "Sprite JSON package version", &mut self.draft.version, 128);
                    field(ui, "Sprite JSON package destination", &mut self.draft.destination, 4096);
                    self.sync(context.clone());
                    if let Err(error) = self.current_inspection() {
                        ui.weak(error);
                    }
                    ui.checkbox(&mut self.rights, "Sprite JSON rights acknowledgement");
                    self.sync(context.clone());
                    if ui.add_enabled(
                        !self.busy() && self.rights && self.creation_pin().is_ok(),
                        egui::Button::new("Create sprite JSON package"),
                    ).clicked() {
                        self.start_creation();
                    }
                });
                if self.busy() {
                    ui.spinner();
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
                }
                if blocked_by_other {
                    ui.weak("Another sprite package authoring or install preview operation is active; wait for it to finish.");
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                if let Some(path) = &self.last_output {
                    ui.label(format!("Last package destination: {}", path.display()));
                }
                if let Some(report) = &self.created {
                    ui.label(format!(
                        "Created {} {}: {} x {}, {} regions, {} clips, image {}, JSON SHA-256 {}, PNG SHA-256 {}",
                        report.package, report.document, report.width, report.height,
                        report.region_count, report.clip_count, report.image_filename,
                        report.json_sha256, report.png_sha256
                    ));
                }
            });
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    let label = ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    );
    response.labelled_by(label.id);
}

impl SpritePanel {
    fn json_package_context(&self, editor: &Editor) -> Option<Context> {
        if self.require_unpinned().is_err()
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.sim().in_tx
            || !self.editable(editor)
        {
            return None;
        }
        let bindings = self.bindings.as_ref()?;
        let project =
            sprite_bindings::resolve_project(bindings.base(), &bindings.document().project).ok()?;
        Some(Context {
            bindings_path: bindings.path.clone(),
            bindings_document: serde_json::to_vec(bindings.document()).ok()?,
            project,
            project_input: bindings.document().project.clone(),
            scene: editor.sim().scene_path.clone()?,
            scene_load_epoch: editor.scene_load_epoch(),
            host_epoch: editor.sim().epoch,
        })
    }

    pub(super) fn poll_json_package(&mut self, editor: &Editor) {
        let context = self.json_package_context(editor);
        self.json_package.poll(context);
    }

    pub(super) fn show_json_package(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        blocked_by_other: bool,
    ) {
        let context = self.json_package_context(editor);
        self.json_package.show(ui, context, blocked_by_other);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "json_package_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "json_package_ui_tests.rs"]
mod tests;
