//! Passive, popup-free protection for the authoring app's native root close.
//! No save, discard, exit consent, worker cancellation or player lifecycle lives here.
use super::{work_status::Inspection, BottomTab, EditorApp};
use egui::{Ui, ViewportCommand, ViewportId};
use orr_remote::work_status::DomainId;

#[derive(Default)]
pub(super) struct State {
    pub(super) blocked: bool,
    // Bounded fixed-domain historical observations, never used to authorize close.
    last_close_only: Vec<DomainId>,
}

pub(super) struct Item {
    pub(super) name: &'static str,
    pub(super) action: &'static str,
    pub(super) last_close: bool,
}

impl EditorApp {
    /// The complete callback runs first, including host/worker polling and any
    /// explicit widget action. Even an early return cannot bypass the final check.
    /// Refusing a close never opens UI, changes focus/tabs, or consumes input.
    pub(super) fn with_close_review(&mut self, ui: &mut Ui, draw: impl FnOnce(&mut Self, &mut Ui)) {
        let ctx = ui.ctx().clone();
        let root_close = ctx.input(|input| {
            input.raw.viewport_id == ViewportId::ROOT && input.viewport().close_requested()
        });
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
        self.poll_arena_export(&ctx);
        draw(self, ui);
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
        self.poll_arena_export(&ctx);
        let observed = self.work_status_domains(Inspection::Observation);
        self.publish_work_status(observed.clone());
        if root_close {
            // These legacy disk-aware/serialized predicates run only for a real
            // explicit close request, never for continuous UI/status publication.
            let closing = self.work_status_domains(Inspection::CloseRequest);
            self.ui.close_review.last_close_only = closing
                .iter()
                .filter(|row| {
                    row.is_blocker()
                        && !observed.iter().any(|current| {
                            current.domain_id == row.domain_id && current.is_blocker()
                        })
                })
                .map(|row| row.domain_id)
                .collect();
            self.ui.close_review.blocked = (self.editor.spec().is_local()
                && self.editor.is_dirty())
                || closing.iter().any(|row| row.is_blocker());
            #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
            if self.arena_export.busy() { self.ui.close_review.blocked = true; }
            if self.ui.close_review.blocked {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
            ctx.request_repaint();
        }
    }

    /// This slot always occupies the same space. Changing close status cannot
    /// reflow the viewport or a staged authored-screen preview.
    pub(super) fn close_review_status(&mut self, ui: &mut Ui) {
        let label = if self.ui.close_review.blocked {
            "Close blocked"
        } else {
            "Unsaved work"
        };
        if ui
            .add_sized(
                [112.0, ui.spacing().interact_size.y],
                egui::Button::new(label),
            )
            .clicked()
        {
            self.ui.bottom_tab = BottomTab::UnsavedWork;
        }
    }

    /// Path-free in-memory positives plus explicitly marked last-close rows.
    /// The latter are historical UI details, not current API facts or close vetoes.
    pub(super) fn close_review_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
        if self.arena_export.busy() {
            items.push(Item { name: "Arena export · worker owned", action: "Wait for export worker completion and join, then explicitly close again. Editing remains available.", last_close: false });
        }
        if self.editor.spec().is_local() && self.editor.is_dirty() {
            items.push(Item {
                name: "Scene · unsaved",
                action: "Use File → Save / Save As, or the scene's Undo controls.",
                last_close: false,
            });
        }
        let observed = self.work_status_domains(Inspection::Observation);
        items.extend(
            observed
                .iter()
                .filter(|record| record.is_blocker())
                .map(|record| super::work_status::close_item(record.domain_id)),
        );
        for id in &self.ui.close_review.last_close_only {
            if !observed
                .iter()
                .any(|row| row.domain_id == *id && row.is_blocker())
            {
                let mut item = super::work_status::close_item(*id);
                item.last_close = true;
                items.push(item);
            }
        }
        items
    }

    /// Details are embedded in an ordinary bottom tab, opened explicitly only.
    /// It projects the same typed predicates as the read-only status snapshot.
    pub(super) fn show_close_review(&mut self, ui: &mut Ui) {
        if self.ui.close_review.blocked {
            ui.label("The last close was blocked. Work was retained; normal editing and agent commands continue.");
            if ui.button("Clear close status").clicked() {
                self.ui.close_review.blocked = false;
                self.ui.close_review.last_close_only.clear();
            }
        }
        let items = self.close_review_items();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if items.is_empty() {
                ui.label("No positive in-memory blocker is listed. Unobserved domains may still block an explicit close request.");
            }
            for item in items {
                if item.last_close {
                    ui.strong(format!("Last close observation: {}", item.name));
                    ui.weak("Historical only; not rechecked and not an API current-state fact.");
                } else {
                    ui.strong(item.name);
                }
                ui.label(item.action);
                ui.add_space(6.0);
            }
            ui.weak("Unknown is lack of observation, not clean or dirty. This list does not probe files or serialize Tilemaps. Only an explicit close request runs the legacy additional checks.");
            ui.weak("This list never saves, applies, discards or closes anything. Scene Save does not save independent documents.");
        });
    }
}
