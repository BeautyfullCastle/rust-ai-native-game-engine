//! `orr_gfx`: what a scene says about how it is drawn.
//!
//! These are ordinary scene components and singletons. They are saved in the
//! scene file, edited in the inspector and with the `orr` command, and undone
//! like every other component. A game opts in with [`register`],
//! [`register_reflect`] and [`init`].
//!
//! - [`Transform3`]: where an entity without a physics body is.
//! - [`Light`]: a directional, point or spot light on an entity.
//! - [`RenderLayers`]: the render layers an entity is drawn on. A light
//!   affects an entity only when the two share a layer.
//! - [`Material`]: a surface shader and its parameter values.
//! - [`Environment`]: sky and ground ambient light of the scene.
//! - [`PostProcess`]: exposure, tone mapping and bloom of the scene.
//!
//! The values are fixed point, as all scene state is. Simulation systems may
//! write them (a flickering lamp, a day cycle), so they are part of the
//! deterministic state. Nothing here draws: the view layer reads the
//! components each frame (`orr_gfx_view`) and converts them for the renderer.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec3, FP};
use orr_reflect::{Reflect, TypeRegistry};

/// [`Light::kind`]: parallel light from far away, such as the sun.
pub const LIGHT_DIRECTIONAL: u32 = 0;
/// [`Light::kind`]: light in all directions from the entity's position.
pub const LIGHT_POINT: u32 = 1;
/// [`Light::kind`]: a cone of light from the entity's position.
pub const LIGHT_SPOT: u32 = 2;

/// The render layer of an entity without a [`RenderLayers`] component.
pub const DEFAULT_LAYER: u32 = 1;
/// Every render layer.
pub const ALL_LAYERS: u32 = 255;

/// [`PostProcess::tone_mapping`]: bright colors are cut off at white.
pub const TONE_CLAMP: u32 = 0;
/// [`PostProcess::tone_mapping`]: a filmic curve keeps detail in bright areas.
pub const TONE_ACES: u32 = 1;

/// Placement of an entity that has no physics body: a light, a camera, a marker.
/// An entity with a physics body is placed by that body, and this component is ignored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Transform3 {
    /// World position.
    #[reflect(range = "-30000..=30000")]
    pub position: FPVec3,
    /// Pitch, yaw and roll in degrees. With all zero the entity faces +z.
    /// Positive pitch turns it downward; yaw turns it around the vertical axis.
    #[reflect(range = "-360..=360")]
    pub rotation: FPVec3,
    /// Scale along each local axis.
    #[reflect(range = "0.001..=1000")]
    pub scale: FPVec3,
}

impl Default for Transform3 {
    fn default() -> Self {
        Self { position: FPVec3::ZERO, rotation: FPVec3::ZERO, scale: FPVec3::new(FP::ONE, FP::ONE, FP::ONE) }
    }
}

/// A light on an entity. The entity's [`Transform3`] or physics body places it:
/// a point or spot light shines from the position, and a directional or spot
/// light shines along the direction the entity faces.
///
/// The first enabled directional light of a scene is its sun: the only light
/// that can cast shadows. A scene without any `Light` component keeps the
/// default lighting of its game.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Light {
    /// Linear red, green and blue.
    #[reflect(range = "0..=1")]
    pub color: FPVec3,
    /// Brightness. Zero gives no light.
    #[reflect(range = "0..=100")]
    pub intensity: FP,
    /// Distance in meters where a point or spot light reaches zero.
    #[reflect(range = "0.01..=1000")]
    pub range: FP,
    /// Half angle in degrees of the fully lit cone of a spot light.
    #[reflect(range = "0..=88")]
    pub inner_angle: FP,
    /// Half angle in degrees where a spot light reaches zero. At least the inner angle.
    #[reflect(range = "0.1..=89")]
    pub outer_angle: FP,
    /// How dark the shadows of this light are: 0 is no shadow, 1 is full shadow.
    #[reflect(range = "0..=1")]
    pub shadow_strength: FP,
    /// What the light is.
    #[reflect(enumeration = "directional=0,point=1,spot=2")]
    pub kind: u32,
    /// A disabled light gives no light and casts no shadow.
    #[reflect(bool)]
    pub enabled: u32,
    /// Whether the light casts shadows. Only the sun of a scene can.
    #[reflect(bool)]
    pub cast_shadows: u32,
    /// The render layers this light affects. A light with no layer lights nothing.
    #[reflect(flags = "default=1,layer2=2,layer3=4,layer4=8,layer5=16,layer6=32,layer7=64,layer8=128")]
    pub layers: u32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            color: FPVec3::new(FP::ONE, FP::ONE, FP::ONE),
            intensity: FP::ONE,
            range: FP::from_int(10),
            inner_angle: FP::from_int(20),
            outer_angle: FP::from_int(30),
            shadow_strength: FP::ONE,
            kind: LIGHT_POINT,
            enabled: 1,
            cast_shadows: 0,
            layers: ALL_LAYERS,
        }
    }
}

/// The render layers an entity is drawn on. An entity without this component
/// is on the default layer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct RenderLayers {
    /// The layers of the entity. A light affects it when the two share a layer.
    /// An entity with no layer is drawn on the default layer.
    #[reflect(flags = "default=1,layer2=2,layer3=4,layer4=8,layer5=16,layer6=32,layer7=64,layer8=128")]
    pub layers: u32,
    #[reflect(skip)]
    _pad: u32,
}

impl RenderLayers {
    /// An entity on the layers of `layers`.
    pub const fn new(layers: u32) -> Self {
        Self { layers, _pad: 0 }
    }
}

impl Default for RenderLayers {
    fn default() -> Self {
        Self::new(DEFAULT_LAYER)
    }
}

/// Most bytes of a surface shader name in [`Material::shader`].
pub const SHADER_NAME_BYTES: usize = 48;
/// Parameter values a [`Material`] holds.
pub const MATERIAL_VALUES: usize = 16;

/// A surface shader on an entity. The entity's shape is drawn with the shader
/// instead of the built-in material, and the engine still lights it.
///
/// The editor reads the shader from `shaders/<shader>.wgsl` next to the scene
/// file. The shader declares its parameters (`// @param tint color`); their
/// numbers are stored here in declaration order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct Material {
    /// Name of the surface shader: letters, digits, `_` and `-`.
    #[reflect(text)]
    pub shader: [u8; SHADER_NAME_BYTES],
    /// Parameter values in the order the shader declares them.
    pub values: [FP; MATERIAL_VALUES],
    /// Bit i set: `values[i]` is used. Bit i clear: the shader's default is used.
    #[reflect(range = "0..=65535")]
    pub overrides: u32,
    #[reflect(skip)]
    _pad: u32,
}

impl Material {
    /// A material with the shader `name` and the shader's default values.
    /// A name longer than [`SHADER_NAME_BYTES`] is cut.
    pub fn new(name: &str) -> Self {
        let mut shader = [0; SHADER_NAME_BYTES];
        // Cut at a character boundary so the stored text stays valid UTF-8.
        let mut len = name.len().min(SHADER_NAME_BYTES);
        while !name.is_char_boundary(len) {
            len -= 1;
        }
        shader[..len].copy_from_slice(&name.as_bytes()[..len]);
        Self { shader, values: [FP::ZERO; MATERIAL_VALUES], overrides: 0, _pad: 0 }
    }

    /// The shader name, without the zero padding.
    pub fn shader_name(&self) -> &str {
        let end = self.shader.iter().position(|b| *b == 0).unwrap_or(SHADER_NAME_BYTES);
        core::str::from_utf8(&self.shader[..end]).unwrap_or("")
    }
}

/// Ambient light of the scene: light from the sky above and from the ground below.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Environment {
    /// Linear color of the light from above.
    #[reflect(range = "0..=1")]
    pub sky_color: FPVec3,
    /// Linear color of the light from below.
    #[reflect(range = "0..=1")]
    pub ground_color: FPVec3,
    /// Strength of the ambient light. Zero leaves shaded surfaces black.
    #[reflect(range = "0..=10")]
    pub ambient: FP,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_color: FPVec3::new(FP::from_ratio(50, 100), FP::from_ratio(66, 100), FP::ONE),
            ground_color: FPVec3::new(FP::from_ratio(26, 100), FP::from_ratio(24, 100), FP::from_ratio(22, 100)),
            ambient: FP::from_ratio(30, 100),
        }
    }
}

/// How the finished picture is shown: exposure, tone mapping and bloom.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct PostProcess {
    /// Multiplies the lit picture before tone mapping.
    #[reflect(range = "0.01..=16")]
    pub exposure: FP,
    /// Scene brightness where bloom starts.
    #[reflect(range = "0..=64")]
    pub bloom_threshold: FP,
    /// How much of the bloom is added to the picture.
    #[reflect(range = "0..=8")]
    pub bloom_strength: FP,
    /// How bright colors are brought into the range of the display.
    #[reflect(enumeration = "clamp=0,aces=1")]
    pub tone_mapping: u32,
    /// Glow around very bright areas.
    #[reflect(bool)]
    pub bloom: u32,
    /// Half width of the bloom blur, in half-resolution pixels.
    #[reflect(range = "1..=8")]
    pub bloom_radius: u32,
    /// Number of blur passes. More passes give a wider, softer glow.
    #[reflect(range = "1..=4")]
    pub bloom_iterations: u32,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            exposure: FP::ONE,
            bloom_threshold: FP::ONE,
            bloom_strength: FP::from_ratio(15, 100),
            tone_mapping: TONE_ACES,
            bloom: 0,
            bloom_radius: 4,
            bloom_iterations: 2,
        }
    }
}

/// Scene names of the types, the same in the simulation and in scene files.
pub mod names {
    /// [`super::Transform3`].
    pub const TRANSFORM: &str = "orr_gfx::Transform3";
    /// [`super::Light`].
    pub const LIGHT: &str = "orr_gfx::Light";
    /// [`super::RenderLayers`].
    pub const RENDER_LAYERS: &str = "orr_gfx::RenderLayers";
    /// [`super::Material`].
    pub const MATERIAL: &str = "orr_gfx::Material";
    /// [`super::Environment`].
    pub const ENVIRONMENT: &str = "orr_gfx::Environment";
    /// [`super::PostProcess`].
    pub const POST_PROCESS: &str = "orr_gfx::PostProcess";
}

/// Registers the components and singletons with a game's simulation.
/// Call it from `Game::register`, and [`init`] from `Game::setup`.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Transform3>(names::TRANSFORM);
    builder.register_component::<Light>(names::LIGHT);
    builder.register_component::<RenderLayers>(names::RENDER_LAYERS);
    builder.register_component::<Material>(names::MATERIAL);
    builder.register_singleton::<Environment>(names::ENVIRONMENT);
    builder.register_singleton::<PostProcess>(names::POST_PROCESS);
}

/// Gives the singletons their default values in a new frame.
pub fn init(frame: &mut Frame) {
    frame.set_singleton(Environment::default());
    frame.set_singleton(PostProcess::default());
}

fn init_environment(frame: &mut Frame, present: bool) {
    if !present {
        frame.set_singleton(Environment::default());
    }
}

fn init_post_process(frame: &mut Frame, present: bool) {
    if !present {
        frame.set_singleton(PostProcess::default());
    }
}

/// Registers the types for scene files, the inspector and the `orr` command,
/// with the same names as [`register`]. A scene that does not mention a
/// singleton gets its default value.
pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_component::<Transform3>(names::TRANSFORM);
    types.register_component::<Light>(names::LIGHT);
    types.register_component::<RenderLayers>(names::RENDER_LAYERS);
    types.register_component::<Material>(names::MATERIAL);
    types.register_singleton_with_init::<Environment>(names::ENVIRONMENT, init_environment);
    types.register_singleton_with_init::<PostProcess>(names::POST_PROCESS, init_post_process);
}
