//! Read-only saved Room usage; no mutation plan or live editor state is accepted.
use super::{Completion, Panel};
use orr_model_bindings::model_bindings::ModelKind;
use orr_package::{Lock, Project, Runtime};
use std::{path::PathBuf, sync::mpsc};

// Keep the CLI and editor on the exact same saved-binding query. Its argument
// parser is intentionally unused here; there is no subprocess or Cargo call.
#[allow(dead_code)]
#[path = "../../../orr_sample/examples/room_package_usage/authoring.rs"]
mod authoring;
pub(super) use authoring::UsageReport;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    root: PathBuf,
    project_generation: u64,
    generation: u64,
    package: String,
    digest: String,
}

#[derive(Default)]
pub(super) struct State {
    name: String,
    generation: u64,
    ready: Option<(Identity, UsageReport)>,
}

impl State {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.ready = None;
    }

    fn select(&mut self, name: String) {
        if self.name != name {
            self.invalidate();
            self.name = name;
        }
    }
}

impl Panel {
    fn usage_identity(&self) -> Option<Identity> {
        let package = self.verified.as_ref()?.packages.get(&self.usage.name)?;
        Some(Identity {
            root: self.root.clone()?,
            project_generation: self.project_generation,
            generation: self.usage.generation,
            package: self.usage.name.clone(),
            digest: package.digest.clone(),
        })
    }

    pub(super) fn show_usage(&mut self, ui: &mut egui::Ui, can_operate: bool) {
        let Some(lock) = &self.verified else {
            return;
        };
        ui.separator();
        ui.label("Saved model package usage");
        ui.label("Saved bindings only; unsaved edits are excluded. An empty report is not removal permission or proof of safety.");
        for (name, package) in &lock.packages {
            if (package.manifest.capabilities.contains("models")
                || package.manifest.capabilities.contains("animation"))
                && ui
                    .add_enabled(
                        can_operate && !self.busy(),
                        egui::Button::new(format!("Select usage {name}")),
                    )
                    .clicked()
            {
                self.usage.select(name.clone());
            }
        }
        ui.label(format!("Usage selection: {}", self.usage.name));
        if ui
            .add_enabled(
                can_operate && !self.busy() && self.usage_identity().is_some(),
                egui::Button::new("Inspect saved usage"),
            )
            .clicked()
        {
            self.start_usage();
        }
        if let Some((identity, report)) = &self.usage.ready {
            if self.usage_identity().as_ref() == Some(identity) {
                ui.label(format!("Saved scene: {}", report.scene));
                ui.label(format!("Usage package: {}", report.package.name));
                ui.label(format!("Usage SHA-256: {}", report.package.digest));
                ui.label(report.warning);
                ui.label(format!("Saved bindings: {}", report.bindings.len()));
                egui::ScrollArea::vertical()
                    .id_salt("saved-package-usage-rows")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        // The shared query enumerates every binding in GUID order.
                        for binding in &report.bindings {
                            ui.group(|ui| {
                                ui.label(format!("Entity: {}", binding.guid));
                                ui.label(format!(
                                    "Name: {}",
                                    binding.name.as_deref().unwrap_or("(unnamed)")
                                ));
                                ui.label(format!("Asset: {}", binding.asset));
                                ui.label(format!(
                                    "Kind: {}",
                                    match binding.kind {
                                        ModelKind::Static => "static",
                                        ModelKind::Animated => "animated",
                                    }
                                ));
                                ui.label(format!("Package SHA-256: {}", binding.package_digest));
                                ui.label(format!("Source SHA-256: {}", binding.source_hash));
                            });
                        }
                    });
            }
        }
    }

    fn start_usage(&mut self) {
        if self.busy() {
            return;
        }
        self.usage.invalidate();
        let Some(identity) = self.usage_identity() else {
            self.status =
                "Verify installed packages and select a package before inspecting saved usage."
                    .into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-saved-usage".into())
            .spawn(move || {
                let result = inspect(&identity);
                let _ = sender.send(Completion::UsageInspected { identity, result });
            }) {
            Ok(_) => {
                // Reuse the package worker fence for project switching/export.
                // This is not a mutation and never enables the authoring freeze.
                self.pending = Some(receiver);
                self.status = "Inspecting saved Room model bindings without changing files…".into();
            }
            Err(error) => self.status = format!("Cannot start saved usage inspection: {error}"),
        }
    }

    pub(super) fn finish_usage(&mut self, identity: Identity, result: Result<UsageReport, String>) {
        self.usage.ready = None;
        if self.usage_identity().as_ref() != Some(&identity) {
            self.status =
                "Saved usage completed for a prior project, package or authoring state; discarded."
                    .into();
            return;
        }
        match result {
            Ok(report) => {
                self.status = "Saved usage inspected. This observation grants no removal or update permission.".into();
                self.usage.ready = Some((identity, report));
            }
            Err(error) => self.status = format!("Saved usage inspection failed: {error}"),
        }
    }
}

fn verified_lock(identity: &Identity) -> Result<Lock, String> {
    // Deliberately editor-local: dependency feature unification must not admit
    // animation when this editor's Room animation route is disabled. Verify the
    // ENTIRE installed lock, including unqueried packages, before the shared query.
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    #[cfg(feature = "room-animated-models")]
    runtime.capabilities.insert("animation".into());
    let project = Project::open(&identity.root, runtime).map_err(|error| error.to_string())?;
    if project.root() != identity.root {
        return Err("saved usage project path identity changed".into());
    }
    project.verify().map_err(|error| error.to_string())
}

fn inspect(identity: &Identity) -> Result<UsageReport, String> {
    let before = verified_lock(identity)?;
    let report = authoring::inspect_usage(&authoring::Request {
        project: identity.root.clone(),
        package: identity.package.clone(),
        digest: identity.digest.clone(),
    })?;
    if report.package.name != identity.package
        || report.package.digest != identity.digest
        || report.bindings.iter().any(|binding| {
            binding.package_digest != identity.digest
                || (!cfg!(feature = "room-animated-models") && binding.kind == ModelKind::Animated)
        })
        || verified_lock(identity)? != before
    {
        return Err(
            "saved usage identity or supported package inventory changed; inspect again".into(),
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
