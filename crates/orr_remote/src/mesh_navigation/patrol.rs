//! Atomic authored patrol proposals over the existing singleton edit boundary.
use super::{mesh_from_view, mesh_navigation_types, MeshView};
use orr_bridge::FrameView;
use orr_edit::{EditorDoc, Op, Origin};
use orr_games::mesh_navigation_game::patrol;
pub use orr_games::mesh_navigation_game::patrol::{
    MeshPatrol, MeshPatrolRun, PATROL_NAME, RUN_NAME,
};
use serde_json::{json, Value as JsonValue};
pub struct PatrolView {
    pub config: MeshPatrol,
    pub run: MeshPatrolRun,
}
pub fn from_view(frame: FrameView<'_>) -> Result<Option<PatrolView>, String> {
    let config = *frame.singleton::<MeshPatrol>();
    let run = *frame.singleton::<MeshPatrolRun>();
    patrol::validate_run(&config, &run)?;
    if bytemuck::bytes_of(&config).iter().all(|b| *b == 0) {
        return Ok(None);
    }
    let view = mesh_from_view(frame)?;
    patrol::validate_config(&config, &view.graph, view.pin.start, view.pin.max_vertical)?;
    if run.paused == 0
        && (view.pin.goal != config.points[run.cursor as usize]
            || view.agent.spec.goal != view.pin.goal
            || ((run.arrived == 1 || run.completed == 1)
                && view.agent.status != orr_navigation::NavigationStatus::Arrived))
    {
        return Err("Patrol runtime goal or arrival does not match cursor".into());
    }
    Ok(Some(PatrolView { config, run }))
}
pub fn patch_request(config: &MeshPatrol, view: &MeshView) -> Result<JsonValue, String> {
    if view.pin.graph_revision != view.graph.revision() {
        return Err("Patrol proposal graph differs from admitted scene pin".into());
    }
    patrol::validate_config(config, &view.graph, view.pin.start, view.pin.max_vertical)?;
    let types = mesh_navigation_types();
    let descriptor = types
        .get(PATROL_NAME)
        .ok_or("MeshPatrol reflection is missing")?;
    let value = crate::json::value_to_json(&descriptor.read(bytemuck::bytes_of(config)));
    if !matches!(value, JsonValue::Object(_)) {
        return Err("MeshPatrol reflection did not produce an object".into());
    }
    Ok(json!({"name":PATROL_NAME,"path":"","value":value}))
}
pub fn set_patrol(document: &mut EditorDoc, config: &MeshPatrol) -> Result<(), String> {
    let view = mesh_from_view(FrameView::of(document.frame()))?;
    patch_request(config, &view)?;
    let descriptor = document
        .types()
        .get(PATROL_NAME)
        .ok_or("MeshPatrol reflection is missing")?;
    let value = descriptor.read(bytemuck::bytes_of(config));
    document
        .apply_atomic_batch(
            "Apply waypoint patrol",
            vec![Op::SetSingletonField {
                singleton: PATROL_NAME.into(),
                path: String::new(),
                value,
            }],
            Origin::User,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
#[path = "patrol/tests.rs"]
mod tests;
