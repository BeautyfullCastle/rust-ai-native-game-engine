use crate::package_manager::{self, Plan};
use egui::Context;
use orr_package::{Lock, PackagePreview};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

enum Completion {
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
    verified: Option<Lock>,
    preview: Option<PreviewedPlan>,
    model_request: Option<ModelRequest>,
    pending: Option<Receiver<Completion>>,
}

impl Panel {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn take_model_request(&mut self) -> Option<(String, String)> {
        let request = self.model_request.take()?;
        if self.root.as_ref() == Some(&request.root)
            && self.source.trim() == request.source_identity.as_str()
            && self.project_generation == request.project_generation
            && self.source_generation == request.source_generation
        {
            Some((request.package, request.asset))
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
        self.poll();
        if !self.open {
            return;
        }
        let can_operate = self.root.is_some() && blocked_reason.is_none() && !self.busy();
        let mut open = self.open;
        egui::Window::new("Content packages")
            .open(&mut open)
            .default_width(700.0)
            .default_pos(egui::pos2(250.0, 350.0))
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
                if self.busy() {
                    ui.spinner();
                    context.request_repaint_after(std::time::Duration::from_millis(40));
                }
                if let Some(lock) = &self.verified {
                    ui.separator();
                    ui.label("Installed package bytes verified from this project's lock:");
                    lock_table(ui, "verified-installed-packages", lock);
                    ui.label("Select an entity, then use a verified static model; binding save is separate.");
                    model_asset_buttons(
                        ui,
                        lock,
                        can_operate,
                        &mut self.model_request,
                        self.root.as_ref(),
                        self.source.trim(),
                        (self.project_generation, self.source_generation),
                    );
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
                self.status = "Package worker disconnected before returning a result.".into();
                return;
            }
        };
        self.pending = None;
        match completion {
            Completion::Verified {
                root,
                project_generation,
                result,
            } if self.root.as_ref() == Some(&root)
                && self.project_generation == project_generation =>
            {
                match result {
                    Ok(lock) => {
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

    fn start_verify(&mut self) {
        if self.busy() {
            return;
        }
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

fn model_asset_buttons(
    ui: &mut egui::Ui,
    lock: &Lock,
    enabled: bool,
    request: &mut Option<ModelRequest>,
    root: Option<&PathBuf>,
    source: &str,
    generations: (u64, u64),
) {
    let mut found = false;
    for (package_name, package) in &lock.packages {
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
            found = true;
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(format!("Use model {package_name}/{asset}")),
                )
                .clicked()
            {
                if let Some(root) = root {
                    *request = Some(ModelRequest {
                        root: root.clone(),
                        source_identity: source.to_owned(),
                        project_generation: generations.0,
                        source_generation: generations.1,
                        package: package_name.clone(),
                        asset: asset.clone(),
                    });
                }
            }
        }
    }
    if !found {
        ui.label("No verified GLB, glTF or Orrmodel assets are available.");
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
            }),
            ..Panel::default()
        }
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
}
