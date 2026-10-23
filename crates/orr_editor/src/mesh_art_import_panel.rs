use crate::mesh_art_import::{Report, Request};
use egui::Ui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct ContextStamp {
    scene: Option<PathBuf>,
    scene_epoch: u64,
    graph_revision: Option<[u8; 32]>,
    input_generation: u64,
}

struct Completion {
    context: ContextStamp,
    result: Result<Report, String>,
}

#[derive(Default)]
pub(crate) struct Panel {
    source: String,
    license: String,
    destination: String,
    name: String,
    version: String,
    license_acknowledged: bool,
    input_generation: u64,
    pending: Option<Receiver<Completion>>,
    status: String,
}

impl Panel {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        current_scene: Option<&Path>,
        scene_epoch: u64,
        graph_revision: Option<[u8; 32]>,
        can_edit: bool,
    ) -> Option<Report> {
        let completed = self.poll(current_scene, scene_epoch, graph_revision);
        egui::CollapsingHeader::new("Import raw agent art")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Import a local self-contained GLB and license file into a new agent-art package.");
                ui.label("External resources, scripts, network access, package installation, and automatic application are not supported.");
                ui.label("A supplied license file is metadata only; its presence is not legal review or a rights determination.");
                if !can_edit {
                    ui.label("Open an admitted local MeshNavigation scene in Edit mode before importing agent art.");
                }
                let fields_enabled = can_edit && !self.busy();
                ui.add_enabled_ui(fields_enabled, |ui| {
                    let source_changed = path_field(ui, "Raw agent GLB", &mut self.source);
                    let license_changed = path_field(ui, "Raw agent license", &mut self.license);
                    let destination_changed = path_field(
                        ui,
                        "New agent package directory",
                        &mut self.destination,
                    );
                    let name_changed = text_field(ui, "Agent package name", &mut self.name, 128);
                    let version_changed =
                        text_field(ui, "Agent package version", &mut self.version, 128);
                    let acknowledgement_changed = ui
                        .checkbox(
                            &mut self.license_acknowledged,
                            "I have reviewed rights to import this agent art",
                        )
                        .changed();
                    if source_changed || license_changed {
                        self.license_acknowledged = false;
                    }
                    if source_changed
                        || license_changed
                        || destination_changed
                        || name_changed
                        || version_changed
                        || acknowledgement_changed
                    {
                        self.advance_input_generation();
                    }
                });
                let request_ready = fields_enabled
                    && current_scene.is_some()
                    && graph_revision.is_some()
                    && self.license_acknowledged
                    && !self.source.trim().is_empty()
                    && !self.license.trim().is_empty()
                    && !self.destination.trim().is_empty()
                    && !self.name.trim().is_empty()
                    && !self.version.trim().is_empty();
                if ui
                    .add_enabled(request_ready, egui::Button::new("Import agent GLB"))
                    .clicked()
                {
                    match (current_scene, graph_revision) {
                        (Some(scene), Some(revision)) => self.start_import(
                            scene.to_path_buf(),
                            scene_epoch,
                            revision,
                        ),
                        _ => {
                            self.status = "Import requires a current admitted mesh scene and graph revision.".into();
                        }
                    }
                }
                if self.busy() {
                    ui.spinner();
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        completed
    }

    fn advance_input_generation(&mut self) {
        self.input_generation = self.input_generation.saturating_add(1);
    }

    fn start_import(&mut self, scene: PathBuf, scene_epoch: u64, graph_revision: [u8; 32]) {
        if self.busy() {
            return;
        }
        if !self.license_acknowledged {
            self.status =
                "Explicitly review and acknowledge rights before importing agent art.".into();
            return;
        }
        let request = Request {
            source: PathBuf::from(self.source.trim()),
            license: PathBuf::from(self.license.trim()),
            destination: PathBuf::from(self.destination.trim()),
            name: self.name.trim().to_owned(),
            version: self.version.trim().to_owned(),
            license_acknowledged: self.license_acknowledged,
            graph_revision,
        };
        let context = ContextStamp {
            scene: Some(scene),
            scene_epoch,
            graph_revision: Some(graph_revision),
            input_generation: self.input_generation,
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-mesh-agent-art-import".into())
            .spawn(move || {
                let result = crate::mesh_art_import::import_package(request);
                let _ = sender.send(Completion { context, result });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Importing and validating the self-contained GLB on a worker…".into();
            }
            Err(error) => self.status = format!("Cannot start agent-art import: {error}"),
        }
    }

    fn poll(
        &mut self,
        current_scene: Option<&Path>,
        scene_epoch: u64,
        graph_revision: Option<[u8; 32]>,
    ) -> Option<Report> {
        let Some(receiver) = &self.pending else {
            return None;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = "Agent-art import worker disconnected; inspect the destination before retrying.".into();
                return None;
            }
        };
        self.pending = None;
        let current = context_matches(
            &completion.context,
            current_scene,
            scene_epoch,
            graph_revision,
            self.input_generation,
        );
        match completion.result {
            Ok(report) if current => {
                self.status = format!(
                    "Imported agent art package: {} ({}/{}). Source SHA-256: {}; license SHA-256: {}. Not applied; use Apply agent art and Save agent art explicitly.",
                    report.root.display(),
                    report.asset,
                    report.license,
                    report.source_sha256,
                    report.license_sha256
                );
                Some(report)
            }
            Ok(report) => {
                self.status = format!(
                    "Agent-art package retained at {}; the scene, graph, or import inputs changed, so it was not selected.",
                    report.root.display()
                );
                None
            }
            Err(error) if current => {
                self.status = format!("Agent-art import failed: {error}");
                None
            }
            Err(error) => {
                self.status =
                    format!("Agent-art import for a prior scene or input state failed: {error}");
                None
            }
        }
    }
}

fn context_matches(
    context: &ContextStamp,
    current_scene: Option<&Path>,
    scene_epoch: u64,
    graph_revision: Option<[u8; 32]>,
    input_generation: u64,
) -> bool {
    context.scene.as_deref() == current_scene
        && context.scene_epoch == scene_epoch
        && context.graph_revision == graph_revision
        && context.input_generation == input_generation
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

fn text_field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) -> bool {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(limit),
    )
    .labelled_by(response.id)
    .changed()
}

#[cfg(test)]
#[path = "mesh_art_import_panel/tests.rs"]
mod tests;
