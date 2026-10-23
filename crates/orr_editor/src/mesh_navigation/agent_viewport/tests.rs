use super::{Endpoint, State};
use crate::{editor::Editor, game::EditorGame};
use orr_fp::FP;
use orr_remote::mesh_navigation::{mesh_from_view, MeshScenePin};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct TestScene(PathBuf);

impl TestScene {
    fn create() -> Self {
        static NEXT_SCENE: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "orr_mesh_agent_viewport_{}_{}",
            std::process::id(),
            NEXT_SCENE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let scene = root.join("viewport.scene.yaml");
        fs::write(
            &scene,
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
        )
        .unwrap();
        Self(scene)
    }
}

impl Drop for TestScene {
    fn drop(&mut self) {
        if let Some(root) = self.0.parent() {
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[test]
fn external_agent_edit_cancels_armed_viewport_pick_without_mutating_host() {
    let scene = TestScene::create();
    let mut editor = Editor::open_game(&scene.0, EditorGame::MeshNavigation).unwrap();
    editor.sync();

    let mut panel = crate::mesh_navigation::Panel::default();
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    panel.sync(&editor);

    let original_view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
    let graph = original_view.graph.clone();
    let mut changed_spec = original_view.pin.spec();
    changed_spec.distance_per_tick = FP::from_raw(changed_spec.distance_per_tick.raw() + 1);
    let changed_pin = MeshScenePin::new(&graph, changed_spec).unwrap();
    let initial_history_len = editor.history().entries.len();
    let initial_checksum = editor.checksum();

    let mut state = State::default();
    state.begin(&mut panel, &editor, Endpoint::Start);
    assert_eq!(state.pending, Some(Endpoint::Start));
    assert_eq!(editor.checksum(), initial_checksum);
    assert_eq!(editor.history().entries.len(), initial_history_len);

    editor
        .host_call(
            "world.singleton.patch",
            crate::mesh_navigation::pin_patch(&changed_pin),
        )
        .unwrap();
    editor.sync();
    let external_checksum = editor.checksum();
    let external_history_len = editor.history().entries.len();
    assert_ne!(external_checksum, initial_checksum);

    panel.sync(&editor);
    state.sync(&panel, &editor);

    assert_eq!(state.pending, None);
    assert_eq!(state.context, None);
    assert!(state.status.contains("scene or graph changed"));
    let current_view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(current_view.pin, changed_pin);
    assert_eq!(editor.checksum(), external_checksum);
    assert_eq!(editor.history().entries.len(), external_history_len);
}
