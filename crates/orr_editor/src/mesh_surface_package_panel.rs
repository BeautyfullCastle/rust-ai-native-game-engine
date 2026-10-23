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
    ) {
        egui::CollapsingHeader::new("Cook GLB as walkable package")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Cook a self-contained GLB and license into a new package. This does not stage or apply geometry.");
                ui.label("Only the existing static-model GLB subset is supported; external resources are rejected.");
                ui.label("The supplied license file is metadata, not legal review or a rights determination.");
                if !can_edit {
                    ui.label("Open an admitted local MeshNavigation scene in Edit mode before cooking a walkable package.");
                }
                let fields_enabled = can_edit && !self.busy();
                ui.add_enabled_ui(fields_enabled, |ui| {
                    let source_changed = path_field(ui, "Walkable GLB source", &mut self.source);
                    let license_changed =
                        path_field(ui, "Walkable model license file", &mut self.license);
                    let destination_changed = path_field(
                        ui,
                        "New walkable model package directory",
                        &mut self.destination,
                    );
                    let name_changed =
                        text_field(ui, "Walkable package name", &mut self.name, 64);
                    let version_changed =
                        text_field(ui, "Walkable package version", &mut self.version, 128);
                    let acknowledgement_changed = ui
                        .checkbox(
                            &mut self.license_acknowledged,
                            "I have reviewed rights to use this walkable model",
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
                    .add_enabled(
                        request_ready,
                        egui::Button::new("Cook walkable model package"),
                    )
                    .clicked()
                {
                    match (current_scene, graph_revision) {
                        (Some(scene), Some(revision)) => {
                            self.start_cook(scene.to_path_buf(), scene_epoch, revision)
                        }
                        _ => {
                            self.status = "Cooking requires a current admitted mesh scene and graph revision.".into();
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
    }

    fn advance_input_generation(&mut self) {
        self.input_generation = self.input_generation.saturating_add(1);
    }

    fn start_cook(&mut self, scene: PathBuf, scene_epoch: u64, graph_revision: [u8; 32]) {
        if self.busy() {
            return;
        }
        if !self.license_acknowledged {
            self.status =
                "Explicitly review and acknowledge rights before cooking a walkable package."
                    .into();
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
            .name("orr-mesh-walkable-package-cook".into())
            .spawn(move || {
                let result = crate::mesh_art_import::import_package(request);
                let _ = sender.send(Completion { context, result });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status =
                    "Cooking and validating the self-contained GLB package on a worker…".into();
            }
            Err(error) => self.status = format!("Cannot start walkable package cook: {error}"),
        }
    }

    pub(crate) fn poll(
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
                self.status = "Walkable package cook worker disconnected; inspect the destination before retrying.".into();
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
                    "Walkable model package created at {} ({} and {}). Source SHA-256: {}; license SHA-256: {}. Geometry was not staged or applied.",
                    report.root.display(),
                    report.asset,
                    report.license,
                    report.source_sha256,
                    report.license_sha256,
                );
                Some(report)
            }
            Ok(report) => {
                self.status = format!(
                    "Walkable model package retained at {}; the scene, graph, or cook inputs changed, so it was not selected.",
                    report.root.display()
                );
                None
            }
            Err(error) if current => {
                self.status = format!("Walkable model package cook failed: {error}");
                None
            }
            Err(error) => {
                self.status = format!(
                    "Walkable package cook for a prior scene or input state failed: {error}"
                );
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
#[path = "mesh_surface_package_panel/tests.rs"]
mod tests;
