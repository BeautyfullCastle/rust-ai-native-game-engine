//! Local package production only. Admission stages a candidate; Apply/Save stay explicit.
use super::{text_field, Panel};
use crate::{
    editor::Editor,
    mesh_animation_import::{self, Report, Request},
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::JoinHandle,
};

fn supported_source(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    [".glb", ".animated.json", ".orrmodel.json"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Inputs {
    source: String,
    license: String,
    destination: String,
    name: String,
    version: String,
    acknowledged: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    revision: [u8; 32],
    generation: u64,
    candidate_generation: u64,
    inputs: Inputs,
}

struct Job {
    context: Context,
    receiver: Receiver<Result<Report, String>>,
    handle: Option<JoinHandle<()>>,
    terminal: Option<Result<Report, String>>,
}

impl Job {
    fn poll(&mut self) -> Option<Result<Report, String>> {
        if self.terminal.is_none() {
            self.terminal = match self.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("Animation import worker disconnected; inspect the destination before retrying.".into())),
            };
        }
        // A sent result is not proof that the worker has released its resources.
        if self.terminal.is_none() || !self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            return None;
        }
        let joined = self.handle.take().unwrap().join();
        let result = self.terminal.take().unwrap();
        Some(match (result, joined) {
            (Ok(mut report), Err(_)) => {
                report.publication_warning = Some(
                    "Worker panicked after publishing; inspect the package before using it.".into(),
                );
                Ok(report)
            }
            (Err(error), Err(_)) => Err(format!(
                "{error} Worker panicked; destination outcome may be uncertain."
            )),
            (result, Ok(())) => result,
        })
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) struct State {
    inputs: Inputs,
    generation: u64,
    job: Option<Job>,
    status: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            inputs: Inputs {
                version: "1.0.0".into(),
                ..Default::default()
            },
            generation: 0,
            job: None,
            status: String::new(),
        }
    }
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn changed(&mut self, source_changed: bool) {
        self.generation = self.generation.wrapping_add(1);
        if source_changed {
            self.inputs.acknowledged = false;
        }
    }
}

fn context(panel: &Panel, editor: &Editor) -> Option<Context> {
    let (scene, revision) = panel.current_identity(editor)?;
    Some(Context {
        scene,
        revision,
        epoch: editor.scene_load_epoch(),
        generation: panel.import.generation,
        candidate_generation: panel.candidate_generation,
        inputs: panel.import.inputs.clone(),
    })
}

fn show_inputs(ui: &mut egui::Ui, state: &mut State, editable: bool) -> bool {
    let mut launch = false;
    ui.add_enabled_ui(editable, |ui| {
        let source_changed =
            text_field(
                ui,
                "Animation source (.glb or cooked JSON)",
                &mut state.inputs.source,
            ) | text_field(ui, "Animation source license", &mut state.inputs.license);
        let changed = text_field(
            ui,
            "New animation package directory",
            &mut state.inputs.destination,
        ) | text_field(ui, "Animation package name", &mut state.inputs.name)
            | text_field(ui, "Animation package version", &mut state.inputs.version);
        if source_changed || changed {
            state.changed(source_changed);
        }
        if ui
            .checkbox(
                &mut state.inputs.acknowledged,
                "I have reviewed rights to import this animation",
            )
            .changed()
        {
            state.changed(false);
        }
        let inputs = &state.inputs;
        let ready = inputs.acknowledged
            && supported_source(&inputs.source)
            && [
                &inputs.source,
                &inputs.license,
                &inputs.destination,
                &inputs.name,
                &inputs.version,
            ]
            .iter()
            .all(|v| !v.trim().is_empty());
        launch = ui
            .add_enabled(ready, egui::Button::new("Import animation package"))
            .clicked();
    });
    launch
}
pub(super) fn show(ui: &mut egui::Ui, panel: &mut Panel, editor: &Editor) {
    // Source suffix selects the existing importer; no implicit format fallback.
    crate::theme::collapsing(ui, "Import animation source", |ui| {
        ui.label("Create a new local animation package from a self-contained GLB or a fully embedded cooked .animated.json model, with a separate UTF-8 license. Use .glb for raw input or .animated.json/.orrmodel.json for cooked input. Existing destinations are never replaced.");
        ui.weak("Cooked import preserves animation bytes; it does not resolve historical dependencies, scripts or external resources. Apply and Save remain explicit.");
        let editable=panel.can_edit(editor);
        if show_inputs(ui,&mut panel.import,editable) {launch(panel,editor);}
        if panel.import.busy() {
            ui.label("Importing animation package; waiting for the worker to finish. Closing and workspace changes are blocked.");
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        if !panel.import.status.is_empty() {ui.label(&panel.import.status);}
    });
}

fn launch(panel: &mut Panel, editor: &Editor) {
    if !panel.can_edit(editor) || !panel.import.inputs.acknowledged {
        return;
    }
    if !supported_source(&panel.import.inputs.source) {
        panel.import.status =
            "Choose a .glb, .animated.json or .orrmodel.json source; no worker was started.".into();
        return;
    }
    let Some(context) = context(panel, editor) else {
        return;
    };
    let inputs = &context.inputs;
    let request = Request {
        source: PathBuf::from(inputs.source.trim()),
        license: PathBuf::from(inputs.license.trim()),
        destination: PathBuf::from(inputs.destination.trim()),
        name: inputs.name.trim().into(),
        version: inputs.version.trim().into(),
        license_acknowledged: inputs.acknowledged,
        graph_revision: context.revision,
    };
    let (sender, receiver) = mpsc::channel();
    match std::thread::Builder::new()
        .name("mesh-animation-import".into())
        .spawn(move || {
            let result = mesh_animation_import::import_package(request);
            let _ = sender.send(result);
        }) {
        Ok(handle) => {
            panel.preview.reset();
            panel.import.job = Some(Job {
                context,
                receiver,
                handle: Some(handle),
                terminal: None,
            });
            panel.import.status.clear();
        }
        Err(error) => {
            panel.import.status =
                format!("Could not start animation import worker: {error}. No retry was started.")
        }
    }
}

pub(super) fn poll(panel: &mut Panel, editor: &Editor) {
    let Some(result) = panel.import.job.as_mut().and_then(Job::poll) else {
        return;
    };
    let job = panel.import.job.take().unwrap();
    let matches = context(panel, editor).as_ref() == Some(&job.context) && panel.can_edit(editor);
    match result {
        Ok(report) => {
            let root = report.root.display().to_string();
            let digests = format!(
                " Source SHA-256: {}; license SHA-256: {}.",
                report.source_sha256, report.license_sha256
            );
            let warning = report
                .publication_warning
                .as_deref()
                .map(|text| format!(" Warning: {text}"))
                .unwrap_or_default();
            if matches
                && report.publication_warning.is_none()
                && report.snapshot.graph_revision() == job.context.revision
            {
                panel.preview.reset();
                panel.candidate_generation = panel.candidate_generation.wrapping_add(1);
                panel.package_root = root.clone();
                panel.asset = report.asset;
                panel.license = report.license;
                panel.copy_settings(&report.snapshot);
                panel.candidate = Some(report.snapshot);
                panel.form_dirty = false;
                panel.source_dirty = false;
                panel.license_acknowledged = false;
                panel.error = None;
                panel.import.status = format!("Imported animation package {root}; staged for review, not applied.{digests}{warning}");
            } else {
                panel.import.status = format!("Imported animation package {root}; not staged because the inputs, editor context, or publication verification changed. Load it explicitly after review.{digests}{warning}");
            }
            panel.import.inputs.acknowledged = false;
            panel.import.changed(false);
        }
        Err(error) => panel.import.status = format!("Animation import failed: {error}"),
    }
}

#[cfg(test)]
pub(super) fn start_blocked_for_test(panel: &mut Panel, editor: &Editor) -> mpsc::Sender<()> {
    assert!(!panel.import_busy());
    let context = context(panel, editor).expect("coherent mesh scene");
    let (sender, receiver) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = wait.recv();
        let _ = sender.send(Err("Controlled import worker finished".into()));
    });
    panel.import.job = Some(Job {
        context,
        receiver,
        handle: Some(handle),
        terminal: None,
    });
    release
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "import_panel/cooked_ui_tests.rs"]
mod cooked_ui_tests;
