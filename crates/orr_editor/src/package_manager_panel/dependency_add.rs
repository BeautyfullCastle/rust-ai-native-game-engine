//! Explicit dependency closure preview and installation, on the shared worker lane.
use super::{lock_table, package_manager, parse_source_path, Completion, Lock, Panel};
use std::{path::PathBuf, sync::mpsc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    root: PathBuf,
    source: String,
    project_generation: u64,
    source_generation: u64,
    candidates: String,
    generation: u64,
}
#[derive(Default)]
pub(super) struct State {
    candidates: String,
    observed: String,
    generation: u64,
    ready: Option<(Identity, package_manager::DependencyAddPlan)>,
}
impl State {
    pub(super) fn candidate_identity(&self) -> (&str, u64) {
        (&self.candidates, self.generation)
    }
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.ready = None;
    }
    pub(super) fn observe(&mut self) {
        if self.observed != self.candidates {
            self.invalidate();
            self.observed.clone_from(&self.candidates);
        }
    }
}
pub(super) fn candidates(text: &str) -> Result<Vec<PathBuf>, String> {
    if text.len() > 32768 {
        return Err("Dependency directory text exceeds 32768 bytes.".into());
    }
    let mut paths = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if paths.len() == 8 {
            return Err("Supply 1..8 dependency candidate directories, one per line.".into());
        }
        if line.len() > 4096 {
            return Err("A dependency directory exceeds 4096 bytes.".into());
        }
        paths.push(parse_source_path(line)?);
    }
    if paths.is_empty() {
        return Err("Supply 1..8 dependency candidate directories, one per line.".into());
    }
    Ok(paths)
}
impl Panel {
    fn dependency_identity(&self) -> Option<Identity> {
        Some(Identity {
            root: self.root.clone()?,
            source: self.source.trim().into(),
            project_generation: self.project_generation,
            source_generation: self.source_generation,
            candidates: self.dependency_add.candidates.clone(),
            generation: self.dependency_add.generation,
        })
    }
    pub(super) fn show_dependency_add(
        &mut self,
        ui: &mut egui::Ui,
        can_operate: bool,
        was_mutating: bool,
    ) {
        ui.separator();
        let label = ui.label("Dependency candidate directories (one per line)");
        ui.add_enabled(
            !was_mutating && !self.removal_busy(),
            egui::TextEdit::multiline(&mut self.dependency_add.candidates)
                .desired_rows(3)
                .char_limit(32768),
        )
        .labelled_by(label.id);
        self.dependency_add.observe();
        let valid = candidates(&self.dependency_add.candidates).is_ok()
            && parse_source_path(&self.source).is_ok();
        if ui
            .add_enabled(
                can_operate && !self.busy() && valid,
                egui::Button::new("Preview package with dependencies"),
            )
            .clicked()
        {
            self.start_dependencies_preview();
        }
        let current = self
            .dependency_add
            .ready
            .as_ref()
            .is_some_and(|(id, _)| self.dependency_identity().as_ref() == Some(id));
        if ui
            .add_enabled(
                can_operate && !self.busy() && current,
                egui::Button::new("Add package with dependencies"),
            )
            .clicked()
        {
            self.start_dependencies_add();
        }
        if let Some((identity, plan)) = &self.dependency_add.ready {
            if self.dependency_identity().as_ref() == Some(identity) {
                ui.label("Dependency preview only; no packages installed. Only the primary is selected directly; transitive dependencies are resolved from the supplied candidates and retained cache.");
                lock_table(ui, "dependency-add-preview", &plan.preview().candidate);
                for name in plan.preview().candidate.packages.keys() {
                    let relation = if plan.preview().candidate.direct.contains_key(name) {
                        "Direct"
                    } else {
                        "Indirect"
                    };
                    ui.label(format!("{relation} package: {name}"));
                }
                for name in plan.unused() {
                    ui.label(format!("Unused candidate: {name}"));
                }
            }
        }
    }
    pub(super) fn start_dependencies_preview(&mut self) {
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
        let candidates = match candidates(&self.dependency_add.candidates) {
            Ok(x) => x,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let Some(identity) = self.dependency_identity() else {
            return;
        };
        self.preview = None;
        self.update_preview = None;
        self.model_request = None;
        self.preview_request = None;
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-dependency-preview".into())
            .spawn(move || {
                let result = package_manager::plan_add_with_dependencies(
                    &identity.root,
                    &source,
                    &candidates,
                );
                let _ = tx.send(Completion::DependenciesPreviewed { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.status = "Resolving package dependencies without installing…".into();
            }
            Err(e) => self.status = format!("Cannot start dependency preview: {e}"),
        }
    }
    pub(super) fn finish_dependencies_preview(
        &mut self,
        identity: Identity,
        result: Result<package_manager::DependencyAddPlan, String>,
    ) {
        self.dependency_add.observe();
        self.dependency_add.ready = None;
        if self.dependency_identity().as_ref() != Some(&identity) {
            self.status="Dependency preview discarded: project, source, candidates, or authoring context changed.".into();
            return;
        }
        match result {
            Ok(plan) if plan.root() == identity.root => {
                self.dependency_add.ready = Some((identity, plan));
                self.status="Dependency preview ready; review direct, indirect, and unused entries, then Add explicitly.".into();
            }
            Ok(_) => self.status = "Dependency preview root changed; discarded.".into(),
            Err(e) => self.status = format!("Dependency preview failed: {e}"),
        }
    }
    pub(super) fn start_dependencies_add(&mut self) {
        if self.busy() {
            return;
        }
        self.select_source();
        self.dependency_add.observe();
        let Some((identity, plan)) = self.dependency_add.ready.take() else {
            return;
        };
        if self.dependency_identity().as_ref() != Some(&identity) {
            self.status = "Dependency inputs changed; preview again.".into();
            return;
        }
        // Dispatch cannot be cancelled by later field/project changes. Drop all
        // stale mutation tickets before publication and never auto-retry.
        self.invalidate_package_cache();
        self.restoration_generation = self.restoration_generation.saturating_add(1);
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-dependency-add".into())
            .spawn(move || {
                let result = package_manager::apply_add_with_dependencies(&plan);
                let _ = tx.send(Completion::DependenciesAdded { identity, result });
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.installing_dependencies = true;
                self.status =
                    "Installing the explicitly previewed package dependency closure…".into();
            }
            Err(e) => self.status = format!("Cannot start dependency Add; preview again: {e}"),
        }
    }
    pub(super) fn finish_dependencies_add(
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
            Ok(_) => format!("Package dependencies added to {target}. Verify installed packages before using models; bindings still require explicit Save."),
            Err(e) => format!("Dependency Add for {target} failed or could not be confirmed: {e}. Verify installed packages; no automatic retry or rollback was performed."),
        };
    }
}
#[cfg(test)]
#[path = "dependency_add/tests.rs"]
mod tests;
