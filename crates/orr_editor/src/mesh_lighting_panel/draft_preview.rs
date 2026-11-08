//! Explicit, view-only point-light draft preview. Invalid observations retire opt-in.
use super::{Document, Editor, EditorGame, Panel};
use orr_render::PointLightSettings;
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    profile: EditorGame,
    scene: PathBuf,
    scene_epoch: u64,
    graph_revision: [u8; 32],
    host_epoch: u64,
}
#[derive(Default)]
pub(super) struct State {
    context: Option<Context>,
}
impl State {
    fn sample(
        &mut self,
        current: Option<Context>,
        candidate: Result<Document, String>,
    ) -> Option<PointLightSettings> {
        self.context.as_ref()?;
        if current.as_ref() != self.context.as_ref() {
            self.context = None;
            return None;
        }
        let settings = candidate.and_then(|d| d.point_light_settings());
        match settings {
            Ok(settings) => Some(settings),
            Err(_) => {
                self.context = None;
                None
            }
        }
    }
}
impl Panel {
    fn preview_context(&self, editor: &Editor) -> Option<Context> {
        if !self.can_edit(editor) {
            return None;
        }
        Some(Context {
            profile: editor.game(),
            scene: self.scene.clone()?,
            scene_epoch: editor.scene_load_epoch(),
            graph_revision: self.graph_revision?,
            host_epoch: editor.sim().epoch,
        })
    }
    fn preview_candidate(&self) -> Result<Document, String> {
        self.parse_document(
            self.scene.as_deref().ok_or("Missing mesh scene")?,
            self.graph_revision.ok_or("Missing admitted graph")?,
        )
    }
    pub(crate) fn point_light_preview_enabled(&self) -> bool {
        self.draft_preview.context.is_some()
    }
    pub(crate) fn set_point_light_preview(
        &mut self,
        editor: &Editor,
        enabled: bool,
    ) -> Result<(), String> {
        self.draft_preview.context = None;
        if !enabled {
            return Ok(());
        }
        let context = self
            .preview_context(editor)
            .ok_or("Point light preview requires the matching idle local Edit scene")?;
        let candidate = self.preview_candidate()?;
        candidate.point_light_settings()?;
        self.draft_preview.context = Some(context);
        Ok(())
    }
    pub(crate) fn preview_point_light_settings(
        &mut self,
        editor: &Editor,
    ) -> Option<PointLightSettings> {
        if !self.point_light_preview_enabled() {
            return None;
        }
        let context = self.preview_context(editor);
        let candidate = self.preview_candidate();
        self.draft_preview.sample(context, candidate)
    }
    pub(crate) fn sync_preview(&mut self, editor: &Editor) {
        let _ = self.preview_point_light_settings(editor);
    }
}
#[cfg(test)]
#[path = "draft_preview_tests.rs"]
mod tests;
