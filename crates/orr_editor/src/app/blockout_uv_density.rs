//! Recipe-space density is an explicit draft convenience, never a recipe edit.
//! The existing Apply face UV widget owns recipe validation and history.
#![allow(clippy::float_arithmetic)]

use crate::{
    blockout::{Face, FaceUv, Recipe},
    blockout_uv_density::face_scale,
    editor::Editor,
    model::{Mode, Target},
    model_bindings::Document,
    model_panel::ModelPanel,
};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
struct Context {
    root: PathBuf,
    bindings_path: PathBuf,
    bindings: Document,
    scene: PathBuf,
    selected: Target,
    recipe_path: PathBuf,
    path_field: String,
    recipe: Recipe,
    face: Face,
    draft: FaceUv,
}

impl Context {
    fn admitted(editor: &Editor, models: &ModelPanel) -> Result<Self, String> {
        if !models.scene_matches(editor)
            || !editor.yard_rows_coherent()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.previewing().is_some()
        {
            return Err("UV density requires admitted local model bindings in Edit mode".into());
        }
        let selected = editor
            .selection()
            .ok_or("Select a scene entity for UV density")?;
        if !matches!(selected, Target::Guid(_)) || editor.row_of(selected).is_none() {
            return Err("Select an existing document entity for UV density".into());
        }
        let bindings = models
            .bindings
            .as_ref()
            .ok_or("Open model bindings first")?;
        let root = bindings.project_root()?;
        let doc = models
            .blockout
            .document
            .as_ref()
            .ok_or("New or open a box recipe first")?;
        if root != doc.root() {
            return Err("Recipe and model bindings must share the admitted project root".into());
        }
        Ok(Self {
            root,
            bindings_path: bindings.path.clone(),
            bindings: bindings.document().clone(),
            scene: editor.path().ok_or("Save the admitted local scene first")?,
            selected: selected.clone(),
            recipe_path: doc.path().to_path_buf(),
            path_field: models.blockout.path.clone(),
            recipe: doc.recipe().clone(),
            face: models.blockout.face,
            draft: models.blockout.uv,
        })
    }
}

struct Calculation {
    context: Context,
    units_text: String,
    scale: [f32; 2],
}

impl Calculation {
    fn calculate(context: Context, units_text: &str) -> Result<Self, String> {
        let units = units_text
            .trim()
            .parse::<f32>()
            .map_err(|_| "Model units per repeat must be a positive finite number")?;
        let scale = face_scale(&context.recipe, context.face, units)?;
        let mut candidate = context.recipe.clone();
        candidate.faces[context.face.index()].uv = FaceUv {
            scale,
            ..context.draft
        };
        candidate.validate()?;
        Ok(Self {
            context,
            units_text: units_text.into(),
            scale,
        })
    }

    fn staged(&self, current: &Context, units_text: &str) -> Result<FaceUv, String> {
        if &self.context != current || self.units_text != units_text {
            return Err("UV density context changed; calculate again before staging".into());
        }
        Ok(FaceUv {
            scale: self.scale,
            ..current.draft
        })
    }
}

pub(super) struct Panel {
    units: String,
    calculation: Option<Calculation>,
    status: Option<String>,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            units: "1".into(),
            calculation: None,
            status: None,
            error: None,
        }
    }
}

impl Panel {
    fn calculate(&mut self, editor: &Editor, models: &ModelPanel) {
        self.calculation = None;
        self.status = None;
        match Context::admitted(editor, models)
            .and_then(|context| Calculation::calculate(context, &self.units))
        {
            Ok(calculation) => {
                self.calculation = Some(calculation);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn stage(&mut self, editor: &Editor, models: &mut ModelPanel) {
        let result = (|| {
            let calculation = self.calculation.take().ok_or("Calculate UV scale first")?;
            let current = Context::admitted(editor, models)?;
            calculation.staged(&current, &self.units)
        })();
        self.status = None;
        match result {
            Ok(uv) => {
                // Do not call Blockout::set_uv here: no recipe/history/file/binding
                // changes are permitted until the existing Apply face UV action.
                models.blockout.uv.scale = uv.scale;
                self.error = None;
                self.status = Some(
                    "Scale staged in Face U/V scale. Use Apply face UV to update the recipe."
                        .into(),
                );
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, editor: &Editor, models: &mut ModelPanel) {
        if editor.game() != crate::game::EditorGame::Yard3D && !editor.game().is_room() {
            return;
        }
        ui.collapsing("Recipe-space UV density", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.weak("Uses box recipe dimensions; binding scale is excluded");
            ui.weak("Explicit recalculation after resizing; no automatic density lock");
            let admitted = Context::admitted(editor, models);
            if let Err(error) = &admitted {
                ui.label(error);
            }
            ui.add_enabled_ui(admitted.is_ok(), |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Model units per repeat");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.units)
                            .desired_width(80.0)
                            .char_limit(40),
                    )
                    .labelled_by(label.id);
                });
                if ui.button("Calculate UV scale").clicked() {
                    self.calculate(editor, models);
                }
                if let Some(calculation) = &self.calculation {
                    ui.label(format!(
                        "{} scale: U {} · V {}",
                        calculation.context.face.name(),
                        calculation.scale[0],
                        calculation.scale[1]
                    ));
                }
                if ui
                    .add_enabled(
                        self.calculation.is_some(),
                        egui::Button::new("Stage UV scale"),
                    )
                    .clicked()
                {
                    self.stage(editor, models);
                }
            });
            if let Some(status) = &self.status {
                ui.label(status);
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{blockout::Blockout, game::EditorGame, model_bindings::Bindings};
    use orr_reflect::Guid;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Editor, ModelPanel) {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("yard.scene.yaml");
        fs::write(
            &scene,
            include_str!("../../../../scenes/yard3d_authoring.scene.yaml"),
        )
        .unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
        for _ in 0..8 {
            editor.sync();
        }
        editor.select(Some(Target::Guid(Guid::parse("e_00000003").unwrap())));
        for _ in 0..8 {
            editor.sync();
        }
        let mut models = ModelPanel::default();
        models.bindings = Some(
            Bindings::create(
                temp.path().join("yard.models.json"),
                "yard.scene.yaml".into(),
                ".".into(),
            )
            .unwrap(),
        );
        models.blockout.document =
            Some(Blockout::create(temp.path(), "box.blockout.json").unwrap());
        (temp, editor, models)
    }

    #[test]
    fn staging_changes_only_selected_draft_scale_and_no_history_or_files() {
        let (temp, editor, mut models) = fixture();
        models
            .blockout
            .document
            .as_mut()
            .unwrap()
            .move_face(Face::PosY, 1.0)
            .unwrap();
        models
            .blockout
            .document
            .as_mut()
            .unwrap()
            .move_face(Face::PosZ, 2.0)
            .unwrap();
        models.blockout.uv = FaceUv {
            offset: [0.125, -0.25],
            scale: [0.5, 0.75],
            quarter_turn: 3,
        };
        let context = Context::admitted(&editor, &models).unwrap();
        let history = models.blockout.document.as_ref().unwrap().history_len();
        let mut panel = Panel::default();
        panel.calculate(&editor, &models);
        assert!(panel.error.is_none(), "{:?}", panel.error);
        assert_eq!(models.blockout.uv, context.draft);
        panel.stage(&editor, &mut models);
        assert!(panel.error.is_none(), "{:?}", panel.error);
        assert_eq!(
            models.blockout.uv,
            FaceUv {
                scale: [2.0, 3.0],
                ..context.draft
            }
        );
        assert_eq!(
            models.blockout.document.as_ref().unwrap().recipe(),
            &context.recipe
        );
        assert_eq!(
            models.blockout.document.as_ref().unwrap().history_len(),
            history
        );
        assert_eq!(
            models.bindings.as_ref().unwrap().document(),
            &context.bindings
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn all_context_fields_and_every_draft_uv_component_are_pinned() {
        let (_temp, editor, models) = fixture();
        let original = Context::admitted(&editor, &models).unwrap();
        let calculation = Calculation::calculate(original.clone(), "1").unwrap();
        let mut variants = Vec::new();
        macro_rules! changed {
            ($field:ident, $value:expr) => {{
                let mut next = original.clone();
                next.$field = $value;
                variants.push(next);
            }};
        }
        changed!(root, PathBuf::from("other"));
        changed!(bindings_path, PathBuf::from("other.models.json"));
        let mut bindings = original.bindings.clone();
        bindings.project = "other".into();
        changed!(bindings, bindings);
        changed!(scene, PathBuf::from("other.scene.yaml"));
        changed!(selected, Target::Guid(Guid::parse("e_00000002").unwrap()));
        changed!(recipe_path, PathBuf::from("other.blockout.json"));
        changed!(path_field, "other.blockout.json".into());
        let mut recipe = original.recipe.clone();
        recipe.bounds.max[0] = 1.5;
        changed!(recipe, recipe);
        let mut recipe = original.recipe.clone();
        recipe.faces[4].material = 1;
        changed!(recipe, recipe);
        changed!(face, Face::NegX);
        for i in 0..2 {
            let mut uv = original.draft;
            uv.offset[i] += 0.25;
            changed!(draft, uv);
            let mut uv = original.draft;
            uv.scale[i] += 0.25;
            changed!(draft, uv);
        }
        let mut uv = original.draft;
        uv.quarter_turn = 1;
        changed!(draft, uv);
        for current in variants {
            assert!(calculation.staged(&current, "1").is_err());
        }
        assert!(calculation.staged(&original, "2").is_err());
        assert_eq!(calculation.staged(&original, "1").unwrap().scale, [1.0; 2]);
    }

    #[test]
    fn stale_stage_refuses_atomically_and_requires_fresh_calculation() {
        let (_temp, editor, mut models) = fixture();
        let mut panel = Panel::default();
        panel.calculate(&editor, &models);
        models.blockout.uv.offset[1] = 0.25;
        let before = Context::admitted(&editor, &models).unwrap();
        panel.stage(&editor, &mut models);
        assert!(panel.error.as_deref().unwrap().contains("context changed"));
        assert_eq!(Context::admitted(&editor, &models).unwrap(), before);
        assert!(panel.calculation.is_none());
        panel.calculate(&editor, &models);
        panel.stage(&editor, &mut models);
        assert!(panel.error.is_none());
        assert_eq!(models.blockout.uv.offset[1], 0.25);
    }

    #[test]
    fn invalid_units_or_result_leave_draft_recipe_and_bindings_unchanged() {
        let (_temp, editor, mut models) = fixture();
        let before = Context::admitted(&editor, &models).unwrap();
        for text in ["", "x", "NaN", "inf", "0", "-1", "0.00001", "1e30"] {
            let mut panel = Panel {
                units: text.into(),
                ..Panel::default()
            };
            panel.calculate(&editor, &models);
            assert!(panel.error.is_some(), "accepted {text}");
            assert!(panel.calculation.is_none());
            panel.stage(&editor, &mut models);
            assert_eq!(Context::admitted(&editor, &models).unwrap(), before);
        }
    }

    #[test]
    fn stage_rechecks_admission_selection_root_and_open_document() {
        let (_temp, mut editor, mut models) = fixture();
        let mut panel = Panel::default();
        panel.calculate(&editor, &models);
        let draft = models.blockout.uv;
        editor.select(None);
        panel.stage(&editor, &mut models);
        assert!(panel.error.is_some());
        assert_eq!(models.blockout.uv, draft);
        editor.select(Some(Target::Guid(Guid::parse("e_00000003").unwrap())));
        panel.calculate(&editor, &models);
        assert!(editor.start_play());
        for _ in 0..8 {
            editor.sync();
        }
        panel.stage(&editor, &mut models);
        assert!(panel.error.is_some());
        assert_eq!(models.blockout.uv, draft);
        assert!(editor.stop().is_some());
        for _ in 0..8 {
            editor.sync();
        }
        let other = tempfile::tempdir().unwrap();
        models.blockout.document =
            Some(Blockout::create(other.path(), "box.blockout.json").unwrap());
        assert!(Context::admitted(&editor, &models).is_err());
        models.blockout.document = None;
        assert!(Context::admitted(&editor, &models).is_err());
    }
}
