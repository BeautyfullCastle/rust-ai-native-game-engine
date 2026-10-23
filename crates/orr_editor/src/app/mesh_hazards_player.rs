use std::path::PathBuf;

use crate::{Mode, editor::Editor, game::EditorGame};
use egui::{Button, Ui, Window};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene_path: PathBuf,
    scene_epoch: u64,
    sim_epoch: u64,
    snapshot_seq: u64,
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

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &mut Editor) -> bool {
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
            self.open_dialog(editor);
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

        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        confirmed && self.dialog_open
    }

    fn open_dialog(&mut self, editor: &mut Editor) {
        self.error = None;
        if !Self::eligible(editor) {
            self.error = Some("Restart requires a mutable local MeshHazardsV1 Play session".into());
            return;
        }
        editor.set_mesh_play_input_enabled(false);
        editor.pause();
        editor.sync();
        let Some(context) = Self::current_context(editor) else {
            self.error = Some("Restart requires a saved local MeshHazardsV1 scene".into());
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

    fn stop_confirmed(&mut self, editor: &mut Editor) -> Option<crate::model::Stopped> {
        if !self.dialog_open || !self.context_matches(editor) {
            self.close_with_error("Restart refused because the game session changed");
            return None;
        }
        editor.set_mesh_play_input_enabled(false);
        if editor.stop().is_none() {
            editor.sync();
            self.close_with_error(Self::editor_error(
                editor,
                "Could not stop the current game",
            ));
            return None;
        }
        editor.sync();
        if editor.mode() != Mode::Edit {
            self.close_with_error("The game did not return to its authored scene after stopping");
            return None;
        }
        editor.last_stopped().cloned()
    }

    fn start_fresh(&mut self, editor: &mut Editor, stopped: &crate::model::Stopped) -> bool {
        // This phase is private and requires the completed confirmed Stop receipt.
        // Presentation refresh must not silently replace the authored scene.
        if !self.dialog_open
            || editor.mode() != Mode::Edit
            || editor.last_stopped() != Some(stopped)
            || !self.context.as_ref().is_some_and(|context| {
                editor.path().as_ref() == Some(&context.scene_path)
                    && editor.scene_load_epoch() == context.scene_epoch
            })
        {
            self.close_with_error("Restart refused because the stopped scene changed");
            return false;
        }
        if !editor.start_play() {
            let error = Self::editor_error(editor, "Could not restart the current game");
            editor.sync();
            self.close_with_error(error);
            return false;
        }
        editor.set_mesh_play_input_enabled(false);
        editor.sync();
        if let Err(error) = Self::validate_fresh_session(editor, Some(stopped)) {
            self.close_with_error(error);
            return false;
        }
        self.dialog_open = false;
        self.context = None;
        self.error = None;
        true
    }

    #[cfg(test)]
    fn confirm_restart(&mut self, editor: &mut Editor) -> bool {
        self.stop_confirmed(editor)
            .is_some_and(|stopped| self.start_fresh(editor, &stopped))
    }

    fn validate_fresh_session(
        editor: &Editor,
        stopped: Option<&crate::model::Stopped>,
    ) -> Result<(), String> {
        validate_fresh_session(editor)?;
        if editor.game() != EditorGame::MeshHazardsV1
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
            sim_epoch: editor.sim().epoch,
            snapshot_seq: editor.capture_snapshot_seq()?,
        })
    }

    fn matches(context: &Context, editor: &Editor) -> bool {
        editor.path().as_ref() == Some(&context.scene_path)
            && editor.scene_load_epoch() == context.scene_epoch
            && editor.sim().epoch == context.sim_epoch
            && editor.capture_snapshot_seq() == Some(context.snapshot_seq)
    }

    fn eligible(editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshHazardsV1
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

/// Check the hazard registry before accessing its typed singleton state.
pub(crate) fn validate_fresh_session(editor: &Editor) -> Result<(), String> {
    if editor.game() != EditorGame::MeshHazardsV1 || editor.mode() != Mode::Play {
        return Err("Expected an admitted MeshHazardsV1 Play session".into());
    }
    if editor.sim().head_tick != 0 || editor.sim().playing || editor.mesh_play_input_enabled() {
        return Err("Fresh hazard session must be paused at tick zero with input off".into());
    }
    let snapshot = editor
        .snapshot()
        .ok_or("Fresh hazard session has no frame")?;
    let timeline = snapshot
        .timeline()
        .ok_or("Fresh hazard session has no timeline")?;
    if snapshot.tick() != 0
        || timeline.tick != 0
        || timeline.playing
        || timeline.epoch != editor.sim().epoch
    {
        return Err("Fresh hazard session has no coherent paused tick-zero frame".into());
    }
    let hazards = orr_remote::mesh_navigation::hazards::from_view(snapshot.predicted())?;
    if hazards.is_some_and(|view| {
        view.run != orr_remote::mesh_navigation::hazards::MeshHazardRun::default()
    }) {
        return Err("Fresh hazard session did not reset loss state".into());
    }
    let mission = orr_remote::mesh_navigation::mission::from_view(snapshot.predicted())?;
    if mission.is_some_and(|view| {
        view.run != orr_remote::mesh_navigation::mission::MeshMissionRun::default()
    }) {
        return Err("Fresh hazard session did not reset mission progress".into());
    }
    Ok(())
}

impl super::EditorApp {
    fn restart_mesh_hazards_player(&mut self, session: &mut State) -> bool {
        let Some(stopped) = session.stop_confirmed(&mut self.editor) else {
            return false;
        };
        // Authoring panels must adopt/check the restored Edit frame before Start.
        // This recalculates the real blockers; it never clears them unconditionally.
        self.sync_mesh_presentation();
        if !session.start_fresh(&mut self.editor, &stopped) {
            return false;
        }
        self.mesh_play_controls = crate::mesh_play_controls::Panel::default();
        self.sync_mesh_presentation();
        true
    }

    /// Hazard-only capture keeps the legacy writer untouched. Output admission
    /// is atomic at creation, including if a file or symlink appears meanwhile.
    fn hazards_screenshot(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let waiting_for = self.editor.screenshot_waiting_for();
        let Some(job) = &mut self.shot else {
            return Ok(());
        };
        ctx.request_repaint();
        match job.requested_at {
            None => {
                if !job.ready(self.frames, ctx.input(|input| input.time), waiting_for)? {
                    return Ok(());
                }
                let framebuffer = super::framebuffer_info(ctx)
                    .ok_or("capture framebuffer dimensions unavailable")?;
                capture_size(framebuffer.width as usize, framebuffer.height as usize)?;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                job.requested_at = Some(self.frames);
            }
            Some(at) => {
                let image = ctx.input(|input| {
                    input.events.iter().find_map(|event| match event {
                        egui::Event::Screenshot {
                            viewport_id,
                            user_data,
                            image,
                        } if *viewport_id == egui::ViewportId::ROOT && user_data.data.is_none() => {
                            Some(image.clone())
                        }
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    write_capture(&job.path, &image)?;
                    eprintln!(
                        "screenshot written: {} ({}x{})",
                        job.path.display(),
                        image.width(),
                        image.height()
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    self.shot = None;
                } else if self.frames > at + 600 {
                    return Err("the window never answered".into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn mesh_hazards_player_ui(&mut self, ui: &mut Ui, session: &mut State) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        if ctx.input(|input| !input.focused || input.key_pressed(egui::Key::Escape)) {
            self.editor.set_mesh_play_input_enabled(false);
        }
        self.editor.pump();
        self.sync_mesh_presentation();
        egui::Panel::top("mesh_hazards_player_controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                let playing = self
                    .editor
                    .timeline()
                    .is_some_and(|timeline| timeline.playing);
                let available = self.editor.down().is_none()
                    && self.editor.mode() == Mode::Play
                    && !session.dialog_open();
                if ui
                    .add_enabled(
                        available,
                        Button::new(if playing { "Pause" } else { "Resume" }),
                    )
                    .clicked()
                {
                    self.editor.set_mesh_play_input_enabled(false);
                    if playing {
                        self.editor.pause();
                    } else {
                        self.editor.play();
                    }
                }
                if session.show(ui, &mut self.editor) {
                    self.restart_mesh_hazards_player(session);
                }
                if let Some(timeline) = self.editor.timeline() {
                    ui.label(format!(
                        "Tick {} · {}",
                        timeline.tick,
                        if timeline.playing {
                            "Playing"
                        } else {
                            "Paused"
                        }
                    ));
                }
                if let Some(error) = self.editor.down() {
                    ui.colored_label(egui::Color32::RED, &error.reason);
                }
            });
            ui.add_enabled_ui(!session.dialog_open(), |ui| {
                self.mesh_play_controls.show(ui, &mut self.editor)
            });
            if let Some(snapshot) = self.editor.snapshot() {
                match orr_remote::mesh_navigation::hazards::from_view(snapshot.predicted()) {
                    Ok(Some(view)) if view.run.lost != 0 => {
                        ui.colored_label(egui::Color32::RED, "Lost · restart to try again");
                    }
                    Err(error) => {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                    _ => {}
                }
            }
            if let Some(status) = self.editor.status().filter(|status| status.error) {
                ui.colored_label(egui::Color32::RED, &status.text);
            }
        });
        // The shared viewport reuses gameplay rendering and input only. Its legacy
        // timeline controller is never invoked by this standalone player.
        self.set_mesh_player_input_allowed(!session.dialog_open());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.yard_viewport(ui));
        if let Err(error) = self.hazards_screenshot(&ctx) {
            eprintln!("screenshot failed: {error}");
            std::process::exit(2);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn capture_size(width: usize, height: usize) -> Result<usize, String> {
    let pixels = width
        .checked_mul(height)
        .ok_or("capture dimensions overflow")?;
    if width == 0 || height == 0 || pixels > super::MAX_CAPTURE_PIXELS as usize {
        return Err(
            "physical screenshot exceeds the 1048576 pixel limit or has empty dimensions".into(),
        );
    }
    Ok(pixels)
}

fn write_capture(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let pixels = capture_size(image.width(), image.height())?;
    if image.pixels.len() != pixels {
        return Err("screenshot pixel storage does not match its dimensions".into());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create new screenshot: {error}"))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.width() as u32,
        image.height() as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Best);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    writer
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(root: &std::path::Path) -> Editor {
        crate::mesh_hazards_creator::prepare(
            &root.join("hazards.scene.yaml"),
            &Default::default(),
            true,
        )
        .unwrap()
    }

    #[test]
    fn cancel_is_paused_and_does_not_reset_session() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        assert!(editor.start_play());
        editor.step(8);
        editor.sync();
        let checksum = editor.checksum();
        let epoch = editor.sim().epoch;
        let mut state = State::default();
        state.open_dialog(&mut editor);
        assert!(state.dialog_open());
        state.cancel_dialog();
        editor.sync();
        assert!(!state.dialog_open());
        assert_eq!(editor.sim().head_tick, 8);
        assert_eq!(editor.sim().epoch, epoch);
        assert_eq!(editor.checksum(), checksum);
        assert!(!editor.sim().playing);
        assert!(!editor.mesh_play_input_enabled());
    }

    #[test]
    fn confirmed_restart_uses_new_session_and_clears_progress() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        let authored_checksum = editor.checksum();
        assert!(editor.start_play());
        editor.step(120);
        editor.sync();
        let mut state = State::default();
        state.open_dialog(&mut editor);
        assert!(state.confirm_restart(&mut editor));
        validate_fresh_session(&editor).unwrap();
        assert_eq!(editor.last_stopped().unwrap().tick, 120);
        assert_eq!(editor.checksum(), authored_checksum);
        assert!(!state.dialog_open());
        assert!(state.error.is_none());
    }

    #[test]
    fn restart_after_actual_loss_resets_loss_and_mission() {
        use orr_remote::mesh_navigation::{hazards, mesh_from_view};
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        let mut config = hazards::MeshHazards {
            enabled: 1,
            count: 1,
            graph_revision: view.graph.revision(),
            ..Default::default()
        };
        config.spheres[0] = hazards::HazardSphere {
            center: view.agent.position,
            radius: orr_fp::FP::from_int(1),
        };
        editor
            .host_call(
                "world.singleton.patch",
                hazards::patch_request(&config, &view).unwrap(),
            )
            .unwrap();
        editor.sync();
        assert!(editor.start_play());
        editor.step(2);
        editor.sync();
        assert_eq!(
            hazards::from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap()
                .run
                .lost,
            1
        );
        assert!(!editor.set_mesh_play_input_enabled(true));
        let mut state = State::default();
        state.open_dialog(&mut editor);
        assert!(state.confirm_restart(&mut editor));
        validate_fresh_session(&editor).unwrap();
        assert_eq!(
            hazards::from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap()
                .run
                .lost,
            0
        );
    }

    #[test]
    fn actual_player_ui_restart_refreshes_authored_panels_after_loss() {
        use egui_kittest::{Harness, kittest::Queryable};
        use orr_remote::mesh_navigation::{hazards, mesh_from_view};
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        let view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        let mut config = hazards::MeshHazards {
            enabled: 1,
            count: 1,
            graph_revision: view.graph.revision(),
            ..Default::default()
        };
        config.spheres[0] = hazards::HazardSphere {
            center: view.agent.position,
            radius: orr_fp::FP::from_int(1),
        };
        editor
            .host_call(
                "world.singleton.patch",
                hazards::patch_request(&config, &view).unwrap(),
            )
            .unwrap();
        editor.sync();
        let authored_checksum = editor.checksum();
        assert!(editor.start_play());
        editor.step(2);
        editor.sync();
        assert_eq!(
            hazards::from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap()
                .run
                .lost,
            1
        );
        // Match actual launch ordering: panels are new while the host is in Play.
        let app = super::super::EditorApp::new(editor, None);
        let mut h = Harness::builder().with_size([960.0, 600.0]).build_ui_state(
            |ui, state: &mut (super::super::EditorApp, State)| {
                state.0.mesh_hazards_player_ui(ui, &mut state.1)
            },
            (app, State::default()),
        );
        h.run_steps(3);
        h.get_by_label("Restart game…").click_accesskit();
        h.run_steps(3);
        assert!(h.state().1.dialog_open());
        h.get_by_label("Restart game").click_accesskit();
        h.run_steps(3);
        let (app, session) = h.state();
        assert!(
            session.error.is_none(),
            "restart error: {:?}",
            session.error
        );
        validate_fresh_session(&app.editor).unwrap();
        assert_eq!(app.editor.checksum(), authored_checksum);
        assert_eq!(app.editor.last_stopped().unwrap().tick, 2);
        assert!(!session.dialog_open());
    }

    #[test]
    fn paused_confirmation_survives_pump_and_sync_frames() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        assert!(editor.start_play());
        editor.step(3);
        editor.sync();
        let mut state = State::default();
        state.open_dialog(&mut editor);
        for _ in 0..8 {
            editor.pump();
            editor.sync();
            assert!(state.context_matches(&editor));
        }
        assert!(state.confirm_restart(&mut editor));
    }

    #[test]
    fn same_scene_replaced_session_rejects_old_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        assert!(editor.start_play());
        editor.sync();
        let mut state = State::default();
        state.open_dialog(&mut editor);
        let context = state.context.clone().unwrap();
        editor.stop().unwrap();
        editor.sync();
        assert!(editor.start_play());
        editor.sync();
        assert_eq!(editor.scene_load_epoch(), context.scene_epoch);
        assert_eq!(
            editor.sim().epoch,
            context.sim_epoch,
            "host reuses epoch zero for fresh sessions"
        );
        assert!(!state.confirm_restart(&mut editor));
        assert!(state.error.as_deref().unwrap().contains("session changed"));
        validate_fresh_session(&editor).unwrap();
    }

    #[test]
    fn capture_bounds_apply_to_physical_dimensions_and_before_file_creation() {
        assert_eq!(capture_size(1024, 1024).unwrap(), 1_048_576);
        for (width, height) in [(0, 100), (100, 0), (1920, 1200), (usize::MAX, 2)] {
            assert!(capture_size(width, height).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.png");
        let mut image = egui::ColorImage::filled([1, 1], egui::Color32::BLACK);
        image.size = [1920, 1200];
        assert!(write_capture(&path, &image).is_err());
        assert!(!path.exists());
        image.size = [2, 2];
        assert!(write_capture(&path, &image).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn capture_atomically_refuses_file_that_appeared_after_preflight() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        let output = directory.path().join("capture.png");
        assert!(!output.exists());
        std::fs::write(&output, b"preserve raced output").unwrap();
        let image = egui::ColorImage::filled([2, 2], egui::Color32::BLACK);
        assert!(write_capture(&output, &image).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"preserve raced output");
        let fresh = directory.path().join("fresh.png");
        write_capture(&fresh, &image).unwrap();
        assert!(
            std::fs::read(&fresh)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn capture_atomically_refuses_raced_symlink_to_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("admitted-scene");
        std::fs::write(&target, b"preserve admitted bytes").unwrap();
        let output = directory.path().join("capture.png");
        std::os::unix::fs::symlink(&target, &output).unwrap();
        let image = egui::ColorImage::filled([2, 2], egui::Color32::BLACK);
        assert!(write_capture(&output, &image).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"preserve admitted bytes");
    }

    #[test]
    fn stale_or_missing_confirmation_cannot_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = editor(directory.path());
        assert!(editor.start_play());
        editor.sync();
        let mut state = State::default();
        assert!(!state.confirm_restart(&mut editor));
        state.open_dialog(&mut editor);
        editor.stop().unwrap();
        editor.sync();
        assert!(!state.confirm_restart(&mut editor));
        assert_eq!(editor.mode(), Mode::Edit);
    }
}
