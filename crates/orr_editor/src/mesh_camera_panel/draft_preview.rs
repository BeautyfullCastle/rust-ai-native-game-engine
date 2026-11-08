//! Render-local static camera preview; never changes the editor orbit or saved policy.
use super::{Document, Editor, EditorGame, Panel};
use orr_render::Camera3D;
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
    busy: bool,
}
fn camera(document: &Document, size: (u32, u32)) -> Result<Camera3D, String> {
    if document.follow_offset.is_some() {
        return Err("Turn Follow mesh agent off before static camera preview".into());
    }
    document.camera(&document.orbit(), size)
}
impl State {
    fn sample(
        &mut self,
        current: Option<Context>,
        candidate: Result<Document, String>,
        size: (u32, u32),
    ) -> Option<Camera3D> {
        self.context.as_ref()?;
        if self.context.as_ref() != current.as_ref() {
            self.context = None;
            return None;
        }
        match candidate.and_then(|document| camera(&document, size)) {
            Ok(camera) => Some(camera),
            Err(_) => {
                self.context = None;
                None
            }
        }
    }
}
impl Panel {
    pub(crate) fn set_static_preview_busy(&mut self, busy: bool) {
        self.draft_preview.busy = busy;
    }
    pub(crate) fn static_preview_busy(&self) -> bool {
        self.draft_preview.busy
    }
    pub(crate) fn clear_static_preview(&mut self) {
        self.draft_preview.context = None;
    }
    pub(crate) fn static_preview_enabled(&self) -> bool {
        self.draft_preview.context.is_some()
    }
    fn static_preview_context(&self, editor: &Editor) -> Option<Context> {
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
    fn static_preview_candidate(&self) -> Result<Document, String> {
        self.parse_form(
            self.scene.as_deref().ok_or("Missing mesh scene")?,
            self.graph_revision.ok_or("Missing admitted graph")?,
        )
    }
    pub(crate) fn set_static_preview(
        &mut self,
        editor: &Editor,
        enabled: bool,
    ) -> Result<(), String> {
        if !enabled {
            self.clear_static_preview();
            return Ok(());
        }
        if self.static_preview_busy() {
            return Err(
                "Finish other viewport tools and release input before camera preview".into(),
            );
        }
        self.clear_static_preview();
        let context = self
            .static_preview_context(editor)
            .ok_or("Camera preview requires the matching idle local Edit scene")?;
        camera(&self.static_preview_candidate()?, (1, 1))?;
        self.draft_preview.context = Some(context);
        Ok(())
    }
    pub(crate) fn static_preview_camera(
        &mut self,
        editor: &Editor,
        size: (u32, u32),
    ) -> Option<Camera3D> {
        if !self.static_preview_enabled() {
            return None;
        }
        let context = self.static_preview_context(editor);
        let candidate = self.static_preview_candidate();
        self.draft_preview.sample(context, candidate, size)
    }
    pub(crate) fn sync_static_preview(&mut self, editor: &Editor) {
        let _ = self.static_preview_camera(editor, (1, 1));
    }
}
#[cfg(test)]
#[path = "draft_preview_tests.rs"]
mod tests;
