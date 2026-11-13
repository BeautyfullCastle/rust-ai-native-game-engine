//! The lights of a scene beyond the sun: directional, point and spot lights
//! with a layer mask each.
//!
//! A light affects a surface only when the two share a render layer. The sun
//! of [`crate::Lighting`] has a layer mask too ([`SceneLights::sun_layers`]).
//! Surfaces carry their layers in [`crate::Material::layers`].
//!
//! All lights use the bounded diffuse model of the single point light: the
//! light reaches zero at its range. This is a stylized model, not a
//! photometric one. Only the sun casts shadows.

use bytemuck::{Pod, Zeroable};

/// Lights that one frame can hold in addition to the sun.
pub const MAX_SCENE_LIGHTS: usize = 8;
/// Number of render layers. A layer mask uses the low bits.
pub const RENDER_LAYER_COUNT: u32 = 8;
/// The layer of a surface that names no layer.
pub const DEFAULT_RENDER_LAYER: u32 = 1;
/// Every render layer.
pub const ALL_RENDER_LAYERS: u32 = (1 << RENDER_LAYER_COUNT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneLightKind {
    /// Parallel light from far away. Position and range are not used.
    Directional,
    /// Light in all directions from a position.
    Point,
    /// A cone of light from a position.
    Spot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLight {
    pub kind: SceneLightKind,
    pub position: [f32; 3],
    /// The direction the light travels. It need not be a unit vector.
    pub direction: [f32; 3],
    /// Linear color.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Distance where a point or spot light reaches zero.
    pub range: f32,
    /// Half angle of the fully lit cone of a spot light, in radians.
    pub inner_angle: f32,
    /// Half angle where a spot light reaches zero, in radians.
    pub outer_angle: f32,
    /// Render layers this light affects.
    pub layers: u32,
}

impl Default for SceneLight {
    fn default() -> Self {
        Self {
            kind: SceneLightKind::Point,
            position: [0.0; 3],
            direction: [0.0, -1.0, 0.0],
            color: [1.0; 3],
            intensity: 1.0,
            range: 10.0,
            inner_angle: 0.35,
            outer_angle: 0.5,
            layers: ALL_RENDER_LAYERS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneLightError {
    /// The frame already holds [`MAX_SCENE_LIGHTS`] lights.
    Full,
    /// A value is not finite or is outside its bounds.
    Invalid,
}

impl std::fmt::Display for SceneLightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "a frame holds at most {MAX_SCENE_LIGHTS} scene lights"),
            Self::Invalid => f.write_str("scene light values must be finite and inside their bounds"),
        }
    }
}

impl std::error::Error for SceneLightError {}

impl SceneLight {
    /// The bounds match the single point light: positions within 1e9,
    /// color and intensity from 0 to 1e4, range from 1e-6 to 1e9.
    pub fn validate(&self) -> Result<(), SceneLightError> {
        let finite = self.position.iter().chain(&self.direction).chain(&self.color).all(|v| v.is_finite())
            && self.intensity.is_finite()
            && self.range.is_finite()
            && self.inner_angle.is_finite()
            && self.outer_angle.is_finite();
        let direction = self.direction.iter().map(|v| v * v).sum::<f32>();
        let bounded = self.position.iter().all(|v| v.abs() <= 1.0e9)
            && self.color.iter().all(|v| (0.0..=1.0e4).contains(v))
            && (0.0..=1.0e4).contains(&self.intensity)
            && (1.0e-6..=1.0e9).contains(&self.range)
            && (0.0..=1.55).contains(&self.inner_angle)
            && (self.inner_angle..=1.56).contains(&self.outer_angle)
            && (self.kind == SceneLightKind::Point || (1.0e-12..=1.0e12).contains(&direction));
        if finite && bounded {
            Ok(())
        } else {
            Err(SceneLightError::Invalid)
        }
    }
}

/// The scene lights of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLights {
    lights: [SceneLight; MAX_SCENE_LIGHTS],
    count: usize,
    /// Render layers the sun of [`crate::Lighting`] affects.
    pub sun_layers: u32,
    /// How dark the sun's shadows are: 0 is no shadow, 1 is full shadow.
    pub sun_shadow_strength: f32,
}

impl Default for SceneLights {
    fn default() -> Self {
        Self { lights: [SceneLight::default(); MAX_SCENE_LIGHTS], count: 0, sun_layers: ALL_RENDER_LAYERS, sun_shadow_strength: 1.0 }
    }
}

impl SceneLights {
    /// Adds a light. An invalid light or a full frame changes nothing.
    pub fn push(&mut self, light: SceneLight) -> Result<(), SceneLightError> {
        light.validate()?;
        if self.count == MAX_SCENE_LIGHTS {
            return Err(SceneLightError::Full);
        }
        self.lights[self.count] = light;
        self.count += 1;
        Ok(())
    }

    pub fn lights(&self) -> &[SceneLight] {
        &self.lights[..self.count]
    }

    pub fn clear(&mut self) {
        self.count = 0;
    }

    pub(crate) fn uniform(&self) -> SceneLightsUniform {
        let strength = if self.sun_shadow_strength.is_finite() { self.sun_shadow_strength.clamp(0.0, 1.0) } else { 1.0 };
        let mut uniform = SceneLightsUniform {
            info: [self.count as f32, (self.sun_layers & ALL_RENDER_LAYERS) as f32, strength, 0.0],
            lights: [[[0.0; 4]; 4]; MAX_SCENE_LIGHTS],
        };
        for (slot, light) in uniform.lights.iter_mut().zip(self.lights()) {
            let length = light.direction.iter().map(|v| v * v).sum::<f32>().sqrt().max(1.0e-6);
            let kind = match light.kind {
                SceneLightKind::Directional => 0.0,
                SceneLightKind::Point => 1.0,
                SceneLightKind::Spot => 2.0,
            };
            // The outer cosine stays below the inner one so the cone edge has a width.
            let inner = light.inner_angle.cos();
            let outer = light.outer_angle.cos().min(inner - 1.0e-4);
            *slot = [
                [light.position[0], light.position[1], light.position[2], light.range],
                [light.color[0], light.color[1], light.color[2], light.intensity],
                [light.direction[0] / length, light.direction[1] / length, light.direction[2] / length, kind],
                [inner, outer, (light.layers & ALL_RENDER_LAYERS) as f32, 0.0],
            ];
        }
        uniform
    }
}

/// [`SceneLights`] as the shaders read it: 528 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct SceneLightsUniform {
    /// x: number of lights, y: layers the sun affects, z: strength of the sun's shadows.
    info: [f32; 4],
    lights: [[[f32; 4]; 4]; MAX_SCENE_LIGHTS],
}

impl SceneLightsUniform {
    /// No scene lights, and a sun that affects every layer.
    pub(crate) fn none() -> Self {
        SceneLights::default().uniform()
    }
}

/// Appends the scene light functions to a shader whose globals are named `global`.
/// The shader declares `scene_light_info` and `scene_lights` in its globals itself.
pub(crate) fn shader(source: &str, global: &str) -> String {
    format!("{source}\n{}", include_str!("scene_lights.wgsl").replace("SCENE_GLOBAL", global))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_takes_eight_valid_lights_and_refuses_the_rest() {
        let mut lights = SceneLights::default();
        assert_eq!(lights.uniform().info, [0.0, 255.0, 1.0, 0.0]);
        for index in 0..MAX_SCENE_LIGHTS {
            lights.push(SceneLight { intensity: index as f32, ..SceneLight::default() }).unwrap();
        }
        assert_eq!(lights.push(SceneLight::default()), Err(SceneLightError::Full));
        assert_eq!(lights.lights().len(), MAX_SCENE_LIGHTS);
        assert_eq!(lights.uniform().info[0], 8.0);
        lights.clear();
        assert!(lights.lights().is_empty());
    }

    #[test]
    fn invalid_lights_are_refused_and_change_nothing() {
        let mut lights = SceneLights::default();
        let bad = [
            SceneLight { intensity: f32::NAN, ..SceneLight::default() },
            SceneLight { color: [-0.1, 0.0, 0.0], ..SceneLight::default() },
            SceneLight { range: 0.0, ..SceneLight::default() },
            SceneLight { position: [2.0e9, 0.0, 0.0], ..SceneLight::default() },
            SceneLight { inner_angle: 0.6, outer_angle: 0.5, ..SceneLight::default() },
            SceneLight { kind: SceneLightKind::Spot, direction: [0.0; 3], ..SceneLight::default() },
            SceneLight { kind: SceneLightKind::Directional, direction: [f32::INFINITY, 0.0, 0.0], ..SceneLight::default() },
        ];
        for light in bad {
            assert_eq!(lights.push(light), Err(SceneLightError::Invalid), "{light:?}");
        }
        assert!(lights.lights().is_empty());
        // A point light ignores its direction.
        lights.push(SceneLight { kind: SceneLightKind::Point, direction: [0.0; 3], ..SceneLight::default() }).unwrap();
    }

    #[test]
    fn the_uniform_holds_unit_directions_cone_cosines_and_layer_masks() {
        let mut lights = SceneLights { sun_layers: 0b101, sun_shadow_strength: 0.25, ..SceneLights::default() };
        lights
            .push(SceneLight {
                kind: SceneLightKind::Spot,
                position: [1.0, 2.0, 3.0],
                direction: [0.0, -4.0, 0.0],
                color: [0.5, 0.25, 1.0],
                intensity: 3.0,
                range: 12.0,
                inner_angle: 0.3,
                outer_angle: 0.3,
                layers: 0xFFFF_FF06,
            })
            .unwrap();
        let uniform = lights.uniform();
        assert_eq!(uniform.info, [1.0, 5.0, 0.25, 0.0]);
        assert_eq!(uniform.lights[0][0], [1.0, 2.0, 3.0, 12.0]);
        assert_eq!(uniform.lights[0][1], [0.5, 0.25, 1.0, 3.0]);
        assert_eq!(uniform.lights[0][2], [0.0, -1.0, 0.0, 2.0]);
        let [inner, outer, layers, _] = uniform.lights[0][3];
        assert!((inner - 0.3_f32.cos()).abs() < 1.0e-6 && outer < inner);
        assert_eq!(layers, 6.0);
        assert_eq!(std::mem::size_of::<SceneLightsUniform>(), 16 + MAX_SCENE_LIGHTS * 64);
    }
}
