//! Explicit inspected-PNG rectangle creation. It never installs, loads or assigns assets.
use super::*;
use crate::sprite_rectangle_authoring as backend;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

const MAX_REGIONS: usize = 256;
const ROWS_HEIGHT: f32 = 280.0;

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
struct RectangleDraft {
    /// UI identity is monotonic and separate from the semantic region ID.
    row_key: u64,
    id: String,
    x: String,
    y: String,
    width: String,
    height: String,
}

impl RectangleDraft {
    fn new(row_key: u64) -> Self {
        Self {
            row_key,
            id: "0".into(),
            x: "0".into(),
            y: "0".into(),
            width: "1".into(),
            height: "1".into(),
        }
    }

    fn parse(&self) -> Result<orr_sprite::Region, String> {
        Ok(orr_sprite::Region {
            id: decimal_u32(&self.id, true).map_err(|error| format!("ID: {error}"))?,
            x: decimal_u32(&self.x, true).map_err(|error| format!("X: {error}"))?,
            y: decimal_u32(&self.y, true).map_err(|error| format!("Y: {error}"))?,
            width: decimal_u32(&self.width, false).map_err(|error| format!("Width: {error}"))?,
            height: decimal_u32(&self.height, false).map_err(|error| format!("Height: {error}"))?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    png: String,
    rectangles: Vec<RectangleDraft>,
    license: String,
    name: String,
    version: String,
    destination: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            png: String::new(),
            rectangles: vec![RectangleDraft::new(1)],
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
    spec: backend::RectangleSpec,
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
    observed_draft: Draft,
    next_row_key: u64,
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

fn decimal_u32(raw: &str, allow_zero: bool) -> Result<u32, String> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("use ASCII decimal digits only".into());
    }
    let value = raw
        .parse::<u32>()
        .map_err(|_| "decimal value exceeds u32".to_string())?;
    if !allow_zero && value == 0 {
        return Err("value must be at least 1".into());
    }
    Ok(value)
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
        if self.observed_draft.png != self.draft.png {
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
            self.status = "Rectangle authoring tickets exhausted; reopen the editor before creating another package.".into();
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
            png: PathBuf::from(&self.draft.png),
        })
    }

    fn current_image(&self) -> Result<&backend::PngInspection, String> {
        let inspected = self
            .inspection
            .as_ref()
            .ok_or("Inspect sprite PNG explicitly before creating the package")?;
        if self.inspect_pin().as_ref() != Some(&inspected.pin) {
            return Err(
                "Inspect the current PNG in the current local bindings context again".into(),
            );
        }
        Ok(&inspected.image)
    }

    fn rectangle_spec(&self) -> Result<backend::RectangleSpec, String> {
        if !(1..=MAX_REGIONS).contains(&self.draft.rectangles.len()) {
            return Err("rectangle atlas must contain 1..=256 rows".into());
        }
        let regions = self
            .draft
            .rectangles
            .iter()
            .enumerate()
            .map(|(index, row)| {
                row.parse().map_err(|error| {
                    format!(
                        "Rectangle row {} (key {}, ID draft {:?}): {error}",
                        index + 1,
                        row.row_key,
                        row.id
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(backend::RectangleSpec { regions })
    }

    fn validate_draft(&self) -> Result<orr_sprite::SpriteDocument, String> {
        backend::validate_rectangle_spec(&self.rectangle_spec()?, self.current_image()?)
            .map_err(|error| self.add_backend_row_context(error))
    }

    fn add_backend_row_context(&self, error: String) -> String {
        let duplicate = error.contains("duplicate region ");
        let outside = error.contains(" outside atlas or empty");
        let marker = if duplicate {
            "duplicate region "
        } else if outside {
            "region "
        } else {
            return error;
        };
        let Some(id) = error.split_once(marker).and_then(|(_, suffix)| {
            let digits = suffix
                .bytes()
                .take_while(u8::is_ascii_digit)
                .map(char::from)
                .collect::<String>();
            digits.parse::<u32>().ok()
        }) else {
            return error;
        };
        let rows = self
            .draft
            .rectangles
            .iter()
            .enumerate()
            .filter(|&(_, row)| decimal_u32(&row.id, true).ok() == Some(id))
            .map(|(index, row)| format!("row {} (key {}, ID {})", index + 1, row.row_key, id))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            error
        } else {
            format!(
                "{error}; matching {}: {}",
                if duplicate { "rows" } else { "row" },
                rows.join(", ")
            )
        }
    }

    fn creation_pin(&self) -> Result<CreationPin, String> {
        if self.draft.license.trim().is_empty()
            || self.draft.name.trim().is_empty()
            || self.draft.version.trim().is_empty()
            || self.draft.destination.trim().is_empty()
        {
            return Err("Provide a license file, package name, version and new destination".into());
        }
        let source = self
            .inspect_pin()
            .ok_or("Matching local Edit sprite bindings context and PNG path required")?;
        let image = self.current_image()?.clone();
        let spec = self.rectangle_spec()?;
        self.validate_draft()?;
        Ok(CreationPin {
            source,
            draft_ticket: self.draft_ticket,
            draft: self.draft.clone(),
            image,
            spec,
        })
    }

    fn start_inspection(&mut self) {
        if self.busy() || self.blocked_by_other || self.exhausted() {
            return;
        }
        let Some(pin) = self.inspect_pin() else {
            self.status = "A local Edit bindings context and PNG path are required.".into();
            return;
        };
        let png = pin.png.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-rectangle-inspect".into())
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
            png: pin.source.png.clone(),
            license: PathBuf::from(&pin.draft.license),
            destination: PathBuf::from(&pin.draft.destination),
            name: pin.draft.name.clone(),
            version: pin.draft.version.clone(),
            license_acknowledged: self.rights,
            inspection: pin.image.clone(),
            spec: pin.spec.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-rectangle-create".into())
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
                self.status =
                    "Creating the explicit sprite rectangle package on a worker...".into();
            }
            Err(error) => self.status = format!("Cannot start rectangle package creation: {error}"),
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
            && Path::new(&self.draft.png) == pin.source.png.as_path()
            && self
                .inspection
                .as_ref()
                .is_some_and(|image| image.pin == pin.source && image.image == pin.image)
            && self.rectangle_spec().is_ok_and(|spec| spec == pin.spec)
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
                    pin.draft.destination.clone().into(),
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
                    self.status = "Discarded PNG inspection from a prior editor context or source; inspect explicitly again.".into();
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
            Completion::Create(pin, result) => match result {
                Ok(report) => {
                    self.last_output = Some(report.root.clone());
                    let expected = report.root.as_path() == Path::new(&pin.draft.destination)
                        && report.package == pin.draft.name
                        && report.document == "sprites.json"
                        && report.width == pin.image.width
                        && report.height == pin.image.height
                        && report.region_count == pin.spec.regions.len() as u32
                        && report.source_sha256.eq_ignore_ascii_case(&pin.image.sha256);
                    if self.creation_pin_matches(&pin, context.as_ref()) && expected {
                        self.status = format!(
                            "Created sprite package {} ({} rectangles). It is not installed, loaded or assigned. Use the existing local package preview/install flow explicitly.",
                            report.root.display(), report.region_count
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
                    let destination = PathBuf::from(&pin.draft.destination);
                    self.last_output = Some(destination.clone());
                    self.created = None;
                    self.status = format!(
                        "Sprite rectangle creation failed for {}: {error}. Retain any partial output for inspection; no automatic retry.",
                        destination.display()
                    );
                }
            },
            Completion::Disconnected(destination) => {
                self.created = None;
                if let Some(destination) = destination {
                    self.last_output = Some(destination.clone());
                    self.status = format!(
                        "Rectangle package worker disconnected. Publication at {} may have completed or left partial output; inspect it before retrying. No automatic retry.",
                        destination.display()
                    );
                } else {
                    self.inspection = None;
                    self.status =
                        "PNG inspection worker disconnected; inspect explicitly again.".into();
                }
            }
        }
    }

    fn show(&mut self, ui: &mut Ui, context: Option<Context>, blocked_by_other: bool) {
        self.sync(context.clone());
        self.blocked_by_other = blocked_by_other;
        crate::theme::header("Create sprite package from rectangles")
            .default_open(false)
            .show(ui, |ui| {
                ui.weak("Inspect a PNG explicitly, then define 1..=256 independent atlas rectangles. Creation writes a new source package only; it does not install, load or assign it.");
                let enabled = context.is_some()
                    && !self.busy()
                    && !self.exhausted()
                    && !blocked_by_other;
                ui.add_enabled_ui(enabled, |ui| {
                    field(ui, "Rectangle PNG file", &mut self.draft.png, 4096);
                    if ui
                        .add_enabled(
                            self.inspect_pin().is_some(),
                            egui::Button::new("Inspect rectangle PNG"),
                        )
                        .clicked()
                    {
                        self.start_inspection();
                    }
                    if let Ok(image) = self.current_image() {
                        ui.label(format!(
                            "Inspected PNG {} x {}; SHA-256 {}",
                            image.width, image.height, image.sha256
                        ));
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.strong("Rectangle rows");
                        ui.label(format!("{} / {MAX_REGIONS}", self.draft.rectangles.len()));
                    });
                    let mut remove = Vec::new();
                    let can_remove = self.draft.rectangles.len() > 1;
                    egui::ScrollArea::vertical()
                        .max_height(ROWS_HEIGHT)
                        .show(ui, |ui| {
                            for rectangle in &mut self.draft.rectangles {
                                ui.push_id(rectangle.row_key, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(format!("Rectangle row {}", rectangle.row_key));
                                        field(ui, &format!("Rectangle ID row {}", rectangle.row_key), &mut rectangle.id, 32);
                                        field(ui, &format!("Rectangle X row {}", rectangle.row_key), &mut rectangle.x, 32);
                                        field(ui, &format!("Rectangle Y row {}", rectangle.row_key), &mut rectangle.y, 32);
                                        field(ui, &format!("Rectangle Width row {}", rectangle.row_key), &mut rectangle.width, 32);
                                        field(ui, &format!("Rectangle Height row {}", rectangle.row_key), &mut rectangle.height, 32);
                                        if ui.add_enabled(can_remove, egui::Button::new(format!("Remove rectangle row {}", rectangle.row_key))).clicked() {
                                            remove.push(rectangle.row_key);
                                        }
                                    });
                                });
                            }
                        });
                    if !remove.is_empty() {
                        self.draft.rectangles.retain(|row| !remove.contains(&row.row_key));
                    }
                    if ui
                        .add_enabled(
                            self.draft.rectangles.len() < MAX_REGIONS
                                && self.next_row_key != u64::MAX,
                            egui::Button::new("Add rectangle"),
                        )
                        .clicked()
                    {
                        if self.next_row_key == 0 {
                            self.next_row_key = self
                                .draft
                                .rectangles
                                .iter()
                                .map(|row| row.row_key)
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1);
                        }
                        let row_key = self.next_row_key;
                        self.next_row_key = row_key.saturating_add(1);
                        self.draft.rectangles.push(RectangleDraft::new(row_key));
                    }

                    field(ui, "Rectangle license file", &mut self.draft.license, 4096);
                    field(ui, "Rectangle package name", &mut self.draft.name, 128);
                    field(ui, "Rectangle package version", &mut self.draft.version, 128);
                    field(ui, "Rectangle package destination", &mut self.draft.destination, 4096);
                    self.sync(context.clone());
                    match self.validate_draft() {
                        Ok(document) => ui.label(format!("{} valid rectangles. PNG hash is checked again before package publication.", document.regions().len())),
                        Err(error) => ui.colored_label(Color32::LIGHT_RED, error),
                    };
                    ui.checkbox(&mut self.rights, "Rectangle rights acknowledgement");
                    self.sync(context.clone());
                    if ui
                        .add_enabled(
                            !self.busy()
                                && self.rights
                                && self.creation_pin().is_ok(),
                            egui::Button::new("Create rectangle sprite package"),
                        )
                        .clicked()
                    {
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
                        "Created {} {}: {} x {}, {} rectangles, source SHA-256 {}",
                        report.package,
                        report.document,
                        report.width,
                        report.height,
                        report.region_count,
                        report.source_sha256
                    ));
                }
            });
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    let id = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(if limit <= 32 { 90.0 } else { f32::INFINITY }),
    )
    .labelled_by(id.id);
}

impl SpritePanel {
    fn rectangle_package_context(&self, editor: &Editor) -> Option<Context> {
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
        // Resolve only the project admitted by the local sidecar. Remote host
        // project paths never authorize local file access.
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

    pub(super) fn poll_rectangle_package(&mut self, editor: &Editor) {
        self.rectangle_package
            .poll(self.rectangle_package_context(editor));
    }

    pub(super) fn show_rectangle_package(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        blocked_by_other: bool,
    ) {
        let context = self.rectangle_package_context(editor);
        self.rectangle_package.show(ui, context, blocked_by_other);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "rectangle_package_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "rectangle_package_ui_tests.rs"]
mod tests;
