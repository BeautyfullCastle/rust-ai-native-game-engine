use super::*;
impl Panel {
    pub(super) fn show_static_preview_controls(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        can_edit: bool,
    ) {
        let focused = ui.input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        let valid = self
            .scene
            .as_deref()
            .zip(self.graph_revision)
            .is_some_and(|(scene, revision)| self.parse_form(scene, revision).is_ok());
        let ready = can_edit
            && self.can_edit(editor)
            && focused
            && !self.follow_agent
            && !self.static_preview_busy()
            && valid;
        let mut enabled = self.static_preview_enabled();
        if ui
            .add_enabled(
                ready || enabled,
                egui::Checkbox::new(&mut enabled, "Preview static camera draft"),
            )
            .changed()
            && (!enabled || ready)
        {
            if let Err(error) = self.set_static_preview(editor, enabled) {
                self.error = Some(error);
            }
        }
        ui.weak("Viewport preview only; Apply and Save explicitly. Static camera drafts only: enabling Follow ends preview. Reopening starts with preview off.");
    }
}
