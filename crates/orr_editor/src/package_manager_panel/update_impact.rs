//! Read-only candidate revision observations; never an update/apply token.
use super::{parse_source_path, Completion, Panel};
use crate::package_manager::{self, AssetChange, UpdateImpactReport};
use orr_model_bindings::model_bindings::ModelKind;
use std::{path::PathBuf, sync::mpsc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    root: PathBuf,
    source: PathBuf,
    source_text: String,
    project_generation: u64,
    source_generation: u64,
    generation: u64,
}

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    ready: Option<(Identity, UpdateImpactReport)>,
}

impl State {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.ready = None;
    }
}

impl Panel {
    fn impact_identity(&self) -> Option<Identity> {
        Some(Identity {
            root: self.root.clone()?,
            source: parse_source_path(&self.source).ok()?,
            source_text: self.source.trim().into(),
            project_generation: self.project_generation,
            source_generation: self.source_generation,
            generation: self.update_impact.generation,
        })
    }

    pub(super) fn show_update_impact(&mut self, ui: &mut egui::Ui, can_operate: bool) {
        ui.separator();
        ui.label("Candidate revision impact on saved model bindings");
        if ui
            .add_enabled(
                can_operate && !self.busy() && self.impact_identity().is_some(),
                egui::Button::new("Inspect update impact"),
            )
            .clicked()
        {
            self.start_update_impact();
        }
        ui.label("Read-only observation at the last inspection, not continuous monitoring. Inspect again to refresh after external edits. No bindings or package selections are changed.");
        if let Some((identity, report)) = &self.update_impact.ready {
            if self.impact_identity().as_ref() != Some(identity) {
                return;
            }
            ui.label(format!(
                "Impact package: {} ({} -> {})",
                report.name(),
                report.before_version(),
                report.after_version()
            ));
            ui.label(format!(
                "Affected saved bindings: {}",
                report.bindings().len()
            ));
            ui.label("Candidate model kind/decoding is unchecked. Identical asset bytes do not establish unchanged dependencies or update safety. Even zero bindings grants no update permission.");
            egui::ScrollArea::vertical()
                .id_salt("package-update-impact-rows")
                .max_height(250.0)
                .show(ui, |ui| {
                    for binding in report.bindings() {
                        ui.group(|ui| {
                            ui.label(format!("Impact entity: {}", binding.guid));
                            ui.label(format!("Impact asset: {}", binding.asset));
                            ui.label(format!(
                                "Saved kind: {}",
                                match binding.kind {
                                    ModelKind::Static => "static",
                                    ModelKind::Animated => "animated",
                                }
                            ));
                            ui.label(match binding.change {
                                AssetChange::Missing => {
                                    "Referenced asset missing from candidate revision"
                                }
                                AssetChange::BytesChanged => {
                                    "Referenced asset bytes changed; package identity changed"
                                }
                                AssetChange::BytesUnchanged => {
                                    "Referenced asset bytes unchanged; package identity changed"
                                }
                            });
                        });
                    }
                });
        }
    }

    fn start_update_impact(&mut self) {
        if self.busy() {
            return;
        }
        self.update_impact.invalidate();
        let Some(identity) = self.impact_identity() else {
            self.status =
                "Open a saved Room project and choose a local candidate package directory.".into();
            return;
        };
        self.update_preview = None;
        self.preview = None;
        self.removal_preview = None;
        self.restoration_preview = None;
        self.usage.invalidate();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-update-impact".into())
            .spawn(move || {
                let result =
                    package_manager::inspect_update_impact(&identity.root, &identity.source);
                let _ = sender.send(Completion::UpdateImpactInspected { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status =
                    "Inspecting candidate revision impact without changing packages…".into();
            }
            Err(error) => self.status = format!("Cannot start update impact inspection: {error}"),
        }
    }

    pub(super) fn recheck_update_impact(
        &mut self,
        identity: Identity,
        result: Result<UpdateImpactReport, String>,
    ) {
        self.update_impact.ready = None;
        if self.impact_identity().as_ref() != Some(&identity) {
            self.status = "Update impact completed for a prior project, source or authoring state; discarded.".into();
            return;
        }
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                self.status = format!("Update impact inspection failed: {error}");
                return;
            }
        };
        // Recheck after the queued first completion, off the UI thread. No report
        // is displayed while saved bindings/lock/source are being revalidated.
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-impact-recheck".into())
            .spawn(move || {
                let result = report.validate_current().map(|()| report);
                let _ = sender.send(Completion::UpdateImpactChecked { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status =
                    "Rechecking saved state and candidate revision before showing impact…".into();
            }
            Err(error) => self.status = format!("Cannot recheck update impact: {error}"),
        }
    }

    pub(super) fn finish_update_impact(
        &mut self,
        identity: Identity,
        result: Result<UpdateImpactReport, String>,
    ) {
        self.update_impact.ready = None;
        if self.impact_identity().as_ref() != Some(&identity) {
            self.status = "Update impact completed for a prior project, source or authoring state; discarded.".into();
            return;
        }
        match result {
            Ok(report) => {
                self.status = "Update impact observed without changing packages. This report never authorizes an update or rebinding.".into();
                self.update_impact.ready = Some((identity, report));
            }
            Err(error) => {
                self.status = format!("Update impact recheck failed; report discarded: {error}")
            }
        }
    }
}

#[cfg(test)]
mod tests;
