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
    /// Remove existing bindings on explicit current body GUIDs in one history
    /// step. Valid unbound bodies are no-ops; missing bodies reject the batch.
    pub fn remove_targets(&mut self, editor: &Editor, guids: &[Guid]) -> Result<usize, String> {
        if !self.editable(editor) {
            return Err("Model removal requires coherent local Edit mode".into());
        }
        validate_targets(editor, guids)?;
        let bindings = self.bindings.as_ref().ok_or("Open model bindings first")?;
        let mut candidate = bindings.document().clone();
        let mut removed = 0;
        for guid in guids {
            if candidate.bindings.remove(&guid.to_string()).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(0);
        }
        let mut loaded = self.loaded.clone();
        loaded.retain(|guid, _| candidate.bindings.contains_key(guid));
        Self::validate_candidate(editor, &candidate, &loaded, self.reserved_scene_models)?;
        self.bindings
            .as_mut()
            .ok_or("Open model bindings first")?
            .remove(guids)?;
        self.loaded = loaded;
        self.retire_character_playback();
        Ok(removed)
    }
    pub(super) fn show_batch(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        crate::theme::collapsing(ui, "Batch model targets", |ui| {
            if !self.editable(editor) { ui.label("Wait for coherent local Edit mode"); return; }
            let Ok(scene) = Self::local_scene(editor) else { return; };
            if self.batch.scene.as_ref() != Some(&scene) {
                self.batch = Targets { scene: Some(scene), ..Default::default() };
            }
            ui.weak("Mark bodies explicitly. Assign uses the same local transform and clip for all targets. Assign or Remove is one model Undo step; bodies and source assets stay intact.");
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
            let remove_count = self.batch.guids.iter().filter(|guid| self.bindings.as_ref().is_some_and(|bindings| bindings.document().bindings.contains_key(&guid.to_string()))).count();
            ui.label(format!("Marked model bindings to remove: {remove_count}"));
            if ui.add_enabled(remove_count > 0 && missing == 0, egui::Button::new("Remove models from marked bodies")).clicked() {
                let guids: Vec<_> = self.batch.guids.iter().cloned().collect();
                self.error = self.remove_targets(editor, &guids).map(|_| ()).err();
            }
        });
    }
}
