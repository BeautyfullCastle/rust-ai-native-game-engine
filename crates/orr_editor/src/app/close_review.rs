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
        let mut add = |condition, name, action| {
            if condition {
                items.push(Item { name, action });
            }
        };
        if self.editor.spec().is_local() {
            add(
                self.editor.is_dirty(),
                "Scene · unsaved",
                "Use File → Save / Save As, or the scene's Undo controls.",
            );
        }
        #[cfg(feature = "sprites")]
        add(
            self.sprites.bindings.as_ref().is_some_and(|bindings| bindings.dirty()),
            "Sprite bindings · unsaved",
            "Use Save bindings in Sprite bindings (view only). Scene Save does not save this sidecar.",
        );
        #[cfg(feature = "tilemap")]
        add(
            self.tilemap.as_ref().is_some_and(|panel| panel.dirty()),
            "Tilemap · unsaved",
            "Use Save tilemap or Undo tile. Scene Save does not save tiles.",
        );
        #[cfg(feature = "collect-ui")]
        {
            add(
                self.collect_ui.as_ref().is_some_and(|panel| panel.dirty()),
                "Collect UI document · unsaved",
                "Use Save UI document in Collect UI document. Scene Save does not save this document.",
            );
            add(
                self.collect_ui_staged_authoring(),
                "Collect UI preview · unapplied or unfinished",
                "Finish or cancel the gesture in the authored-screen preview. Apply accepted changes there, then Save UI document.",
            );
        }
        #[cfg(feature = "models")]
        add(
            self.models
                .bindings
                .as_ref()
                .is_some_and(|bindings| bindings.dirty()),
            "Model bindings · unsaved",
            "Use Save model bindings in the model inspector.",
        );
        #[cfg(feature = "animated-models")]
        add(
            self.animated_models
                .bindings
                .as_ref()
                .is_some_and(|bindings| bindings.dirty()),
            "Animation bindings · unsaved",
            "Use Save animation bindings in Animation authoring.",
        );
        #[cfg(feature = "terrain")]
        add(
            self.terrain.document.dirty(),
            "Terrain · unsaved",
            "Use Save terrain or Terrain Undo in the terrain inspector.",
        );
        #[cfg(feature = "irradiance-probes")]
        add(
            self.irradiance
                .bindings
                .as_ref()
                .is_some_and(|bindings| bindings.dirty()),
            "Irradiance bindings · unsaved",
            "Use Save irradiance in the irradiance inspector.",
        );
        #[cfg(feature = "room-project")]
        add(
            self.room_camera.as_ref().is_some_and(|panel| panel.dirty()),
            "Room camera · unsaved or unapplied",
            "Use Apply camera for staged fields, then Save camera; Undo camera edit is separate.",
        );
        #[cfg(feature = "mesh-navigation")]
        {
            add(
                self.mesh_navigation.has_unplaced_draft(),
                "Mesh geometry / art · unsaved, unapplied or unfinished",
                "Finish geometry/vertex work and apply or resolve drafts in Static-mesh navigation. Save accepted scene and art changes with the existing workspace controls.",
            );
            add(
                self.mesh_camera.dirty(),
                "Mesh camera · unsaved or unapplied",
                "Use Apply game camera for staged fields, then Save game camera.",
            );
            add(
                self.mesh_lighting.dirty(),
                "Mesh lighting · unsaved or unapplied",
                "Use Apply game lighting for staged fields, then Save game lighting.",
            );
            add(
                self.mesh_hud.dirty(),
                "Mesh HUD · unsaved or unapplied",
                "Use Apply game HUD for staged fields, then Save game HUD.",
            );
        }
        #[cfg(feature = "mesh-mission")]
        add(
            self.mesh_mission_dirty(),
            "Mesh mission · unapplied or unfinished",
            "Finish or cancel point picking and apply or resolve mission fields in Mesh mission, then save the scene.",
        );
        #[cfg(feature = "mesh-hazards")]
        add(
            self.mesh_hazards_dirty(),
            "Mesh hazards · unapplied or unfinished",
            "Finish or cancel hazard picking and apply or resolve hazard fields in Mesh hazards, then save the scene.",
        );
        #[cfg(feature = "mesh-animated-models")]
        add(
            self.mesh_animation_dirty(),
            "Mesh animation · unsaved or unapplied",
            "Resolve the loaded candidate and staged fields with Apply mesh animation, then Save mesh animation.",
        );
        items
    }

    /// Details are embedded in an ordinary bottom tab, opened explicitly only.
    /// This is UI/accessibility inventory, not a new ERP status or save API.
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
