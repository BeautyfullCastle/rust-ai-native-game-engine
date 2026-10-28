//! Separate, opt-in hazard export UI with an optional authored camera. No process launching.
use crate::{
    editor::Editor,
    mesh_hazards_export::{self as bundle, ExportRequest, Receipt},
    model::Mode,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    path: PathBuf,
    bytes: u64,
    modified: std::time::SystemTime,
}
impl Identity {
    fn read(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("Use an absolute local file path".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Choose a regular local file, not a symlink".into());
        }
        Ok(Self {
            path: path.to_path_buf(),
            bytes: metadata.len(),
            modified: metadata.modified().map_err(|e| e.to_string())?,
        })
    }
    fn unchanged(&self) -> bool {
        Self::read(&self.path).as_ref() == Ok(self)
    }
}
#[derive(Clone)]
struct Request {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
    scene_identity: Identity,
    /// The optional source sidecar is part of the export input, even when it is
    /// absent. Keeping its exact bytes (rather than a timestamp) lets us reject
    /// additions, removals, and replacements across the async boundary.
    camera_source: Option<Vec<u8>>,
    runtime: Identity,
    destination: PathBuf,
}
struct Completion {
    request: Request,
    result: Result<Receipt, String>,
}
#[derive(Default)]
pub(crate) struct Panel {
    pub open: bool,
    pub runtime: String,
    pub output: String,
    pub trusted_runtime: bool,
    pub status: String,
    pending: Option<Receiver<Completion>>,
    cancelled: bool,
}
impl Panel {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    #[cfg(test)]
    pub(crate) fn hold_pending_for_test(&mut self) -> impl FnOnce() {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending = Some(receiver);
        move || drop(sender)
    }
    pub(crate) fn show(&mut self, ctx: &egui::Context, editor: &mut Editor, blocked: Option<&str>) {
        self.poll(editor, blocked);
        // A closed window must still poll/repaint until the worker is observed.
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Export hazard game").open(&mut open).default_width(600.0).show(ctx, |ui| {
            ui.label("MeshHazardsV1 bundle: scene, optional authored camera and manifest, plus a PE/ELF hazard player. Enabled hazards are required; other adjacent presentation sidecars are unsupported.");
            ui.label("Use only a player you built and QA-tested yourself. Format/capability validation does not establish trust. The editor never runs the copied executable.");
            if let Some(reason) = blocked { ui.label(reason); }
            ui.add_enabled_ui(!self.busy(), |ui| {
                let runtime_label = ui.label("Hazard player executable");
                if ui.text_edit_singleline(&mut self.runtime).labelled_by(runtime_label.id).changed() { self.trusted_runtime = false; }
                let output_label = ui.label("New output directory (must not exist)");
                ui.text_edit_singleline(&mut self.output).labelled_by(output_label.id);
                ui.checkbox(&mut self.trusted_runtime, "I trust this self-built, QA-tested hazard player");
                if self.runtime.trim().is_empty() { ui.label("Required: choose a trusted hazard player executable."); }
                if self.output.trim().is_empty() { ui.label("Required: enter a new output directory."); }
                if !self.trusted_runtime { ui.label("Required: explicitly confirm runtime trust; format checks do not establish trust."); }
                let enabled = eligible(editor, blocked, self.trusted_runtime) && !self.runtime.trim().is_empty() && !self.output.trim().is_empty();
                if ui.add_enabled(enabled, egui::Button::new("Export hazard game")).clicked() { self.export(editor, blocked); }
            });
            if self.busy() {
                ui.spinner();
                if ui.add_enabled(!self.cancelled, egui::Button::new("Cancel result acceptance")).clicked() { self.cancelled = true; self.status = "Cancellation requested: writing may finish. Any published bundle is retained; wait for the result before inspecting or retrying.".into(); }
            }
            ui.label(&self.status);
        });
        self.open = open;
    }

    fn export(&mut self, editor: &mut Editor, blocked: Option<&str>) -> bool {
        if self.busy() {
            return false;
        }
        let prepared = (|| {
            if !eligible(editor, blocked, self.trusted_runtime) {
                return Err(blocked.unwrap_or("Export requires a trusted player and a saved, idle local MeshHazardsV1 scene in Edit mode").to_owned());
            }
            let (game, scene, checksum) = editor.mesh_creation_source_stamp()?;
            if !game.is_mesh_hazards() {
                return Err("Export requires MeshHazardsV1".into());
            }
            let request = Request {
                epoch: editor.scene_load_epoch(),
                checksum,
                scene_identity: Identity::read(&scene)?,
                camera_source: bundle::capture_camera_source(&scene)?,
                scene,
                runtime: Identity::read(Path::new(self.runtime.trim()))?,
                destination: PathBuf::from(self.output.trim()),
            };
            if !source_matches(editor, blocked, &request) {
                return Err(
                    "Scene changed before export dispatch; retry with the saved current scene"
                        .into(),
                );
            }
            Ok(request)
        })();
        let request = match prepared {
            Ok(request) => request,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("orr-hazard-export".into())
            .spawn(move || {
                let result = run(&request);
                let _ = sender.send(Completion { request, result });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.cancelled = false;
                self.status = "Exporting and validating the saved hazard game…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start hazard export: {error}");
                false
            }
        }
    }
    fn poll(&mut self, editor: &mut Editor, blocked: Option<&str>) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status =
                    "Export worker disconnected; inspect the destination before retrying".into();
                return;
            }
        };
        self.pending = None;
        match completion.result {
            Err(error) => self.status = format!("Hazard export failed: {error}"),
            Ok(receipt) => {
                let current = inputs_match(
                    &self.runtime,
                    &self.output,
                    &completion.request,
                    self.cancelled,
                    self.trusted_runtime,
                ) && source_matches(editor, blocked, &completion.request)
                    && receipt.checksum == completion.request.checksum;
                self.status = format!(
                    "{} at {}. Platform: {:?}; scene checksum {:016x}; player SHA-256 {}; camera SHA-256 {}. Player was not launched.",
                    if current {
                        "Hazard export complete"
                    } else {
                        "Bundle retained, but cancelled or changed inputs mean this result is not current"
                    },
                    receipt.root.display(),
                    receipt.platform,
                    receipt.checksum,
                    receipt.runtime_sha256,
                    receipt.camera_sha256.as_deref().unwrap_or("none")
                );
            }
        }
    }
}
fn eligible(editor: &Editor, blocked: Option<&str>, trusted: bool) -> bool {
    blocked.is_none()
        && trusted
        && editor.game().is_mesh_hazards()
        && editor.spec().is_local()
        && editor.mode() == Mode::Edit
        && editor.can_mutate()
        && !editor.is_dirty()
        && editor.previewing().is_none()
}
fn source_matches(editor: &mut Editor, blocked: Option<&str>, request: &Request) -> bool {
    if !eligible(editor, blocked, true)
        || editor.scene_load_epoch() != request.epoch
        || !request.runtime.unchanged()
        || !request.scene_identity.unchanged()
        || bundle::capture_camera_source(&request.scene).as_ref() != Ok(&request.camera_source)
    {
        return false;
    }
    matches!(editor.mesh_creation_source_stamp(), Ok((game, scene, checksum)) if game.is_mesh_hazards() && scene == request.scene && checksum == request.checksum)
}
fn inputs_match(
    runtime: &str,
    output: &str,
    request: &Request,
    cancelled: bool,
    trusted: bool,
) -> bool {
    // Compare the same entered path form on both sides. Canonicalizing just one side
    // breaks ordinary Windows drive paths (verbatim prefix) and redundant separators.
    !cancelled
        && trusted
        && Path::new(runtime.trim()) == request.runtime.path
        && Path::new(output.trim()) == request.destination
}
fn digest_runtime(path: &Path) -> Result<String, String> {
    bundle::runtime_file_digest(path)
}

fn run(request: &Request) -> Result<Receipt, String> {
    if !request.runtime.unchanged() || !request.scene_identity.unchanged() {
        return Err("Scene or runtime identity changed before dispatch".into());
    }
    let digest = digest_runtime(&request.runtime.path)?;
    let receipt = bundle::export_interactive_bound(
        ExportRequest {
            scene: &request.scene,
            expected_checksum: request.checksum,
            destination: &request.destination,
            runtime: &request.runtime.path,
            trusted_runtime: true,
        },
        &request.camera_source,
    )?;
    if receipt.runtime_sha256 != digest
        || digest_runtime(&request.runtime.path)? != digest
        || !request.runtime.unchanged()
        || !request.scene_identity.unchanged()
        || bundle::capture_camera_source(&request.scene).as_ref() != Ok(&request.camera_source)
    {
        return Err(format!(
            "Source identity changed during export; any published bundle remains at {} and is not accepted as current",
            receipt.root.display()
        ));
    }
    Ok(receipt)
}
#[cfg(test)]
#[path = "mesh_hazards_export_panel/tests.rs"]
mod tests;
