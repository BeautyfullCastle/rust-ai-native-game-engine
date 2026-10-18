//! New-file-only hazard starter; never reads or upgrades an existing scene.
use crate::{
    editor::Editor,
    game::EditorGame,
    mesh_workspace::{self, mission::Settings},
};
use orr_bridge::FrameView;
use orr_fp::{fp, FP};
use orr_remote::mesh_navigation::{hazards, mesh_from_view, mission};
use std::path::Path;

pub(crate) fn prepare(
    path: &Path,
    settings: &Settings,
    include_hazard: bool,
) -> Result<Editor, String> {
    let path = mesh_workspace::checked_scene_path(path, false)?;
    mesh_workspace::refuse_existing_sidecar(&path)?;
    let yaml = scene_yaml(settings, include_hazard)?;
    let admitted = hazards::hazards_doc_from_yaml(&yaml).map_err(|error| error.to_string())?;
    let expected_checksum = admitted.frame().checksum();
    if mesh_workspace::checked_scene_path(&path, false)? != path {
        return Err("hazard game destination changed before creation".into());
    }
    mesh_workspace::refuse_existing_sidecar(&path)?;
    orr_remote::mesh_navigation::authoring::create_new_scene_file(&path, &yaml)?;
    if mesh_workspace::read_scene(&path)? != yaml.as_bytes() {
        return Err("created hazard game differs from validated bytes".into());
    }
    let mut editor = Editor::open_game(&path, EditorGame::MeshHazardsV1)?;
    editor.sync();
    if editor.game() != EditorGame::MeshHazardsV1 || !editor.spec().is_local() {
        return Err("new hazard game opened an unexpected host or profile".into());
    }
    let snapshot = editor
        .snapshot()
        .ok_or("hazard game host did not publish an admitted Frame")?;
    let frame = snapshot.predicted();
    if frame.checksum() != expected_checksum {
        return Err("hazard game host Frame differs from admitted scene".into());
    }
    mesh_from_view(frame)?;
    mission::from_view(frame)?.ok_or("new hazard game lost its mission")?;
    hazards::from_view(frame)?;
    if mesh_workspace::read_scene(&path)? != yaml.as_bytes() {
        return Err("hazard scene changed while its host was starting".into());
    }
    Ok(editor)
}

fn scene_yaml(settings: &Settings, include_hazard: bool) -> Result<String, String> {
    use orr_remote::mesh_navigation::{MeshScenePin, MISSION_NAME, PIN_NAME};
    let (_, pin, mission_config) = mesh_workspace::mission::configured_scene(settings)?;
    let mut config = hazards::MeshHazards::default();
    if include_hazard {
        config.enabled = 1;
        config.count = 1;
        config.graph_revision = pin.graph_revision;
        // The authored route is z=depth/2 (minimum 2); this sphere is at z=0.5.
        config.spheres[0] = hazards::HazardSphere {
            center: [
                FP::from_raw(i64::from(settings.width) * FP::SCALE / 2),
                FP::ZERO,
                fp!(0.5),
            ],
            radius: fp!(0.5),
        };
    }
    let types = hazards::hazards_types();
    let mut singletons = serde_json::Map::new();
    for (name, bytes) in [
        (PIN_NAME, orr_reflect::bytemuck::bytes_of(&pin)),
        (
            MISSION_NAME,
            orr_reflect::bytemuck::bytes_of(&mission_config),
        ),
        (
            hazards::HAZARDS_NAME,
            orr_reflect::bytemuck::bytes_of(&config),
        ),
    ] {
        let descriptor = types
            .get(name)
            .ok_or_else(|| format!("missing {name} reflection"))?;
        singletons.insert(
            name.into(),
            orr_remote::json::value_to_json(&descriptor.read(bytes)),
        );
    }
    let source =
        serde_json::json!({"schema":"orr.scene/1", "singletons": singletons, "entities":{}})
            .to_string();
    let mut document =
        hazards::hazards_doc_from_yaml(&source).map_err(|error| error.to_string())?;
    let expected = document.frame().to_bytes();
    let yaml = document.save_yaml();
    if yaml.len() > 1024 * 1024 {
        return Err("generated hazard game exceeds 1 MiB".into());
    }
    let reopened = hazards::hazards_doc_from_yaml(&yaml).map_err(|error| error.to_string())?;
    if reopened.frame().to_bytes() != expected {
        return Err("new hazard game failed exact save/reopen".into());
    }
    let frame = FrameView::of(reopened.frame());
    let mesh = mesh_from_view(frame)?;
    let mission = mission::from_view(frame)?.ok_or("new hazard game needs an enabled mission")?;
    hazards::from_view(frame)?;
    if *frame.singleton::<hazards::MeshHazards>() != config
        || *frame.singleton::<hazards::MeshHazardRun>() != Default::default()
        || mission.config != mission_config
        || mission.run != Default::default()
        || frame.singleton::<MeshScenePin>().spec() != pin.spec()
        || mesh.graph.revision() != pin.graph_revision
    {
        return Err("new hazard game changed during canonical serialization".into());
    }
    Ok(yaml)
}

#[cfg(test)]
#[path = "mesh_hazards_creator/tests.rs"]
mod tests;
