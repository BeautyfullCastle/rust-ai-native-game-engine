//! Read-only saved preset/font preview. This does not adapt standalone game UI
//! actions into the editor's ERP Play lifecycle or alter saved-scene Restart.
use orr_sample::project::PreparedUi;

pub struct Preview {
    pub descriptor: orr_package::ProjectUi,
    pub font_bytes: usize,
}

impl Preview {
    pub fn install(prepared: PreparedUi, ctx: &egui::Context) -> Self {
        let font_bytes = prepared.font.len();
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "project-ui-preview".into(),
            egui::FontData::from_owned(prepared.font).into(),
        );
        fonts.families.insert(
            egui::FontFamily::Name("project-ui-preview".into()),
            vec!["project-ui-preview".into()],
        );
        ctx.set_fonts(fonts);
        Self {
            descriptor: prepared.descriptor,
            font_bytes,
        }
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        ui.collapsing("Saved game UI (read-only)", |ui| {
            ui.label("Preset: arena-korean-v1");
            ui.label(format!(
                "Font: {}/{} ({} bytes)",
                self.descriptor.font.package, self.descriptor.font.asset, self.font_bytes
            ));
            ui.label(
                egui::RichText::new("오러리 투기장 · 플레이 · 메뉴 · 계속하기 · 다시 시작").font(
                    egui::FontId::new(16.0, egui::FontFamily::Name("project-ui-preview".into())),
                ),
            );
            ui.label("Preview only. Edit the saved project metadata outside this panel.");
            ui.label("Editor Play controls and saved-scene Restart keep their existing behavior.");
        });
    }
}
