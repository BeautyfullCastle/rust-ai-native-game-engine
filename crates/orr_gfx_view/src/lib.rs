//! `orr_gfx_view`: the rendering components of a scene, read for the renderer.
//!
//! [`scene_look`] reads [`orr_gfx`] components from one frame and returns what
//! the renderer needs: the sun and ambient light ([`orr_render::Lighting`]),
//! the other lights with their layer masks ([`orr_render::SceneLights`]) and
//! the bloom settings. This is the only place where the fixed-point scene
//! values become floats. Nothing is written back.
//!
//! The function works for every game. A game that did not register the
//! `orr_gfx` types keeps the lighting passed as `fallback`.
#![allow(clippy::float_arithmetic)]

#[cfg(feature = "models")]
pub mod terrain_mesh;

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{FPVec3, FP};
use orr_gfx::{
    Environment, Light, PostEffect, PostProcess, RenderLayers, Transform3, EFFECT_AUTO_EXPOSURE, EFFECT_BLOOM, EFFECT_COLOR_GRADE, EFFECT_FILM_GRAIN,
    EFFECT_LIGHT_SHAFTS, EFFECT_VIGNETTE, LIGHT_DIRECTIONAL, LIGHT_SPOT, PROJECTION_ORTHOGRAPHIC, TONE_CLAMP,
};
use orr_physics3d::Body;
use orr_render::{Camera3D, Lighting, Projection, RenderList3D, SceneLight, SceneLightKind, SceneLights, ScreenEffect};

/// Bloom as the scene asks for it. The caller passes it to the post chain it uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    pub threshold: f32,
    pub strength: f32,
    pub radius: u32,
    pub iterations: u32,
}

/// One light of the scene with the entity that carries it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedLight {
    pub entity: Entity,
    pub light: SceneLight,
    /// True for the light that became the sun of [`SceneLook::lighting`].
    pub sun: bool,
    /// False for a light that is switched off or has no intensity. It gives no light.
    pub enabled: bool,
}

/// What a scene says about its lighting and its final picture, in renderer terms.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneLook {
    /// The sun, the ambient light, exposure and tone mapping.
    pub lighting: Lighting,
    /// The lights beyond the sun, and the sun's layers and shadow strength.
    pub scene_lights: SceneLights,
    /// `Some` when the scene turns bloom on.
    pub bloom: Option<Bloom>,
    /// The enabled effects beyond bloom, in the scene's order.
    pub effects: Vec<ScreenEffect>,
    /// Every light of the scene in entity order, switched off ones included, for editor markers.
    pub lights: Vec<PlacedLight>,
    /// Enabled lights beyond the frame limit of `orr_render::MAX_SCENE_LIGHTS`. They give no light.
    pub dropped_lights: u32,
}

fn f(v: FP) -> f32 {
    v.to_f32()
}

fn v3(v: FPVec3) -> [f32; 3] {
    [f(v.x), f(v.y), f(v.z)]
}

fn unit_color(v: FPVec3) -> [f32; 3] {
    v3(v).map(|c| c.clamp(0.0, 1.0))
}

/// The direction an entity with the rotation `degrees` (pitch, yaw, roll) faces.
pub fn facing(degrees: [f32; 3]) -> [f32; 3] {
    let (pitch, yaw) = (degrees[0].to_radians(), degrees[1].to_radians());
    [pitch.cos() * yaw.sin(), -pitch.sin(), pitch.cos() * yaw.cos()]
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let w = cross(u, t);
    [v[0] + q[3] * t[0] + w[0], v[1] + q[3] * t[1] + w[1], v[2] + q[3] * t[2] + w[2]]
}

/// Position and facing direction of an entity: from its physics body when it
/// has one, else from its [`Transform3`], else the origin facing +z.
pub fn placement(frame: FrameView<'_>, entity: Entity) -> ([f32; 3], [f32; 3]) {
    if frame.registers::<Body>() {
        if let Some(body) = frame.get::<Body>(entity) {
            let q = [f(body.rot.x), f(body.rot.y), f(body.rot.z), f(body.rot.w)];
            return (v3(body.pos), rotate(q, [0.0, 0.0, 1.0]));
        }
    }
    match frame.get::<Transform3>(entity) {
        Some(transform) => (v3(transform.position), facing(v3(transform.rotation))),
        None => ([0.0; 3], [0.0, 0.0, 1.0]),
    }
}

/// The render layers of an entity: its [`RenderLayers`], else the default layer.
/// Also the default layer when the game did not register the component.
pub fn layers_of(frame: FrameView<'_>, entity: Entity) -> u32 {
    if !frame.registers::<RenderLayers>() {
        return orr_gfx::DEFAULT_LAYER;
    }
    frame.get::<RenderLayers>(entity).map_or(orr_gfx::DEFAULT_LAYER, |layers| layers.layers & orr_gfx::ALL_LAYERS)
}

/// Reads the scene's lights, environment and post-processing.
///
/// `fallback` is the lighting of the game when the scene says nothing: it is
/// kept whole for a game without `orr_gfx` types, its sun is kept for a scene
/// without any [`Light`] component, and its ambient light is kept while the
/// scene's [`Environment`] has its default values. Its shadow area is always kept.
pub fn scene_look(frame: FrameView<'_>, fallback: &Lighting) -> SceneLook {
    let mut look = SceneLook { lighting: *fallback, scene_lights: SceneLights::default(), bloom: None, effects: Vec::new(), lights: Vec::new(), dropped_lights: 0 };
    // An environment left at its default values keeps the ambient light of the game.
    // Simulation systems can write any value, so every value is brought into
    // the range the inspector allows before the renderer sees it.
    if let Some(environment) = frame.try_singleton::<Environment>().filter(|e| **e != Environment::default()) {
        look.lighting.sky = unit_color(environment.sky_color);
        look.lighting.ground = unit_color(environment.ground_color);
        look.lighting.ambient = f(environment.ambient).clamp(0.0, 10.0);
    }
    if let Some(post) = frame.try_singleton::<PostProcess>() {
        look.lighting.exposure = f(post.exposure).clamp(0.01, 16.0);
        look.lighting.tonemap = post.tone_mapping != TONE_CLAMP;
        for effect in post.effects.iter().filter(|e| e.enabled != 0) {
            read_effect(effect, &mut look);
        }
    }
    if !frame.registers::<Light>() || frame.count::<Light>() == 0 {
        return look;
    }
    // The scene has lights: only they light it. Without an enabled directional light there is no sun.
    look.lighting.intensity = 0.0;
    look.lighting.shadows = false;
    let mut has_sun = false;
    for (entity, light) in frame.iter::<Light>() {
        let enabled = light.enabled != 0 && light.intensity > FP::ZERO;
        let (position, direction) = placement(frame, entity);
        let kind = match light.kind {
            LIGHT_DIRECTIONAL => SceneLightKind::Directional,
            LIGHT_SPOT => SceneLightKind::Spot,
            _ => SceneLightKind::Point,
        };
        let inner = f(light.inner_angle).clamp(0.0, 88.0).to_radians();
        let scene_light = SceneLight {
            kind,
            position: position.map(|c| c.clamp(-1.0e6, 1.0e6)),
            direction,
            color: unit_color(light.color),
            intensity: f(light.intensity).clamp(0.0, 100.0),
            range: f(light.range).clamp(0.01, 1000.0),
            inner_angle: inner,
            outer_angle: f(light.outer_angle).clamp(0.1, 89.0).to_radians().max(inner),
            layers: light.layers & orr_gfx::ALL_LAYERS,
        };
        if !enabled {
            look.lights.push(PlacedLight { entity, light: scene_light, sun: false, enabled });
            continue;
        }
        // The first enabled directional light with some intensity is the sun.
        let sun = kind == SceneLightKind::Directional && !has_sun;
        if sun {
            has_sun = true;
            look.lighting.direction = direction;
            look.lighting.color = scene_light.color;
            look.lighting.intensity = scene_light.intensity;
            look.lighting.shadows = light.cast_shadows != 0;
            look.scene_lights.sun_layers = scene_light.layers;
            look.scene_lights.sun_shadow_strength = f(light.shadow_strength);
        } else if look.scene_lights.push(scene_light).is_err() {
            look.dropped_lights += 1;
            continue;
        }
        look.lights.push(PlacedLight { entity, light: scene_light, sun, enabled });
    }
    look
}

/// Effect number `i` of `effect`, inside the range [`orr_gfx::effect_params`] gives it.
fn effect_number(effect: &PostEffect, i: usize) -> f32 {
    let (_, low, high, _) = orr_gfx::effect_params(effect.kind).get(i).copied().unwrap_or(("", FP::ZERO, FP::ZERO, FP::ZERO));
    f(effect.params[i].max(low).min(high))
}

fn read_effect(effect: &PostEffect, look: &mut SceneLook) {
    let p = |i| effect_number(effect, i);
    let screen = match effect.kind {
        // The first bloom wins; bloom has its own place in the chain.
        EFFECT_BLOOM => {
            if look.bloom.is_none() {
                look.bloom = Some(Bloom { threshold: p(0), strength: p(1), radius: p(2).round() as u32, iterations: p(3).round() as u32 });
            }
            None
        }
        EFFECT_LIGHT_SHAFTS => Some(ScreenEffect::LightShafts { strength: p(0), length: p(1) }),
        EFFECT_AUTO_EXPOSURE => Some(ScreenEffect::AutoExposure { darkest: p(0).min(p(1)), brightest: p(1), speed: p(2), target: p(3) }),
        EFFECT_COLOR_GRADE => Some(ScreenEffect::ColorGrade { saturation: p(0), contrast: p(1), warmth: p(2), tint: p(3) }),
        EFFECT_VIGNETTE => Some(ScreenEffect::Vignette { strength: p(0), radius: p(1), softness: p(2) }),
        EFFECT_FILM_GRAIN => Some(ScreenEffect::FilmGrain { strength: p(0) }),
        _ => None,
    };
    // Fixed point rounds differently from the renderer's float ranges; the effect is brought into them.
    if let Some(screen) = screen.filter(|_| look.effects.len() < orr_render::post_effects::MAX_SCREEN_EFFECTS) {
        look.effects.push(screen.clamped());
    }
}

/// The scene's main camera as the renderer takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraLook {
    pub entity: Entity,
    pub camera: Camera3D,
    /// The render layers the camera draws.
    pub layers: u32,
}

/// The active [`orr_gfx::Camera`] with the highest priority; among equal
/// priorities the first in entity order. `None` when the scene has none.
pub fn main_camera(frame: FrameView<'_>) -> Option<CameraLook> {
    if !frame.registers::<orr_gfx::Camera>() {
        return None;
    }
    let mut best: Option<(i32, Entity, orr_gfx::Camera)> = None;
    for (entity, camera) in frame.iter::<orr_gfx::Camera>() {
        if camera.active != 0 && best.as_ref().is_none_or(|(priority, _, _)| camera.priority > *priority) {
            best = Some((camera.priority, entity, *camera));
        }
    }
    let (_, entity, camera) = best?;
    let (eye, facing) = placement(frame, entity);
    let near = f(camera.near).clamp(0.01, 1000.0);
    let far = f(camera.far).clamp(near + 0.01, 100_000.0);
    let projection = if camera.projection == PROJECTION_ORTHOGRAPHIC {
        Projection::Orthographic { half_height: f(camera.half_height).clamp(0.01, 10_000.0), near: -far, far }
    } else {
        Projection::Perspective { fov_y: f(camera.fov_y).clamp(1.0, 179.0).to_radians(), near, far }
    };
    // Looking straight up or down, the world's up is no help; z is then the camera's up.
    let up = if facing[1].abs() > 0.999 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let target = [eye[0] + facing[0], eye[1] + facing[1], eye[2] + facing[2]];
    Some(CameraLook { entity, camera: Camera3D { eye, target, up, projection }, layers: camera.layers & orr_gfx::ALL_LAYERS })
}

/// A terrain of one entity as the view reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainRef {
    pub entity: Entity,
    /// Path of the heightmap image, relative to the scene's folder; empty for none.
    pub heightmap: String,
    /// World position of the terrain's center at height zero.
    pub center: [f32; 3],
    /// Size in meters along x and z.
    pub size: [f32; 2],
    /// Height in meters of a white pixel.
    pub height: f32,
    /// Most grid points per side.
    pub resolution: u32,
    pub cast_shadows: bool,
}

/// Every entity's terrain, in entity order. Empty for a game without `orr_gfx` terrains.
pub fn terrains(frame: FrameView<'_>) -> Vec<TerrainRef> {
    if !frame.registers::<orr_gfx::Terrain>() {
        return Vec::new();
    }
    frame
        .iter::<orr_gfx::Terrain>()
        .map(|(entity, terrain)| TerrainRef {
            entity,
            heightmap: terrain.heightmap_path().to_string(),
            center: placement(frame, entity).0,
            size: [f(terrain.size.x).clamp(1.0, 20_000.0), f(terrain.size.y).clamp(1.0, 20_000.0)],
            height: f(terrain.height).clamp(0.0, 5000.0),
            resolution: terrain.resolution.clamp(8, 352),
            cast_shadows: terrain.cast_shadows != 0,
        })
        .collect()
}

impl SceneLook {
    /// Puts the lighting into a render list.
    pub fn apply(&self, list: &mut RenderList3D) {
        list.lighting = self.lighting;
        list.scene_lights = self.scene_lights;
    }

    /// Draws a marker for every light, so a person can see and pick lights in
    /// an editor view: a small cross at a point or spot light, and a line along
    /// the direction of a directional or spot light. The marker has the light's
    /// color; the sun's is yellow and a switched off light's is gray. The
    /// selected light's marker is wider.
    pub fn draw_markers(&self, list: &mut RenderList3D, selected: Option<Entity>) {
        for placed in &self.lights {
            let light = &placed.light;
            let highlight = selected == Some(placed.entity);
            let brightest = light.color.iter().copied().fold(0.2, f32::max);
            let color = if !placed.enabled {
                [0.45, 0.45, 0.45, 0.8]
            } else if placed.sun {
                [1.0, 0.85, 0.2, 1.0]
            } else {
                [light.color[0] / brightest, light.color[1] / brightest, light.color[2] / brightest, 1.0]
            };
            let width = if highlight { 3.0 } else { 1.5 };
            let p = light.position;
            if light.kind != SceneLightKind::Directional {
                for axis in 0..3 {
                    let mut offset = [0.0; 3];
                    offset[axis] = 0.25;
                    list.line([p[0] - offset[0], p[1] - offset[1], p[2] - offset[2]], [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]], width, color);
                }
            }
            if light.kind != SceneLightKind::Point {
                let length = light.direction.iter().map(|c| c * c).sum::<f32>().sqrt().max(1.0e-6);
                let reach = if light.kind == SceneLightKind::Spot { light.range.min(3.0) } else { 1.5 };
                let end = [p[0] + light.direction[0] / length * reach, p[1] + light.direction[1] / length * reach, p[2] + light.direction[2] / length * reach];
                list.line(p, end, width, color);
            }
        }
    }
}

/// A material of one entity as the view reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRef {
    pub entity: Entity,
    /// Name of the surface shader.
    pub shader: String,
    values: [f32; orr_gfx::MATERIAL_VALUES],
    overrides: u32,
}

impl MaterialRef {
    /// The numbers the shader receives: the stored value where the material
    /// overrides it, the shader's default elsewhere, all inside the declared ranges.
    pub fn values_for(&self, shader: &orr_render::SurfaceShader) -> [f32; orr_render::MAX_SURFACE_FLOATS] {
        let mut values = shader.default_values();
        for (i, value) in values.iter_mut().enumerate().take(orr_gfx::MATERIAL_VALUES) {
            if self.overrides & (1 << i) != 0 {
                *value = self.values[i];
            }
        }
        shader.clamp(&values)
    }
}

/// Every entity's material, in entity order. Empty for a game without `orr_gfx` materials.
pub fn materials(frame: FrameView<'_>) -> Vec<MaterialRef> {
    if !frame.registers::<orr_gfx::Material>() {
        return Vec::new();
    }
    frame
        .iter::<orr_gfx::Material>()
        .map(|(entity, material)| MaterialRef {
            entity,
            shader: material.shader_name().to_string(),
            values: material.values.map(f),
            overrides: material.overrides,
        })
        .collect()
}
