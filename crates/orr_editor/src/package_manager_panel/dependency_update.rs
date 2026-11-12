//! Separate opaque dependency Update plan, sharing only candidate inputs and busy lane.
use super::{lock_table, package_manager, parse_source_path, Completion, Lock, Panel};
use std::{path::PathBuf, sync::mpsc};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    root: PathBuf,
    source: String,
    project_generation: u64,
    source_generation: u64,
    candidates: String,
    candidates_generation: u64,
    generation: u64,
}
#[derive(Default)]
pub(super) struct State {
    generation: u64,
    ready: Option<(Identity, package_manager::DependencyUpdatePlan)>,
}
impl State {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.ready = None;
    }
}
impl Panel {
    fn dependency_update_identity(&self) -> Option<Identity> {
        let (candidates, candidates_generation) = self.dependency_add.candidate_identity();
        Some(Identity {
            root: self.root.clone()?,
            source: self.source.trim().into(),
            project_generation: self.project_generation,
            source_generation: self.source_generation,
            candidates: candidates.into(),
            candidates_generation,
            generation: self.dependency_update.generation,
        })
    }
    pub(super) fn clear_stale_dependency_update(&mut self) {
        self.dependency_add.observe();
        if self
            .dependency_update
            .ready
            .as_ref()
            .is_some_and(|(identity, _)| {
                self.dependency_update_identity().as_ref() != Some(identity)
            })
        {
            self.dependency_update.invalidate();
        }
    }
    pub(super) fn show_dependency_update(&mut self, ui: &mut egui::Ui, can_operate: bool) {
        self.clear_stale_dependency_update();
        let valid = super::dependency_add::candidates(self.dependency_add.candidate_identity().0)
            .is_ok()
            && parse_source_path(&self.source).is_ok();
        if ui
            .add_enabled(
                can_operate && !self.busy() && valid,
                egui::Button::new("Preview package update with dependencies"),
            )
            .clicked()
        {
            self.start_dependencies_update_preview();
        }
        let current = self
            .dependency_update
            .ready
            .as_ref()
            .is_some_and(|(id, _)| self.dependency_update_identity().as_ref() == Some(id));
        if ui
            .add_enabled(
                can_operate && !self.busy() && current,
                egui::Button::new("Update package with dependencies"),
            )
            .clicked()
        {
            self.start_dependencies_update();
        }
        if let Some((identity, plan)) = &self.dependency_update.ready {
            if self.dependency_update_identity().as_ref() == Some(identity) {
                ui.label(format!(
                    "Dependency update preview only: {}. No installed revision changed.",
                    plan.name()
                ));
                super::update_preview_details(ui, plan.preview());
                lock_table(ui, "dependency-update-candidate", &plan.preview().candidate);
                for name in plan.unused() {
                    ui.label(format!("Unused update candidate: {name}"));
                }
            }
        }
    }
    pub(super) fn start_dependencies_update_preview(&mut self) {
        if self.busy() {
            return;
        }
        self.select_source();
        self.dependency_update.invalidate();
        self.usage.invalidate();
        self.update_impact.invalidate();
        self.dependency_add.observe();
        self.dependency_add.invalidate();
        let source = match parse_source_path(&self.source) {
            Ok(x) => x,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let candidates =
            match super::dependency_add::candidates(self.dependency_add.candidate_identity().0) {
                Ok(x) => x,
                Err(e) => {
                    self.status = e;
                    return;
                }
            };
        let Some(identity) = self.dependency_update_identity() else {
            return;
        };
        self.preview = None;
        self.update_preview = None;
        self.model_request = None;
        self.preview_request = None;
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-dependency-update-preview".into())
            .spawn(move || {
                let result = package_manager::plan_update_with_dependencies(
                    &identity.root,
                    &source,
                    &candidates,
                );
                let _ = tx.send(Completion::DependenciesUpdatePreviewed { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.status = "Resolving package dependencies without installing…".into();
            }
            Err(e) => self.status = format!("Cannot start dependency preview: {e}"),
        }
    }
    pub(super) fn finish_dependencies_update_preview(
        &mut self,
        identity: Identity,
        result: Result<package_manager::DependencyUpdatePlan, String>,
    ) {
        self.dependency_add.observe();
        self.dependency_update.ready = None;
        if self.dependency_update_identity().as_ref() != Some(&identity) {
            self.status="Dependency update preview discarded: project, source, candidates, or authoring context changed.".into();
            return;
        }
        match result {
            Ok(plan) if super::same_project_root(plan.root(), &identity.root) => {
                self.dependency_update.ready = Some((identity, plan));
                self.status="Dependency update preview ready; review direct, indirect, and unused entries, then Update explicitly.".into();
            }
            Ok(_) => self.status = "Dependency update preview root changed; discarded.".into(),
            Err(e) => self.status = format!("Dependency update preview failed: {e}"),
        }
    }
    pub(super) fn start_dependencies_update(&mut self) {
        if self.busy() {
            return;
        }
        self.select_source();
        self.dependency_add.observe();
        let Some((identity, plan)) = self.dependency_update.ready.take() else {
            return;
        };
        if self.dependency_update_identity().as_ref() != Some(&identity) {
            self.status = "Dependency inputs changed; preview again.".into();
            return;
        }
        // Dispatch cannot be cancelled by later field/project changes. Drop all
        // stale mutation tickets before publication and never auto-retry.
        self.invalidate_package_cache();
        self.restoration_generation = self.restoration_generation.saturating_add(1);
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-dependency-update".into())
            .spawn(move || {
                let result = package_manager::apply_update_with_dependencies(&plan);
                let _ = tx.send(Completion::DependenciesUpdated { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.updating_dependencies = true;
                self.status =
                    "Updating the explicitly previewed package dependency closure…".into();
            }
            Err(e) => self.status = format!("Cannot start dependency Update; preview again: {e}"),
        }
    }
    pub(super) fn finish_dependencies_update(
        &mut self,
        identity: Identity,
        result: Result<Lock, String>,
    ) {
        // Dispatch invalidates the preview generation itself, so applied result
        // attribution uses project/source only; no result becomes another plan.
        let current = self.root.as_ref() == Some(&identity.root)
            && self.project_generation == identity.project_generation
            && self.source.trim() == identity.source
            && self.source_generation == identity.source_generation;
        self.invalidate_package_cache();
        let target = if current {
            "current project".to_owned()
        } else {
            format!("prior project {}", identity.root.display())
        };
        self.status=match result {
            Ok(_) => format!("Package dependencies updated to {target}. Verify installed packages before using models; bindings still require explicit Save."),
            Err(e) => format!("Dependency Update for {target} failed or could not be confirmed: {e}. Verify installed packages; no automatic retry or rollback was performed."),
        };
    }
}
#[cfg(test)]
#[path = "dependency_update/tests.rs"]
mod tests;
