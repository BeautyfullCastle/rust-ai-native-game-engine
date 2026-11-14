//! Explicit clipboard export of the host's existing stopped Room recording.
use super::*;
use orr_sample::room_project::{recorded_route, PreparedProject};

#[derive(Default)]
pub(super) struct State {
    prepared: Option<Prepared>,
}
struct Prepared {
    stamp: (Option<PathBuf>, u64, u64),
    source: String,
    saved_source: String,
    recording: crate::model::Stopped,
    script: String,
}
impl Panel {
    fn route_ready(&self, editor: &Editor, blocker: Option<&str>) -> bool {
        self.import_ready(editor, blocker) && editor.last_stopped().is_some()
    }
    pub(super) fn sync_recorded_route(&mut self, editor: &Editor, blocker: Option<&str>) {
        if self.recorded_route.prepared.as_ref().is_some_and(|p| {
            !self.route_ready(editor, blocker)
                || p.stamp != (editor.path(), editor.scene_load_epoch(), editor.checksum())
                || editor.last_stopped() != Some(&p.recording)
        }) {
            self.recorded_route.prepared = None;
        }
    }
    fn route_project(editor: &Editor) -> Result<PreparedProject, String> {
        let path = editor
            .path()
            .ok_or("Save the Room project scene before preparing a route")?;
        let root = path.parent().ok_or("Room project root missing")?;
        let project = PreparedProject::open(root)?;
        if project.path() != path {
            return Err("Route export requires the saved scene at its project root".into());
        }
        Ok(project)
    }
    fn prepare_recorded_route(
        &mut self,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) -> Result<(), String> {
        self.recorded_route.prepared = None;
        if !self.route_ready(editor, blocker) {
            return Err(
                "Stop Play in an idle local saved Room project before preparing a route".into(),
            );
        }
        let project = Self::route_project(editor)?;
        let source = editor.room_import_source()?;
        // Formatting is not simulation identity. Requiring canonical document
        // equality also preserves authored names/GUIDs absent from Frame bytes.
        let saved_canonical =
            orr_reflect::Scene::parse(project.scene().text(), &orr_sample::room_project::types())
                .map_err(|e| e.to_string())?
                .to_yaml();
        let live_canonical = orr_reflect::Scene::parse(&source, &orr_sample::room_project::types())
            .map_err(|e| e.to_string())?
            .to_yaml();
        if saved_canonical != live_canonical {
            return Err("Save the current Room scene before preparing a route".into());
        }
        let recording = editor.last_stopped().ok_or("No stopped Room recording")?;
        let script = recorded_route::from_project(
            &project,
            &recording.replay,
            recording.tick,
            recording.checksum,
        )?;
        self.recorded_route.prepared = Some(Prepared {
            stamp: (editor.path(), editor.scene_load_epoch(), editor.checksum()),
            source,
            saved_source: project.scene().text().to_owned(),
            recording: recording.clone(),
            script,
        });
        Ok(())
    }
    fn copy_recorded_route(
        &mut self,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) -> Result<String, String> {
        self.sync_recorded_route(editor, blocker);
        let p = self
            .recorded_route
            .prepared
            .take()
            .ok_or("Prepare the current stopped route before Copy")?;
        let project = Self::route_project(editor)?;
        if editor.room_import_source()? != p.source || project.scene().text() != p.saved_source {
            return Err("Room source changed; prepare the route again".into());
        }
        let checked = recorded_route::from_project(
            &project,
            &p.recording.replay,
            p.recording.tick,
            p.recording.checksum,
        )?;
        if checked != p.script {
            return Err("Recorded route changed; prepare again".into());
        }
        Ok(p.script)
    }
    pub(super) fn show_recorded_route(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        blocker: Option<&str>,
    ) {
        self.sync_recorded_route(editor, blocker);
        crate::theme::header("Recorded Room regression route").show(ui, |ui| {
            ui.label("Play, then Stop. Prepare checks the existing recording against the saved project. Copy exports logical schema-1 JSON, not physical keys. Default-controls/static projects only.");
            if ui.add_enabled(self.route_ready(editor, blocker), egui::Button::new("Prepare last Room route")).clicked() {
                self.status = match self.prepare_recorded_route(editor, blocker) {
                    Ok(()) => "Recorded route verified. Copy JSON, save it as a script, then run orr_playtest_room on this project.".into(),
                    Err(e) => e,
                };
            }
            if ui.add_enabled(self.recorded_route.prepared.is_some(), egui::Button::new("Copy Room route JSON")).clicked() {
                self.status = match self.copy_recorded_route(editor, blocker) {
                    Ok(json) => { ui.ctx().copy_text(json); "Route JSON copied. No project or script file was written.".into() },
                    Err(e) => e,
                };
            }
            if ui.button("Cancel Room route").clicked() { self.recorded_route.prepared = None; }
        });
    }
}
#[cfg(test)]
mod tests;
