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

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{FPVec3, FP};
use orr_gfx::{Environment, Light, PostProcess, RenderLayers, Transform3, LIGHT_DIRECTIONAL, LIGHT_SPOT, TONE_CLAMP};
use orr_physics3d::Body;
use orr_render::{Lighting, RenderList3D, SceneLight, SceneLightKind, SceneLights};

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
    let mut look = SceneLook { lighting: *fallback, scene_lights: SceneLights::default(), bloom: None, lights: Vec::new(), dropped_lights: 0 };
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
        if post.bloom != 0 {
            look.bloom = Some(Bloom {
                threshold: f(post.bloom_threshold).clamp(0.0, 64.0),
                strength: f(post.bloom_strength).clamp(0.0, 8.0),
                radius: post.bloom_radius.clamp(1, 8),
                iterations: post.bloom_iterations.clamp(1, 4),
            });
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
