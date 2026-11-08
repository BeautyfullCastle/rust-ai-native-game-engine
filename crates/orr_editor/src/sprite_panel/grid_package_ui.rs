//! Explicit inspected-PNG grid creation. It never installs, loads or assigns assets.
use super::*;
use crate::sprite_grid_authoring as backend;
use orr_sprite::PlaybackMode;
use std::{
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Context {
    bindings_path: PathBuf,
    project: String,
    project_input: String,
    scene: String,
    load_epoch: u64,
    host_epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    png: String,
    cell_width: String,
    cell_height: String,
    clip: String,
    mode: PlaybackMode,
    duration: String,
    license: String,
    name: String,
    version: String,
    destination: String,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            png: String::new(),
            cell_width: "16".into(),
            cell_height: "16".into(),
            clip: String::new(),
            mode: PlaybackMode::Loop,
            duration: "100".into(),
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
    png: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Inspected {
    pin: InspectPin,
    image: backend::PngInspection,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct CreationPin {
    source: InspectPin,
    draft_ticket: u64,
    draft: Draft,
    image: backend::PngInspection,
    spec: backend::GridSpec,
}
enum Pending {
    Inspect {
        pin: InspectPin,
        receiver: Receiver<Result<backend::PngInspection, String>>,
    },
    Create {
        pin: Box<CreationPin>,
        receiver: Receiver<Result<backend::Report, String>>,
    },
}
enum Completion {
    Inspect(InspectPin, Result<backend::PngInspection, String>),
    Create(Box<CreationPin>, Result<backend::Report, String>),
    Disconnected(Option<PathBuf>),
}
#[derive(Default)]
pub(super) struct State {
    draft: Draft,
    observed: Draft,
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
}
fn decimal(raw: &str, max_digits: usize) -> Result<u64, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > max_digits || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Use a positive decimal integer".into());
    }
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "Positive decimal integer is outside its supported range".into())
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
        if self.observed.png != self.draft.png {
            self.source_ticket = self.source_ticket.saturating_add(1);
            self.inspection = None;
            self.rights = false;
            self.created = None;
        }
        if self.observed != self.draft {
            self.draft_ticket = self.draft_ticket.saturating_add(1);
            self.observed = self.draft.clone();
            self.rights = false;
            self.created = None;
        }
        if self.exhausted() {
            self.inspection = None;
            self.rights = false;
            self.created = None;
            self.status = "Grid authoring tickets exhausted; reopen the editor before creating another package.".into();
        }
    }
    fn inspect_pin(&self) -> Option<InspectPin> {
        if self.exhausted() || self.draft.png.trim().is_empty() {
            return None;
        }
        Some(InspectPin {
            context: self.context.clone()?,
            context_ticket: self.context_ticket,
            source_ticket: self.source_ticket,
            png: self.draft.png.trim().into(),
        })
    }
    fn current_image(&self) -> Result<&backend::PngInspection, String> {
        let inspected = self
            .inspection
            .as_ref()
            .ok_or("Inspect sprite PNG explicitly before creation")?;
        if self.inspect_pin().as_ref() != Some(&inspected.pin) {
            return Err(
                "Inspect the current PNG in the current local bindings context again".into(),
            );
        }
        Ok(&inspected.image)
    }
    fn spec(&self) -> Result<backend::GridSpec, String> {
        let spec = backend::GridSpec {
            cell_width: u32::try_from(decimal(&self.draft.cell_width, 10)?)
                .map_err(|_| "Cell width exceeds u32")?,
            cell_height: u32::try_from(decimal(&self.draft.cell_height, 10)?)
                .map_err(|_| "Cell height exceeds u32")?,
            clip: self.draft.clip.clone(),
            mode: self.draft.mode,
            duration_ms: decimal(&self.draft.duration, 20)?,
        };
        backend::validate_grid_spec(&spec, self.current_image()?)?;
        Ok(spec)
    }
    fn creation_pin(&self) -> Result<CreationPin, String> {
        let spec = self.spec()?;
        if self.draft.license.trim().is_empty()
            || self.draft.name.trim().is_empty()
            || self.draft.version.trim().is_empty()
            || self.draft.destination.trim().is_empty()
        {
            return Err("Provide a license file, package name, version and new destination".into());
        }
        Ok(CreationPin {
            source: self
                .inspect_pin()
                .ok_or("Matching local Edit bindings context required")?,
            draft_ticket: self.draft_ticket,
            draft: self.draft.clone(),
            image: self.current_image()?.clone(),
            spec,
        })
    }
    fn start_inspection(&mut self) {
        if self.busy() {
            return;
        }
        let Some(pin) = self.inspect_pin() else {
            return;
        };
        let png = pin.png.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-grid-inspect".into())
            .spawn(move || {
                let _ = sender.send(backend::inspect_png(&png));
            }) {
            Ok(_) => {
                self.pending = Some(Pending::Inspect { pin, receiver });
                self.inspection = None;
                self.rights = false;
                self.created = None;
                self.status = "Inspecting and validating the complete PNG on a worker...".into();
            }
            Err(error) => self.status = format!("Cannot start PNG inspection: {error}"),
        }
    }
    fn start_creation(&mut self) {
        if self.busy() || !self.rights {
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
            png: pin.source.png.clone(),
            license: self.draft.license.trim().into(),
            destination: self.draft.destination.trim().into(),
            name: self.draft.name.trim().into(),
            version: self.draft.version.trim().into(),
            license_acknowledged: self.rights,
            inspection: pin.image.clone(),
            spec: pin.spec.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-grid-create".into())
            .spawn(move || {
                let _ = sender.send(backend::create_package(request));
            }) {
            Ok(_) => {
                self.pending = Some(Pending::Create {
                    pin: Box::new(pin),
                    receiver,
                });
                self.created = None;
                self.status = "Creating an immutable sprite grid package on a worker...".into();
            }
            Err(error) => self.status = format!("Cannot start sprite grid creation: {error}"),
        }
    }
    fn poll(&mut self, context: Option<Context>) {
        self.sync(context);
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
                    self.status = "Discarded PNG inspection from a prior context or source; inspect explicitly again.".into();
                    return;
                }
                match result {
                    Ok(image) => {
                        self.status = format!(
                            "Inspected PNG: {} x {}; SHA-256 {}. Nothing was created or loaded.",
                            image.width, image.height, image.sha256
                        );
                        self.inspection = Some(Inspected { pin, image });
                    }
                    Err(error) => {
                        self.inspection = None;
                        self.status = format!("PNG inspection failed: {error}");
                    }
                }
            }
            Completion::Create(pin, result) => {
                let current = self
                    .creation_pin()
                    .is_ok_and(|current| &current == pin.as_ref());
                match result {
                    Ok(report) => {
                        self.last_output = Some(report.root.clone());
                        let destination = PathBuf::from(pin.draft.destination.trim());
                        let cells = backend::validate_grid_spec(&pin.spec, &pin.image)
                            .map(|doc| doc.regions().len() as u32);
                        let expected = report.root == destination
                            && report.package == pin.draft.name.trim()
                            && report.document == "sprites.json"
                            && report.width == pin.image.width
                            && report.height == pin.image.height
                            && cells == Ok(report.cells)
                            && report.source_sha256.eq_ignore_ascii_case(&pin.image.sha256);
                        if current && expected {
                            self.status = format!("Created sprite package {} ({} cells). It is not installed, loaded or assigned. Install it explicitly with the existing package API/CLI, then Load sprite document.", report.root.display(), report.cells);
                            self.created = Some(report);
                        } else {
                            self.created = None;
                            self.status = format!("Worker completed package {} for a prior context/draft or unexpected result; it was not adopted. Retain and inspect the output before continuing.", report.root.display());
                        }
                    }
                    Err(error) => {
                        let destination = PathBuf::from(pin.draft.destination.trim());
                        self.last_output = Some(destination.clone());
                        self.created = None;
                        self.status = format!("Sprite grid creation failed for {}: {error}. Retain any partial output for inspection; no automatic retry.", destination.display());
                    }
                }
            }
            Completion::Disconnected(destination) => {
                self.created = None;
                if let Some(destination) = destination {
                    self.last_output = Some(destination.clone());
                    self.status = format!("Sprite grid creation worker disconnected. Publication at {} may have completed or left partial output; inspect it before retrying. No automatic retry.", destination.display());
                } else {
                    self.inspection = None;
                    self.status =
                        "PNG inspection worker disconnected; inspect explicitly again.".into();
                }
            }
        }
    }
    fn show(&mut self, ui: &mut Ui, context: Option<Context>) {
        self.sync(context.clone());
        ui.collapsing("Create sprite package from PNG grid", |ui| {
            ui.weak("Inspect a PNG explicitly, then create one regular row-major grid and one named clip, up to 256 cells. No image preview, packing, installation, loading or assignment is performed.");
            let enabled = context.is_some() && !self.busy() && !self.exhausted();
            ui.add_enabled_ui(enabled, |ui| {
                field(ui, "Sprite grid PNG file", &mut self.draft.png, 4096);
                field(ui, "Grid cell width", &mut self.draft.cell_width, 10);
                field(ui, "Grid cell height", &mut self.draft.cell_height, 10);
                field(ui, "Grid clip name", &mut self.draft.clip, 128);
                egui::ComboBox::from_label("Grid playback mode").selected_text(match self.draft.mode { PlaybackMode::Loop => "Loop grid clip", PlaybackMode::Once => "Play grid clip once" }).show_ui(ui, |ui| {
                    for (mode, label) in [(PlaybackMode::Loop, "Loop grid clip"), (PlaybackMode::Once, "Play grid clip once")] {
                        if ui.selectable_value(&mut self.draft.mode, mode, label).clicked() { ui.close(); }
                    }
                });
                field(ui, "Grid frame duration (ms)", &mut self.draft.duration, 20);
                field(ui, "Grid license file", &mut self.draft.license, 4096);
                field(ui, "Grid package name", &mut self.draft.name, 128);
                field(ui, "Grid package version", &mut self.draft.version, 128);
                field(ui, "Grid package destination", &mut self.draft.destination, 4096);
                self.sync(context.clone());
                if ui.add_enabled(self.inspect_pin().is_some(), egui::Button::new("Inspect sprite PNG")).clicked() { self.start_inspection(); }
                if let Ok(image) = self.current_image() { ui.label(format!("Inspected grid PNG {} x {}; SHA-256 {}", image.width, image.height, image.sha256)); }
                match self.spec().and_then(|spec| backend::validate_grid_spec(&spec, self.current_image()?)) {
                    Ok(document) => { ui.label(format!("{} row-major cells; one clip. PNG hash is checked again before package publication.", document.regions().len())); }
                    Err(error) => { ui.weak(error); }
                }
                ui.checkbox(&mut self.rights, "Grid rights acknowledgement");
                if ui.add_enabled(!self.busy() && self.rights && self.creation_pin().is_ok(), egui::Button::new("Create sprite grid package")).clicked() { self.start_creation(); }
            });
            if self.busy() { ui.spinner(); }
            if !self.status.is_empty() { ui.label(&self.status); }
            if let Some(path) = &self.last_output { ui.label(format!("Last package destination: {}", path.display())); }
        });
    }
}
fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    let label = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
}
impl SpritePanel {
    fn grid_package_context(&self, editor: &Editor) -> Option<Context> {
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
            project: project.to_str()?.into(),
            project_input: bindings.document().project.clone(),
            scene: editor.sim().scene_path.clone()?,
            load_epoch: editor.scene_load_epoch(),
            host_epoch: editor.sim().epoch,
        })
    }
    pub(super) fn poll_grid_package(&mut self, editor: &Editor) {
        let context = self.grid_package_context(editor);
        self.grid_package.poll(context);
    }
    pub(super) fn show_grid_package(&mut self, ui: &mut Ui, editor: &Editor) {
        let context = self.grid_package_context(editor);
        self.grid_package.show(ui, context);
    }
}
#[cfg(all(test, target_os = "linux"))]
#[path = "grid_package_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "grid_package_ui_tests.rs"]
mod tests;
