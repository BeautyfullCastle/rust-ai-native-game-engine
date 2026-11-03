//! Narrow actor authoring without replacing the shared atlas set or compositor.
use super::*;
use crate::sprite_bindings::{Bindings, Document, Source};
use orr_reflect::Guid;

#[derive(Default)]
pub(super) struct State {
    selection: Vec<Guid>,
    source: Option<Source>,
}

/// Compare the complete admitted document with only Source substituted. This
/// also pins GUID membership, count, document version, scale, orientation/follow.
fn same_actor_contract(admitted: &Document, candidate: &Document) -> bool {
    let mut source_only = admitted.clone();
    for (guid, binding) in &mut source_only.bindings {
        let Some(next) = candidate.bindings.get(guid) else {
            return false;
        };
        binding.source.clone_from(&next.source);
    }
    &source_only == candidate
}

impl TilemapPanel {
    fn check_actor_document(
        &self,
        sprites: &SpritePanel,
        candidate: &Bindings,
    ) -> Result<(), String> {
        let (path, bytes) = self
            .stamp
            .actors
            .as_ref()
            .ok_or("project has no actor sidecar")?;
        if !sprites.tilemap_is_pinned() || &candidate.path != path {
            return Err("actor bindings must belong to the admitted tilemap project".into());
        }
        let admitted = Document::from_bytes(bytes)?;
        if !same_actor_contract(&admitted, candidate.document())
            || crate::sprite_bindings::resolve_project(
                candidate.base(),
                &candidate.document().project,
            )? != self.stamp.root
            || std::fs::canonicalize(candidate.base().join(&candidate.document().scene))
                .map_err(|e| e.to_string())?
                != self.stamp.scene
        {
            return Err(
                "actor appearance may change only Source on existing admitted bindings".into(),
            );
        }
        candidate.document().validate()?;
        for binding in candidate.document().bindings.values() {
            let key = (binding.package.clone(), binding.document.clone());
            let (asset, _) = sprites
                .tilemap_asset(&key)
                .ok_or("admitted actor atlas missing")?;
            binding.region(&asset.document, 0)?;
        }
        Self::budget(&self.history.current, candidate.document().bindings.len())?;
        Ok(())
    }

    fn actors_editable(&self, editor: &Editor) -> bool {
        self.editable(editor) && !self.painting && self.rectangle.is_none()
    }

    /// Verify actual installed files, not just an unchanged lock or decoded cache.
    fn check_actor_inputs(&self, editor: &Editor, sprites: &SpritePanel) -> Result<(), String> {
        if !self.actors_editable(editor) {
            return Err("actor appearance requires the matching editable Arena project without an active tile gesture".into());
        }
        self.check_stamp()?;
        self.check_actor_document(
            sprites,
            sprites
                .bindings
                .as_ref()
                .ok_or("project has no actor bindings")?,
        )?;
        orr_package::Project::open(&self.stamp.root, crate::sprite_bindings::compiled_runtime())
            .and_then(|project| project.verify())
            .map_err(|e| format!("installed actor assets: {e}"))?;
        self.check_stamp()
    }

    fn selected_actor_key(
        &self,
        editor: &Editor,
        sprites: &SpritePanel,
    ) -> Result<AssetKey, String> {
        if editor.selected_guids().is_empty() {
            return Err("Select one or more already-bound actors in the scene.".into());
        }
        let bindings = sprites
            .bindings
            .as_ref()
            .ok_or("project has no actor bindings")?;
        let mut common = None;
        for guid in editor.selected_guids() {
            let row = editor
                .rows()
                .iter()
                .find(|row| row.guid.as_ref() == Some(guid))
                .ok_or("selected actor is absent from the current scene")?;
            if !editor.bodies().iter().any(|body| body.entity == row.entity) {
                return Err("selected actor has no displayed sprite body".into());
            }
            let binding = bindings
                .document()
                .bindings
                .get(guid.as_str())
                .ok_or("selected entity has no admitted actor binding")?;
            let key = (binding.package.clone(), binding.document.clone());
            if common.as_ref().is_some_and(|previous| previous != &key) {
                return Err("Select actors that share the same admitted sprite document.".into());
            }
            common = Some(key);
        }
        common.ok_or_else(|| "select an already-bound actor".into())
    }

    pub(crate) fn set_actor_source(
        &mut self,
        editor: &Editor,
        sprites: &mut SpritePanel,
        source: Source,
    ) -> Result<bool, String> {
        self.check_actor_inputs(editor, sprites)?;
        self.selected_actor_key(editor, sprites)?;
        let mut candidate = sprites.bindings.as_ref().unwrap().clone();
        candidate.set_existing_sources(editor.selected_guids(), source)?;
        self.install_actor_candidate(editor, sprites, candidate)
    }

    fn install_actor_candidate(
        &self,
        editor: &Editor,
        sprites: &mut SpritePanel,
        candidate: Bindings,
    ) -> Result<bool, String> {
        self.check_actor_document(sprites, &candidate)?;
        // Recheck after candidate validation and before changing any history.
        self.check_actor_inputs(editor, sprites)?;
        let changed = candidate.document() != sprites.bindings.as_ref().unwrap().document();
        if changed {
            sprites.bindings = Some(candidate);
        }
        Ok(changed)
    }

    pub(crate) fn undo_actor_source(
        &mut self,
        editor: &Editor,
        sprites: &mut SpritePanel,
    ) -> Result<bool, String> {
        self.check_actor_inputs(editor, sprites)?;
        let mut candidate = sprites.bindings.as_ref().unwrap().clone();
        candidate.undo();
        self.install_actor_candidate(editor, sprites, candidate)
    }

    pub(crate) fn redo_actor_source(
        &mut self,
        editor: &Editor,
        sprites: &mut SpritePanel,
    ) -> Result<bool, String> {
        self.check_actor_inputs(editor, sprites)?;
        let mut candidate = sprites.bindings.as_ref().unwrap().clone();
        candidate.redo();
        self.install_actor_candidate(editor, sprites, candidate)
    }

    pub(crate) fn save_actors(
        &mut self,
        editor: &Editor,
        sprites: &mut SpritePanel,
    ) -> Result<(), String> {
        self.save_actors_with(editor, sprites, |_, _| Ok(()))
    }

    fn save_actors_with(
        &mut self,
        editor: &Editor,
        sprites: &mut SpritePanel,
        mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check_actor_inputs(editor, sprites)?;
        let bindings = sprites.bindings.as_ref().unwrap();
        if !bindings.memory_dirty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(bindings.document()).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > crate::sprite_bindings::MAX_BYTES {
            return Err("actor sidecar exceeds byte limit".into());
        }
        let path = bindings.path.clone();
        // Publication follows tilemap Save's trusted, quiescent filesystem
        // contract. The staged file is flushed, then all old inputs are checked.
        let mut temp =
            tempfile::NamedTempFile::new_in(bindings.base()).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        checkpoint("staged", &path)?;
        self.check_actor_inputs(editor, sprites)?;
        temp.persist(&path).map_err(|e| e.to_string())?;
        // A failure after persist cannot truthfully promise rollback. Do not
        // advance either marker until the published bytes have been observed.
        let published = checkpoint("published", &path).and_then(|_| {
            let observed = regular(&path, crate::sprite_bindings::MAX_BYTES as usize)?;
            if observed != bytes {
                return Err("published actor bytes differ".into());
            }
            Ok(())
        });
        if let Err(error) = published {
            return Err(format!("actor sidecar was published but verification failed ({error}); outcome uncertain, reopen full project before further edits"));
        }
        self.stamp.actors.as_mut().unwrap().1 = bytes;
        sprites.bindings.as_mut().unwrap().mark_tilemap_saved();
        Ok(())
    }

    pub(super) fn show_actors(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        sprites: &mut SpritePanel,
    ) {
        ui.separator();
        ui.heading("Actor appearance");
        ui.weak("Existing actors only: change Region or Clip within their admitted atlas. Scale, orientation, follow, actor count and tile rendering stay unchanged.");
        let Some(bindings) = &sprites.bindings else {
            ui.label("This project has no actor sidecar.");
            return;
        };
        ui.label(if bindings.dirty() {
            "Unsaved actor appearances: Save or Undo before Play, export, switching or closing."
        } else {
            "Saved actor appearances"
        });
        let (undo, redo) = bindings.history_available();
        if self.actors.selection != editor.selected_guids() {
            self.actors.selection = editor.selected_guids().to_vec();
            self.actors.source = editor
                .selected_guids()
                .first()
                .and_then(|guid| bindings.document().bindings.get(guid.as_str()))
                .map(|binding| binding.source.clone())
                .filter(|source| !matches!(source, Source::Locomotion { .. }))
                .filter(|source| {
                    editor.selected_guids().iter().all(|guid| {
                        bindings
                            .document()
                            .bindings
                            .get(guid.as_str())
                            .is_some_and(|binding| &binding.source == source)
                    })
                });
        }
        let editable = self.actors_editable(editor);
        ui.add_enabled_ui(editable, |ui| {
            match self.selected_actor_key(editor, sprites) {
                Ok(key) => {
                    ui.label(format!(
                        "{} selected · {}/{}",
                        editor.selected_guids().len(),
                        key.0,
                        key.1
                    ));
                    if let Some((asset, _)) = sprites.tilemap_asset(&key) {
                        ui.horizontal_wrapped(|ui| {
                            for region in asset.document.regions() {
                                ui.selectable_value(
                                    &mut self.actors.source,
                                    Some(Source::Region(region.id)),
                                    format!("Actor region {}", region.id),
                                );
                            }
                            for clip in asset.document.clips() {
                                ui.selectable_value(
                                    &mut self.actors.source,
                                    Some(Source::Clip(clip.id().into())),
                                    format!("Actor clip {}", clip.id()),
                                );
                            }
                        });
                    }
                    if ui
                        .add_enabled(
                            self.actors.source.is_some(),
                            egui::Button::new("Apply actor source"),
                        )
                        .clicked()
                    {
                        self.error = self
                            .set_actor_source(editor, sprites, self.actors.source.clone().unwrap())
                            .err();
                    }
                }
                Err(message) => {
                    ui.label(message);
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(undo, egui::Button::new("Undo actor source"))
                    .clicked()
                {
                    self.error = self.undo_actor_source(editor, sprites).err();
                }
                if ui
                    .add_enabled(redo, egui::Button::new("Redo actor source"))
                    .clicked()
                {
                    self.error = self.redo_actor_source(editor, sprites).err();
                }
                if ui.button("Save actor appearances").clicked() {
                    self.error = self.save_actors(editor, sprites).err();
                }
            });
        });
        if !editable {
            ui.label("Actor appearance requires matching Edit viewport without proposal, replay, disconnected host or active tile gesture.");
        }
    }
}

#[cfg(all(test, feature = "project-create", target_os = "linux"))]
#[path = "actors_tests.rs"]
mod tests;
