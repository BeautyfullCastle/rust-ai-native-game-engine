//! Lights for the text of a Room scene: the scene form of what a Room lighting
//! document held (an environment, a sun and one point light with a rule).
//! Shared by tests with `#[path]`; the scene text is edited, no product code is.
#![allow(dead_code)]

/// The default environment the Room templates carry.
const DEFAULT_ENVIRONMENT: &str = "  orr_gfx::Environment:\n    sky_color: [0.5, 0.66, 1]\n    ground_color: [0.26, 0.24, 0.22]\n    ambient: 0.3\n";
const LIT_ENVIRONMENT: &str = "  orr_gfx::Environment:\n    sky_color: [0.4, 0.55, 0.9]\n    ground_color: [0.2, 0.18, 0.16]\n    ambient: 0.4\n";

const LIGHT_TAIL: &str = "      inner_angle: 20\n      outer_angle: 30\n      shadow_strength: 1\n      shadow_range: 0\n      shadow_softness: 1\n      shadow_sharp_range: 0\n      kind: KIND\n      enabled: true\n      cast_shadows: false\n      layers: [default, layer2, layer3, layer4, layer5, layer6, layer7, layer8]\n      shadow_cascades: 1\n";

/// The sun entity. Faces (-0.3, -0.8, -0.5): pitch 53.913 degrees, yaw -149.036.
pub fn sun_entity(guid: &str) -> String {
    format!(
        "  {guid}:\n    name: sun\n    orr_gfx::Transform3:\n      position: [0, 20, 0]\n      rotation: [53.913, -149.036, 0]\n      scale: [1, 1, 1]\n    orr_gfx::Light:\n      color: [1, 0.85, 0.6]\n      intensity: 2.4\n      range: 10\n{}",
        LIGHT_TAIL.replace("KIND", "directional")
    )
}

/// The point light entity, following the player 2 above, on until the key is collected.
pub fn point_entity(guid: &str) -> String {
    format!(
        "  {guid}:\n    name: point_light\n    orr_gfx::Transform3:\n      position: [0, 4, 0]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]\n    orr_gfx::Light:\n      color: [1, 0.9, 0.7]\n      intensity: 3\n      range: 12\n{}    RoomEscapeV1::LightRule:\n      follow: true\n      activation: key_missing\n      offset: [0, 2, 0]\n",
        LIGHT_TAIL.replace("KIND", "point")
    )
}

/// The scene with a non-default environment, the sun and the point light.
pub fn with_lights(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    assert!(text.contains(DEFAULT_ENVIRONMENT), "the scene carries the default environment");
    text = text.replace(DEFAULT_ENVIRONMENT, LIT_ENVIRONMENT);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let prefix = guid_prefix(&text);
    text.push_str(&sun_entity(&format!("e_{prefix}20000001")));
    text.push_str(&point_entity(&format!("e_{prefix}20000002")));
    text
}

/// The scene with only the sun that lit every Room before it had lights: the yard's low
/// afternoon sun, facing (-0.42, -0.72, -0.55). It draws the same picture as no light at all.
pub fn with_default_sun(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let prefix = guid_prefix(&text);
    let sun = sun_entity(&format!("e_{prefix}20000001"))
        .replace("rotation: [53.913, -149.036, 0]", "rotation: [46.120, -142.620, 0]")
        .replace("color: [1, 0.85, 0.6]", "color: [1, 0.94, 0.82]")
        .replace("intensity: 2.4", "intensity: 2.1");
    text.push_str(&sun);
    text
}

/// The first eight hex digits of the first entity key are shared with a project's
/// keys (`e_` + 24 digits + 8 ordinal digits) or are the whole key (`e_` + 8 digits).
fn guid_prefix(text: &str) -> String {
    let key = text
        .lines()
        .find(|line| line.starts_with("  e_") && line.ends_with(':'))
        .expect("the scene has an entity");
    let digits = &key[4..key.len() - 1];
    digits[..digits.len() - 8].to_owned()
}
