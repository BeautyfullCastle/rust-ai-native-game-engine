//! Explicit standalone-only project linking. The main editor never shows this UI.
use super::Panel;
use orr_input::ActionMap;
use orr_sample::room_controls_attach::{self, Attachment, Outcome};
use std::path::Path;

#[derive(Clone, Default, PartialEq, Eq)]
struct Request {
    root: String,
    controls: String,
    revise: bool,
    expected: String,
    backup: String,
}

struct Preview {
    request: Request,
    document: ActionMap,
    attachment: Attachment,
}

#[derive(Default)]
pub(super) struct ProjectLink {
    request: Request,
    preview: Option<Preview>,
    message: String,
}

impl ProjectLink {
    fn invalidate(&mut self, panel: &Panel) {
        if self.preview.as_ref().is_some_and(|preview| {
            preview.request != self.request
                || panel.has_pending_draft()
                || preview.document != *panel.document()
        }) {
            self.preview = None;
            self.message = "Preview cancelled: fields or map changed. Preview again.".into();
        }
    }

    fn prepare(&mut self, panel: &Panel) -> Result<(), String> {
        self.preview = None;
        if panel.has_pending_draft() {
            return Err("Apply or Discard the action draft before previewing.".into());
        }
        let request = &self.request;
        let root = Path::new(&request.root);
        // Production preparation validates filename, project, sidecar and backup.
        let attachment = if request.revise {
            room_controls_attach::prepare_update(
                root,
                &request.controls,
                &request.expected,
                &request.backup,
            )?
        } else {
            room_controls_attach::prepare(root, &request.controls)?
        };
        let saved = ActionMap::load_file(root.join(&request.controls))?;
        if saved != *panel.document() {
            return Err("Saved sidecar differs from the accepted map. Save As a new file or Open the intended sidecar first.".into());
        }
        self.preview = Some(Preview {
            request: request.clone(),
            document: panel.document().clone(),
            attachment,
        });
        Ok(())
    }

    fn commit(&mut self, panel: &Panel) -> Result<String, String> {
        self.invalidate(panel);
        // Consume before any I/O: late failures and repeated clicks cannot retry.
        let preview = self.preview.take().ok_or("Preview again before linking.")?;
        let outcome = preview.attachment.commit()?;
        Ok(outcome_message(outcome))
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, panel: &Panel) {
        ui.separator();
        ui.heading("Link saved controls to a project");
        ui.small("Linux x86_64 · plain Room only. Close other editors/writers first. Use a trusted, stable local directory. Save As the accepted map in the project root first.");
        field(ui, "Project directory", &mut self.request.root, 4096);
        field(
            ui,
            "Saved controls filename",
            &mut self.request.controls,
            100,
        );
        ui.checkbox(
            &mut self.request.revise,
            "Revise an existing controls reference",
        );
        if self.request.revise {
            field(
                ui,
                "Expected current controls filename",
                &mut self.request.expected,
                100,
            );
            field(ui, "Fresh backup filename", &mut self.request.backup, 100);
        }
        self.invalidate(panel);
        if ui
            .add_enabled(
                !panel.has_pending_draft(),
                egui::Button::new("Preview project link"),
            )
            .clicked()
        {
            self.message = self.prepare(panel).map_or_else(
                |error| error,
                |()| "Validated preview. Project files unchanged.".into(),
            );
        }
        if let Some(preview) = &self.preview {
            ui.label(format!("Project: {}", preview.request.root));
            let old = if preview.request.revise {
                preview.request.expected.as_str()
            } else {
                "(none)"
            };
            let backup = if preview.request.revise {
                preview.request.backup.as_str()
            } else {
                "orr.project.before-controls.json"
            };
            ui.label(format!("Controls: {old} → {}", preview.request.controls));
            ui.label(format!(
                "Create backup: {backup}. Replace only the manifest; do not write either sidecar."
            ));
            ui.horizontal(|ui| {
                if ui.button("Confirm project link").clicked() {
                    self.message = self.commit(panel).unwrap_or_else(|error| format!("Link failed: {error}. Inspect project and backup before previewing again."));
                }
                if ui.button("Cancel project link").clicked() {
                    self.preview = None;
                    self.message = "Preview cancelled. No project write requested.".into();
                }
            });
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, limit: usize) {
    let label = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
}

fn outcome_message(outcome: Outcome) -> String {
    let message = format!("Project linked. Backup: {}. Reopen the complete project to use these controls. Undo map edit does not undo the project change.", outcome.backup.display());
    match outcome.durability_warning {
        Some(warning) => format!("{message} Applied, but durability is uncertain: {warning}. Inspect files before any retry."),
        None => message,
    }
}

#[cfg(all(test, feature = "project-create"))]
#[path = "project_link_tests.rs"]
mod tests;
