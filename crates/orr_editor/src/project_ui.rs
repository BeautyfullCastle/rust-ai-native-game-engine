//! Read-only saved preset/font preview. This does not adapt standalone game UI
//! actions into the editor's ERP Play lifecycle or alter saved-scene Restart.
use orr_sample::project::PreparedUi;

pub struct Preview {
    pub descriptor: orr_package::ProjectUi,
    pub font_bytes: usize,
    // Attests installation input, not later arbitrary egui context mutations.
    #[cfg(all(
        feature = "arena-project-export",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    installed: Option<orr_sample::project_export::ArenaUiSource>,
}

impl Preview {
    pub fn install(prepared: PreparedUi, ctx: &egui::Context) -> Self {
        let font_bytes = prepared.font.len();
        #[cfg(all(
            feature = "arena-project-export",
            target_os = "linux",
            target_arch = "x86_64"
        ))]
        let installed = {
            use sha2::{Digest, Sha256};
            orr_sample::project_export::ArenaUiSource::new(
                prepared.descriptor.clone(),
                font_bytes as u64,
                Sha256::digest(&prepared.font)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
            .ok()
        };
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
            #[cfg(all(
                feature = "arena-project-export",
                target_os = "linux",
                target_arch = "x86_64"
            ))]
            installed,
        }
    }

    #[cfg(all(
        feature = "arena-project-export",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn arena_export_identity(
        &self,
    ) -> Result<&orr_sample::project_export::ArenaUiSource, String> {
        let installed = self
            .installed
            .as_ref()
            .ok_or("Loaded UI has no valid font installation identity; reopen explicitly")?;
        if installed.descriptor() != &self.descriptor
            || installed.font_bytes() != self.font_bytes as u64
        {
            return Err(
                "Loaded UI descriptor or length differs from its font installation identity".into(),
            );
        }
        Ok(installed)
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

#[cfg(all(
    test,
    feature = "arena-project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn prepared() -> PreparedUi {
        PreparedUi {
            descriptor: orr_package::ProjectUi {
                profile: orr_package::ProjectUiProfile::ArenaKoreanV1,
                font: orr_package::ProjectFont {
                    package: "korean-game-ui".into(),
                    asset: "OrreryKoreanUI.otf".into(),
                },
                document: None,
            },
            font: include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
        }
    }
    #[test]
    fn arena_installation_identity_is_exact_and_public_metadata_cannot_launder_it() {
        let p = prepared();
        let ctx = egui::Context::default();
        let mut preview = Preview::install(p.clone(), &ctx);
        let identity = preview.arena_export_identity().unwrap();
        assert_eq!(identity.descriptor(), &p.descriptor);
        assert_eq!(identity.font_bytes(), p.font.len() as u64);
        assert_eq!(
            identity.font_sha256(),
            Sha256::digest(&p.font)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        preview.descriptor.font.asset = "different.otf".into();
        assert!(preview.arena_export_identity().is_err());
        preview.descriptor = p.descriptor;
        preview.font_bytes += 1;
        assert!(preview.arena_export_identity().is_err());
        preview.font_bytes -= 1;
        preview.installed = None;
        assert!(preview.arena_export_identity().is_err());
    }
    #[test]
    fn arena_installation_empty_oversized_and_wrong_document_have_no_identity() {
        for size in [0, 16 * 1024 * 1024 + 1] {
            let mut p = prepared();
            p.font.resize(size, 0);
            let preview = Preview::install(p, &egui::Context::default());
            assert!(preview.arena_export_identity().is_err());
        }
        let mut p = prepared();
        p.descriptor.document = Some("ui.json".into());
        assert!(Preview::install(p, &egui::Context::default())
            .arena_export_identity()
            .is_err());
    }
    #[test]
    fn arena_installation_valid_font_above_scene_bound_remains_supported() {
        let mut p = prepared();
        p.font.resize(4 * 1024 * 1024 + 1, 0);
        orr_sample::game_ui::GameUi::validate_font(&p.font).unwrap();
        let preview = Preview::install(p, &egui::Context::default());
        assert_eq!(
            preview.arena_export_identity().unwrap().font_bytes(),
            4 * 1024 * 1024 + 1
        );
    }
}
