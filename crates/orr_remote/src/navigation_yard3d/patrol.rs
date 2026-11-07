//! Authored terrain patrol proposals and immutable runtime readback.
use super::{NavigationView, navigation_from_view, navigation_yard3d_types};
use orr_bridge::FrameView;
use orr_edit::{EditorDoc, Op, Origin};
use orr_games::navigation_yard3d_game::patrol;
pub use patrol::{PATROL_NAME, RUN_NAME, TerrainPatrol, TerrainPatrolRun};
use serde_json::{Value, json};
pub struct PatrolView {
    pub config: TerrainPatrol,
    pub run: TerrainPatrolRun,
}
pub fn from_view(frame: FrameView<'_>) -> Result<Option<PatrolView>, String> {
    // Shared navigation readback validates the complete cursor/leg context.
    navigation_from_view(frame)?;
    let config = *frame.singleton::<TerrainPatrol>();
    let run = *frame.singleton::<TerrainPatrolRun>();
    if config.enabled == 0 {
        Ok(None)
    } else {
        Ok(Some(PatrolView { config, run }))
    }
}
pub fn patch_request(config: &TerrainPatrol, view: &NavigationView) -> Result<Value, String> {
    patrol::validate_config(config, &view.terrain, &view.graph, view.pin.agent.start)?;
    let types = navigation_yard3d_types();
    let descriptor = types
        .get(PATROL_NAME)
        .ok_or("TerrainPatrol reflection missing")?;
    let value = crate::json::value_to_json(&descriptor.read(bytemuck::bytes_of(config)));
    Ok(json!({"name":PATROL_NAME,"path":"","value":value}))
}
pub fn set_patrol(document: &mut EditorDoc, config: &TerrainPatrol) -> Result<(), String> {
    let view = navigation_from_view(FrameView::of(document.frame()))?;
    patch_request(config, &view)?;
    let value = document
        .types()
        .get(PATROL_NAME)
        .ok_or("TerrainPatrol reflection missing")?
        .read(bytemuck::bytes_of(config));
    document
        .apply_atomic_batch(
            "Apply terrain waypoint patrol",
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
