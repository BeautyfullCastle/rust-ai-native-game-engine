//! Explicit model-only body targets; global 3D selection remains single-body.
use super::{Editor, ModelPanel};
use orr_reflect::Guid;
use std::{collections::BTreeSet, path::PathBuf};

const MAX_TARGETS: usize = 128;
#[derive(Default)]
pub(super) struct Targets {
    scene: Option<PathBuf>,
    guids: BTreeSet<Guid>,
}

pub(super) fn validate_targets(editor: &Editor, guids: &[Guid]) -> Result<(), String> {
    if guids.is_empty() || guids.len() > MAX_TARGETS {
        return Err("Mark between 1 and 128 persistent model body targets".into());
    }
    let mut seen = BTreeSet::new();
    for guid in guids {
        if !seen.insert(guid) {
            return Err("Model target GUIDs must be unique".into());
        }
        let row = editor
            .rows()
            .iter()
            .find(|row| row.guid.as_ref() == Some(guid))
            .ok_or_else(|| format!("Model target {guid} is absent from the current scene"))?;
        if !editor
            .yard_frame()
            .poses
            .iter()
            .any(|(entity, _)| *entity == row.entity)
        {
            return Err(format!("Model target {guid} has no current body pose"));
        }
    }
    Ok(())
}

impl ModelPanel {
    pub(super) fn show_batch(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        ui.collapsing("Batch model targets", |ui| {
            if !self.editable(editor) { ui.label("Wait for coherent local Edit mode"); return; }
            let Ok(scene) = Self::local_scene(editor) else { return; };
            if self.batch.scene.as_ref() != Some(&scene) {
                self.batch = Targets { scene: Some(scene), ..Default::default() };
            }
            ui.weak("Mark bodies explicitly. One assignment uses the same local transform and clip for all targets; one model Undo restores all.");
            egui::ScrollArea::vertical().id_salt("model-batch-targets").max_height(220.0).show(ui, |ui| {
                for row in editor.rows() {
                    let Some(guid) = &row.guid else { continue; };
                    if !editor.yard_frame().poses.iter().any(|(entity, _)| *entity == row.entity) { continue; }
                    let mut marked = self.batch.guids.contains(guid);
                    let enabled = marked || self.batch.guids.len() < MAX_TARGETS;
                    let label = format!("Model target: {} [{guid}]", row.name.as_deref().unwrap_or("unnamed"));
                    if ui.add_enabled(enabled, egui::Checkbox::new(&mut marked, label)).changed() {
                        if marked { self.batch.guids.insert(guid.clone()); } else { self.batch.guids.remove(guid); }
                    }
                }
            });
            let missing = self.batch.guids.iter().filter(|guid| !editor.rows().iter().any(|row| row.guid.as_ref() == Some(*guid))).count();
            if missing > 0 { ui.colored_label(egui::Color32::YELLOW, format!("{missing} marked GUID(s) are missing; Clear model targets and mark again")); }
            ui.label(format!("Marked model bodies: {} / {MAX_TARGETS}", self.batch.guids.len()));
            if ui.button("Clear model targets").clicked() { self.batch.guids.clear(); }
            if ui.add_enabled(!self.batch.guids.is_empty() && missing == 0, egui::Button::new("Assign model to marked bodies")).clicked() {
                let guids: Vec<_> = self.batch.guids.iter().cloned().collect();
                self.error = self.assign_targets(editor, &guids).err();
            }
        });
    }
}
