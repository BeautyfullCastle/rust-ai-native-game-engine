use crate::package_manager::{self, Plan, RemovalPlan, RemovalReceipt};
use egui::Context;
use orr_model_bindings::model_bindings::ModelKind;
use orr_package::{Lock, PackagePreview};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

enum Completion {
    RestorationPreviewed {
        generation: u64,
        root: PathBuf,
        project_generation: u64,
        receipt: RemovalReceipt,
        result: Result<PackagePreview, String>,
    },
    Restored {
        root: PathBuf,
        project_generation: u64,
        result: Result<Lock, String>,
    },
    RemovalPreviewed {
        root: PathBuf,
        project_generation: u64,
        name: String,
        result: Result<RemovalPlan, String>,
    },
    Removed {
        receipt: Option<RemovalReceipt>,
        root: PathBuf,
        project_generation: u64,
        result: Result<Lock, String>,
    },
    Verified {
        root: PathBuf,
        project_generation: u64,
        result: Result<Lock, String>,
    },
    Previewed {
        root: PathBuf,
        source_identity: String,
        project_generation: u64,
        source_generation: u64,
        result: Result<Plan, String>,
    },
    Added {
        root: PathBuf,
        source_identity: String,
        project_generation: u64,
        source_generation: u64,
        result: Result<Lock, String>,
    },
}

struct PreviewedPlan {
    plan: Plan,
    source_identity: String,
    project_generation: u64,
    source_generation: u64,
}

struct ModelRequest {
    root: PathBuf,
    source_identity: String,
    project_generation: u64,
    source_generation: u64,
    package: String,
    asset: String,
    kind: ModelKind,
}

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub source: String,
    pub status: String,
    root: Option<PathBuf>,
    source_identity: String,
    project_generation: u64,
    source_generation: u64,
    model_filter: String,
    verified: Option<Lock>,
    preview: Option<PreviewedPlan>,
    removal_name: String,
    removal_preview: Option<RemovalPlan>,
    last_removal: Option<RemovalReceipt>,
    restoration_preview: Option<PackagePreview>,
    restoration_generation: u64,
    model_request: Option<ModelRequest>,
    static_preview_request: Option<crate::package_model_preview::PreviewRequest>,
    pending: Option<Receiver<Completion>>,
    removing: bool,
    model_import: crate::model_package_import_panel::Panel,
}

impl Panel {
    pub(crate) fn removal_busy(&self) -> bool {
        self.removing && self.pending.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.model_import.busy()
    }

    pub fn take_model_request(&mut self) -> Option<(String, String)> {
        let request = self.take_current_request()?;
        (request.kind == ModelKind::Static).then_some((request.package, request.asset))
    }

    pub(crate) fn take_model_asset_request(&mut self) -> Option<(String, String, ModelKind)> {
        let request = self.take_current_request()?;
        Some((request.package, request.asset, request.kind))
    }

    pub(crate) fn take_static_preview_request(
        &mut self,
    ) -> Option<crate::package_model_preview::PreviewRequest> {
        let request = self.static_preview_request.take()?;
        let package = self.verified.as_ref()?.packages.get(&request.package_id)?;
        (self.root.as_ref() == Some(&request.project_root)
            && self.project_generation == request.generation
            && package.digest == request.digest
            && package.manifest.files.contains(&request.asset_path)
            && !self.busy())
        .then_some(request)
    }

    pub(crate) fn static_preview_is_current(
        &self,
        request: &crate::package_model_preview::PreviewRequest,
    ) -> bool {
        self.root.as_ref() == Some(&request.project_root)
            && self.project_generation == request.generation
            && !self.busy()
            && self
                .verified
                .as_ref()
                .and_then(|lock| lock.packages.get(&request.package_id))
                .is_some_and(|package| {
                    package.digest == request.digest
                        && package.manifest.files.contains(&request.asset_path)
                })
    }

    fn take_current_request(&mut self) -> Option<ModelRequest> {
        let request = self.model_request.take()?;
        if self.root.as_ref() == Some(&request.root)
            && self.source.trim() == request.source_identity.as_str()
            && self.project_generation == request.project_generation
            && self.source_generation == request.source_generation
        {
            Some(request)
        } else {
            None
        }
    }

    pub fn show(
        &mut self,
        context: &Context,
        project: Option<&Path>,
        blocked_reason: Option<&str>,
    ) {
        self.select_project(project);
        self.select_source();
        let was_mutating = self.removal_busy();
        self.poll();
        if blocked_reason.is_some() {
            self.removal_preview = None;
            self.restoration_preview = None;
            if !was_mutating {
                self.restoration_generation = self.restoration_generation.saturating_add(1);
                self.last_removal = None;
            }
        }
        let import_was_busy = self.model_import.busy();
        let import_report = self.model_import.poll(
            self.root.as_deref(),
            self.project_generation,
            self.source_generation,
        );
        if import_was_busy && !self.model_import.busy() {
            self.status = self.model_import.status().to_owned();
        }
        if let Some(report) = import_report {
            self.source = report.root.display().to_string();
            self.select_source();
            self.preview = None;
            self.model_request = None;
            self.status = format!(
                "Imported raw package {}/{}; not installed or assigned. Preview and add it explicitly. Source SHA-256: {}; license SHA-256: {}.",
                report.package, report.asset, report.source_sha256, report.license_sha256
            );
        }
        if !self.open {
            return;
        }
        let can_operate = self.root.is_some() && blocked_reason.is_none() && !self.busy();
        let mut open = self.open;
        egui::Window::new("Content packages")
            .open(&mut open)
            .default_width(700.0)
            .default_pos(egui::pos2(250.0, 350.0))
            .vscroll(true)
            .show(context, |ui| {
                ui.label("Manage validated local content packages for the current Room project.");
                if let Some(root) = &self.root {
                    ui.label(format!("Current Room project: {}", root.display()));
                } else {
                    ui.label("Open a saved Room project to manage content packages.");
                }
                if let Some(reason) = blocked_reason {
                    ui.label(reason);
                }
                ui.label("Package metadata is not a byte-integrity check. A license file does not establish legal trust.");
                ui.label("Preview does not activate content or reserve a permanent lock; Add revalidates before writing.");
                let field_label = ui.label("Local content package directory");
                ui.add(
                    egui::TextEdit::singleline(&mut self.source)
                        .desired_width(f32::INFINITY)
                        .char_limit(4096),
                )
                .labelled_by(field_label.id);
                let source = parse_source_path(&self.source);
                self.select_source();
                self.clear_stale_preview();
                let matching_preview = self.matching_preview(source.is_ok());
                let source_error = source.as_ref().err();
                if !self.source.trim().is_empty() {
                    if let Some(error) = source_error {
                        ui.label(error);
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can_operate,
                            egui::Button::new("Verify installed packages"),
                        )
                        .clicked()
                    {
                        self.start_verify();
                    }
                    if ui
                        .add_enabled(
                            can_operate && source.is_ok(),
                            egui::Button::new("Preview package addition"),
                        )
                        .clicked()
                    {
                        if let Ok(source) = &source {
                            self.start_preview(source.clone());
                        }
                    }
                    if ui
                        .add_enabled(
                            can_operate && matching_preview,
                            egui::Button::new("Add previewed package"),
                        )
                        .clicked()
                    {
                        self.start_add();
                    }
                });
                self.model_import.show(
                    ui,
                    can_operate,
                    self.root.as_deref(),
                    self.project_generation,
                    self.source_generation,
                );
                if !self.model_import.status().is_empty() {
                    ui.label(self.model_import.status());
                }
                if self.busy() {
                    ui.spinner();
                    context.request_repaint_after(std::time::Duration::from_millis(40));
                }
                if let Some(lock) = &self.verified {
                    ui.separator();
                    ui.label("Installed package bytes verified from this project's lock:");
                    lock_table(ui, "verified-installed-packages", lock);
                    ui.label("Preview a static model without selecting an entity. Use model assigns to the selected entity; binding save is separate.");
                    ui.separator();
                    ui.label("Remove a direct package selection (cached bytes and source files are retained):");
                    for name in lock.direct.keys() {
                        if ui.add_enabled(can_operate, egui::Button::new(format!("Select removal {name}"))).clicked() {
                            self.removal_name = name.clone();
                            self.removal_preview = None;
                        }
                    }
                    model_asset_buttons(
                        ui,
                        ModelBrowser { lock, filter: &mut self.model_filter, preview_request: &mut self.static_preview_request },
                        can_operate,
                        &mut self.model_request,
                        self.root.as_ref(),
                        self.source.trim(),
                        (self.project_generation, self.source_generation),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label(format!("Removal selection: {}", self.removal_name));
                    if ui.add_enabled(can_operate && self.verified.as_ref().is_some_and(|lock| lock.direct.contains_key(&self.removal_name)), egui::Button::new("Preview package removal")).clicked() { self.start_removal_preview(); }
                    if ui.add_enabled(can_operate && self.removal_preview.is_some(), egui::Button::new("Remove previewed package")).clicked() { self.start_remove(); }
                });
                if let Some(plan) = &self.removal_preview {
                    ui.label("Removal preview only; nothing has been deactivated. Remove revalidates saved Room references and unchanged files.");
                    for (name, change) in &plan.preview.changes {
                        if change.after.is_none() { ui.label(format!("Deactivate package: {name}")); }
                        else if change.before.as_ref().is_some_and(|before| before.direct) && change.after.as_ref().is_some_and(|after| !after.direct) { ui.label(format!("Remains active as a dependency: {name}")); }
                    }
                    for name in plan.preview.candidate.packages.keys().filter(|name| !plan.preview.candidate.direct.contains_key(*name)) {
                        ui.label(format!("Retained dependency: {name}"));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_operate && self.last_removal.is_some(), egui::Button::new("Preview restoration")).clicked() { self.start_restoration_preview(); }
                    if ui.add_enabled(can_operate && self.last_removal.is_some() && self.restoration_preview.is_some(), egui::Button::new("Restore last removal")).clicked() { self.start_restore(); }
                });
                if let Some(preview) = &self.restoration_preview {
                    ui.label("Restoration preview only: restore the exact prior lock from retained cache, including its direct selections and dependencies.");
                    for (name, change) in &preview.changes {
                        if change.before.is_none() && change.after.is_some() { ui.label(format!("Reactivate cached package: {name}")); }
                        if change.after.as_ref().is_some_and(|after| after.direct) && change.before.as_ref().is_none_or(|before| !before.direct) { ui.label(format!("Restore direct selection: {name}")); }
                    }
                }
                if let Some(plan) = &self.preview {
                    if matching_preview {
                        ui.separator();
                        ui.label("Addition preview only; no package was added or activated.");
                        preview_table(ui, &plan.plan.preview);
                    }
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        self.open = open;
    }

    fn select_project(&mut self, project: Option<&Path>) {
        let next_root = project.map(Path::to_path_buf);
        if self.root != next_root {
            self.root = next_root;
            self.project_generation = self.project_generation.saturating_add(1);
            self.last_removal = None;
            self.restoration_preview = None;
            self.model_filter.clear();
            self.static_preview_request = None;
            self.removal_name.clear();
            self.removal_preview = None;
            self.verified = None;
            self.preview = None;
            self.model_request = None;
            if self.pending.is_none() {
                self.status =
                    "Project changed; prior package inspection and preview were cleared.".into();
            }
        }
    }

    fn select_source(&mut self) {
        let next_source = self.source.trim().to_owned();
        if self.source_identity != next_source {
            self.removal_preview = None;
            self.source_identity = next_source;
            self.source_generation = self.source_generation.saturating_add(1);
            self.model_request = None;
        }
    }

    fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.removing = false;
                self.status = "Package worker disconnected before returning a result.".into();
                return;
            }
        };
        self.pending = None;
        self.removing = false;
        match completion {
            Completion::RestorationPreviewed {
                generation,
                root,
                project_generation,
                receipt,
                result,
            } => {
                self.restoration_preview = None;
                self.last_removal = None;
                if self.root.as_ref() == Some(&root)
                    && self.project_generation == project_generation
                    && self.restoration_generation == generation
                {
                    match result {
                        Ok(preview) if receipt.root() == root => {
                            self.last_removal = Some(receipt);
                            self.restoration_preview = Some(preview);
                            self.status = "Restoration preview ready. Nothing changed; explicitly Restore last removal to reactivate the exact cached selection.".into();
                        }
                        Ok(_) => {
                            self.status = "Restoration identity changed; receipt discarded.".into()
                        }
                        Err(error) => {
                            self.status =
                                format!("Restoration preview failed; receipt discarded: {error}")
                        }
                    }
                } else {
                    self.status =
                        "Restoration preview belongs to a prior project; receipt discarded.".into();
                }
            }
            Completion::Restored {
                root,
                project_generation,
                result,
            } => {
                self.invalidate_package_cache();
                if self.root.as_ref() == Some(&root)
                    && self.project_generation == project_generation
                {
                    match result {
                        Ok(lock) => {
                            self.verified = Some(lock);
                            self.status = "Package restoration complete. Exact prior selections restored from cache; source files were not needed.".into();
                        }
                        Err(error) => {
                            self.status =
                                format!("Package restoration failed; receipt discarded: {error}")
                        }
                    }
                } else {
                    self.status = match result {
                    Ok(_) => format!("Package restoration completed for prior project {}; current inventory was not reused.",root.display()),
                    Err(error) => format!("Package restoration for prior project {} failed: {error}",root.display()),
                };
                }
            }

            Completion::RemovalPreviewed {
                root,
                project_generation,
                name,
                result,
            } => {
                if self.root.as_ref() != Some(&root)
                    || self.project_generation != project_generation
                    || self.removal_name != name
                {
                    self.status = "Removal preview completed for a prior project or selection; it was discarded.".into();
                } else {
                    match result {
                        Ok(plan) if plan.root == root && plan.name == name => {
                            self.removal_preview = Some(plan);
                            self.status = "Removal preview ready. Review deactivated and retained packages before explicit removal.".into();
                        }
                        Ok(_) => {
                            self.removal_preview = None;
                            self.status = "Removal preview identity changed; discarded.".into();
                        }
                        Err(error) => {
                            self.removal_preview = None;
                            self.status = format!("Package removal preview failed: {error}");
                        }
                    }
                }
            }
            Completion::Removed {
                receipt,
                root,
                project_generation,
                result,
            } => {
                self.invalidate_package_cache();
                if self.root.as_ref() == Some(&root)
                    && self.project_generation == project_generation
                {
                    match result {
                        Ok(lock) => {
                            self.verified = Some(lock);
                            self.last_removal = receipt;
                            self.removal_name.clear();
                            self.status = "Package removal complete. Verified inventory refreshed; cached bytes and source files retained.".into();
                        }
                        Err(error) => self.status = format!("Package removal failed: {error}"),
                    }
                } else {
                    self.status = match result {
                    Ok(_) => format!("Package removal completed for prior project {}; current state was not reused. Cached bytes and sources remain.", root.display()),
                    Err(error) => format!("Package removal for prior project {} failed: {error}", root.display()),
                };
                }
            }

            Completion::Verified {
                root,
                project_generation,
                result,
            } if self.root.as_ref() == Some(&root)
                && self.project_generation == project_generation =>
            {
                match result {
                    Ok(lock) => {
                        self.removal_preview = None;
                        self.verified = Some(lock);
                        self.status =
                            "Installed package bytes match the project's validated lock.".into();
                    }
                    Err(error) => {
                        self.verified = None;
                        self.status = format!("Installed package verification failed: {error}");
                    }
                }
            }
            Completion::Previewed {
                root,
                source_identity,
                project_generation,
                source_generation,
                result,
            } if self.root.as_ref() == Some(&root)
                && self.source.trim() == source_identity.as_str()
                && self.project_generation == project_generation
                && self.source_generation == source_generation =>
            {
                match result {
                    Ok(plan) if plan.root == root => {
                        self.preview = Some(PreviewedPlan {
                            plan,
                            source_identity,
                            project_generation,
                            source_generation,
                        });
                        self.status =
                            "Preview ready. No package was added; review the change table.".into();
                    }
                    Ok(_) => {
                        self.preview = None;
                        self.status = "Package preview identity changed; preview discarded.".into();
                    }
                    Err(error) => {
                        self.preview = None;
                        self.status = format!("Package preview failed: {error}");
                    }
                }
            }
            Completion::Added {
                root,
                source_identity,
                project_generation,
                source_generation,
                result,
            } if self.root.as_ref() == Some(&root)
                && self.source.trim() == source_identity.as_str()
                && self.project_generation == project_generation
                && self.source_generation == source_generation =>
            {
                self.invalidate_package_cache();
                match result {
                    Ok(_) => {
                        self.status = "Package added. Verify installed packages to recheck bytes, then use a model and save model bindings.".into();
                    }
                    Err(error) => {
                        self.status = format!("Adding package failed: {error}");
                    }
                }
            }
            Completion::Verified { root, result, .. } => {
                self.status = match result {
                    Ok(_) => format!("Installed package verification completed for prior project {}; current project state was not reused.", root.display()),
                    Err(error) => format!("Verification of prior project {} failed: {error}", root.display()),
                };
            }
            Completion::Previewed {
                root,
                source_identity,
                result,
                ..
            } => {
                self.status = match result {
                    Ok(_) => format!("Preview completed for prior project {} and source {}; no package was added and the preview was not reused.", root.display(), source_identity),
                    Err(error) => format!("Preview for prior project {} and source {} failed: {error}", root.display(), source_identity),
                };
            }
            Completion::Added {
                root,
                source_identity,
                result,
                ..
            } => {
                self.invalidate_package_cache();
                self.status = match result {
                    Ok(_) => format!("Package was added to prior project {} from {}; current project state was not reused.", root.display(), source_identity),
                    Err(error) => format!("Adding package to prior project {} from {} failed: {error}", root.display(), source_identity),
                };
            }
        }
    }

    fn invalidate_package_cache(&mut self) {
        self.static_preview_request = None;
        self.last_removal = None;
        self.restoration_preview = None;
        self.removal_preview = None;
        self.verified = None;
        self.preview = None;
        self.model_request = None;
    }

    fn clear_stale_preview(&mut self) {
        let stale = self.preview.as_ref().is_some_and(|plan| {
            self.root.as_ref() != Some(&plan.plan.root)
                || self.source.trim() != plan.source_identity.as_str()
                || self.project_generation != plan.project_generation
                || self.source_generation != plan.source_generation
        });
        if stale {
            self.preview = None;
            self.status = "Package source changed; the old preview was cleared.".into();
        }
    }

    fn matching_preview(&self, source_is_valid: bool) -> bool {
        source_is_valid
            && self.preview.as_ref().is_some_and(|preview| {
                self.root.as_ref() == Some(&preview.plan.root)
                    && self.source.trim() == preview.source_identity.as_str()
                    && self.project_generation == preview.project_generation
                    && self.source_generation == preview.source_generation
            })
    }

    fn start_removal_preview(&mut self) {
        if self.busy() {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        if !self
            .verified
            .as_ref()
            .is_some_and(|lock| lock.direct.contains_key(&self.removal_name))
        {
            return;
        }
        let name = self.removal_name.clone();
        let project_generation = self.project_generation;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-remove-preview".into())
            .spawn(move || {
                let result = package_manager::plan_remove(&root, &name);
                let _ = sender.send(Completion::RemovalPreviewed {
                    root,
                    project_generation,
                    name,
                    result,
                });
            }) {
            Ok(_) => {
                self.removal_preview = None;
                self.pending = Some(receiver);
                self.status =
                    "Checking saved Room references and previewing package removal…".into();
            }
            Err(error) => self.status = format!("Cannot start removal preview: {error}"),
        }
    }

    fn start_remove(&mut self) {
        if self.busy() {
            return;
        }
        let Some(plan) = self.removal_preview.take() else {
            return;
        };
        if self.root.as_ref() != Some(&plan.root) || self.removal_name != plan.name {
            self.status = "Project or selection changed; preview removal again.".into();
            return;
        }
        self.model_request = None;
        self.last_removal = None;
        self.restoration_preview = None;
        let project_generation = self.project_generation;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-remove".into())
            .spawn(move || {
                let (result, receipt) = match package_manager::apply_remove_with_receipt(&plan) {
                    Ok((lock, receipt)) => (Ok(lock), Some(receipt)),
                    Err(error) => (Err(error), None),
                };
                let root = plan.root;
                let _ = sender.send(Completion::Removed {
                    receipt,
                    root,
                    project_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.removing = true;
                self.status =
                    "Removing the previewed direct selection with locked revalidation…".into();
            }
            Err(error) => self.status = format!("Cannot start package removal: {error}"),
        }
    }

    fn start_restoration_preview(&mut self) {
        if self.busy() {
            return;
        }
        let Some(receipt) = self.last_removal.take() else {
            return;
        };
        if self.root.as_deref() != Some(receipt.root()) {
            self.restoration_preview = None;
            return;
        }
        let root = receipt.root().to_path_buf();
        let project_generation = self.project_generation;
        let generation = self.restoration_generation;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-restore-preview".into())
            .spawn(move || {
                let result = package_manager::plan_restore(&receipt);
                let _ = sender.send(Completion::RestorationPreviewed {
                    generation,
                    root,
                    project_generation,
                    receipt,
                    result,
                });
            }) {
            Ok(_) => {
                self.restoration_preview = None;
                self.pending = Some(receiver);
                self.status = "Checking the successful removal receipt and retained cache…".into();
            }
            Err(error) => self.status = format!("Cannot start restoration preview: {error}"),
        }
    }

    fn start_restore(&mut self) {
        if self.busy() || self.restoration_preview.is_none() {
            return;
        }
        let Some(receipt) = self.last_removal.take() else {
            return;
        };
        self.restoration_preview = None;
        if self.root.as_deref() != Some(receipt.root()) {
            return;
        }
        self.model_request = None;
        self.removal_preview = None;
        let root = receipt.root().to_path_buf();
        let project_generation = self.project_generation;
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-package-restore".into())
            .spawn(move || {
                let result = package_manager::apply_restore(receipt);
                let _ = sender.send(Completion::Restored {
                    root,
                    project_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.removing = true;
                self.status =
                    "Restoring the exact prior package selection with locked revalidation…".into();
            }
            Err(error) => self.status = format!("Cannot start package restoration: {error}"),
        }
    }

    fn start_verify(&mut self) {
        self.removal_preview = None;
        if self.busy() {
            return;
        }
        self.last_removal = None;
        self.restoration_preview = None;
        self.restoration_generation = self.restoration_generation.saturating_add(1);
        let Some(root) = self.root.clone() else {
            self.status = "Open a Room project before verifying packages.".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let worker_root = root.clone();
        let project_generation = self.project_generation;
        match std::thread::Builder::new()
            .name("orr-package-verify".into())
            .spawn(move || {
                let result = package_manager::inspect(&worker_root);
                let _ = sender.send(Completion::Verified {
                    root,
                    project_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Verifying installed package bytes against the active lock…".into();
            }
            Err(error) => self.status = format!("Cannot start package verification: {error}"),
        }
    }

    fn start_preview(&mut self, source: PathBuf) {
        if self.busy() {
            return;
        }
        let Some(root) = self.root.clone() else {
            self.status = "Open a Room project before previewing a package.".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let worker_root = root.clone();
        let worker_source = source.clone();
        let source_identity = self.source.trim().to_owned();
        let project_generation = self.project_generation;
        let source_generation = self.source_generation;
        match std::thread::Builder::new()
            .name("orr-package-preview".into())
            .spawn(move || {
                let result = package_manager::plan_add(&worker_root, &worker_source);
                let _ = sender.send(Completion::Previewed {
                    root,
                    source_identity,
                    project_generation,
                    source_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.preview = None;
                self.pending = Some(receiver);
                self.status = "Inspecting package bytes and building an addition preview…".into();
            }
            Err(error) => self.status = format!("Cannot start package preview: {error}"),
        }
    }

    fn start_add(&mut self) {
        self.last_removal = None;
        self.restoration_preview = None;
        self.removal_preview = None;
        if self.busy() {
            return;
        }
        if let Err(error) = parse_source_path(&self.source) {
            self.status = error;
            return;
        }
        let Some(root) = self.root.clone() else {
            self.status = "Open a Room project before adding a package.".into();
            return;
        };
        let Some(plan) = self.preview.take() else {
            self.status = "Preview the package addition before adding it.".into();
            return;
        };
        let source_identity = self.source.trim().to_owned();
        if plan.plan.root != root
            || plan.source_identity != source_identity
            || plan.project_generation != self.project_generation
            || plan.source_generation != self.source_generation
        {
            self.preview = None;
            self.status = "Project or package source changed; preview the addition again.".into();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let completion_root = root.clone();
        let completion_source = source_identity;
        let project_generation = self.project_generation;
        let source_generation = self.source_generation;
        let plan = plan.plan;
        match std::thread::Builder::new()
            .name("orr-package-add".into())
            .spawn(move || {
                let result = package_manager::apply_add(&plan);
                let _ = sender.send(Completion::Added {
                    root: completion_root,
                    source_identity: completion_source,
                    project_generation,
                    source_generation,
                    result,
                });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Adding the previewed package with backend revalidation…".into();
            }
            Err(error) => self.status = format!("Cannot start package addition: {error}"),
        }
    }
}

fn parse_source_path(source: &str) -> Result<PathBuf, String> {
    let text = source.trim();
    if text.is_empty() {
        return Err("Enter a local content package directory.".into());
    }
    let path = PathBuf::from(text);
    if path.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err("Package source path must not exceed 4096 native bytes.".into());
    }
    Ok(path)
}

fn lock_table(ui: &mut egui::Ui, id: &str, lock: &Lock) {
    egui::Grid::new(id).striped(true).show(ui, |ui| {
        for heading in ["Package", "Version", "Digest", "Direct", "Capabilities"] {
            ui.strong(heading);
        }
        ui.end_row();
        for (name, package) in &lock.packages {
            ui.label(name);
            ui.label(&package.manifest.version);
            ui.label(compact_digest(&package.digest))
                .on_hover_text(&package.digest);
            ui.label(if lock.direct.contains_key(name) {
                "yes"
            } else {
                "no"
            });
            ui.label(
                package
                    .manifest
                    .capabilities
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            ui.end_row();
        }
    });
}

struct ModelBrowser<'a> {
    lock: &'a Lock,
    filter: &'a mut String,
    preview_request: &'a mut Option<crate::package_model_preview::PreviewRequest>,
}

fn model_matches_filter(package: &str, asset: &str, filter: &str) -> bool {
    let identity = format!("{package}/{asset}").to_ascii_lowercase();
    filter
        .split_whitespace()
        .all(|term| identity.contains(&term.to_ascii_lowercase()))
}

fn model_asset_buttons(
    ui: &mut egui::Ui,
    browser: ModelBrowser<'_>,
    enabled: bool,
    request: &mut Option<ModelRequest>,
    root: Option<&PathBuf>,
    source: &str,
    generations: (u64, u64),
) {
    let field_label = ui.label("Filter verified models");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(browser.filter).char_limit(256))
            .labelled_by(field_label.id);
        if ui
            .add_enabled(
                !browser.filter.is_empty(),
                egui::Button::new("Clear model filter"),
            )
            .clicked()
        {
            browser.filter.clear();
        }
    });
    ui.label("Search package names and asset paths; all words must match (case-insensitive).");
    let mut total = 0;
    let mut shown = 0;
    for (package_name, package) in &browser.lock.packages {
        for asset in &package.manifest.files {
            let supported = Path::new(asset)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("glb")
                        || extension.eq_ignore_ascii_case("gltf")
                        || extension.eq_ignore_ascii_case("orrmodel")
                });
            if !supported {
                continue;
            }
            total += 1;
            if !model_matches_filter(package_name, asset, browser.filter) {
                continue;
            }
            shown += 1;
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(format!("Preview static model {package_name}/{asset}")),
                )
                .clicked()
            {
                if let Some(root) = root {
                    *browser.preview_request = Some(crate::package_model_preview::PreviewRequest {
                        project_root: root.clone(),
                        package_id: package_name.clone(),
                        asset_path: asset.clone(),
                        digest: package.digest.clone(),
                        generation: generations.0,
                    });
                }
            }
            #[cfg(not(feature = "room-animated-models"))]
            let kinds = [ModelKind::Static];
            #[cfg(feature = "room-animated-models")]
            let kinds = [ModelKind::Static, ModelKind::Animated];
            for kind in kinds {
                if kind == ModelKind::Animated
                    && (!package.manifest.capabilities.contains("animation")
                        || asset.to_ascii_lowercase().ends_with(".orrmodel"))
                {
                    continue;
                }
                let label = match kind {
                    ModelKind::Static => format!("Use model {package_name}/{asset}"),
                    ModelKind::Animated => format!("Use animated model {package_name}/{asset}"),
                };
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    if let Some(root) = root {
                        *request = Some(ModelRequest {
                            root: root.clone(),
                            source_identity: source.to_owned(),
                            project_generation: generations.0,
                            source_generation: generations.1,
                            package: package_name.clone(),
                            asset: asset.clone(),
                            kind,
                        });
                    }
                }
            }
        }
    }
    ui.label(format!("Showing {shown} of {total} verified model assets"));
    if total == 0 {
        ui.label("No verified GLB, glTF or Orrmodel assets are available.");
    } else if shown == 0 {
        ui.label("No verified models match this filter. Clear it to show all models.");
    }
}

fn preview_table(ui: &mut egui::Ui, preview: &PackagePreview) {
    egui::Grid::new("package-addition-preview")
        .striped(true)
        .show(ui, |ui| {
            for heading in [
                "Package",
                "Before",
                "Proposed version",
                "Digest",
                "Direct",
                "Capabilities",
            ] {
                ui.strong(heading);
            }
            ui.end_row();
            for (name, change) in &preview.changes {
                let before = change
                    .before
                    .as_ref()
                    .map_or("—", |selection| selection.version.as_str());
                let after = change.after.as_ref();
                ui.label(name);
                ui.label(before);
                ui.label(after.map_or("(removed)", |selection| selection.version.as_str()));
                let digest = after.map(|selection| selection.digest.as_str());
                let digest_label = digest.map_or_else(|| "—".to_owned(), compact_digest);
                let digest_response = ui.label(digest_label);
                if let Some(digest) = digest {
                    digest_response.on_hover_text(digest);
                }
                ui.label(
                    after.map_or("—", |selection| if selection.direct { "yes" } else { "no" }),
                );
                let capabilities = preview
                    .candidate
                    .packages
                    .get(name)
                    .map(|package| {
                        package
                            .manifest
                            .capabilities
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_else(|| "—".into());
                ui.label(capabilities);
                ui.end_row();
            }
        });
}

fn compact_digest(digest: &str) -> String {
    let mut short: String = digest.chars().take(12).collect();
    if digest.chars().count() > 12 {
        short.push('…');
    }
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn empty_lock() -> Lock {
        Lock {
            schema: 1,
            direct: BTreeMap::new(),
            packages: BTreeMap::new(),
        }
    }

    fn panel_with_cached_state() -> Panel {
        let root = PathBuf::from("C:/rooms/current");
        let lock = empty_lock();
        Panel {
            root: Some(root.clone()),
            source: "C:/packages/current".into(),
            source_identity: "C:/packages/current".into(),
            project_generation: 4,
            source_generation: 7,
            verified: Some(lock.clone()),
            preview: Some(PreviewedPlan {
                plan: Plan {
                    root,
                    source: PathBuf::from("C:/packages/current"),
                    preview: PackagePreview {
                        previous: lock.clone(),
                        candidate: lock,
                        changes: BTreeMap::new(),
                    },
                },
                source_identity: "C:/packages/current".into(),
                project_generation: 4,
                source_generation: 7,
            }),
            model_request: Some(ModelRequest {
                root: PathBuf::from("C:/rooms/current"),
                source_identity: "C:/packages/current".into(),
                project_generation: 4,
                source_generation: 7,
                package: "art".into(),
                asset: "hero.glb".into(),
                kind: ModelKind::Static,
            }),
            ..Panel::default()
        }
    }

    fn model_lock() -> Lock {
        let mut lock = empty_lock();
        for (name, files) in [
            (
                "characters",
                vec!["actors/Hero.GLB", "actors/villain.gltf", "LICENSE.txt"],
            ),
            ("scenery", vec!["props/tree.orrmodel", "tree.png"]),
        ] {
            let manifest = orr_package::Manifest {
                schema: 1,
                name: name.into(),
                version: "1.0.0".into(),
                engine: "*".into(),
                capabilities: ["models".into()].into_iter().collect(),
                dependencies: BTreeMap::new(),
                files: files.into_iter().map(str::to_owned).collect(),
            };
            lock.packages.insert(
                name.into(),
                orr_package::LockedPackage {
                    files: manifest
                        .files
                        .iter()
                        .map(|file| (file.clone(), "a".repeat(64)))
                        .collect(),
                    manifest,
                    digest: "b".repeat(64),
                },
            );
        }
        lock
    }

    #[test]
    fn model_filter_matches_all_terms_across_package_and_path() {
        assert!(model_matches_filter(
            "characters",
            "actors/Hero.GLB",
            "  CHARACTERS hero  "
        ));
        assert!(model_matches_filter(
            "characters",
            "actors/Hero.GLB",
            "characters/actors"
        ));
        assert!(model_matches_filter("characters", "actors/Hero.GLB", " \t"));
        assert!(!model_matches_filter(
            "characters",
            "actors/Hero.GLB",
            "hero scenery"
        ));
        assert!(!model_matches_filter(
            "characters",
            "actors/Hero.GLB",
            "*.glb"
        ));
    }

    #[test]
    fn model_browser_widgets_filter_clear_and_select_without_changing_lock() {
        use egui::accesskit::Role;
        use egui_kittest::{
            kittest::{NodeT, Queryable},
            Harness,
        };
        let mut panel = panel_with_cached_state();
        panel.verified = Some(model_lock());
        panel.model_request = None;
        let original_lock = panel.verified.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui_state(
                |ui, state: &mut (Panel, bool)| {
                    let panel = &mut state.0;
                    model_asset_buttons(
                        ui,
                        ModelBrowser {
                            lock: panel.verified.as_ref().unwrap(),
                            filter: &mut panel.model_filter,
                            preview_request: &mut panel.static_preview_request,
                        },
                        state.1,
                        &mut panel.model_request,
                        panel.root.as_ref(),
                        &panel.source,
                        (panel.project_generation, panel.source_generation),
                    );
                },
                (panel, true),
            );
        harness.run_steps(3);
        assert!(harness
            .query_by_label("Showing 3 of 3 verified model assets")
            .is_some());
        assert!(harness
            .query_by_label("Use model scenery/tree.png")
            .is_none());
        harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .focus();
        harness.run_steps(2);
        assert!(harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .is_focused());
        harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .type_text("CHARACTERS hero");
        harness.run_steps(3);
        assert_eq!(harness.state().0.model_filter, "CHARACTERS hero");
        assert!(harness
            .query_by_label("Showing 1 of 3 verified model assets")
            .is_some());
        assert!(harness
            .query_by_label("Use model characters/actors/villain.gltf")
            .is_none());
        harness
            .get_by_label("Use model characters/actors/Hero.GLB")
            .click();
        harness.run_steps(3);
        assert_eq!(
            harness.state_mut().0.take_model_asset_request(),
            Some((
                "characters".into(),
                "actors/Hero.GLB".into(),
                ModelKind::Static
            ))
        );
        harness
            .get_by_label("Preview static model characters/actors/Hero.GLB")
            .click();
        harness.run_steps(3);
        let preview = harness.state_mut().0.take_static_preview_request().unwrap();
        assert_eq!(preview.package_id, "characters");
        assert_eq!(preview.asset_path, "actors/Hero.GLB");
        assert_eq!(preview.digest, "b".repeat(64));
        assert!(harness.state_mut().0.take_model_asset_request().is_none());
        harness.get_by_label("Clear model filter").click();
        harness.run_steps(3);
        assert!(harness
            .query_by_label("Showing 3 of 3 verified model assets")
            .is_some());
        assert!(harness
            .get_by_label("Clear model filter")
            .accesskit_node()
            .is_disabled());
        harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .focus();
        harness.run_steps(2);
        assert!(harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .is_focused());
        harness
            .get_by_role_and_label(Role::TextInput, "Filter verified models")
            .type_text("no-such-model");
        harness.run_steps(3);
        assert_eq!(harness.state().0.model_filter, "no-such-model");
        assert!(harness
            .query_by_label("No verified models match this filter. Clear it to show all models.")
            .is_some());
        harness.get_by_label("Clear model filter").click();
        harness.state_mut().1 = false;
        harness.run_steps(3);
        assert!(harness
            .get_by_label("Use model scenery/props/tree.orrmodel")
            .accesskit_node()
            .is_disabled());
        assert!(harness
            .get_by_label("Preview static model scenery/props/tree.orrmodel")
            .accesskit_node()
            .is_disabled());
        assert!(harness.state().0.static_preview_request.is_none());
        assert!(harness.state().0.model_request.is_none());
        assert_eq!(harness.state().0.verified, original_lock);
    }

    #[test]
    fn model_filter_resets_only_on_project_change_and_never_authorizes_stale_requests() {
        let mut panel = panel_with_cached_state();
        panel.model_filter = "hero".into();
        panel.select_project(Some(Path::new("C:/rooms/current")));
        assert_eq!(panel.model_filter, "hero");
        panel.source = "C:/packages/other".into();
        panel.select_source();
        assert_eq!(panel.model_filter, "hero");
        assert!(panel.take_model_asset_request().is_none());
        panel.select_project(Some(Path::new("C:/rooms/other")));
        assert!(panel.model_filter.is_empty());
        assert!(panel.verified.is_none());
        assert!(panel.take_model_asset_request().is_none());
    }

    #[test]
    fn stale_add_completion_invalidates_cached_state_for_success_and_failure() {
        for result in [
            Ok(empty_lock()),
            Err("publication outcome uncertain".into()),
        ] {
            let mut panel = panel_with_cached_state();
            let (sender, receiver) = mpsc::channel();
            sender
                .send(Completion::Added {
                    root: PathBuf::from("C:/rooms/current"),
                    source_identity: "C:/packages/current".into(),
                    project_generation: 4,
                    source_generation: 6,
                    result,
                })
                .unwrap();
            panel.pending = Some(receiver);

            panel.poll();

            assert!(panel.verified.is_none());
            assert!(panel.preview.is_none());
            assert!(panel.model_request.is_none());
            assert!(panel.status.contains("prior project"));
        }
    }

    #[test]
    fn matching_add_failure_invalidates_cached_state() {
        let mut panel = panel_with_cached_state();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Completion::Added {
                root: PathBuf::from("C:/rooms/current"),
                source_identity: "C:/packages/current".into(),
                project_generation: 4,
                source_generation: 7,
                result: Err("publication outcome uncertain".into()),
            })
            .unwrap();
        panel.pending = Some(receiver);

        panel.poll();

        assert!(panel.verified.is_none());
        assert!(panel.preview.is_none());
        assert!(panel.model_request.is_none());
        assert!(panel.status.contains("publication outcome uncertain"));
    }

    #[test]
    fn typed_model_request_preserves_kind_and_requires_current_identity() {
        let mut panel = panel_with_cached_state();
        assert_eq!(
            panel.take_model_asset_request(),
            Some(("art".into(), "hero.glb".into(), ModelKind::Static))
        );
        assert!(panel.take_model_asset_request().is_none());

        let mut panel = panel_with_cached_state();
        panel.source_generation += 1;
        assert!(panel.take_model_asset_request().is_none());

        let mut panel = panel_with_cached_state();
        panel.root = Some(PathBuf::from("C:/rooms/other"));
        assert!(panel.take_model_asset_request().is_none());
    }

    #[cfg(feature = "room-animated-models")]
    #[test]
    fn animated_requests_never_degrade_to_static_requests() {
        let mut panel = panel_with_cached_state();
        panel.model_request.as_mut().unwrap().kind = ModelKind::Animated;
        assert!(panel.take_model_request().is_none());

        let mut panel = panel_with_cached_state();
        panel.model_request.as_mut().unwrap().kind = ModelKind::Animated;
        assert_eq!(
            panel.take_model_asset_request(),
            Some(("art".into(), "hero.glb".into(), ModelKind::Animated))
        );
    }
    #[test]
    fn stale_removal_completion_never_installs_prior_project_inventory() {
        let mut panel = panel_with_cached_state();
        let (sender, receiver) = mpsc::channel();
        panel.pending = Some(receiver);
        sender
            .send(Completion::Removed {
                receipt: None,
                root: panel.root.clone().unwrap(),
                project_generation: panel.project_generation,
                result: Ok(empty_lock()),
            })
            .unwrap();
        panel.select_project(Some(Path::new("C:/rooms/new")));
        panel.poll();
        assert!(panel.verified.is_none());
        assert!(panel.status.contains("prior project"));
        assert!(!panel.busy());
    }

    #[test]
    fn removal_dispatch_requires_idle_verified_direct_selection() {
        let mut panel = panel_with_cached_state();
        panel.removal_name = "not-direct".into();
        panel.start_removal_preview();
        assert!(!panel.busy());
        let (_sender, receiver) = mpsc::channel();
        panel.pending = Some(receiver);
        panel.start_remove();
        assert!(panel.busy());
        assert!(panel.removal_preview.is_none());
    }

    #[test]
    fn project_change_clears_removal_selection() {
        let mut panel = panel_with_cached_state();
        panel.removal_name = "old-package".into();
        panel.select_project(Some(Path::new("C:/rooms/new")));
        assert!(panel.removal_name.is_empty());
        assert!(panel.removal_preview.is_none());
        assert!(panel.verified.is_none());
    }
    #[test]
    fn mutating_removal_freezes_editor_input_through_completion_frame() {
        use crate::{Editor, EditorApp, HostSpec, Mode};
        use egui_kittest::{kittest::Queryable, Harness};
        use orr_sample::{
            room_game::{RoomConfig, RoomEscapeV1},
            room_project,
        };
        for restoring in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("room.scene.yaml");
            let simulation =
                orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
            let text = orr_reflect::Scene::unbake(&room_project::types(), simulation.frame(), None)
                .unwrap()
                .to_yaml();
            std::fs::write(&path, &text).unwrap();
            let mut editor = Editor::start(&HostSpec::PreparedRoom {
                scene: path.clone(),
                text,
                listen: None,
                debug_hooks: false,
            })
            .unwrap();
            editor.sync();
            let checksum = editor.checksum();
            let history = editor.history().clone();
            let before = std::fs::read(&path).unwrap();
            let mut h = Harness::builder()
                .with_size([1200.0, 800.0])
                .build_eframe(move |_| EditorApp::new(editor, None));
            h.run_steps(3);
            // Queue a real stale accessibility action before activating the held worker.
            h.get_by_label(crate::app::LBL_PLAY).click_accesskit();
            let (sender, receiver) = mpsc::channel();
            h.state_mut().package_manager.pending = Some(receiver);
            h.state_mut().package_manager.removing = true;
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.key_press(egui::Key::Delete);
            h.run_steps(3);
            assert!(h.query_by_label(crate::app::LBL_PLAY).is_none());
            assert!(h.query_by_label(crate::app::LBL_STEP).is_none());
            assert_eq!(h.state().editor.mode(), Mode::Edit);
            assert_eq!(h.state().editor.checksum(), checksum);
            assert_eq!(h.state().editor.history(), &history);
            assert_eq!(std::fs::read(&path).unwrap(), before);
            sender
                .send(if restoring {
                    Completion::Restored {
                        root: temp.path().into(),
                        project_generation: 0,
                        result: Err("held restore worker released".into()),
                    }
                } else {
                    Completion::Removed {
                        receipt: None,
                        root: temp.path().into(),
                        project_generation: 0,
                        result: Err("held test worker released".into()),
                    }
                })
                .unwrap();
            h.run_steps(1);
            assert!(!h.state().package_manager.removal_busy());
            assert!(
                h.query_by_label(crate::app::LBL_PLAY).is_none(),
                "completion frame must remain frozen"
            );
            h.run_steps(2);
            assert!(h.query_by_label(crate::app::LBL_PLAY).is_some());
            assert_eq!(h.state().editor.mode(), Mode::Edit);
            assert_eq!(h.state().editor.checksum(), checksum);
        }
    }
    #[test]
    fn restoration_dispatch_requires_successful_receipt_and_explicit_preview() {
        let mut panel = panel_with_cached_state();
        panel.start_restoration_preview();
        panel.start_restore();
        assert!(!panel.busy());
        assert!(panel.last_removal.is_none());
        assert!(panel.restoration_preview.is_none());
    }
    #[test]
    fn fresh_verification_discards_real_removal_receipt_and_restoration_preview() {
        use orr_sample::{
            room_game::{RoomConfig, RoomEscapeV1},
            room_project,
        };
        use std::{
            fs,
            time::{Duration, Instant},
        };
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("room");
        let source = temp.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&source).unwrap();
        let simulation =
            orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
        let scene = orr_reflect::Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), scene).unwrap();
        fs::write(root.join("orr.project.json"),r#"{"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}}"#).unwrap();
        fs::write(
            root.join("room.models.json"),
            r#"{"version":2,"scene":"room.scene.yaml","project":".","bindings":{}}"#,
        )
        .unwrap();
        fs::write(source.join("LICENSE.txt"), "Owned fixture").unwrap();
        fs::write(source.join("orr.package.json"),r#"{"schema":1,"name":"unused","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{},"files":["LICENSE.txt"]}"#).unwrap();
        orr_package::Project::open_for_install(
            &root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[
            source,
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/imported_scene_demo")
                .canonicalize()
                .unwrap(),
        ])
        .unwrap();
        let admitted = room_project::PreparedScene::parse(
            &fs::read_to_string(root.join("room.scene.yaml")).unwrap(),
        )
        .unwrap();
        let player = admitted
            .frame()
            .entities()
            .find(|entity| {
                admitted
                    .frame()
                    .get::<orr_sample::room_game::RoomActor>(*entity)
                    .is_some_and(|actor| actor.kind == orr_sample::room_game::PLAYER)
            })
            .unwrap();
        let guid = admitted.index().guid(player).unwrap().to_string();
        let loaded = orr_model_bindings::model_bindings::load_asset(
            &root,
            "sample-imported-scene",
            "foreground.glb",
        )
        .unwrap();
        let binding = orr_model_bindings::model_bindings::Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            orr_model_bindings::model_bindings::LocalTransform::default(),
        )
        .unwrap();
        let document = orr_model_bindings::model_bindings::Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: [(guid, binding)].into_iter().collect(),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let plan = package_manager::plan_remove(&root, "unused").unwrap();
        let (_, receipt) = package_manager::apply_remove_with_receipt(&plan).unwrap();
        let preview = package_manager::plan_restore(&receipt).unwrap();
        let mut panel = Panel {
            root: Some(root),
            last_removal: Some(receipt),
            restoration_preview: Some(preview),
            ..Panel::default()
        };
        let generation = panel.restoration_generation;
        panel.start_verify();
        assert!(panel.last_removal.is_none());
        assert!(panel.restoration_preview.is_none());
        assert!(panel.restoration_generation > generation);
        let deadline = Instant::now() + Duration::from_secs(10);
        while panel.busy() {
            panel.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(panel.last_removal.is_none());
        assert!(panel.restoration_preview.is_none());
        assert!(panel.verified.is_some());
    }
}
