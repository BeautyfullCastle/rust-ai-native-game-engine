use crate::{Editor, Mode};
use orr_sample::windows_export::{self, ExportOptions, ExportReport};
use std::{path::Path, path::PathBuf, sync::mpsc};

enum Completed {
    Measured { path: String, sha256: String },
    Exported(Box<ExportReport>),
}

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub project: String,
    pub runtime: String,
    pub runtime_sha256: String,
    pub output: String,
    pub trusted_runtime: bool,
    pub status: String,
    pub last_output: Option<PathBuf>,
    pub last_digest: Option<String>,
    pending: Option<mpsc::Receiver<Result<Completed, String>>>,
}

impl Panel {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn use_current_scene(&mut self, scene: Option<&Path>) {
        if self.busy() {
            return;
        }
        if let Some(root) = scene.and_then(Path::parent).and_then(|parent| {
            parent
                .ancestors()
                .take(16)
                .find(|root| root.join("orr.project.json").is_file())
        }) {
            self.project = root.display().to_string();
        }
    }

    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Export worker disconnected; inspect the destination before retrying".into())
            }
        };
        self.pending = None;
        match result {
            Ok(Completed::Measured { path, sha256 }) if self.runtime == path => {
                self.runtime_sha256 = sha256;
                self.status = "Runtime measured. Its hash does not establish trust; confirm trusted code separately.".into();
            }
            Ok(Completed::Measured { .. }) => {
                self.status = "Runtime path changed; measure the selected executable again.".into();
            }
            Ok(Completed::Exported(report)) => {
                self.status = format!(
                    "Export complete: {} project files; initial checksum {}",
                    report.project_files, report.manifest.payload.initial_checksum
                );
                self.last_output = Some(report.output);
                self.last_digest = Some(report.content_digest);
            }
            Err(error) => self.status = format!("Export failed: {error}"),
        }
    }

    pub fn measure(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let path = self.runtime.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-export-measure".into())
            .spawn(move || {
                let result = windows_export::measure_runtime(Path::new(&path)).map(|measured| {
                    Completed::Measured {
                        path,
                        sha256: measured.sha256,
                    }
                });
                let _ = sender.send(result);
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Measuring the selected runtime…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start runtime measurement: {error}");
                false
            }
        }
    }

    pub fn export(&mut self, editor: &mut Editor, presentation_dirty: bool) -> bool {
        if self.busy() {
            return false;
        }
        if presentation_dirty {
            self.status =
                "Apply or discard camera drafts and save camera/model sidecars before exporting; scene Save does not save those sidecars."
                    .into();
            return false;
        }
        if !self.trusted_runtime {
            self.status =
                "Explicitly confirm that this runtime is known trusted code before exporting."
                    .into();
            return false;
        }
        let (scene, checksum) = match editor.windows_export_scene() {
            Ok(stamp) => stamp,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let options = ExportOptions {
            project: PathBuf::from(self.project.trim()),
            runtime: PathBuf::from(self.runtime.trim()),
            runtime_sha256: self.runtime_sha256.trim().to_string(),
            output: PathBuf::from(self.output.trim()),
            trusted_runtime: self.trusted_runtime,
            source_revision: None,
        };
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-windows-export".into())
            .spawn(move || {
                let result = windows_export::export_room_for_scene(&options, &scene, checksum)
                    .map(|report| Completed::Exported(Box::new(report)));
                let _ = sender.send(result);
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.last_output = None;
                self.last_digest = None;
                self.status = "Exporting the saved Room project and checking its runtime…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start export: {error}");
                false
            }
        }
    }

    pub fn show(&mut self, context: &egui::Context, editor: &mut Editor, presentation_dirty: bool) {
        self.poll();
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Export Windows game").open(&mut open).default_width(560.0).show(context, |ui| {
            ui.label("Exports the saved Room project. Save the scene, model bindings and camera first.");
            ui.label("Supply a known trusted, already-built Windows x86-64 Room runtime. A hash is not authentication.");
            ui.add_enabled_ui(!self.busy(), |ui| {
                field(ui, "Export project", &mut self.project);
                if ui.button("Use current project").clicked() {
                    self.use_current_scene(editor.path().as_deref());
                }
                field(ui, "Runtime executable", &mut self.runtime);
                field(ui, "Runtime SHA-256", &mut self.runtime_sha256);
                if ui.button("Check runtime SHA-256").clicked() {
                    self.measure();
                }
                field(ui, "Export destination", &mut self.output);
                ui.checkbox(&mut self.trusted_runtime, "I trust this runtime executable");
                let editable_room = editor.game().is_room() && editor.mode() == Mode::Edit && !editor.is_dirty() && !presentation_dirty;
                if ui.add_enabled(editable_room && self.trusted_runtime, egui::Button::new("Export Windows game")).clicked() {
                    self.export(editor, presentation_dirty);
                }
            });
            if self.busy() {
                ui.spinner();
                context.request_repaint_after(std::time::Duration::from_millis(40));
            }
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            if let Some(output) = &self.last_output {
                ui.label(format!("Output: {}", output.display()));
            }
            if let Some(digest) = &self.last_digest {
                ui.label(format!("Content digest: {digest}"));
            }
        });
        self.open = open;
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let label = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label.id);
}
