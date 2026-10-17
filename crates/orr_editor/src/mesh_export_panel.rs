use crate::{editor::Editor, game::EditorGame, model::Mode};
use orr_remote::mesh_navigation::mesh_from_view;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct Request {
    scene: PathBuf,
    checksum: u64,
    graph_revision: [u8; 32],
    destination: PathBuf,
    runtime: PathBuf,
}

struct Completion {
    request: Request,
    result: Result<crate::mesh_export::Receipt, String>,
}

#[derive(Default)]
pub(crate) struct Panel {
    pub open: bool,
    pub runtime: String,
    pub output: String,
    pub trusted_runtime: bool,
    pub status: String,
    last_output: Option<PathBuf>,
    last_receipt: Option<crate::mesh_export::Receipt>,
    pending: Option<Receiver<Completion>>,
}

impl Panel {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn show(
        &mut self,
        context: &egui::Context,
        editor: &mut Editor,
        blocked: Option<&str>,
    ) {
        self.poll(editor, blocked);
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Export mesh game")
            .open(&mut open)
            .default_width(560.0)
            .vscroll(true)
            .show(context, |ui| {
                ui.label("Exports the saved MeshNavigation scene and optional camera, agent-art, mesh-props, and animated-model sidecars.");
                ui.label("Choose a trusted local mesh player executable. The executable is validated and copied, never run by the editor.");
                if let Some(reason) = blocked {
                    ui.label(reason);
                }
                ui.add_enabled_ui(!self.busy(), |ui| {
                    field(ui, "Mesh player executable", &mut self.runtime);
                    field(ui, "Mesh game output directory", &mut self.output);
                    ui.checkbox(
                        &mut self.trusted_runtime,
                        "I trust this local mesh player executable",
                    );
                    let can_export = blocked.is_none()
                        && editor.game() == EditorGame::MeshNavigation
                        && editor.mode() == Mode::Edit
                        && editor.spec().is_local()
                        && self.trusted_runtime
                        && !self.runtime.trim().is_empty()
                        && !self.output.trim().is_empty();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export mesh game"))
                        .clicked()
                    {
                        self.export(editor, blocked);
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
                    ui.label(format!("Bundle directory: {}", output.display()));
                }
                if let Some(receipt) = &self.last_receipt {
                    ui.label(format!("Scene checksum: {:016x}", receipt.checksum));
                    ui.label(format!(
                        "Graph revision: {}",
                        hex(&receipt.graph_revision)
                    ));
                    ui.label(format!("Scene SHA-256: {}", receipt.scene_sha256));
                    if let Some(sidecar_sha256) = &receipt.sidecar_sha256 {
                        ui.label(format!("Agent-art sidecar SHA-256: {sidecar_sha256}"));
                    }
                    if let Some(props_sha256) = &receipt.props_sha256 {
                        ui.label(format!("Mesh props sidecar SHA-256: {props_sha256}"));
                    }
                    if let Some(camera_sha256) = &receipt.camera_sha256 {
                        ui.label(format!("Mesh camera sidecar SHA-256: {camera_sha256}"));
                    }
                    if let Some(animation_sha256) = &receipt.animation_sha256 {
                        ui.label(format!("Mesh animation sidecar SHA-256: {animation_sha256}"));
                    }
                    ui.label(format!("Mesh player SHA-256: {}", receipt.runtime_sha256));
                    ui.label(format!("Scene copy: {}", receipt.scene.display()));
                    ui.label(format!("Manifest: {}", receipt.manifest.display()));
                    ui.label(format!("Mesh player: {}", receipt.runtime.display()));
                }
            });
        self.open = open;
    }

    fn export(&mut self, editor: &mut Editor, blocked: Option<&str>) -> bool {
        if self.busy() {
            return false;
        }
        if let Some(reason) = blocked {
            self.status = reason.to_owned();
            return false;
        }
        if !self.trusted_runtime {
            self.status =
                "Explicitly confirm that this local mesh player executable is trusted.".into();
            return false;
        }
        if editor.game() != EditorGame::MeshNavigation
            || editor.mode() != Mode::Edit
            || !editor.spec().is_local()
        {
            self.status = "Export requires a local MeshNavigation editor in Edit mode.".into();
            return false;
        }
        let (game, scene, checksum) = match editor.mesh_workspace_source_stamp() {
            Ok(stamp) => stamp,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        if game != EditorGame::MeshNavigation {
            self.status = "Export requires a MeshNavigation scene.".into();
            return false;
        }
        let Some(snapshot) = editor.snapshot() else {
            self.status = "Wait for the admitted MeshNavigation Frame before exporting.".into();
            return false;
        };
        let frame = snapshot.predicted();
        let view = match mesh_from_view(frame) {
            Ok(view) => view,
            Err(error) => {
                self.status = format!("Place and admit a mesh agent before exporting: {error}");
                return false;
            }
        };
        if frame.checksum() != checksum {
            self.status =
                "Wait for the current admitted MeshNavigation Frame before exporting.".into();
            return false;
        }
        let request = Request {
            scene,
            checksum,
            graph_revision: view.graph.revision(),
            destination: PathBuf::from(self.output.trim()),
            runtime: PathBuf::from(self.runtime.trim()),
        };
        let worker_request = Request {
            scene: request.scene.clone(),
            checksum: request.checksum,
            graph_revision: request.graph_revision,
            destination: request.destination.clone(),
            runtime: request.runtime.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-mesh-export".into())
            .spawn(move || {
                let result = crate::mesh_export::export(
                    &worker_request.scene,
                    worker_request.checksum,
                    &worker_request.destination,
                    &worker_request.runtime,
                );
                let _ = sender.send(Completion {
                    request: worker_request,
                    result,
                });
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.last_output = None;
                self.last_receipt = None;
                self.status =
                    "Exporting the admitted MeshNavigation scene and checking its player…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start mesh export: {error}");
                false
            }
        }
    }

    fn poll(&mut self, editor: &mut Editor, blocked: Option<&str>) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status =
                    "Mesh export worker disconnected; inspect the destination before retrying."
                        .into();
                return;
            }
        };
        self.pending = None;
        match completion.result {
            Err(error) => {
                self.last_output = None;
                self.last_receipt = None;
                self.status = format!("Mesh export failed: {error}");
            }
            Ok(receipt) => {
                let current = source_matches(
                    editor,
                    blocked,
                    &completion.request,
                    &self.runtime,
                    &self.output,
                    &receipt,
                );
                let output = receipt.root.clone();
                self.last_output = Some(output.clone());
                self.last_receipt = Some(receipt);
                if current {
                    self.status = format!("Mesh export complete: {}", output.display());
                } else {
                    self.status = format!(
                        "Bundle retained at {}; the current scene or export inputs changed, so this result is not marked current.",
                        output.display()
                    );
                }
            }
        }
    }
}

fn source_matches(
    editor: &mut Editor,
    blocked: Option<&str>,
    request: &Request,
    runtime: &str,
    destination: &str,
    receipt: &crate::mesh_export::Receipt,
) -> bool {
    if !completion_metadata_matches(request, runtime, destination, blocked, receipt) {
        return false;
    }
    let Ok((game, scene, checksum)) = editor.mesh_workspace_source_stamp() else {
        return false;
    };
    if game != EditorGame::MeshNavigation || scene != request.scene || checksum != request.checksum
    {
        return false;
    }
    if !saved_sidecars_match(&scene, receipt) {
        return false;
    }
    editor
        .snapshot()
        .and_then(|snapshot| mesh_from_view(snapshot.predicted()).ok())
        .is_some_and(|view| view.graph.revision() == request.graph_revision)
}

fn saved_sidecars_match(scene: &std::path::Path, receipt: &crate::mesh_export::Receipt) -> bool {
    matches!(crate::mesh_art::sidecar_stamp(scene), Ok(stamp) if stamp == receipt.sidecar_sha256)
        && matches!(crate::mesh_props::sidecar_stamp(scene), Ok(stamp) if stamp == receipt.props_sha256)
        && matches!(crate::mesh_camera_store::sidecar_stamp(scene), Ok(stamp) if stamp == receipt.camera_sha256)
        && animation_sidecar_matches(scene, receipt)
}

#[cfg(feature = "mesh-animated-models")]
fn animation_sidecar_matches(
    scene: &std::path::Path,
    receipt: &crate::mesh_export::Receipt,
) -> bool {
    matches!(crate::mesh_animation_store::sidecar_stamp(scene), Ok(stamp) if stamp == receipt.animation_sha256)
}

#[cfg(not(feature = "mesh-animated-models"))]
fn animation_sidecar_matches(
    scene: &std::path::Path,
    receipt: &crate::mesh_export::Receipt,
) -> bool {
    let Some(filename) = scene.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let path = scene.with_file_name(format!("{filename}.mesh-animation.json"));
    receipt.animation_sha256.is_none()
        && matches!(
            std::fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
}

fn completion_metadata_matches(
    request: &Request,
    runtime: &str,
    destination: &str,
    blocked: Option<&str>,
    receipt: &crate::mesh_export::Receipt,
) -> bool {
    let expected_runtime = request.runtime.to_string_lossy();
    let expected_destination = request.destination.to_string_lossy();
    blocked.is_none()
        && runtime.trim() == expected_runtime.as_ref()
        && destination.trim() == expected_destination.as_ref()
        && receipt.checksum == request.checksum
        && receipt.graph_revision == request.graph_revision
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let label_response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label_response.id);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "mesh_export_panel/tests.rs"]
mod tests;
