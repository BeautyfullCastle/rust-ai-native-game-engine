use super::State;

#[test]
fn state_starts_without_an_open_dialog_or_error() {
    let state = State::default();
    assert!(!state.dialog_open());
    assert!(state.error.is_none());
}

#[cfg(feature = "mesh-mission")]
mod local_editor_tests {
    use super::*;
    use crate::{
        editor::Editor,
        game::EditorGame,
        mesh_workspace::mission::{self, Settings},
        Mode,
    };
    use std::path::Path;

    fn mesh_editor(root: &Path) -> Editor {
        let scene = root.join("restart-session.scene.yaml");
        mission::prepare(&scene, &Settings::default()).unwrap()
    }

    fn step_paused(editor: &mut Editor, ticks: u32) {
        assert!(editor.start_play());
        editor.step(ticks);
        editor.sync();
        assert_eq!(editor.mode(), Mode::Play);
        assert!(!editor.sim().playing);
        assert_eq!(editor.sim().head_tick, u64::from(ticks));
    }

    #[test]
    fn cancel_restart_keeps_session_paused_and_preserves_prior_recording() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = mesh_editor(directory.path());
        step_paused(&mut editor, 3);
        assert!(editor.stop().is_some());
        editor.sync();
        let prior_recording = editor.last_stopped().unwrap().clone();

        step_paused(&mut editor, 6);
        editor.play();
        editor.sync();
        let mut recorded = crate::app::mesh_recorded_playback::State::default();
        assert!(recorded.restart(&mut editor, 0.0));
        editor.sync();
        assert!(recorded.active());

        let mut state = State::default();
        state.open_dialog(&mut editor, &mut recorded);
        editor.sync();
        assert!(state.dialog_open());
        assert!(!recorded.active());
        assert_eq!(editor.mode(), Mode::Play);
        assert!(!editor.sim().playing);
        assert!(!editor.mesh_play_input_enabled());

        let paused_head = editor.sim().head_tick;
        let paused_checksum = editor.checksum();
        state.cancel_dialog();
        editor.sync();
        assert!(!state.dialog_open());
        assert_eq!(editor.sim().head_tick, paused_head);
        assert_eq!(editor.checksum(), paused_checksum);
        assert_eq!(editor.last_stopped(), Some(&prior_recording));
    }

    #[test]
    fn confirmation_stops_and_restarts_at_paused_tick_zero() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = mesh_editor(directory.path());
        step_paused(&mut editor, 8);
        let mut recorded = crate::app::mesh_recorded_playback::State::default();
        let mut state = State::default();

        state.open_dialog(&mut editor, &mut recorded);
        editor.sync();
        assert!(state.dialog_open());
        assert!(state.confirm_restart(&mut editor));
        assert!(!state.dialog_open());
        assert_eq!(editor.game(), EditorGame::MeshNavigation);
        assert_eq!(editor.mode(), Mode::Play);
        assert_eq!(editor.sim().head_tick, 0);
        assert!(!editor.sim().playing);
        assert_eq!(editor.timeline().unwrap().tick, 0);
        assert_eq!(editor.last_stopped().unwrap().tick, 8);
        assert!(!editor.mesh_play_input_enabled());

        let mission =
            orr_remote::mesh_navigation::mission::from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap();
        assert_eq!(
            mission.run,
            orr_remote::mesh_navigation::mission::MeshMissionRun::default()
        );
    }

    #[test]
    fn restart_confirmation_rejects_non_mesh_game() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("physics.scene.yaml");
        std::fs::copy(EditorGame::PhysGame.default_scene_path(), &scene).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::PhysGame).unwrap();
        editor.sync();
        assert!(editor.start_play());
        editor.sync();

        let mut state = State::default();
        let mut recorded = crate::app::mesh_recorded_playback::State::default();
        state.open_dialog(&mut editor, &mut recorded);

        assert!(!state.dialog_open());
        assert!(state.error.is_some());
        assert_eq!(editor.mode(), Mode::Play);
    }
}
