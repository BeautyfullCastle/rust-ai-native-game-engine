//! Parsed summary is a draft. Only Apply submits the admitted undo transaction.
use super::*;

#[derive(Default)]
pub(super) struct State {
    text: String,
    preview: Option<Preview>,
}
struct Preview {
    text: String,
    stamp: (Option<PathBuf>, u64, u64),
    wall_count: usize,
    source: String,
}
impl Panel {
    pub(super) fn import_ready(&self, editor: &Editor, blocker: Option<&str>) -> bool {
        self.open
            && blocker.is_none()
            && editor.game().is_room()
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && !editor.in_gesture()
            && editor.path().is_some()
            && !self.actor_placement_busy()
            && !self.wall_placement_busy()
            && !self.wall_footprint_busy()
    }
    pub(super) fn sync_layout_import(&mut self, editor: &Editor, blocker: Option<&str>) {
        let stamp = (editor.path(), editor.scene_load_epoch(), editor.checksum());
        if self.layout_import.preview.as_ref().is_some_and(|preview| {
            preview.text != self.layout_import.text
                || preview.stamp != stamp
                || !self.import_ready(editor, blocker)
        }) {
            self.layout_import.preview = None;
        }
    }
    fn preview_layout_import(
        &mut self,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) -> Result<(), String> {
        self.layout_import.preview = None;
        if !self.import_ready(editor, blocker) {
            return Err(blocker
                .unwrap_or("Import requires an idle local saved Room; finish placement first")
                .into());
        }
        let recipe = orr_sample::room_layout::Recipe::parse(self.layout_import.text.as_bytes())?;
        let source = editor.room_import_source()?;
        self.layout_import.preview = Some(Preview {
            text: self.layout_import.text.clone(),
            stamp: (editor.path(), editor.scene_load_epoch(), editor.checksum()),
            wall_count: recipe.walls.len(),
            source,
        });
        Ok(())
    }
    fn apply_layout_import(
        &mut self,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) -> Result<(), String> {
        self.sync_layout_import(editor, blocker);
        let preview = self
            .layout_import
            .preview
            .take()
            .ok_or("Preview the current recipe before Apply")?;
        editor.import_room_layout(&preview.text, &preview.stamp, &preview.source)?;
        self.walls_current = false;
        self.confirm_remove_wall = false;
        Ok(())
    }
    pub(super) fn show_layout_import(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) {
        self.sync_layout_import(editor, blocker);
        crate::theme::header("Import Room layout recipe").show(ui, |ui| {
            ui.label("Paste schema-1 JSON (at most 64 KiB). Replaces all walls and moves PLAYER, KEY and EXIT.");
            ui.add(egui::TextEdit::multiline(&mut self.layout_import.text)
                .id_salt("room_layout_recipe").char_limit(orr_sample::room_layout::MAX_BYTES)
                .desired_rows(5).desired_width(f32::INFINITY));
            self.sync_layout_import(editor, blocker);
            ui.horizontal(|ui| {
                if ui.add_enabled(self.import_ready(editor, blocker), egui::Button::new("Preview layout recipe")).clicked() {
                    self.status = match self.preview_layout_import(editor, blocker) {
                        Ok(()) => "Parsed summary only. Apply checks scene admission; test a route after Save.".into(),
                        Err(error) => error,
                    };
                }
                if ui.add_enabled(self.layout_import.preview.is_some(), egui::Button::new("Apply imported layout")).clicked() {
                    self.status = match self.apply_layout_import(editor, blocker) {
                        Ok(()) => "Imported as one undoable edit. Save the scene to keep it.".into(),
                        Err(error) => error,
                    };
                }
                if ui.button("Cancel layout import").clicked() {
                    self.layout_import.preview = None;
                    self.status = "Import canceled; pasted JSON retained. Scene unchanged.".into();
                }
            });
            if let Some(preview) = &self.layout_import.preview {
                ui.label(format!("Summary: move 3 actors, replace all walls with {} boxes. No changes yet.", preview.wall_count));
            }
            if let Some(reason) = blocker { ui.label(reason); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RECIPE: &str = r#"{"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],"walls":[{"center":["0","0"],"half_extents":["0.25","1"]}]}"#;
    fn panel() -> Panel {
        Panel {
            open: true,
            layout_import: State {
                text: RECIPE.into(),
                preview: None,
            },
            ..Default::default()
        }
    }
    #[test]
    fn draft_close_blocker_and_stale_source_invalidate_preview() {
        let (_temp, _root, mut editor) = Editor::room_import_fixture();
        let mut panel = panel();
        let original = editor.checksum();
        let history = editor.history().entries.len();
        for change in 0..6 {
            panel.preview_layout_import(&mut editor, None).unwrap();
            let blocker = match change {
                0 => {
                    panel.layout_import.text.push(' ');
                    None
                }
                1 => {
                    panel.open = false;
                    None
                }
                2 => Some("Unsaved model bindings"),
                3 => {
                    panel.layout_import.preview.as_mut().unwrap().stamp.0 =
                        Some(PathBuf::from("elsewhere"));
                    None
                }
                4 => {
                    panel.layout_import.preview.as_mut().unwrap().stamp.1 += 1;
                    None
                }
                _ => {
                    panel.layout_import.preview.as_mut().unwrap().stamp.2 ^= 1;
                    None
                }
            };
            assert!(panel.apply_layout_import(&mut editor, blocker).is_err());
            assert!(panel.layout_import.preview.is_none());
            panel.open = true;
        }
        assert_eq!(editor.checksum(), original);
        assert_eq!(editor.history().entries.len(), history);
        assert!(panel
            .preview_layout_import(&mut editor, Some("Unsaved model bindings"))
            .is_err());
    }
    #[test]
    fn actual_widgets_preview_cancel_apply_once_and_disable_for_model_edits() {
        use egui::accesskit::Role;
        use egui_kittest::{
            kittest::{NodeT, Queryable},
            Harness,
        };
        let (_temp, _root, editor) = Editor::room_import_fixture();
        let original = editor.checksum();
        let history = editor.history().entries.len();
        let mut harness = Harness::builder()
            .with_size([1000.0, 1000.0])
            .build_ui_state(
                |ui, state: &mut (Editor, Panel, Option<&'static str>)| {
                    state.0.sync();
                    state.1.show_layout_import(ui, &mut state.0, state.2);
                },
                (editor, panel(), None),
            );
        harness.run_steps(3);
        harness.get_by_label("Import Room layout recipe").click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Preview layout recipe")
            .click();
        harness.run_steps(3);
        assert!(harness.state().1.layout_import.preview.is_some());
        assert_eq!(harness.state().0.checksum(), original);
        harness
            .get_by_role_and_label(Role::Button, "Cancel layout import")
            .click();
        harness.run_steps(3);
        assert!(harness.state().1.layout_import.preview.is_none());
        assert_eq!(harness.state().1.layout_import.text, RECIPE);
        harness
            .get_by_role_and_label(Role::Button, "Preview layout recipe")
            .click();
        harness.run_steps(3);
        harness.state_mut().2 = Some("Unsaved model bindings");
        harness.run_steps(2);
        assert!(harness
            .get_by_role_and_label(Role::Button, "Apply imported layout")
            .accesskit_node()
            .is_disabled());
        harness.state_mut().2 = None;
        harness.run_steps(2);
        harness
            .get_by_role_and_label(Role::Button, "Preview layout recipe")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Apply imported layout")
            .click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.history().entries.len(), history + 1);
        assert_ne!(harness.state().0.checksum(), original);
        assert!(harness
            .get_by_role_and_label(Role::Button, "Apply imported layout")
            .accesskit_node()
            .is_disabled());
        assert!(harness.state_mut().0.undo());
        harness.run_steps(3);
        assert_eq!(harness.state().0.checksum(), original);
        assert!(harness.state_mut().0.redo());
        harness.run_steps(3);
        assert_ne!(harness.state().0.checksum(), original);
    }
}
