//! Shared, profile-preserving identity checks for local mesh authoring.
use crate::{backend::HostSpec, editor::Editor, game::EditorGame, model::Mode};
use orr_remote::mesh_navigation::{mesh_from_view, MeshView};
use std::path::{Path, PathBuf};

pub(crate) struct Context {
    pub game: EditorGame,
    pub scene: PathBuf,
    pub scene_epoch: u64,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub mesh: MeshView,
}

pub(crate) fn reported_scene_matches_request(
    spec: &HostSpec,
    game: EditorGame,
    reported: &Path,
) -> bool {
    let HostSpec::LocalGame { scene, game: requested_game, .. } = spec else {
        return false;
    };
    if !game.has_mesh_view() || *requested_game != game {
        return false;
    }
    scene == reported || matches!(
        (std::fs::canonicalize(scene), std::fs::canonicalize(reported)),
        (Ok(requested), Ok(reported)) if requested == reported
    )
}

pub(crate) fn current(editor: &Editor) -> Result<Context, String> {
    let game = editor.game();
    if !game.has_mesh_view() || !editor.spec().is_local() || editor.down().is_some() {
        return Err("A connected local mesh profile is required".into());
    }
    let scene = editor.path().ok_or("Wait for the requested local mesh scene")?;
    if !reported_scene_matches_request(editor.spec(), game, &scene) {
        return Err("The reported scene or game does not match the requested mesh profile".into());
    }
    let snapshot = editor.snapshot().ok_or("Wait for the current mesh snapshot")?;
    let frame = snapshot.predicted();
    if editor.mode() != Mode::Play && frame.checksum() != editor.sim().checksum {
        return Err("Wait for the authoritative mesh snapshot".into());
    }
    let mesh = mesh_from_view(frame)?;
    Ok(Context {
        game,
        scene,
        scene_epoch: editor.scene_load_epoch(),
        checksum: frame.checksum(),
        graph_revision: mesh.graph.revision(),
        mesh,
    })
}

#[cfg(all(test, feature = "mesh-hazards"))]
mod tests {
    use super::*;

    #[test]
    fn context_preserves_exact_profile_and_rejects_legacy_local_default() {
        let path = Path::new("mesh.scene.yaml");
        let hazards = HostSpec::local_game(path, EditorGame::MeshHazardsV1);
        assert!(reported_scene_matches_request(&hazards, EditorGame::MeshHazardsV1, path));
        assert!(!reported_scene_matches_request(&hazards, EditorGame::MeshNavigation, path));
        assert!(!reported_scene_matches_request(&hazards, EditorGame::MeshHazardsV1, Path::new("other.scene.yaml")));
        assert!(!reported_scene_matches_request(&HostSpec::local(path), EditorGame::MeshHazardsV1, path));
    }
}
