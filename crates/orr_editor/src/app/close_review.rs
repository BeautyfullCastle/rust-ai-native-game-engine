//! Passive, popup-free protection for the authoring app's native root close.
//! No save, discard, exit consent, worker cancellation or player lifecycle lives here.
use super::{BottomTab, EditorApp};
use egui::{Ui, ViewportCommand, ViewportId};

#[derive(Default)]
pub(super) struct State {
    pub(super) blocked: bool,
}

pub(super) struct Item {
    pub(super) name: &'static str,
    pub(super) action: &'static str,
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
        draw(self, ui);
        self.publish_work_status(self.work_status_domains());
        if root_close {
            self.ui.close_review.blocked = !self.close_review_items().is_empty();
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

    /// Inspect retained documents, not only the current game's visible panels.
    /// These predicates distinguish saved documents from unapplied/unfinished
    /// authoring. Routine pending host reads are not a close veto.
    pub(super) fn close_review_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        if self.editor.spec().is_local() && self.editor.is_dirty() {
            items.push(Item {
                name: "Scene · unsaved",
                action: "Use File → Save / Save As, or the scene's Undo controls.",
            });
        }
        items.extend(self.work_status_domains().into_iter()
            .filter(|record| record.is_blocker())
            .map(|record| super::work_status::close_item(record.domain_id)));
        items
    }

    /// Details are embedded in an ordinary bottom tab, opened explicitly only.
    /// It projects the same typed predicates as the read-only status snapshot.
    pub(super) fn show_close_review(&mut self, ui: &mut Ui) {
        if self.ui.close_review.blocked {
            ui.label("The last close was blocked. Work was retained; normal editing and agent commands continue.");
            if ui.button("Clear close status").clicked() {
                self.ui.close_review.blocked = false;
            }
        }
        let items = self.close_review_items();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if items.is_empty() {
                ui.label("No listed unsaved or unapplied work remains. Close the window again when ready.");
            }
            for item in items {
                ui.strong(item.name);
                ui.label(item.action);
                ui.add_space(6.0);
            }
            ui.weak("This list never saves, applies, discards or closes anything. Scene Save does not save independent documents.");
        });
    }
}
