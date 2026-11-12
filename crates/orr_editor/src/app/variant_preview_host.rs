use super::EditorApp;
impl EditorApp {
    pub(super) fn sync_variant_model_preview(&mut self) {
        if self.package_manager_blocker().is_some() || self.package_removal_pending() {
            self.package_manager.close_variant_preview();
            self.variant_model_preview.close();
        }
    }
    pub(super) fn show_variant_model_preview(&mut self, ctx: &egui::Context) {
        self.sync_variant_model_preview();
        let frame = self.package_manager.variant_preview_frame();
        if self
            .variant_model_preview
            .show(ctx, self.render_state.as_ref(), frame)
        {
            self.package_manager.close_variant_preview();
        }
    }
}
