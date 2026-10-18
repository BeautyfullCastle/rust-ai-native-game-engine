use crate::{mesh_art::Snapshot, mesh_navigation::geometry_import};
use egui::Ui;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use std::path::Path;

const MAX_PATH_BYTES: usize = 4096;
const MAX_ASSET_BYTES: usize = 256;

pub(crate) struct Panel {
    package_root: String,
    model_asset: String,
    license_asset: String,
    license_acknowledged: bool,
    replace_confirmed: bool,
    dirty: bool,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            package_root: String::new(),
            model_asset: "model.glb".into(),
            license_asset: "LICENSE.txt".into(),
            license_acknowledged: false,
            replace_confirmed: false,
            dirty: false,
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        graph_revision: [u8; 32],
        profile: AgentProfile,
    ) -> Option<StaticMeshGraph> {
        let mut candidate = None;
        ui.collapsing("Import walkable model", |ui| {
            ui.label("Imports a verified static model as walkable geometry; source files are not modified.");
            let mut changed = false;
            changed |= field(
                ui,
                "Walkable model package root",
                &mut self.package_root,
                MAX_PATH_BYTES,
            );
            changed |= field(
                ui,
                "Walkable model asset",
                &mut self.model_asset,
                MAX_ASSET_BYTES,
            );
            changed |= field(
                ui,
                "Walkable model license asset",
                &mut self.license_asset,
                MAX_ASSET_BYTES,
            );
            changed |= ui
                .checkbox(
                    &mut self.license_acknowledged,
                    "I have reviewed the rights to use this model license",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut self.replace_confirmed,
                    "I confirm replacing staged geometry with this imported walkable surface",
                )
                .changed();
            self.dirty |= changed;

            let can_import = self.license_acknowledged
                && self.replace_confirmed
                && !self.package_root.trim().is_empty()
                && !self.model_asset.trim().is_empty()
                && !self.license_asset.trim().is_empty();
            if ui
                .add_enabled(
                    can_import,
                    egui::Button::new("Stage imported walkable surface"),
                )
                .clicked()
            {
                let result = Snapshot::from_package(
                    Path::new(self.package_root.trim()),
                    self.model_asset.trim(),
                    self.license_asset.trim(),
                    graph_revision,
                )
                .and_then(|snapshot| {
                    geometry_import::candidate(&snapshot, self.license_acknowledged, profile)
                });
                match result {
                    Ok(graph) => {
                        self.error = None;
                        candidate = Some(graph);
                    }
                    Err(error) => {
                        self.error = Some(error);
                    }
                }
            }
            if let Some(error) = self.error.as_deref() {
                ui.colored_label(egui::Color32::RED, error);
            }
            if ui.button("Reset walkable model import form").clicked() {
                self.reset();
                candidate = None;
            }
        });
        candidate
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn acknowledge(&mut self) {
        self.dirty = false;
        self.license_acknowledged = false;
        self.replace_confirmed = false;
        self.error = None;
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, max_bytes: usize) -> bool {
    let label_response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .char_limit(max_bytes),
    )
    .labelled_by(label_response.id)
    .changed()
}

#[cfg(test)]
#[path = "geometry_import_panel/tests.rs"]
mod tests;
