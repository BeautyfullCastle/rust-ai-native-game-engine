use super::{mesh_from_view, mesh_navigation_types, MeshView};
use orr_bridge::FrameView;
use orr_games::mesh_navigation_game::mission;
use serde_json::{json, Value as JsonValue};

pub use orr_games::mesh_navigation_game::mission::{
    validate_config, MeshMission, MeshMissionRun, MissionPoint, MISSION_NAME, RUN_NAME,
};

pub struct MissionView {
    pub config: MeshMission,
    pub run: MeshMissionRun,
}

pub fn from_view(frame: FrameView<'_>) -> Result<Option<MissionView>, String> {
    let config = *frame.singleton::<MeshMission>();
    let run = *frame.singleton::<MeshMissionRun>();

    if config.disabled() {
        mission::validate_run(&config, &run)?;
        return Ok(None);
    }

    let view = mesh_from_view(frame)?;
    mission::validate_config(&config, &view.graph, view.pin.max_vertical)?;
    mission::validate_run(&config, &run)?;
    Ok(Some(MissionView { config, run }))
}

pub fn patch_request(config: &MeshMission, view: &MeshView) -> Result<JsonValue, String> {
    let pin = view.pin;
    if pin.graph_revision != view.graph.revision() {
        return Err("mission proposal graph does not match the admitted scene pin".into());
    }
    mission::validate_config(config, &view.graph, pin.max_vertical)?;
    let types = mesh_navigation_types();
    let descriptor = types
        .get(MISSION_NAME)
        .ok_or("MeshMission reflection is missing")?;
    let reflected = descriptor.read(bytemuck::bytes_of(config));
    let value = crate::json::value_to_json(&reflected);
    if !matches!(value, JsonValue::Object(_)) {
        return Err("MeshMission reflection did not produce an object".into());
    }
    let request = json!({
        "name": MISSION_NAME,
        "path": "",
        "value": value,
    });
    Ok(request)
}

#[cfg(test)]
#[path = "mission/tests.rs"]
mod tests;
