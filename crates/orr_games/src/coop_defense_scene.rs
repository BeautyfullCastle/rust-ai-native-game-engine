//! Optional strict source admission shared by ERP and the native player.
//! No editor, transport, filesystem or window dependency belongs here.
use crate::coop_defense_game::{
    scenario_types, Campaign, DefenseConfig, DefenseGame, Scenario, CAMPAIGN_NAME, SCENARIO_NAME,
};
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
pub fn config_from_baked(scene: &Scene, frame: &Frame) -> Result<DefenseConfig, String> {
    if !scene.entities.is_empty()
        || scene
            .singletons
            .iter()
            .any(|(name, _)| name != SCENARIO_NAME && name != CAMPAIGN_NAME)
        || scene.has_prefab_links()
    {
        return Err(
            "CoopDefense accepts only Scenario and Campaign, no entities or prefabs".into(),
        );
    }
    // Bake leaves absent singletons zeroed. Presence must be checked per type:
    // a Campaign-only source must not acquire a zero-HP/zero-gold Scenario.
    let present = |name| scene.singletons.iter().any(|(key, _)| key == name);
    Ok(DefenseConfig {
        scenario: if present(SCENARIO_NAME) {
            *frame.singleton::<Scenario>()
        } else {
            Scenario::default()
        },
        campaign: if present(CAMPAIGN_NAME) {
            *frame.singleton::<Campaign>()
        } else {
            Campaign::default()
        },
        ..DefenseConfig::default()
    })
}

/// Reuse the engine's bounded strict YAML, reflected defaults/ranges and bake.
/// Return all admitted authoring state together; consumers must not drop Campaign.
pub fn parse_config(text: &str) -> Result<DefenseConfig, String> {
    validate_source(text)?;
    let types = scenario_types();
    let scene = Scene::parse(text, &types).map_err(|e| e.to_string())?;
    let mut frame = Frame::new(Simulation::<DefenseGame>::build_registry());
    scene.bake(&types, &mut frame).map_err(|e| e.to_string())?;
    config_from_baked(&scene, &frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
    #[test]
    fn defaults_and_authored_values_use_reflection() {
        assert_eq!(parse_config(EMPTY).unwrap(), DefenseConfig::default());
        assert_eq!(parse_config(&format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 150, reactor_hp: 30}}}}\n")).unwrap().scenario, Scenario::default());
        assert_eq!(parse_config(&format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 250, reactor_hp: 75}}}}\n")).unwrap().scenario, Scenario { starting_gold: 250, reactor_hp: 75 });
        // Source YAML is strict/full-field, unlike whole-value ERP edits which
        // use Reflect defaults. Do not silently broaden Scene::parse here.
        for fields in ["{}", "{reactor_hp: 75}"] {
            assert!(parse_config(&format!(
                "{EMPTY}singletons: {{CoopDefense::Scenario: {fields}}}\n"
            ))
            .is_err());
        }
    }
    #[test]
    fn campaign_defaults_independently_and_source_is_strict() {
        for last_wave in [1, 3, 10] {
            let only = parse_config(&format!(
                "{EMPTY}singletons: {{CoopDefense::Campaign: {{last_wave: {last_wave}}}}}\n"
            ))
            .unwrap();
            assert_eq!(only.scenario, Scenario::default());
            assert_eq!(only.campaign, Campaign { last_wave });
            let both = parse_config(&format!("{EMPTY}singletons: {{CoopDefense::Campaign: {{last_wave: {last_wave}}}, CoopDefense::Scenario: {{starting_gold: 250, reactor_hp: 75}}}}\n")).unwrap();
            assert_eq!(
                both.scenario,
                Scenario {
                    starting_gold: 250,
                    reactor_hp: 75
                }
            );
            assert_eq!(both.campaign, only.campaign);
        }
        for fields in [
            "{}",
            "{last_wave: 0}",
            "{last_wave: 11}",
            "{last_wave: 3, extra: 1}",
        ] {
            assert!(parse_config(&format!(
                "{EMPTY}singletons: {{CoopDefense::Campaign: {fields}}}\n"
            ))
            .is_err());
        }
        assert_eq!(parse_config(&format!("{EMPTY}singletons: {{CoopDefense::Scenario: {{starting_gold: 250, reactor_hp: 75}}}}\n")).unwrap().campaign, Campaign::default());
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
        ] { assert!(parse_config(&bad).is_err(), "accepted: {bad}"); }
    }
}
