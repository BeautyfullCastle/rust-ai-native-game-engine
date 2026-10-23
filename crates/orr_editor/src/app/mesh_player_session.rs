use std::path::PathBuf;

use crate::{editor::Editor, game::EditorGame, Mode};
use egui::{Button, Ui, Window};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene_path: PathBuf,
    scene_epoch: u64,
}

#[derive(Default)]
pub(crate) struct State {
    dialog_open: bool,
    context: Option<Context>,
    error: Option<String>,
}

impl State {
    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog_open
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        editor: &mut Editor,
        recorded: &mut super::mesh_recorded_playback::State,
    ) -> bool {
        if self.dialog_open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel_dialog();
        }
        if self.dialog_open && !self.context_matches(editor) {
            self.close_with_error("Restart confirmation closed because the game session changed");
        }

        let can_restart = Self::eligible(editor) && !self.dialog_open;
        if ui
            .add_enabled(can_restart, Button::new("Restart game…"))
            .clicked()
        {
            self.open_dialog(editor, recorded);
        }

        let mut confirmed = false;
        let mut canceled = false;
        if self.dialog_open {
            let mut window_open = true;
            Window::new("Restart the current game?")
                .collapsible(false)
                .resizable(false)
                .open(&mut window_open)
                .show(ui.ctx(), |ui| {
                    ui.label("The current play session will stop and restart from the authored game state.");
                    ui.horizontal(|ui| {
                        if ui.button("Restart game").clicked() {
                            confirmed = true;
                        }
                        if ui.button("Cancel restart").clicked() {
                            canceled = true;
                        }
                    });
                });
            if canceled || !window_open {
                self.cancel_dialog();
            }
        }

        let restarted = confirmed && self.confirm_restart(editor);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        restarted
    }

    fn open_dialog(
        &mut self,
        editor: &mut Editor,
        recorded: &mut super::mesh_recorded_playback::State,
    ) {
        self.error = None;
        if !Self::eligible(editor) {
            self.error =
                Some("Restart requires a mutable local MeshNavigation Play session".into());
            return;
        }
        editor.set_mesh_play_input_enabled(false);
        if recorded.active() {
            recorded.pause(editor);
        } else {
            editor.pause();
        }
        let Some(context) = Self::current_context(editor) else {
            self.error = Some("Restart requires a saved local MeshNavigation scene".into());
            return;
        };
        if !Self::eligible(editor) {
            self.error = Some("The game session changed while opening restart confirmation".into());
            return;
        }
        self.context = Some(context);
        self.dialog_open = true;
    }

    fn cancel_dialog(&mut self) {
        self.dialog_open = false;
        self.context = None;
        self.error = None;
    }

    fn confirm_restart(&mut self, editor: &mut Editor) -> bool {
        if !self.dialog_open || !self.context_matches(editor) {
            self.close_with_error("Restart refused because the game session changed");
            return false;
        }
        editor.set_mesh_play_input_enabled(false);
        if editor.stop().is_none() {
            editor.sync();
            self.close_with_error(Self::editor_error(
                editor,
                "Could not stop the current game",
            ));
            return false;
        }
        editor.sync();
        if editor.mode() != Mode::Edit {
            self.close_with_error("The game did not return to its authored scene after stopping");
            return false;
        }
        let stopped = editor.last_stopped().cloned();
        if !editor.start_play() {
            let error = Self::editor_error(editor, "Could not restart the current game");
            editor.sync();
            self.close_with_error(error);
            return false;
        }
        editor.set_mesh_play_input_enabled(false);
        editor.sync();
        if let Err(error) = Self::validate_fresh_session(editor, stopped.as_ref()) {
            self.close_with_error(error);
            return false;
        }
        self.dialog_open = false;
        self.context = None;
        self.error = None;
        true
    }

    fn validate_fresh_session(
        editor: &Editor,
        stopped: Option<&crate::model::Stopped>,
    ) -> Result<(), String> {
        if editor.game() != EditorGame::MeshNavigation
            || editor.mode() != Mode::Play
            || editor.sim().head_tick != 0
            || editor.sim().playing
        {
            return Err("The restarted game was not paused at tick zero".into());
        }
        let snapshot = editor
            .snapshot()
            .ok_or_else(|| "The restarted game has no published frame".to_string())?;
        let timeline = snapshot
            .timeline()
            .ok_or_else(|| "The restarted game has no published timeline".to_string())?;
        if snapshot.tick() != 0
            || timeline.tick != 0
            || timeline.playing
            || timeline.epoch != editor.sim().epoch
        {
            return Err("The restarted game has no coherent paused tick-zero frame".into());
        }
        if editor.mesh_play_input_enabled() {
            return Err("Mesh gameplay input remained enabled after restart".into());
        }
        if stopped.is_some_and(|expected| editor.last_stopped() != Some(expected)) {
            return Err("The stopped game recording was not preserved across restart".into());
        }
        #[cfg(feature = "mesh-mission")]
        if let Some(mission) = orr_remote::mesh_navigation::mission::from_view(snapshot.predicted())
            .map_err(|error| format!("Could not validate the restarted mission state: {error}"))?
        {
            if mission.run != orr_remote::mesh_navigation::mission::MeshMissionRun::default() {
                return Err("The restarted mission did not reset its runtime progress".into());
            }
        }
        Ok(())
    }

    fn context_matches(&self, editor: &Editor) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| Self::eligible(editor) && Self::matches(context, editor))
    }

    fn current_context(editor: &Editor) -> Option<Context> {
        Some(Context {
            scene_path: editor.path()?,
            scene_epoch: editor.scene_load_epoch(),
        })
    }

    fn matches(context: &Context, editor: &Editor) -> bool {
        editor.path().as_ref() == Some(&context.scene_path)
            && editor.scene_load_epoch() == context.scene_epoch
    }

    fn eligible(editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && !editor.is_viewer()
            && editor.can_mutate()
            && editor.mode() == Mode::Play
            && editor.path().is_some()
    }

    fn editor_error(editor: &Editor, fallback: &str) -> String {
        editor
            .status()
            .filter(|message| message.error)
            .map(|message| message.text.clone())
            .unwrap_or_else(|| fallback.into())
    }

    fn close_with_error(&mut self, error: impl Into<String>) {
        self.dialog_open = false;
        self.context = None;
        self.error = Some(error.into());
    }
}

#[cfg(test)]
#[path = "mesh_player_session/tests.rs"]
mod tests;
