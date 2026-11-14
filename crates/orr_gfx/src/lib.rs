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
//! - [`PostProcess`]: exposure, tone mapping and the ordered list of post effects.
//! - [`Camera`]: a camera on an entity; the active one with the highest priority is the main camera.
//! - [`Terrain`]: ground from a heightmap image, given its look by a [`Material`].
//! - [`Sky`]: the drawn sky, fog and wind of an outdoor scene.
//! - [`Water`]: a water surface; [`Scatter`]: trees, rocks or grass spread over a terrain.
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
use orr_fp::{FPVec2, FPVec3, FP};
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
    /// Radius in meters of the area with sun shadows. The area follows the
    /// view. Zero keeps the game's own shadow area. Only the sun uses it.
    #[reflect(range = "0..=5000")]
    pub shadow_range: FP,
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
    /// Cascades of the sun's shadows while its area follows the view
    /// (`shadow_range` above zero): 1 to 4. Each covers a farther stretch of
    /// the view, so near shadows are sharp and far ones still show.
    #[reflect(range = "1..=4")]
    pub shadow_cascades: u32,
    /// Always zero. It keeps the layout free of padding.
    #[reflect(skip)]
    pub reserved: u32,
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
            shadow_range: FP::ZERO,
            kind: LIGHT_POINT,
            enabled: 1,
            cast_shadows: 0,
            layers: ALL_LAYERS,
            shadow_cascades: 4,
            reserved: 0,
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
/// Texture slots a [`Material`] holds, one per texture the shader declares.
pub const MATERIAL_TEXTURES: usize = 4;
/// Most bytes of one texture path in a [`Material`].
pub const MATERIAL_TEXTURE_PATH_BYTES: usize = 128;

/// A surface shader on an entity. The entity's shape is drawn with the shader
/// instead of the built-in material, and the engine still lights it.
///
/// The editor reads the shader from `shaders/<shader>.wgsl` next to the scene
/// file. The shader declares its parameters (`// @param tint color`); their
/// numbers are stored here in declaration order. It also declares textures
/// (`// @texture grass color`); `texture_0` to `texture_3` hold the PNG path
/// of each, from the scene's folder, in declaration order. An empty path
/// reads the shader's default for that texture.
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
    /// The image of the shader's first declared texture.
    #[reflect(asset = "texture")]
    pub texture_0: [u8; MATERIAL_TEXTURE_PATH_BYTES],
    /// The image of the shader's second declared texture.
    #[reflect(asset = "texture")]
    pub texture_1: [u8; MATERIAL_TEXTURE_PATH_BYTES],
    /// The image of the shader's third declared texture.
    #[reflect(asset = "texture")]
    pub texture_2: [u8; MATERIAL_TEXTURE_PATH_BYTES],
    /// The image of the shader's fourth declared texture.
    #[reflect(asset = "texture")]
    pub texture_3: [u8; MATERIAL_TEXTURE_PATH_BYTES],
}

/// `text` as zero-padded bytes, cut at a character boundary.
fn padded<const N: usize>(text: &str) -> [u8; N] {
    let mut bytes = [0; N];
    let mut len = text.len().min(N);
    while !text.is_char_boundary(len) {
        len -= 1;
    }
    bytes[..len].copy_from_slice(&text.as_bytes()[..len]);
    bytes
}

/// The text of zero-padded `bytes`; empty when it is not UTF-8.
fn unpadded(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).unwrap_or("")
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
        Self {
            shader,
            values: [FP::ZERO; MATERIAL_VALUES],
            overrides: 0,
            _pad: 0,
            texture_0: [0; MATERIAL_TEXTURE_PATH_BYTES],
            texture_1: [0; MATERIAL_TEXTURE_PATH_BYTES],
            texture_2: [0; MATERIAL_TEXTURE_PATH_BYTES],
            texture_3: [0; MATERIAL_TEXTURE_PATH_BYTES],
        }
    }

    fn texture_slot(&self, slot: usize) -> Option<&[u8; MATERIAL_TEXTURE_PATH_BYTES]> {
        [&self.texture_0, &self.texture_1, &self.texture_2, &self.texture_3].get(slot).copied()
    }

    /// The image path of texture `slot`, empty when none is set or the slot does not exist.
    pub fn texture_path(&self, slot: usize) -> &str {
        self.texture_slot(slot).map_or("", |bytes| unpadded(bytes))
    }

    /// Sets the image path of texture `slot`; a slot past [`MATERIAL_TEXTURES`]
    /// is ignored. A path longer than [`MATERIAL_TEXTURE_PATH_BYTES`] is cut.
    pub fn set_texture(&mut self, slot: usize, path: &str) {
        let bytes = padded(path);
        match slot {
            0 => self.texture_0 = bytes,
            1 => self.texture_1 = bytes,
            2 => self.texture_2 = bytes,
            3 => self.texture_3 = bytes,
            _ => {}
        }
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

/// [`PostEffect::kind`]: an unused slot.
pub const EFFECT_NONE: u32 = 0;
/// [`PostEffect::kind`]: glow around bright areas. Params: threshold, strength, radius, passes.
pub const EFFECT_BLOOM: u32 = 1;
/// [`PostEffect::kind`]: rays of sunlight through gaps toward the sun. Params: strength, length.
pub const EFFECT_LIGHT_SHAFTS: u32 = 2;
/// [`PostEffect::kind`]: exposure follows the brightness of the picture, as an eye adapts.
/// Params: darkest and brightest exposure, adaptation speed per second, target brightness.
pub const EFFECT_AUTO_EXPOSURE: u32 = 3;
/// [`PostEffect::kind`]: saturation, contrast, warmth and green-magenta tint.
pub const EFFECT_COLOR_GRADE: u32 = 4;
/// [`PostEffect::kind`]: darker picture corners. Params: strength, radius, softness.
pub const EFFECT_VIGNETTE: u32 = 5;
/// [`PostEffect::kind`]: fine moving noise over the picture. Params: strength.
pub const EFFECT_FILM_GRAIN: u32 = 6;
/// [`PostEffect::kind`]: creases and contact areas darken (screen-space ambient occlusion).
/// Params: strength, radius in meters.
pub const EFFECT_SSAO: u32 = 7;
/// [`PostEffect::kind`]: cel shading, brightness in flat steps. Params: bands, softness.
pub const EFFECT_TOON: u32 = 8;
/// [`PostEffect::kind`]: dark lines where depth jumps. Params: thickness in pixels, sensitivity, darkness.
pub const EFFECT_OUTLINE: u32 = 9;
/// [`PostEffect::kind`]: fast approximate anti-aliasing. Params: strength.
pub const EFFECT_FXAA: u32 = 10;
/// [`PostEffect::kind`]: draws the scene smaller and upscales it with edge-adaptive
/// filtering and sharpening, after AMD FSR 1. Params: scale, sharpness.
pub const EFFECT_UPSCALE: u32 = 11;
/// [`PostEffect::kind`]: temporal anti-aliasing; with `upscale`, upscaling over frames. Params: blend.
pub const EFFECT_TAA: u32 = 12;
/// Slots in [`PostProcess::effects`].
pub const POST_EFFECTS: usize = 8;

/// One step of the post chain. What the four numbers mean depends on `kind`
/// (see the `EFFECT_*` constants and [`effect_params`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct PostEffect {
    /// Which effect this slot holds.
    #[reflect(enumeration = "none=0,bloom=1,light_shafts=2,auto_exposure=3,color_grade=4,vignette=5,film_grain=6,ssao=7,toon=8,outline=9,fxaa=10,upscale=11,taa=12")]
    pub kind: u32,
    /// Whether the effect runs.
    #[reflect(bool)]
    pub enabled: u32,
    /// The numbers of the effect.
    #[reflect(range = "-100..=100")]
    pub params: [FP; 4],
}

impl Default for PostEffect {
    fn default() -> Self {
        Self { kind: EFFECT_NONE, enabled: 0, params: [FP::ZERO; 4] }
    }
}

/// Name, smallest value, largest value and default of one effect number.
pub type EffectParam = (&'static str, FP, FP, FP);

/// The numbers an effect of `kind` reads, in `params` order. Unused numbers are not listed.
pub fn effect_params(kind: u32) -> &'static [EffectParam] {
    const fn r(n: i64, d: i64) -> FP {
        FP::from_ratio(n, d)
    }
    const BLOOM_PARAMS: &[EffectParam] = &[("threshold", r(0, 1), r(64, 1), r(1, 1)), ("strength", r(0, 1), r(8, 1), r(15, 100)), ("radius", r(1, 1), r(8, 1), r(4, 1)), ("passes", r(1, 1), r(4, 1), r(2, 1))];
    const LIGHT_SHAFTS_PARAMS: &[EffectParam] = &[("strength", r(0, 1), r(4, 1), r(6, 10)), ("length", r(1, 10), r(1, 1), r(8, 10))];
    const AUTO_EXPOSURE_PARAMS: &[EffectParam] = &[("darkest", r(1, 10), r(1, 1), r(55, 100)), ("brightest", r(1, 1), r(16, 1), r(45, 10)), ("speed", r(1, 10), r(20, 1), r(15, 10)), ("target", r(1, 100), r(1, 1), r(165, 1000))];
    const COLOR_GRADE_PARAMS: &[EffectParam] = &[("saturation", r(0, 1), r(2, 1), r(1, 1)), ("contrast", r(0, 1), r(1, 1), r(0, 1)), ("warmth", r(-1, 1), r(1, 1), r(0, 1)), ("tint", r(-1, 1), r(1, 1), r(0, 1))];
    const VIGNETTE_PARAMS: &[EffectParam] = &[("strength", r(0, 1), r(1, 1), r(35, 100)), ("radius", r(1, 10), r(15, 10), r(8, 10)), ("softness", r(1, 100), r(1, 1), r(5, 10))];
    const FILM_GRAIN_PARAMS: &[EffectParam] = &[("strength", r(0, 1), r(1, 1), r(15, 100))];
    const SSAO_PARAMS: &[EffectParam] = &[("strength", r(0, 1), r(2, 1), r(1, 1)), ("radius", r(5, 100), r(10, 1), r(1, 1))];
    const TOON_PARAMS: &[EffectParam] = &[("bands", r(2, 1), r(8, 1), r(3, 1)), ("softness", r(0, 1), r(1, 1), r(1, 10))];
    const OUTLINE_PARAMS: &[EffectParam] = &[("thickness", r(1, 2), r(4, 1), r(1, 1)), ("sensitivity", r(1, 100), r(1, 1), r(5, 100)), ("darkness", r(0, 1), r(1, 1), r(85, 100))];
    const FXAA_PARAMS: &[EffectParam] = &[("strength", r(0, 1), r(1, 1), r(1, 1))];
    const UPSCALE_PARAMS: &[EffectParam] = &[("scale", r(1, 2), r(1, 1), r(75, 100)), ("sharpness", r(0, 1), r(1, 1), r(5, 10))];
    const TAA_PARAMS: &[EffectParam] = &[("blend", r(5, 100), r(1, 2), r(1, 10))];
    match kind {
        EFFECT_BLOOM => BLOOM_PARAMS,
        EFFECT_LIGHT_SHAFTS => LIGHT_SHAFTS_PARAMS,
        EFFECT_AUTO_EXPOSURE => AUTO_EXPOSURE_PARAMS,
        EFFECT_COLOR_GRADE => COLOR_GRADE_PARAMS,
        EFFECT_VIGNETTE => VIGNETTE_PARAMS,
        EFFECT_FILM_GRAIN => FILM_GRAIN_PARAMS,
        EFFECT_SSAO => SSAO_PARAMS,
        EFFECT_TOON => TOON_PARAMS,
        EFFECT_OUTLINE => OUTLINE_PARAMS,
        EFFECT_FXAA => FXAA_PARAMS,
        EFFECT_UPSCALE => UPSCALE_PARAMS,
        EFFECT_TAA => TAA_PARAMS,
        _ => &[],
    }
}

impl PostEffect {
    /// An enabled effect of `kind` with its default numbers.
    pub fn new(kind: u32) -> Self {
        let mut params = [FP::ZERO; 4];
        for (slot, (_, _, _, default)) in params.iter_mut().zip(effect_params(kind)) {
            *slot = *default;
        }
        Self { kind, enabled: 1, params }
    }
}

/// How the finished picture is shown: a list of effects, run in list order,
/// then exposure and tone mapping once at the end.
///
/// Effects on the lit scene (bloom, light shafts) run first, in their list
/// order; auto exposure measures the result. Exposure and tone mapping follow
/// exactly once, then the effects on the finished picture (color grade,
/// vignette, film grain), again in list order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct PostProcess {
    /// Multiplies the lit picture before tone mapping.
    #[reflect(range = "0.01..=16")]
    pub exposure: FP,
    /// How bright colors are brought into the range of the display.
    #[reflect(enumeration = "clamp=0,aces=1")]
    pub tone_mapping: u32,
    #[reflect(skip)]
    _pad: u32,
    /// The effects, in the order they run. Slots of kind `none` are skipped.
    pub effects: [PostEffect; POST_EFFECTS],
}

impl Default for PostProcess {
    fn default() -> Self {
        let mut effects = [PostEffect::default(); POST_EFFECTS];
        effects[0] = PostEffect { enabled: 0, ..PostEffect::new(EFFECT_BLOOM) };
        Self { exposure: FP::ONE, tone_mapping: TONE_ACES, _pad: 0, effects }
    }
}

/// [`Camera::projection`]: farther things look smaller.
pub const PROJECTION_PERSPECTIVE: u32 = 0;
/// [`Camera::projection`]: sizes do not change with distance.
pub const PROJECTION_ORTHOGRAPHIC: u32 = 1;

/// A camera on an entity. The entity's [`Transform3`] places and turns it; it
/// looks the way the entity faces. The camera with the highest `priority`
/// among the active ones is the scene's main camera: the editor's camera view
/// and a game's player view look through it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Camera {
    /// Vertical field of view in degrees, for a perspective camera.
    #[reflect(range = "1..=179")]
    pub fov_y: FP,
    /// Half the height of the view in meters, for an orthographic camera.
    #[reflect(range = "0.01..=10000")]
    pub half_height: FP,
    /// Nearest distance drawn, in meters.
    #[reflect(range = "0.01..=1000")]
    pub near: FP,
    /// Farthest distance drawn, in meters.
    #[reflect(range = "1..=100000")]
    pub far: FP,
    /// How the camera projects the scene.
    #[reflect(enumeration = "perspective=0,orthographic=1")]
    pub projection: u32,
    /// The render layers this camera draws. Entities on no shared layer are not drawn.
    #[reflect(flags = "default=1,layer2=2,layer3=4,layer4=8,layer5=16,layer6=32,layer7=64,layer8=128")]
    pub layers: u32,
    /// Among active cameras, the highest priority is the main camera.
    #[reflect(range = "-1000..=1000")]
    pub priority: i32,
    /// Whether the camera can be the main camera.
    #[reflect(bool)]
    pub active: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_y: FP::from_int(55),
            half_height: FP::from_int(10),
            near: FP::from_ratio(1, 10),
            far: FP::from_int(2000),
            projection: PROJECTION_PERSPECTIVE,
            layers: ALL_LAYERS,
            priority: 0,
            active: 1,
        }
    }
}

/// The sky, fog and wind of an outdoor scene. With `enabled` the renderer
/// draws a sky with sun disc and clouds behind everything, the scene gets
/// height fog, and vegetation moves in the wind. The sun is the scene's sun
/// [`Light`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Sky {
    /// Brightness of the sky.
    #[reflect(range = "0..=8")]
    pub brightness: FP,
    /// Size of the bright glow around the sun.
    #[reflect(range = "0..=4")]
    pub sun_glow: FP,
    /// Share of the sky covered by clouds.
    #[reflect(range = "0..=1")]
    pub cloud_cover: FP,
    /// How fast the clouds drift.
    #[reflect(range = "0..=10")]
    pub cloud_speed: FP,
    /// Thickness of the fog at `fog_height`.
    #[reflect(range = "0..=20")]
    pub fog_density: FP,
    /// How fast the fog thins with height, per meter.
    #[reflect(range = "0..=1")]
    pub fog_falloff: FP,
    /// Height in meters where the fog has its full thickness.
    #[reflect(range = "-1000..=5000")]
    pub fog_height: FP,
    /// Share of the fog that takes the sky color, far away.
    #[reflect(range = "0..=1")]
    pub haze: FP,
    /// How far vegetation moves in the wind.
    #[reflect(range = "0..=4")]
    pub wind_strength: FP,
    /// How fast the wind changes.
    #[reflect(range = "0..=4")]
    pub wind_speed: FP,
    /// Whether the sky, fog and wind are drawn.
    #[reflect(bool)]
    pub enabled: u32,
    #[reflect(skip)]
    _pad: u32,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            brightness: FP::from_ratio(3, 2),
            sun_glow: FP::ONE,
            cloud_cover: FP::from_ratio(34, 100),
            cloud_speed: FP::ONE,
            fog_density: FP::ONE,
            fog_falloff: FP::from_ratio(3, 100),
            fog_height: FP::ZERO,
            haze: FP::from_ratio(8, 100),
            wind_strength: FP::ONE,
            wind_speed: FP::ONE,
            enabled: 0,
            _pad: 0,
        }
    }
}

/// A water surface on an entity: a level rectangle centered on the entity's
/// [`Transform3`] position. Over a [`Terrain`] the water is clear where it is
/// shallow and takes its color where it is deep; it reflects the sky.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Water {
    /// Size of the surface in meters along x and z.
    #[reflect(range = "1..=20000")]
    pub size: FPVec2,
    /// Linear color of deep water.
    #[reflect(range = "0..=1")]
    pub color: FPVec3,
    /// How deep light reaches; higher is clearer.
    #[reflect(range = "0.05..=10")]
    pub clarity: FP,
    /// Height of the ripples.
    #[reflect(range = "0..=4")]
    pub ripples: FP,
    /// Direction and speed of the current, meters per second along x and z.
    #[reflect(range = "-10..=10")]
    pub flow: FPVec2,
    /// Strength of the sun's glints.
    #[reflect(range = "0..=4")]
    pub glint: FP,
    /// The terrain under the water, for its depth. None gives deep water everywhere.
    pub terrain: orr_ecs::Entity,
}

impl Default for Water {
    fn default() -> Self {
        Self {
            size: FPVec2::new(FP::from_int(100), FP::from_int(100)),
            color: FPVec3::new(FP::from_ratio(12, 1000), FP::from_ratio(5, 100), FP::from_ratio(4, 100)),
            clarity: FP::ONE,
            ripples: FP::ONE,
            flow: FPVec2::new(FP::ZERO, FP::from_ratio(3, 10)),
            glint: FP::ONE,
            terrain: orr_ecs::Entity::NONE,
        }
    }
}

/// [`Scatter::kind`]: needle trees.
pub const SCATTER_CONIFER: u32 = 0;
/// [`Scatter::kind`]: leaf trees; some turn early in autumn.
pub const SCATTER_BROADLEAF: u32 = 1;
/// [`Scatter::kind`]: boulders with moss on top.
pub const SCATTER_ROCK: u32 = 2;
/// [`Scatter::kind`]: grass tufts, flowers and ferns.
pub const SCATTER_GRASS: u32 = 3;
/// Most objects one [`Scatter`] places.
pub const MAX_SCATTER: u32 = 20000;

/// Objects spread over a [`Terrain`]: trees, rocks or grass in a rectangle
/// centered on the entity's [`Transform3`] position. The same seed always
/// gives the same places. Places outside the height range, steeper than
/// `max_slope` or under water are left empty.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Scatter {
    /// The terrain to stand on.
    pub terrain: orr_ecs::Entity,
    /// Size of the area in meters along x and z.
    #[reflect(range = "1..=20000")]
    pub area: FPVec2,
    /// Lowest and highest ground, in meters of world height.
    #[reflect(range = "-10000..=10000")]
    pub heights: FPVec2,
    /// Steepest ground in degrees.
    #[reflect(range = "0..=90")]
    pub max_slope: FP,
    /// Smallest and largest size, as a factor of the natural size.
    #[reflect(range = "0.05..=20")]
    pub scale: FPVec2,
    /// What is placed.
    #[reflect(enumeration = "conifer=0,broadleaf=1,rock=2,grass=3")]
    pub kind: u32,
    /// How many places are tried.
    #[reflect(range = "0..=20000")]
    pub count: u32,
    /// Changes the places.
    pub seed: u32,
    /// Whether trees and rocks cast sun shadows.
    #[reflect(bool)]
    pub cast_shadows: u32,
}

impl Default for Scatter {
    fn default() -> Self {
        Self {
            terrain: orr_ecs::Entity::NONE,
            area: FPVec2::new(FP::from_int(100), FP::from_int(100)),
            heights: FPVec2::new(FP::from_int(-10000), FP::from_int(10000)),
            max_slope: FP::from_int(35),
            scale: FPVec2::new(FP::from_ratio(8, 10), FP::from_ratio(13, 10)),
            kind: SCATTER_CONIFER,
            count: 200,
            seed: 1,
            cast_shadows: 1,
        }
    }
}

/// Most bytes of the heightmap path in [`Terrain::heightmap`].
pub const TERRAIN_PATH_BYTES: usize = 128;

/// Ground built from a heightmap image on an entity. The entity's
/// [`Transform3`] places the center of the terrain; its rotation and scale are
/// not used. A [`Material`] on the same entity gives the terrain its look; the
/// engine's `terrain` surface shader blends grass, rock and snow by slope and
/// height. The terrain is drawn only; collision stays with the game's own
/// terrain data.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Terrain {
    /// Grayscale PNG next to the scene file. Black is the lowest point, white the highest.
    #[reflect(asset = "heightmap")]
    pub heightmap: [u8; TERRAIN_PATH_BYTES],
    /// Size of the terrain in meters along x and z.
    #[reflect(range = "1..=20000")]
    pub size: FPVec2,
    /// Height in meters of a white pixel above a black one.
    #[reflect(range = "0..=5000")]
    pub height: FP,
    /// Most grid points along each side; a larger image is sampled down.
    #[reflect(range = "8..=352")]
    pub resolution: u32,
    /// Whether the terrain casts sun shadows.
    #[reflect(bool)]
    pub cast_shadows: u32,
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            heightmap: [0; TERRAIN_PATH_BYTES],
            size: FPVec2::new(FP::from_int(200), FP::from_int(200)),
            height: FP::from_int(30),
            resolution: 256,
            cast_shadows: 1,
        }
    }
}

impl Terrain {
    /// The heightmap path, empty when none is set.
    pub fn heightmap_path(&self) -> &str {
        let end = self.heightmap.iter().position(|b| *b == 0).unwrap_or(TERRAIN_PATH_BYTES);
        core::str::from_utf8(&self.heightmap[..end]).unwrap_or("")
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
    /// [`super::Camera`].
    pub const CAMERA: &str = "orr_gfx::Camera";
    /// [`super::Terrain`].
    pub const TERRAIN: &str = "orr_gfx::Terrain";
    /// [`super::Sky`].
    pub const SKY: &str = "orr_gfx::Sky";
    /// [`super::Water`].
    pub const WATER: &str = "orr_gfx::Water";
    /// [`super::Scatter`].
    pub const SCATTER: &str = "orr_gfx::Scatter";
}

/// Registers the components and singletons with a game's simulation.
/// Call it from `Game::register`, and [`init`] from `Game::setup`.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Transform3>(names::TRANSFORM);
    builder.register_component::<Light>(names::LIGHT);
    builder.register_component::<RenderLayers>(names::RENDER_LAYERS);
    builder.register_component::<Material>(names::MATERIAL);
    builder.register_component::<Camera>(names::CAMERA);
    builder.register_component::<Terrain>(names::TERRAIN);
    builder.register_component::<Water>(names::WATER);
    builder.register_component::<Scatter>(names::SCATTER);
    builder.register_singleton::<Environment>(names::ENVIRONMENT);
    builder.register_singleton::<Sky>(names::SKY);
    builder.register_singleton::<PostProcess>(names::POST_PROCESS);
}

/// Gives the singletons their default values in a new frame.
pub fn init(frame: &mut Frame) {
    frame.set_singleton(Environment::default());
    frame.set_singleton(Sky::default());
    frame.set_singleton(PostProcess::default());
}

fn init_environment(frame: &mut Frame, present: bool) {
    if !present {
        frame.set_singleton(Environment::default());
    }
}

fn init_sky(frame: &mut Frame, present: bool) {
    if !present {
        frame.set_singleton(Sky::default());
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
    types.register_component::<Camera>(names::CAMERA);
    types.register_component::<Terrain>(names::TERRAIN);
    types.register_component::<Water>(names::WATER);
    types.register_component::<Scatter>(names::SCATTER);
    types.register_singleton_with_init::<Environment>(names::ENVIRONMENT, init_environment);
    types.register_singleton_with_init::<Sky>(names::SKY, init_sky);
    types.register_singleton_with_init::<PostProcess>(names::POST_PROCESS, init_post_process);
}
