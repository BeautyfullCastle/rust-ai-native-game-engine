//! Optional strict source admission shared by ERP and the native player.
//! No editor, transport, filesystem or window dependency belongs here.
use crate::coop_defense_game::{scenario_types, DefenseGame, Scenario, SCENARIO_NAME};
use orr_ecs::Frame;
use orr_reflect::Scene;
use orr_sim::Simulation;

pub const MAX_SOURCE_BYTES: usize = 64 * 1024;

pub fn validate_source(text: &str) -> Result<(), String> {
    if text.len() > MAX_SOURCE_BYTES {
        return Err("CoopDefense source exceeds 64 KiB".into());
    }
    Ok(())
}

/// Read a freshly baked candidate, never a running frame. The scene must have
/// passed Scene::parse/bake with scenario_types before calling this function.
pub fn scenario_from_baked(scene: &Scene, frame: &Frame) -> Result<Scenario, String> {
    if !scene.entities.is_empty()
        || scene
            .singletons
            .iter()
            .any(|(name, _)| name != SCENARIO_NAME)
        || scene.has_prefab_links()
    {
        return Err(
            "CoopDefense accepts only Scenario starting conditions, no entities or prefabs".into(),
        );
    }
    Ok(if scene.singletons.is_empty() {
        Scenario::default()
    } else {
        *frame.singleton::<Scenario>()
    })
}

/// Reuse the engine's bounded strict YAML, reflected defaults/ranges and bake.
pub fn parse_scenario(text: &str) -> Result<Scenario, String> {
    validate_source(text)?;
    let types = scenario_types();
    let scene = Scene::parse(text, &types).map_err(|e| e.to_string())?;
    let mut frame = Frame::new(Simulation::<DefenseGame>::build_registry());
    scene.bake(&types, &mut frame).map_err(|e| e.to_string())?;
    scenario_from_baked(&scene, &frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
    #[test]
    fn defaults_and_authored_values_use_reflection() {
        assert_eq!(parse_scenario(EMPTY).unwrap(), Scenario::default());
        assert_eq!(parse_scenario(&format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 150, reactor_hp: 30}}}}\n")).unwrap(), Scenario::default());
        assert_eq!(parse_scenario(&format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 250, reactor_hp: 75}}}}\n")).unwrap(), Scenario { starting_gold: 250, reactor_hp: 75 });
        // Source YAML is strict/full-field, unlike whole-value ERP edits which
        // use Reflect defaults. Do not silently broaden Scene::parse here.
        for fields in ["{}", "{reactor_hp: 75}"] {
            assert!(parse_scenario(&format!(
                "{EMPTY}singletons: {{CoopDefense::Scenario: {fields}}}\n"
            ))
            .is_err());
        }
    }
    #[test]
    fn invalid_source_is_rejected_by_shared_admission() {
        for bad in [
            format!("{EMPTY}#{}", "x".repeat(MAX_SOURCE_BYTES)),
            format!("{EMPTY}entities: {{}}\n"),
            format!("{EMPTY}---\n{EMPTY}"),
            format!("{EMPTY}singletons: {{CoopDefense::Run: {{}}}}\n"),
            "schema: orr.scene/1\nentities: {e_00000001: {name: injected}}\n".into(),
            format!("{EMPTY}prefab_links: {{}}\n"),
            format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 10001, reactor_hp: 30}}}}\n"),
            format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 150, reactor_hp: 0}}}}\n"),
            format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 150, reactor_hp: 30, seed: 3}}}}\n"),
            format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 1, starting_gold: 2, reactor_hp: 30}}}}\n"),
        ] { assert!(parse_scenario(&bad).is_err(), "accepted: {bad}"); }
    }
}
