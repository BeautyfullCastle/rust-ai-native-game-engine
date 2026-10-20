//! Popup-free, explicitly dispatched Arena export. Each job owns its worker
//! until terminal AND joined, even when the tab is hidden or drawing returns early.
use super::EditorApp;
use orr_sample::project_export::{ArenaExportSource, ExportOptions, ExportReport};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
};

#[derive(Default)]
pub(super) struct State {
    project: String,
    runtime: String,
    hash: String,
    output: String,
    trusted: bool,
    next_id: u64,
    job: Option<Job>,
    pub(super) status: String,
    completed_successfully: bool,
    uncertain_output: Option<PathBuf>,
}
struct Job {
    id: u64,
    source: ArenaExportSource,
    options: ExportOptions,
    receiver: Receiver<Result<ExportReport, String>>,
    handle: Option<JoinHandle<()>>,
    result: Option<Result<ExportReport, String>>,
    disconnected: bool,
}
impl Drop for Job {
    fn drop(&mut self) {
        // Normal app destruction must not silently detach a publishing worker.
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }
    fn poll(&mut self) {
        let Some(job) = self.job.as_mut() else { return };
        if job.result.is_none() && !job.disconnected {
            match job.receiver.try_recv() {
                Ok(result) => job.result = Some(result),
                Err(TryRecvError::Disconnected) => job.disconnected = true,
                Err(TryRecvError::Empty) => {}
            }
        }
        if !job.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return;
        }
        let joined = job.handle.take().expect("owned worker").join();
        // Receiver readiness and thread terminality are independent observations.
        if job.result.is_none() {
            match job.receiver.try_recv() {
                Ok(result) => job.result = Some(result),
                Err(_) => job.disconnected = true,
            }
        }
        let mut job = self.job.take().expect("owned finished job");
        let identity = format!(
            "Job {} · captured {} · checksum 0x{:016x}",
            job.id,
            job.source.canonical_scene().display(),
            job.source.initial_checksum()
        );
        self.status = if joined.is_err() || job.result.is_none() {
            self.uncertain_output = Some(job.options.output.clone());
            format!("{identity}: worker ended without a reliable result. Publication is uncertain. Inspect destination before any explicit retry; nothing was deleted.")
        } else {
            match job.result.take().expect("observed result") {
                Ok(report) => {
                    self.completed_successfully = true;
                    format!("{identity}: exported to {} ({} project files). This result belongs to the captured source, not later edits. No game was launched after verification.",report.output.display(),report.project_files)
                }
                Err(error) => {
                    // An error can occur after publication. Never assume absence or erase output.
                    self.uncertain_output = Some(job.options.output.clone());
                    format!("{identity}: {error}. Inspect destination before retrying; no automatic retry or deletion.")
                }
            }
        };
    }
}
impl EditorApp {
    pub(super) fn poll_arena_export(&mut self, ctx: &egui::Context) {
        self.arena_export.poll();
        if self.arena_export.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    fn arena_export_dispatch(&mut self) -> Result<(), String> {
        self.arena_export_dispatch_with(|name, task| thread::Builder::new().name(name).spawn(task))
    }
    fn arena_export_dispatch_with(
        &mut self,
        spawn: impl FnOnce(String, Box<dyn FnOnce() + Send>) -> std::io::Result<JoinHandle<()>>,
    ) -> Result<(), String> {
        if self.arena_export.busy() {
            return Err(
                "An Arena export worker is still owned; wait for terminal and joined completion"
                    .into(),
            );
        }
        self.arena_export.completed_successfully = false;
        if self.arena_export.uncertain_output.is_some() {
            return Err("Inspect the previous destination before explicit retry".into());
        }
        if self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
            || self.ui.dialog.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
        {
            return Err("Finish screenshot mode, capture, path dialogs and pending content work before export".into());
        }
        #[cfg(feature = "tilemap")]
        if self.tilemap.is_some() {
            return Err("Loaded tilemap is not supported by this export slice".into());
        }
        #[cfg(feature = "project-ui")]
        if self.project_ui.is_some() {
            return Err("Loaded UI is not supported by this export slice".into());
        }
        #[cfg(feature = "collect-ui")]
        if self.collect_ui.is_some() {
            return Err("Loaded UI is not supported by this export slice".into());
        }
        let options = ExportOptions {
            project: PathBuf::from(&self.arena_export.project),
            runtime: PathBuf::from(&self.arena_export.runtime),
            runtime_sha256: self.arena_export.hash.clone(),
            output: PathBuf::from(&self.arena_export.output),
            trusted_runtime: self.arena_export.trusted,
            source_revision: None,
        };
        if !options.trusted_runtime {
            return Err(
                "Explicitly trust the supplied runtime before executing smoke verification".into(),
            );
        }
        if options.runtime_sha256.len() != 64
            || !options
                .runtime_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Expected SHA256 must be 64 lowercase hexadecimal characters".into());
        }
        if options.project.as_os_str().is_empty()
            || options.runtime.as_os_str().is_empty()
            || options.output.as_os_str().is_empty()
        {
            return Err("Enter project, runtime and new output paths".into());
        }
        let source = self
            .editor
            .arena_export_source(&options.project, &self.sprites)?;
        let id = self
            .arena_export
            .next_id
            .checked_add(1)
            .ok_or("Export job ID exhausted")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_options = options.clone();
        let worker_source = source.clone();
        let handle = spawn(
            format!("arena-export-{id}"),
            Box::new(move || {
                let result = orr_sample::project_export::export_arena_for_source(
                    &worker_options,
                    &worker_source,
                );
                let _ = sender.send(result);
            }),
        )
        .map_err(|e| format!("Worker could not start; no export was run: {e}"))?;
        self.arena_export.completed_successfully = false;
        self.arena_export.next_id = id;
        self.arena_export.status = format!(
            "Job {id} exporting captured {} at checksum 0x{:016x}. Later edits are not included.",
            source.canonical_scene().display(),
            source.initial_checksum()
        );
        self.arena_export.job = Some(Job {
            id,
            source,
            options,
            receiver,
            handle: Some(handle),
            result: None,
            disconnected: false,
        });
        Ok(())
    }
    pub(super) fn show_arena_export(&mut self, ui: &mut egui::Ui) {
        ui.label("Linux x86-64 · saved Arena scene + sprites only. Declared or loaded UI/tilemap is refused.");
        ui.label("Verification executes supplied code. SHA256 checks identity, not trust. Supply a matching, known trusted, already-built Arena player.");
        egui::Grid::new("arena_export_form").show(ui, |ui| {
            for (label, value) in [
                ("Project path", &mut self.arena_export.project),
                ("New output path", &mut self.arena_export.output),
            ] {
                let label = ui.label(label);
                ui.text_edit_singleline(value).labelled_by(label.id);
                ui.end_row();
            }
            let label = ui.label("Runtime path");
            if ui
                .text_edit_singleline(&mut self.arena_export.runtime)
                .labelled_by(label.id)
                .changed()
            {
                self.arena_export.trusted = false;
            }
            ui.end_row();
            let label = ui.label("Expected lowercase SHA256");
            if ui
                .text_edit_singleline(&mut self.arena_export.hash)
                .labelled_by(label.id)
                .changed()
            {
                self.arena_export.trusted = false;
            }
            ui.end_row();
        });
        ui.checkbox(
            &mut self.arena_export.trusted,
            "I trust this supplied runtime and allow smoke verification to execute it",
        );
        if ui
            .add_enabled(
                !self.arena_export.busy() && self.arena_export.uncertain_output.is_none(),
                egui::Button::new("Export saved Arena"),
            )
            .clicked()
        {
            if let Err(error) = self.arena_export_dispatch() {
                self.arena_export.status = error;
            }
        }
        if let Some(path) = self.arena_export.uncertain_output.clone() {
            if ui.button("Inspect previous destination").clicked() {
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) => {
                        self.arena_export.status=format!("Previous destination {} exists (directory: {}, symlink: {}). It was not deleted. Use a different new output path or inspect it yourself before retrying.",path.display(),meta.is_dir(),meta.file_type().is_symlink());
                        self.arena_export.uncertain_output = None;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        self.arena_export.status=format!("Previous destination {} is absent at inspection. Explicit retry is available; no retry was started.",path.display());
                        self.arena_export.uncertain_output = None;
                    }
                    Err(e) => {
                        self.arena_export.status = format!(
                            "Cannot inspect previous destination: {e}. Retry remains blocked."
                        )
                    }
                }
            }
        }
        if self.arena_export.completed_successfully {
            ui.label("Arena export completed and joined");
        }
        ui.label(&self.arena_export.status);
    }
}

#[cfg(test)]
#[path = "arena_export_tests.rs"]
mod tests;
