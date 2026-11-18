//! The settings of the scene window, applied to the scene that is open in the
//! editor.
//!
//! The window's settings (`orr_scenery::Settings`) describe a look: sun, sky,
//! fog, shadows, wind, water and the camera response. An editor scene keeps
//! the same things in `orr_gfx` components: the sun `Light`, the `Sky`,
//! `Environment` and `PostProcess` singletons, and `Water` entities. This
//! module maps each control of the window to the component fields it sets, so
//! a change of a control becomes ordinary scene edits (undoable, saved with the
//! scene). It also reads those fields back, so the window shows the values of
//! the open scene.
//!
//! A control with no component field is not part of an editor scene.
//! [`applies`] says which. Every control of the window has one now.
#![allow(clippy::float_arithmetic)]

use crate::editor::{Editor, Owner};
use crate::model::Target;
use crate::surface_library::SHADER_DIR;
use orr_ecs::Entity;
use orr_gfx::names;
use orr_reflect::{decimal, Value};
use orr_scenery::settings::{COLORS, SLIDERS, TOGGLES};
use orr_scenery::Settings;

/// Where one field of the scene is.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// A field of a scene singleton.
    Singleton(&'static str),
    /// A field of a component of one entity.
    Entity(Entity, &'static str),
}

/// One field to set in the open scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub place: Place,
    pub path: String,
    pub value: Value,
}

/// How a control is stored in the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Binding {
    /// A number of the `Sky` singleton, the setting times `scale`.
    Sky { field: &'static str, scale: f32 },
    /// The ambient strength of the `Environment` singleton.
    Ambient,
    /// The exposure of the `PostProcess` singleton.
    Exposure,
    /// A number of the sun's `Light`.
    Sun { field: &'static str },
    /// Whether the sun's `Light` casts shadows.
    SunShadows,
    /// The rotation of the sun, from the hour of the day.
    SunHour,
    /// A number of every `Water`.
    Water { field: &'static str },
    /// The speed of the current of every `Water`, keeping its direction.
    WaterFlow,
    /// The deep color of every `Water`.
    WaterColor,
    /// One number of a post effect, the setting times `scale`. With `switch`
    /// the effect is on exactly while the setting is above zero.
    Effect { kind: &'static str, param: usize, scale: f32, switch: bool },
    /// A post effect that a checkbox turns on and off.
    EffectSwitch { kind: &'static str },
    /// A number of every `Scatter`. The window shows the value of the first scatter
    /// of `kind`, or of the first one when the scene has none of that kind.
    Scatter { field: &'static str, kind: Option<u32> },
    /// The tint of every `Scatter`.
    ScatterTint,
    /// A number of the surface shader of every terrain, stored in the terrain's `Material`.
    Ground { param: &'static str },
}

/// Speed of the current of a `Water` for the setting `Flow speed` 1, in meters per second.
const FLOW: f32 = 0.6;

fn binding(group: &str, name: &str) -> Option<Binding> {
    use Binding::*;
    let sky = |field, scale| Some(Sky { field, scale });
    let effect = |kind, param, scale, switch| Some(Effect { kind, param, scale, switch });
    let scatter = |field, kind| Some(Scatter { field, kind });
    match (group, name) {
        ("Sun", "Hour") => Some(SunHour),
        ("Sun", "Intensity") => Some(Sun { field: "intensity" }),
        ("Sun", "Ambient") => Some(Ambient),
        ("Sky", "Brightness") => sky("brightness", 1.0),
        ("Sky", "Sun glow") => sky("sun_glow", 1.0),
        ("Sky", "Cloud cover") => sky("cloud_cover", 1.0),
        ("Sky", "Cloud speed") => sky("cloud_speed", 1.0),
        // The generated Forest Valley scene has 0.7 where the window has 1.
        ("Fog", "Density") => sky("fog_density", 0.7),
        ("Fog", "Height falloff") => sky("fog_falloff", 1.0),
        ("Fog", "Haze") => sky("haze", 1.0),
        ("Shadows", "Sun shadows") => Some(SunShadows),
        ("Shadows", "Softness") => Some(Sun { field: "shadow_softness" }),
        ("Shadows", "Sharp range") => Some(Sun { field: "shadow_sharp_range" }),
        ("Shadows", "Wide range") => Some(Sun { field: "shadow_range" }),
        ("Wind", "Strength") => sky("wind_strength", 1.0),
        ("Wind", "Speed") => sky("wind_speed", 1.0),
        ("Water", "Deep water color") => Some(WaterColor),
        ("Water", "Clarity") => Some(Water { field: "clarity" }),
        ("Water", "Ripples") => Some(Water { field: "ripples" }),
        ("Water", "Flow speed") => Some(WaterFlow),
        ("Water", "Foam") => Some(Water { field: "foam" }),
        ("Water", "Sun glint") => Some(Water { field: "glint" }),
        ("Vegetation", "Translucency") => scatter("translucency", None),
        ("Vegetation", "Leaf detail") => scatter("leaf_detail", None),
        ("Vegetation", "Autumn trees") => scatter("autumn", Some(orr_gfx::SCATTER_BROADLEAF)),
        ("Vegetation", "Grass distance") => scatter("grass_distance", Some(orr_gfx::SCATTER_GRASS)),
        ("Vegetation", "Tree detail distance") => scatter("detail_distance", None),
        ("Vegetation", "Leaf tint") => Some(ScatterTint),
        ("Ground", "Bumps") => Some(Ground { param: "bumps" }),
        ("Ground", "Caustics") => Some(Water { field: "caustics" }),
        ("Look", "Exposure") => Some(Exposure),
        ("Look", "Auto exposure") => Some(EffectSwitch { kind: "auto_exposure" }),
        ("Look", "Adaptation speed") => effect("auto_exposure", 2, 1.0, false),
        ("Look", "Bloom") => effect("bloom", 1, 1.0, true),
        ("Look", "Bloom threshold") => effect("bloom", 0, 1.0, false),
        ("Look", "Light shafts") => effect("light_shafts", 0, 1.0, true),
        ("Look", "Saturation") => effect("color_grade", 0, 1.0, false),
        ("Look", "Contrast") => effect("color_grade", 1, 1.0, false),
        ("Look", "Vignette") => effect("vignette", 0, 1.0, true),
        // The generated scene has 0.04 where the window has 1.
        ("Look", "Grain") => effect("film_grain", 0, 0.04, true),
        _ => None,
    }
}

/// Whether the control `name` of `group` is stored in an editor scene.
pub fn applies(group: &str, name: &str) -> bool {
    binding(group, name).is_some()
}

fn fixed(v: f32) -> Value {
    Value::Fixed(decimal::parse_fp(&format!("{v:.4}")).unwrap_or(orr_fp::FP::ZERO))
}

fn number(value: &Value) -> Option<f32> {
    if let Value::Fixed(f) = value { decimal::fp_to_decimal(*f).parse().ok() } else { None }
}

fn singleton<'a>(editor: &'a Editor, name: &str) -> Option<&'a Value> {
    editor.singletons().iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// Whether the open scene keeps the components of the window's settings: a game
/// that registers `orr_gfx`.
pub fn available(editor: &Editor) -> bool {
    [names::SKY, names::ENVIRONMENT, names::POST_PROCESS].iter().all(|name| singleton(editor, name).is_some())
}

/// The entity of the scene's sun: its first enabled directional light.
fn sun(editor: &Editor) -> Option<Entity> {
    editor.yard_frame().look.as_ref()?.lights.iter().find(|light| light.sun).map(|light| light.entity)
}

fn waters(editor: &Editor) -> Vec<(Entity, [f32; 2])> {
    editor.yard_frame().waters.iter().map(|water| (water.entity, water.flow)).collect()
}

/// Pitch, yaw and roll in degrees of an entity that faces `direction`.
fn rotation_facing(direction: [f32; 3]) -> [f32; 3] {
    let length = direction.iter().map(|c| c * c).sum::<f32>().sqrt().max(1.0e-6);
    let d = direction.map(|c| c / length);
    [(-d[1]).asin().to_degrees(), d[0].atan2(d[2]).to_degrees(), 0.0]
}

/// Hour of the day of a sun that shines along `direction`: the inverse of
/// `Settings::sun_direction` for the direction around the compass.
fn hour_of(direction: [f32; 3]) -> f32 {
    let toward_sun = direction.map(|c| -c);
    let azimuth = toward_sun[0].atan2(toward_sun[2]).to_degrees();
    // The angle from the noon azimuth, brought into -180..180 degrees.
    let from_noon = (azimuth - 162.0 + 540.0).rem_euclid(360.0) - 180.0;
    (12.0 + from_noon / 13.0).clamp(5.0, 19.0)
}

fn vec3(v: [f32; 3]) -> Value {
    let Value::Fixed(x) = fixed(v[0]) else { unreachable!("fixed makes a fixed value") };
    let Value::Fixed(y) = fixed(v[1]) else { unreachable!("fixed makes a fixed value") };
    let Value::Fixed(z) = fixed(v[2]) else { unreachable!("fixed makes a fixed value") };
    Value::Vec3(orr_fp::FPVec3::new(x, y, z))
}

fn vec2(v: [f32; 2]) -> Value {
    let Value::Fixed(x) = fixed(v[0]) else { unreachable!("fixed makes a fixed value") };
    let Value::Fixed(y) = fixed(v[1]) else { unreachable!("fixed makes a fixed value") };
    Value::Vec2(orr_fp::FPVec2::new(x, y))
}

/// The terrains whose surface shader declares the number `param`, as (entity, place of the
/// number in the material, the shader's default). The shader is the file of its name in the
/// shader folder next to the scene file, as the viewport reads it.
fn ground_params(editor: &Editor, param: &str) -> Vec<(Entity, usize, f32)> {
    let Some(folder) = editor.path().and_then(|scene| scene.parent().map(std::path::Path::to_path_buf)) else { return Vec::new() };
    let frame = editor.yard_frame();
    let mut found = Vec::new();
    for material in &frame.materials {
        let name = material.shader.as_str();
        let plain = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !plain || !frame.terrains.iter().any(|t| t.entity == material.entity) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(folder.join(SHADER_DIR).join(format!("{name}.wgsl"))) else { continue };
        let Ok(shader) = orr_render::SurfaceShader::compile(name, &source) else { continue };
        if let Some(declared) = shader.params().iter().find(|p| p.name == param && p.kind == orr_render::ParamKind::Float) {
            found.push((material.entity, declared.offset, declared.default[0]));
        }
    }
    found
}

/// The post effects of the scene as (slot, kind name, enabled, numbers).
fn effects(editor: &Editor) -> Vec<(usize, String, bool, [f32; 4])> {
    let Some(Value::Array(items)) = singleton(editor, names::POST_PROCESS).and_then(|post| post.field("effects")) else { return Vec::new() };
    items
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| {
            let Some(Value::Enum(kind)) = item.field("kind") else { return None };
            let Some(Value::Bool(enabled)) = item.field("enabled") else { return None };
            let Some(Value::Array(params)) = item.field("params") else { return None };
            let mut numbers = [0.0; 4];
            for (n, p) in numbers.iter_mut().zip(params) {
                *n = number(p).unwrap_or(0.0);
            }
            Some((slot, kind.clone(), *enabled, numbers))
        })
        .collect()
}

fn kind_id(kind: &str) -> u32 {
    match kind {
        "auto_exposure" => orr_gfx::EFFECT_AUTO_EXPOSURE,
        "bloom" => orr_gfx::EFFECT_BLOOM,
        "light_shafts" => orr_gfx::EFFECT_LIGHT_SHAFTS,
        "color_grade" => orr_gfx::EFFECT_COLOR_GRADE,
        "vignette" => orr_gfx::EFFECT_VIGNETTE,
        "film_grain" => orr_gfx::EFFECT_FILM_GRAIN,
        _ => orr_gfx::EFFECT_NONE,
    }
}

/// The post effects that no control of the window switches on or off: they
/// work as soon as a number of them is set, so an edit of a number turns them on.
const ALWAYS_ON: &[&str] = &["color_grade"];

/// The edits that set `param` of the effect `kind` to `value` and switch it on or off.
/// The effect keeps its slot; a scene without it gets it in the first free slot,
/// with the default numbers. With no free slot nothing changes. An effect that no
/// control switches (`ALWAYS_ON`) is created enabled, and an edit of one of its
/// numbers enables it when the scene has it disabled.
fn effect_changes(editor: &Editor, kind: &'static str, param: Option<(usize, f32)>, enabled: Option<bool>) -> Vec<Change> {
    let enabled = enabled.or_else(|| {
        let was_on = effects(editor).iter().find(|(_, k, _, _)| k == kind).map(|(_, _, on, _)| *on);
        (param.is_some() && ALWAYS_ON.contains(&kind) && was_on != Some(true)).then_some(true)
    });
    let list = effects(editor);
    let set = |slot: usize, path: &str, value: Value| Change { place: Place::Singleton(names::POST_PROCESS), path: format!("effects[{slot}].{path}"), value };
    let mut out = Vec::new();
    let slot = match list.iter().find(|(_, k, _, _)| k == kind) {
        Some((slot, ..)) => *slot,
        None => {
            let Some((slot, ..)) = list.iter().find(|(_, k, _, _)| k == "none") else { return out };
            out.push(set(*slot, "kind", Value::Enum(kind.into())));
            let defaults = orr_gfx::PostEffect::new(kind_id(kind));
            for (i, default) in defaults.params.iter().enumerate() {
                out.push(set(*slot, &format!("params[{i}]"), Value::Fixed(*default)));
            }
            *slot
        }
    };
    if let Some((i, value)) = param {
        out.push(set(slot, &format!("params[{i}]"), fixed(value)));
    }
    if let Some(enabled) = enabled {
        out.push(set(slot, "enabled", Value::Bool(enabled)));
    }
    out
}

/// The scene edits for the control `name` of `group`, which now has the value it
/// has in `settings`. Empty when the control is not part of an editor scene or
/// the scene has nothing to hold it (no sun, no water).
pub fn changes(editor: &Editor, group: &str, name: &str, settings: &Settings) -> Vec<Change> {
    let Some(binding) = binding(group, name).filter(|_| available(editor)) else { return Vec::new() };
    let slider =|| SLIDERS.iter().find(|s| s.group == group && s.name == name).map(|s| (s.get)(settings));
    let toggle = || TOGGLES.iter().find(|t| t.group == group && t.name == name).map(|t| (t.get)(settings));
    let on_sun = |field: &'static str, component: &'static str, value: Value| sun(editor).map(|e| Change { place: Place::Entity(e, component), path: field.into(), value });
    let on_scatters = |field: &'static str, value: Value| -> Vec<Change> {
        editor.yard_frame().scatters.iter().map(|s| Change { place: Place::Entity(s.entity, names::SCATTER), path: field.into(), value: value.clone() }).collect()
    };
    let on_waters = |f: &dyn Fn(Entity, [f32; 2]) -> (&'static str, Value)| -> Vec<Change> {
        waters(editor)
            .into_iter()
            .map(|(entity, flow)| {
                let (path, value) = f(entity, flow);
                Change { place: Place::Entity(entity, names::WATER), path: path.into(), value }
            })
            .collect()
    };
    match binding {
        Binding::Sky { field, scale } => slider().map(|v| vec![Change { place: Place::Singleton(names::SKY), path: field.into(), value: fixed(v * scale) }]).unwrap_or_default(),
        Binding::Ambient => slider().map(|v| vec![Change { place: Place::Singleton(names::ENVIRONMENT), path: "ambient".into(), value: fixed(v) }]).unwrap_or_default(),
        Binding::Exposure => slider().map(|v| vec![Change { place: Place::Singleton(names::POST_PROCESS), path: "exposure".into(), value: fixed(v) }]).unwrap_or_default(),
        Binding::Sun { field } => slider().and_then(|v| on_sun(field, names::LIGHT, fixed(v))).into_iter().collect(),
        Binding::SunShadows => toggle().and_then(|v| on_sun("cast_shadows", names::LIGHT, Value::Bool(v))).into_iter().collect(),
        Binding::SunHour => {
            let Some(entity) = sun(editor) else { return Vec::new() };
            let toward_sun = settings.sun_direction();
            let rotation = rotation_facing(toward_sun.map(|c| -c));
            vec![Change { place: Place::Entity(entity, names::TRANSFORM), path: "rotation".into(), value: vec3(rotation) }]
        }
        Binding::Water { field } => slider().map(|v| on_waters(&|_, _| (field, fixed(v)))).unwrap_or_default(),
        Binding::WaterFlow => slider()
            .map(|v| {
                on_waters(&|_, flow| {
                    let length = (flow[0] * flow[0] + flow[1] * flow[1]).sqrt();
                    let direction = if length > 1.0e-4 { [flow[0] / length, flow[1] / length] } else { [0.0, 1.0] };
                    ("flow", vec2([direction[0] * FLOW * v, direction[1] * FLOW * v]))
                })
            })
            .unwrap_or_default(),
        Binding::WaterColor => COLORS
            .iter()
            .find(|c| c.group == group && c.name == name)
            .map(|c| (c.get)(settings))
            .map(|color| on_waters(&|_, _| ("color", vec3(color.map(|c| c.clamp(0.0, 1.0))))))
            .unwrap_or_default(),
        Binding::Effect { kind, param, scale, switch } => match slider() {
            Some(v) => effect_changes(editor, kind, Some((param, v * scale)), switch.then_some(v > 0.0)),
            None => Vec::new(),
        },
        Binding::EffectSwitch { kind } => toggle().map(|on| effect_changes(editor, kind, None, Some(on))).unwrap_or_default(),
        Binding::Scatter { field, .. } => slider().map(|v| on_scatters(field, fixed(v))).unwrap_or_default(),
        Binding::ScatterTint => COLORS
            .iter()
            .find(|c| c.group == group && c.name == name)
            .map(|c| (c.get)(settings))
            .map(|color| on_scatters("tint", vec3(color.map(|c| c.clamp(0.0, 2.0)))))
            .unwrap_or_default(),
        Binding::Ground { param } => slider().map(|v| ground_changes(editor, param, v)).unwrap_or_default(),
    }
}

/// The edits that set the shader number `param` of every terrain to `value`.
fn ground_changes(editor: &Editor, param: &str, value: f32) -> Vec<Change> {
    let mut out = Vec::new();
    for (entity, offset, _) in ground_params(editor, param) {
        let Some(material) = editor.yard_frame().materials.iter().find(|m| m.entity == entity) else { continue };
        out.push(Change { place: Place::Entity(entity, names::MATERIAL), path: format!("values[{offset}]"), value: fixed(value) });
        // The number counts only while its bit is set.
        if material.overrides() & (1 << offset) == 0 {
            out.push(Change { place: Place::Entity(entity, names::MATERIAL), path: "overrides".into(), value: Value::Int(i128::from(material.overrides() | (1 << offset))) });
        }
    }
    out
}

/// The value of a numeric control as the open scene has it.
fn scene_number(editor: &Editor, binding: Binding) -> Option<f32> {
    let from = |singleton_name: &str, field: &str| singleton(editor, singleton_name)?.field(field).and_then(number);
    match binding {
        Binding::Sky { field, scale } => from(names::SKY, field).map(|v| v / scale),
        Binding::Ambient => from(names::ENVIRONMENT, "ambient"),
        Binding::Exposure => from(names::POST_PROCESS, "exposure"),
        Binding::Sun { field: "intensity" } => editor.yard_frame().look.as_ref()?.lights.iter().find(|l| l.sun).map(|l| l.light.intensity),
        Binding::Sun { field: "shadow_softness" } => {
            let look = editor.yard_frame().look.as_ref()?;
            look.lights.iter().any(|l| l.sun).then_some(look.shadow_softness)
        }
        // Zero is the engine's own split, which has no number to show.
        Binding::Sun { field: "shadow_sharp_range" } => editor.yard_frame().look.as_ref().map(|look| look.shadow_sharp_range).filter(|v| *v > 0.0),
        Binding::Sun { .. } => {
            let look = editor.yard_frame().look.as_ref()?;
            look.shadow_follows_view.then_some(look.lighting.shadow_radius)
        }
        Binding::SunHour => {
            let light = editor.yard_frame().look.as_ref()?.lights.iter().find(|l| l.sun)?;
            Some(hour_of(light.light.direction))
        }
        Binding::Water { field } => {
            let water = editor.yard_frame().waters.first()?;
            Some(match field {
                "clarity" => water.clarity,
                "ripples" => water.ripples,
                "foam" => water.foam,
                "caustics" => water.caustics,
                _ => water.glint,
            })
        }
        Binding::WaterFlow => {
            let flow = editor.yard_frame().waters.first()?.flow;
            Some((flow[0] * flow[0] + flow[1] * flow[1]).sqrt() / FLOW)
        }
        Binding::Effect { kind, param, scale, switch } => {
            let list = effects(editor);
            match list.iter().find(|(_, k, _, _)| k == kind) {
                Some((_, _, enabled, numbers)) => Some(if switch && !enabled { 0.0 } else { numbers[param] / scale }),
                None => switch.then_some(0.0),
            }
        }
        Binding::Scatter { field, kind } => {
            let scatters = &editor.yard_frame().scatters;
            let scatter = kind.and_then(|k| scatters.iter().find(|s| s.kind == k)).or_else(|| scatters.first())?;
            Some(match field {
                "translucency" => scatter.translucency,
                "leaf_detail" => scatter.leaf_detail,
                "autumn" => scatter.autumn,
                "grass_distance" => scatter.grass_distance,
                _ => scatter.detail_distance,
            })
        }
        Binding::Ground { param } => {
            let (entity, offset, default) = *ground_params(editor, param).first()?;
            let material = editor.yard_frame().materials.iter().find(|m| m.entity == entity)?;
            Some(if material.overrides() & (1 << offset) != 0 { material.stored(offset) } else { default })
        }
        Binding::SunShadows | Binding::WaterColor | Binding::EffectSwitch { .. } | Binding::ScatterTint => None,
    }
}

/// `settings` with every control that the open scene stores set to the scene's value.
pub fn read(editor: &Editor, mut settings: Settings) -> Settings {
    if !available(editor) {
        return settings;
    }
    for slider in SLIDERS {
        if let Some(value) = binding(slider.group, slider.name).and_then(|b| scene_number(editor, b)) {
            (slider.set)(&mut settings, value);
        }
    }
    for toggle in TOGGLES {
        match binding(toggle.group, toggle.name) {
            Some(Binding::SunShadows) => {
                if let Some(light) = editor.yard_frame().look.as_ref().and_then(|look| look.lights.iter().find(|l| l.sun)) {
                    (toggle.set)(&mut settings, light.light.shadows);
                }
            }
            Some(Binding::EffectSwitch { kind }) => {
                let on = effects(editor).iter().any(|(_, k, enabled, _)| k == kind && *enabled);
                (toggle.set)(&mut settings, on);
            }
            _ => {}
        }
    }
    for color in COLORS {
        match binding(color.group, color.name) {
            Some(Binding::WaterColor) => {
                if let Some(water) = editor.yard_frame().waters.first() {
                    (color.set)(&mut settings, water.color);
                }
            }
            Some(Binding::ScatterTint) => {
                if let Some(scatter) = editor.yard_frame().scatters.first() {
                    (color.set)(&mut settings, scatter.tint);
                }
            }
            _ => {}
        }
    }
    settings.sanitized()
}

/// Sets the fields in the open scene. Returns how many were accepted. The editor
/// reports a refusal in its own message line. One field of many entities (the
/// water surfaces) goes in one request and so is one edit of a drag, however many
/// entities there are.
pub fn apply(editor: &mut Editor, changes: &[Change]) -> usize {
    let mut accepted = 0;
    let mut done = vec![false; changes.len()];
    for first in 0..changes.len() {
        if done[first] {
            continue;
        }
        let change = &changes[first];
        match &change.place {
            Place::Singleton(name) => {
                accepted += usize::from(editor.set_field(&Owner::Singleton((*name).into()), &change.path, change.value.clone()));
            }
            Place::Entity(_, component) => {
                let same = |other: &Change| matches!(&other.place, Place::Entity(_, c) if c == component) && other.path == change.path;
                let group: Vec<usize> = (first..changes.len()).filter(|&i| !done[i] && same(&changes[i])).collect();
                let patches: Vec<(Target, Value)> = group
                    .iter()
                    .filter_map(|&i| {
                        let Place::Entity(entity, _) = &changes[i].place else { return None };
                        let target = editor.rows().iter().find(|row| row.entity == *entity).map_or(Target::Entity(*entity), |row| row.target());
                        Some((target, changes[i].value.clone()))
                    })
                    .collect();
                let ok = match patches.as_slice() {
                    [(target, value)] => editor.set_entity_field(target, component, &change.path, value.clone()),
                    _ => editor.set_entity_fields(&format!("scene window: {component}.{}", change.path), component, &change.path, &patches),
                };
                accepted += usize::from(ok) * patches.len();
                for i in group {
                    done[i] = true;
                }
            }
        }
        done[first] = true;
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_faces_the_direction_it_was_made_from() {
        for d in [[0.3_f32, -0.8, 0.5], [-0.6, -0.2, -0.77], [0.0, -1.0, 0.01], [0.9, 0.1, -0.4]] {
            let length = d.iter().map(|c| c * c).sum::<f32>().sqrt();
            let unit = d.map(|c| c / length);
            let facing = orr_gfx_view::facing(rotation_facing(unit));
            for i in 0..3 {
                assert!((facing[i] - unit[i]).abs() < 1.0e-4, "{d:?} -> {facing:?}");
            }
        }
    }

    #[test]
    fn the_hour_survives_the_trip_through_a_rotation() {
        for hour in [5.0_f32, 8.5, 12.0, 16.6, 19.0] {
            let mut settings = Settings::default();
            settings.sun.hour = hour;
            let shines = settings.sun_direction().map(|c| -c);
            let back = hour_of(orr_gfx_view::facing(rotation_facing(shines)));
            assert!((back - hour).abs() < 0.01, "{hour} -> {back}");
        }
    }

    #[test]
    fn every_control_is_bound() {
        let controls = SLIDERS.iter().map(|s| (s.group, s.name)).chain(TOGGLES.iter().map(|t| (t.group, t.name))).chain(COLORS.iter().map(|c| (c.group, c.name)));
        for (group, name) in controls {
            assert!(applies(group, name), "{group} / {name}");
        }
    }
}
