//! Explicit local package preview and one-shot activation controls.
use super::*;
use crate::sprite_package_install as backend;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Context {
    bindings_path: PathBuf,
    bindings_document: Vec<u8>,
    project: PathBuf,
    project_input: String,
    scene: String,
    load_epoch: u64,
    host_epoch: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Draft {
    source_directory: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pin {
    context: Context,
    context_ticket: u64,
    source_ticket: u64,
    draft_ticket: u64,
    draft: Draft,
    source: PathBuf,
}

pub(super) struct Prepared {
    pin: Pin,
    pub(super) plan: backend::Plan,
}

enum Pending {
    Preview {
        pin: Pin,
        receiver: Receiver<Result<backend::Plan, String>>,
    },
}

#[derive(Default)]
pub(super) struct State {
    draft: Draft,
    observed_draft: Draft,
    context: Option<Context>,
    context_ticket: u64,
    source_ticket: u64,
    draft_ticket: u64,
    pending: Option<Pending>,
    pub(super) prepared: Option<Prepared>,
    pub(super) status: String,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(feature = "sprite-grid-authoring")]
    pub(super) fn blocks_grid(&self) -> bool {
        self.busy() || self.prepared.is_some()
    }

    fn exhausted(&self) -> bool {
        [self.context_ticket, self.source_ticket, self.draft_ticket].contains(&u64::MAX)
    }

    fn bump(ticket: &mut u64) {
        *ticket = ticket.checked_add(1).unwrap_or(u64::MAX);
    }

    fn sync(&mut self, context: Option<Context>) {
        let had_prepared = self.prepared.is_some();
        let mut invalidated = false;
        if self.context != context {
            Self::bump(&mut self.context_ticket);
            self.context = context;
            self.prepared = None;
            invalidated = true;
        }
        if self.observed_draft.source_directory != self.draft.source_directory {
            Self::bump(&mut self.source_ticket);
            self.prepared = None;
            invalidated = true;
        }
        if self.observed_draft != self.draft {
            Self::bump(&mut self.draft_ticket);
            self.observed_draft.clone_from(&self.draft);
            self.prepared = None;
            invalidated = true;
        }
        if had_prepared && invalidated && !self.exhausted() {
            self.status = "Sprite install preview is stale; preview again.".into();
        }
        if self.exhausted() {
            self.prepared = None;
            self.status = "Sprite installation tickets exhausted; reopen the editor before previewing or installing another package.".into();
        }
    }

    fn pin(&self) -> Option<Pin> {
        if self.exhausted() || self.draft.source_directory.trim().is_empty() {
            return None;
        }
        let source = PathBuf::from(self.draft.source_directory.trim());
        Some(Pin {
            context: self.context.clone()?,
            context_ticket: self.context_ticket,
            source_ticket: self.source_ticket,
            draft_ticket: self.draft_ticket,
            draft: self.draft.clone(),
            source,
        })
    }

    fn pin_matches(&self, pin: &Pin, live_context: Option<&Context>) -> bool {
        !self.exhausted()
            && live_context == Some(&pin.context)
            && self.context.as_ref() == Some(&pin.context)
            && self.context_ticket == pin.context_ticket
            && self.source_ticket == pin.source_ticket
            && self.draft_ticket == pin.draft_ticket
            && self.draft == pin.draft
            && self.observed_draft == pin.draft
            && pin.source.as_path() == std::path::Path::new(self.draft.source_directory.trim())
    }

    fn start_preview(&mut self) {
        if self.busy() || self.exhausted() {
            return;
        }
        let Some(pin) = self.pin() else {
            self.status =
                "Matching local Edit sprite bindings and a local package directory are required."
                    .into();
            return;
        };
        let project = pin.context.project.clone();
        let source = pin.source.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-sprite-install-preview".into())
            .spawn(move || {
                let _ = sender.send(backend::prepare(&project, &source));
            }) {
            Ok(_) => {
                self.prepared = None;
                self.pending = Some(Pending::Preview { pin, receiver });
                self.status = "Preparing a read-only sprite package preview...".into();
            }
            Err(error) => {
                self.prepared = None;
                self.status = format!("Cannot start sprite package preview: {error}");
            }
        }
    }

    fn poll(&mut self, context: Option<Context>) {
        self.sync(context);
        let result = match self.pending.as_ref() {
            Some(Pending::Preview { receiver, .. }) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(
                    "Sprite package preview worker disconnected; preview explicitly again.".into(),
                )),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        let Some(result) = result else {
            return;
        };
        let pending = self.pending.take();
        let Some(Pending::Preview { pin, .. }) = pending else {
            return;
        };
        if !self.pin_matches(&pin, self.context.as_ref()) {
            self.prepared = None;
            self.status =
                "Discarded a sprite package preview from a prior source, draft or editor context."
                    .into();
            return;
        }
        match result {
            Ok(plan) => {
                let summary = plan.summary();
                self.status = format!(
                    "Previewed {} {} ({} x {}, {} regions, {} clips, SHA-256 {}). Review the files, then install explicitly.",
                    summary.name,
                    summary.version,
                    summary.width,
                    summary.height,
                    summary.regions,
                    summary.clips,
                    summary.digest,
                );
                self.prepared = Some(Prepared { pin, plan });
            }
            Err(error) => {
                self.prepared = None;
                self.status = format!("Sprite package preview failed: {error}");
            }
        }
    }

    fn take_prepared(&mut self) -> Option<Prepared> {
        self.prepared.take()
    }

    fn finish_attempt(&mut self, result: Result<(), String>, summary: &backend::Summary) {
        self.prepared = None;
        self.status = match result {
            Ok(()) => format!(
                "Installed {} {}. It was not loaded, assigned or added to the current bindings.",
                summary.name, summary.version
            ),
            Err(error) => format!(
                "Sprite package installation failed: {error}. The prepared preview was consumed; preview again explicitly before another attempt."
            ),
        };
    }

    fn show(&mut self, ui: &mut Ui, context: Option<Context>, authoring_busy: bool) -> bool {
        self.sync(context.clone());
        let mut install_clicked = false;
        egui::CollapsingHeader::new("Install local sprite package")
            .default_open(false)
            .show(ui, |ui| {
                ui.weak("Preview package contents and the one-package lock change before an explicit local install. Preview does not write, load or assign assets.");
                field(
                    ui,
                    "Local sprite package directory",
                    &mut self.draft.source_directory,
                    4096,
                );
                self.sync(context.clone());
                let can_preview = context.is_some()
                    && !authoring_busy
                    && !self.busy()
                    && !self.exhausted()
                    && !self.draft.source_directory.trim().is_empty();
                if ui
                    .add_enabled(
                        can_preview,
                        egui::Button::new("Preview sprite package install"),
                    )
                    .clicked()
                {
                    self.start_preview();
                }
                if self.busy() {
                    ui.spinner();
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
                }
                if let Some(prepared) = self.prepared.as_ref() {
                    let summary = prepared.plan.summary();
                    ui.label(format!(
                        "{} {} - {} x {} - {} regions - {} clips",
                        summary.name, summary.version, summary.width, summary.height,
                        summary.regions, summary.clips
                    ));
                    ui.label(format!("Files: {}", summary.files.join(", ")));
                    ui.label(format!("Package digest: {}", summary.digest));
                }
                let can_install = self.prepared.as_ref().is_some_and(|prepared| {
                    !authoring_busy
                        && !self.exhausted()
                        && self.pin_matches(&prepared.pin, context.as_ref())
                });
                if ui
                    .add_enabled(
                        can_install,
                        egui::Button::new("Install previewed sprite package"),
                    )
                    .clicked()
                {
                    install_clicked = true;
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                if authoring_busy {
                    ui.weak("Sprite grid authoring is active; wait for it to finish before previewing or installing.");
                }
            });
        install_clicked
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    let label = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
}

impl SpritePanel {
    fn package_install_context(&self, editor: &Editor) -> Option<Context> {
        if self.require_unpinned().is_err()
            || !editor.spec().is_local()
            || !editor.can_mutate()
            || editor.mode() != Mode::Edit
            || editor.sim().in_tx
            || editor.screenshot_waiting_for().is_some()
            || !editor.has_local_screenshot_owner()
            || !self.editable(editor)
        {
            return None;
        }
        let bindings = self.bindings.as_ref()?;
        let project =
            sprite_bindings::resolve_project(bindings.base(), &bindings.document().project).ok()?;
        Some(Context {
            bindings_path: bindings.path.clone(),
            bindings_document: serde_json::to_vec(bindings.document()).ok()?,
            project,
            project_input: bindings.document().project.clone(),
            scene: editor.sim().scene_path.clone()?,
            load_epoch: editor.scene_load_epoch(),
            host_epoch: editor.sim().epoch,
        })
    }

    pub(super) fn poll_package_install(&mut self, editor: &Editor) {
        self.package_install
            .poll(self.package_install_context(editor));
    }

    pub(super) fn show_package_install(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        authoring_busy: bool,
    ) {
        let context = self.package_install_context(editor);
        let install_clicked = self
            .package_install
            .show(ui, context.clone(), authoring_busy);
        if !install_clicked {
            return;
        }
        let Some(prepared) = self.package_install.take_prepared() else {
            return;
        };
        let summary = prepared.plan.summary().clone();
        // `take_prepared` ends the state borrow before this synchronous call.
        // The callback rechecks the live panel/editor fence under the writer.
        let result = backend::apply(&prepared.plan, || {
            let context = self.package_install_context(editor);
            if self
                .package_install
                .pin_matches(&prepared.pin, context.as_ref())
            {
                Ok(())
            } else {
                Err("live SpritePanel or Editor context changed after preview".into())
            }
        });
        self.package_install
            .finish_attempt(result.map(|_| ()), &summary);
    }
}

#[cfg(all(test, feature = "sprite-grid-authoring", target_os = "linux"))]
#[path = "package_install_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "package_install_ui_tests.rs"]
mod tests;
