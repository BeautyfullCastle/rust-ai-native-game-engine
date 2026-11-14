//! Post effects beyond bloom: light shafts, auto exposure and the effects on
//! the finished picture (color grade, vignette, film grain). They run inside
//! the HDR post chain of [`crate::PostProcessor`]; exposure and tone mapping
//! still happen once, in the final pass.
use crate::Lighting;
use orr_rhi::{Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUsage};
use std::cell::{Cell, Ref, RefCell};
use std::time::Instant;

/// Slots in [`crate::PostProcessSettings::effects`]: the eight of a scene's
/// effect list and the sky's height fog.
pub const MAX_SCREEN_EFFECTS: usize = 9;

/// One effect of the post chain. Effects on the lit scene (light shafts) and
/// auto exposure run before exposure and tone mapping; the others run after
/// them, in list order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenEffect {
    /// Rays of sunlight through gaps toward the sun. `strength` 0..=4, `length` 0.1..=1.
    LightShafts { strength: f32, length: f32 },
    /// Exposure follows the metered brightness of the picture. `darkest` and
    /// `brightest` bound the factor (0.1..=16), `speed` is per second (0.1..=20),
    /// `target` is the brightness the picture is exposed for (0.01..=1).
    AutoExposure { darkest: f32, brightest: f32, speed: f32, target: f32 },
    /// `saturation` 0..=2 (1 keeps it), `contrast` 0..=1, `warmth` and `tint` -1..=1.
    ColorGrade { saturation: f32, contrast: f32, warmth: f32, tint: f32 },
    /// Darker corners. `strength` 0..=1, `radius` 0.1..=1.5, `softness` 0.01..=1.
    Vignette { strength: f32, radius: f32, softness: f32 },
    /// Moving noise. `strength` 0..=1.
    FilmGrain { strength: f32 },
    /// Fog that thickens toward `height` (meters), tinted by the sky. Applied
    /// to the lit scene by its depth, before everything else. `density`
    /// 0..=0.05 per meter, `falloff` 0..=1 per meter, `haze` 0..=1 is a thin
    /// fog everywhere; `sky_brightness` 0..=8 and `sun_glow` 0..=4 shape the
    /// sky color the fog takes.
    HeightFog { density: f32, falloff: f32, height: f32, haze: f32, sky_brightness: f32, sun_glow: f32 },
    /// Screen-space ambient occlusion: creases and contact areas darken, from
    /// the depth of the lit scene. `strength` 0..=2 (above 1 darkens more than the
    /// measured occlusion), `radius` 0.05..=10 meters.
    Ssao { strength: f32, radius: f32 },
    /// Cel shading of the finished picture: brightness falls into `bands`
    /// (2..=8) flat steps with `softness` 0..=1 between them; hue is kept.
    Toon { bands: f32, softness: f32 },
    /// Dark lines where depth jumps. `thickness` 0.5..=4 pixels, `sensitivity`
    /// 0.01..=1 is the relative depth step that draws a line (smaller draws more),
    /// `darkness` 0..=1.
    Outline { thickness: f32, sensitivity: f32, darkness: f32 },
    /// Fast approximate anti-aliasing of the finished picture. `strength` 0..=1.
    Fxaa { strength: f32 },
    /// Renders the scene at `scale` (0.5..=1) of the target size and upscales it
    /// with an edge-adaptive filter followed by contrast-adaptive sharpening, after
    /// AMD FidelityFX Super Resolution 1 (EASU and RCAS). `sharpness` 0..=1.
    Upscale { scale: f32, sharpness: f32 },
    /// Temporal anti-aliasing: the scene is drawn a different part of a pixel
    /// off every frame and blended with the previous frames, reprojected by
    /// depth through the camera's motion. `blend` 0.05..=0.5 is the share of
    /// the new frame. With [`ScreenEffect::Upscale`] the history is kept at the
    /// target size, which upscales over time instead of from one frame.
    Taa { blend: f32 },
}

/// Whether `effects` ask for temporal anti-aliasing, which jitters the scene.
pub fn temporal(effects: &[Option<ScreenEffect>]) -> bool {
    effects.iter().flatten().any(|e| matches!(e, ScreenEffect::Taa { .. }))
}

fn halton(mut index: u32, base: u32) -> f32 {
    let (mut result, mut fraction) = (0.0, 1.0);
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

/// Subpixel offset of frame `frame` for a scene of `size`: in clip space, and as
/// the texture-coordinate shift that brings a jittered picture back to place.
/// Sixteen frames of the Halton (2, 3) sequence.
pub(crate) fn jitter(frame: u32, size: (u32, u32)) -> ([f32; 2], [f32; 2]) {
    let index = frame % 16 + 1;
    let pixel = [halton(index, 2) - 0.5, halton(index, 3) - 0.5];
    let clip = [pixel[0] * 2.0 / size.0.max(1) as f32, pixel[1] * 2.0 / size.1.max(1) as f32];
    (clip, [clip[0] * 0.5, -clip[1] * 0.5])
}

/// `view_proj` with its picture moved by `clip` in clip space.
pub(crate) fn jittered(view_proj: [[f32; 4]; 4], clip: [f32; 2]) -> [[f32; 4]; 4] {
    let mut m = view_proj;
    for column in &mut m {
        column[0] += clip[0] * column[3];
        column[1] += clip[1] * column[3];
    }
    m
}

/// The size the scene is drawn at for a target of `size`: smaller when an
/// [`ScreenEffect::Upscale`] is in `effects`, otherwise `size`.
pub fn render_size(effects: &[Option<ScreenEffect>], size: (u32, u32)) -> (u32, u32) {
    let scale = effects.iter().flatten().find_map(|e| match e {
        ScreenEffect::Upscale { scale, .. } => Some(scale.clamp(0.5, 1.0)),
        _ => None,
    });
    let scaled = |n: u32, s: f32| ((n as f32 * s).round() as u32).clamp(1, n.max(1));
    scale.map_or(size, |s| (scaled(size.0, s), scaled(size.1, s)))
}

impl ScreenEffect {
    fn numbers(self) -> Vec<(f32, f32, f32)> {
        match self {
            Self::Ssao { strength, radius } => vec![(strength, 0.0, 2.0), (radius, 0.05, 10.0)],
            Self::Toon { bands, softness } => vec![(bands, 2.0, 8.0), (softness, 0.0, 1.0)],
            Self::Outline { thickness, sensitivity, darkness } => vec![(thickness, 0.5, 4.0), (sensitivity, 0.01, 1.0), (darkness, 0.0, 1.0)],
            Self::Fxaa { strength } => vec![(strength, 0.0, 1.0)],
            Self::Upscale { scale, sharpness } => vec![(scale, 0.5, 1.0), (sharpness, 0.0, 1.0)],
            Self::Taa { blend } => vec![(blend, 0.05, 0.5)],
            Self::LightShafts { strength, length } => vec![(strength, 0.0, 4.0), (length, 0.1, 1.0)],
            Self::AutoExposure { darkest, brightest, speed, target } => vec![(darkest, 0.1, 16.0), (brightest, 0.1, 16.0), (speed, 0.1, 20.0), (target, 0.01, 1.0)],
            Self::ColorGrade { saturation, contrast, warmth, tint } => vec![(saturation, 0.0, 2.0), (contrast, 0.0, 1.0), (warmth, -1.0, 1.0), (tint, -1.0, 1.0)],
            Self::Vignette { strength, radius, softness } => vec![(strength, 0.0, 1.0), (radius, 0.1, 1.5), (softness, 0.01, 1.0)],
            Self::FilmGrain { strength } => vec![(strength, 0.0, 1.0)],
            Self::HeightFog { density, falloff, height, haze, sky_brightness, sun_glow } => {
                vec![(density, 0.0, 0.05), (falloff, 0.0, 1.0), (height, -10000.0, 10000.0), (haze, 0.0, 1.0), (sky_brightness, 0.0, 8.0), (sun_glow, 0.0, 4.0)]
            }
        }
    }

    /// The effect with every number inside its range; a number that is not finite becomes the low end.
    /// Auto exposure's darkest is also kept at or below its brightest.
    pub fn clamped(self) -> Self {
        let c = |v: f32, low: f32, high: f32| if v.is_finite() { v.clamp(low, high) } else { low };
        match self {
            Self::LightShafts { strength, length } => Self::LightShafts { strength: c(strength, 0.0, 4.0), length: c(length, 0.1, 1.0) },
            Self::AutoExposure { darkest, brightest, speed, target } => {
                let brightest = c(brightest, 0.1, 16.0);
                Self::AutoExposure { darkest: c(darkest, 0.1, brightest), brightest, speed: c(speed, 0.1, 20.0), target: c(target, 0.01, 1.0) }
            }
            Self::ColorGrade { saturation, contrast, warmth, tint } => Self::ColorGrade { saturation: c(saturation, 0.0, 2.0), contrast: c(contrast, 0.0, 1.0), warmth: c(warmth, -1.0, 1.0), tint: c(tint, -1.0, 1.0) },
            Self::Vignette { strength, radius, softness } => Self::Vignette { strength: c(strength, 0.0, 1.0), radius: c(radius, 0.1, 1.5), softness: c(softness, 0.01, 1.0) },
            Self::FilmGrain { strength } => Self::FilmGrain { strength: c(strength, 0.0, 1.0) },
            Self::HeightFog { density, falloff, height, haze, sky_brightness, sun_glow } => Self::HeightFog {
                density: c(density, 0.0, 0.05),
                falloff: c(falloff, 0.0, 1.0),
                height: c(height, -10000.0, 10000.0),
                haze: c(haze, 0.0, 1.0),
                sky_brightness: c(sky_brightness, 0.0, 8.0),
                sun_glow: c(sun_glow, 0.0, 4.0),
            },
            Self::Ssao { strength, radius } => Self::Ssao { strength: c(strength, 0.0, 2.0), radius: c(radius, 0.05, 10.0) },
            Self::Toon { bands, softness } => Self::Toon { bands: c(bands, 2.0, 8.0), softness: c(softness, 0.0, 1.0) },
            Self::Outline { thickness, sensitivity, darkness } => {
                Self::Outline { thickness: c(thickness, 0.5, 4.0), sensitivity: c(sensitivity, 0.01, 1.0), darkness: c(darkness, 0.0, 1.0) }
            }
            Self::Fxaa { strength } => Self::Fxaa { strength: c(strength, 0.0, 1.0) },
            Self::Upscale { scale, sharpness } => Self::Upscale { scale: c(scale, 0.5, 1.0), sharpness: c(sharpness, 0.0, 1.0) },
            Self::Taa { blend } => Self::Taa { blend: c(blend, 0.05, 0.5) },
        }
    }

    /// Whether every number is finite and in its range, and auto exposure's darkest is not above its brightest.
    pub fn is_valid(self) -> bool {
        let in_range = self.numbers().into_iter().all(|(v, low, high)| v.is_finite() && (low..=high).contains(&v));
        in_range && !matches!(self, Self::AutoExposure { darkest, brightest, .. } if darkest > brightest)
    }
}

/// Bytes of the effect targets `effects` need for a scene drawn at `render`
/// into a target of `output`. Always: half-resolution shafts, the 64 x 36 meter
/// grid and two 1 x 1 exposure values. With SSAO, half-resolution occlusion;
/// with anti-aliasing or upscaling, the finished picture; with both, its
/// anti-aliased copy; with upscaling, the upscaled picture at the target size.
pub(crate) fn effect_bytes(render: (u32, u32), output: (u32, u32), effects: &[ScreenEffect]) -> Option<u64> {
    let asked = Asked::read(effects);
    let area = |size: (u32, u32)| u64::from(size.0).checked_mul(u64::from(size.1));
    let half = area((render.0.div_ceil(2), render.1.div_ceil(2)))?;
    let full = area(render)?;
    let mut texels = half.checked_add(64 * 36 + 2)?;
    if asked.ssao.is_some() {
        texels = texels.checked_add(half)?;
    }
    if asked.presenting() {
        texels = texels.checked_add(full)?;
    }
    if asked.fxaa.is_some() && asked.upscale.is_some() {
        texels = texels.checked_add(full)?;
    }
    if asked.upscale.is_some() {
        texels = texels.checked_add(area(output)?)?;
    }
    if asked.taa.is_some() {
        let history = area(if asked.upscale.is_some() { output } else { render })?;
        texels = texels.checked_add(history.checked_mul(2)?)?;
    }
    texels.checked_mul(8)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    display: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    exposure: [f32; 4],
    frame: [f32; 4],
    list: [[f32; 4]; 16],
    fog: [f32; 4],
    fog_sky: [f32; 4],
    eye: [f32; 4],
    sun_dir: [f32; 4],
    sun_tint: [f32; 4],
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    ao: [f32; 4],
    present: [f32; 4],
    sizes: [f32; 4],
    prev_view_proj: [[f32; 4]; 4],
    taa: [f32; 4],
}

/// The inverse of a column-major 4 x 4 matrix, or the identity when it has none.
fn inverse(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let a: [f64; 16] = std::array::from_fn(|i| f64::from(m[i / 4][i % 4]));
    let mut inv = [0.0f64; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15] + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15] - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15] + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14] - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15] - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15] + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15] - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14] + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15] + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15] - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15] + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14] - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11] - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11] + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11] - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10] + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det.abs() < 1e-30 || !det.is_finite() {
        return [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    }
    std::array::from_fn(|c| std::array::from_fn(|r| (inv[c * 4 + r] / det) as f32))
}

struct Color<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
    size: (u32, u32),
}

fn color<B: Rhi>(rhi: &B, label: &str, size: (u32, u32)) -> Color<B> {
    let texture = rhi.create_texture(&TextureDesc {
        label,
        width: size.0,
        height: size.1,
        format: TextureFormat::Rgba16Float,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
        sample_count: 1,
        view_formats: &[],
    });
    let view = rhi.create_texture_view(&texture, None);
    Color { _texture: texture, view, size }
}

/// Records one full-screen triangle with `pipeline` into `view`.
fn fullscreen<B: Rhi>(rhi: &B, encoder: &mut B::Encoder, label: &str, pipeline: &B::Pipeline, bind: &B::BindGroup, view: &B::TextureView) {
    rhi.encode_pass(
        encoder,
        label,
        Some(&ColorAttachment { view, clear: None, resolve: None }),
        None,
        &[Command::SetPipeline(pipeline), Command::SetBindGroup(0, bind), Command::Draw { vertices: 0..3, instances: 0..1 }],
    )
}

/// A color target made when a frame first needs it, and again when its size changes.
struct OnDemand<B: Rhi> {
    label: &'static str,
    color: RefCell<Option<Color<B>>>,
}

impl<B: Rhi> OnDemand<B> {
    fn new(label: &'static str) -> Self {
        Self { label, color: RefCell::new(None) }
    }

    fn bytes(&self) -> u64 {
        self.color.borrow().as_ref().map_or(0, |c| u64::from(c.size.0) * u64::from(c.size.1) * 8)
    }

    fn view(&self, rhi: &B, size: (u32, u32)) -> Ref<'_, B::TextureView> {
        if self.color.borrow().as_ref().is_none_or(|c| c.size != size) {
            *self.color.borrow_mut() = Some(color(rhi, self.label, size));
        }
        Ref::map(self.color.borrow(), |c| &c.as_ref().expect("made above").view)
    }
}

/// What the temporal history holds between frames.
#[derive(Clone, Copy)]
struct History {
    /// Which of the two histories the next frame writes.
    write: usize,
    /// Whether the other one holds a frame of `size`.
    valid: bool,
    /// The unjittered world-to-clip matrix of that frame.
    view_proj: [[f32; 4]; 4],
    size: (u32, u32),
}

/// GPU objects of the effects, made when a frame first asks for an effect.
pub(crate) struct EffectChain<B: Rhi> {
    shafts_pipeline: B::Pipeline,
    meter_pipeline: B::Pipeline,
    adapt_pipeline: B::Pipeline,
    ao_pipeline: B::Pipeline,
    final_pipeline: B::Pipeline,
    final_float_pipeline: B::Pipeline,
    fxaa_pipeline: B::Pipeline,
    fxaa_float_pipeline: B::Pipeline,
    easu_pipeline: B::Pipeline,
    rcas_pipeline: B::Pipeline,
    taa_pipeline: B::Pipeline,
    uniform: B::Buffer,
    shafts: Color<B>,
    ao: OnDemand<B>,
    grid: Color<B>,
    adapted: [Color<B>; 2],
    /// The finished picture before anti-aliasing or upscaling, its anti-aliased copy, and the upscaled picture.
    display: OnDemand<B>,
    aa: OnDemand<B>,
    upscaled: OnDemand<B>,
    /// Temporal history, written and read in turn.
    history: [OnDemand<B>; 2],
    temporal: Cell<History>,
    size: (u32, u32),
    /// Which of `adapted` the next frame writes, the time of the last frame, and a frame number.
    state: Cell<(usize, Option<Instant>, u32)>,
}

/// What the final pass needs from the frame.
pub(crate) struct FrameInputs<'a, B: Rhi> {
    pub scene: &'a B::TextureView,
    pub bloom: &'a B::TextureView,
    pub bloom_strength: f32,
    pub depth: &'a B::TextureView,
    pub sampler: &'a B::Sampler,
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 3],
    pub lighting: &'a Lighting,
    pub format: TextureFormat,
    /// Size of the target; larger than the scene when it is upscaled.
    pub output_size: (u32, u32),
    /// Texture-coordinate shift that undoes this frame's jitter.
    pub jitter: [f32; 2],
}

/// What each effect asks of this frame, read from the effect list.
struct Asked {
    shafts: Option<(f32, f32)>,
    auto: Option<(f32, f32, f32, f32)>,
    ssao: Option<(f32, f32)>,
    fxaa: Option<f32>,
    upscale: Option<f32>,
    taa: Option<f32>,
    fog: Option<([f32; 4], [f32; 4])>,
    list: [[f32; 4]; 16],
    count: usize,
}

impl Asked {
    fn read(effects: &[ScreenEffect]) -> Self {
        let mut asked = Self { shafts: None, auto: None, ssao: None, fxaa: None, upscale: None, taa: None, fog: None, list: [[0.0; 4]; 16], count: 0 };
        for effect in effects {
            // Effects on the finished picture keep list order; the first of a kind wins elsewhere.
            let entry = match *effect {
                ScreenEffect::LightShafts { strength, length } => {
                    asked.shafts.get_or_insert((strength, length));
                    None
                }
                ScreenEffect::AutoExposure { darkest, brightest, speed, target } => {
                    asked.auto.get_or_insert((darkest, brightest, speed, target));
                    None
                }
                ScreenEffect::HeightFog { density, falloff, height, haze, sky_brightness, sun_glow } => {
                    asked.fog.get_or_insert(([density, falloff, height, haze], [sky_brightness, sun_glow, 1.0, 0.0]));
                    None
                }
                ScreenEffect::Ssao { strength, radius } => {
                    asked.ssao.get_or_insert((strength, radius));
                    None
                }
                ScreenEffect::Fxaa { strength } => {
                    asked.fxaa.get_or_insert(strength);
                    None
                }
                ScreenEffect::Upscale { sharpness, .. } => {
                    asked.upscale.get_or_insert(sharpness);
                    None
                }
                ScreenEffect::Taa { blend } => {
                    asked.taa.get_or_insert(blend);
                    None
                }
                ScreenEffect::ColorGrade { saturation, contrast, warmth, tint } => Some(([4.0, saturation, contrast, warmth], tint)),
                ScreenEffect::Vignette { strength, radius, softness } => Some(([5.0, strength, radius, softness], 0.0)),
                ScreenEffect::FilmGrain { strength } => Some(([6.0, strength, 0.0, 0.0], 0.0)),
                ScreenEffect::Toon { bands, softness } => Some(([8.0, bands, softness, 0.0], 0.0)),
                ScreenEffect::Outline { thickness, sensitivity, darkness } => Some(([9.0, thickness, sensitivity, darkness], 0.0)),
            };
            if let Some((a, b)) = entry.filter(|_| asked.count < 8) {
                asked.list[asked.count * 2] = a;
                asked.list[asked.count * 2 + 1] = [b, 0.0, 0.0, 0.0];
                asked.count += 1;
            }
        }
        asked
    }

    /// Anti-aliasing and upscaling read the finished picture from a target of its own.
    fn presenting(&self) -> bool {
        self.fxaa.is_some() || self.upscale.is_some() || self.taa.is_some()
    }

    /// Size of the temporal history: the target size when it upscales.
    fn history_size(&self, render: (u32, u32), output: (u32, u32)) -> (u32, u32) {
        if self.upscale.is_some() { output } else { render }
    }
}

impl<B: Rhi> EffectChain<B> {
    pub fn new(rhi: &B, size: (u32, u32), format: TextureFormat) -> Self {
        let shader = rhi.create_shader("post effects", include_str!("shader_post_effects.wgsl"));
        let pipeline = |label, entry, format| rhi.create_pipeline(&PipelineDesc::color(label, &shader, ("vs_fullscreen", entry), &[], format, Blend::Opaque));
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        Self {
            shafts_pipeline: pipeline("post light shafts", "fs_shafts", TextureFormat::Rgba16Float),
            meter_pipeline: pipeline("post light meter", "fs_meter", TextureFormat::Rgba16Float),
            adapt_pipeline: pipeline("post eye adaptation", "fs_adapt", TextureFormat::Rgba16Float),
            ao_pipeline: pipeline("post ambient occlusion", "fs_ao", TextureFormat::Rgba16Float),
            final_pipeline: pipeline("post final with effects", "fs_final", format),
            final_float_pipeline: pipeline("post final before presenting", "fs_final", TextureFormat::Rgba16Float),
            fxaa_pipeline: pipeline("post fxaa", "fs_fxaa", format),
            fxaa_float_pipeline: pipeline("post fxaa before upscaling", "fs_fxaa_float", TextureFormat::Rgba16Float),
            easu_pipeline: pipeline("post edge-adaptive upscale", "fs_easu", TextureFormat::Rgba16Float),
            rcas_pipeline: pipeline("post contrast-adaptive sharpening", "fs_rcas", format),
            taa_pipeline: pipeline("post temporal anti-aliasing", "fs_taa", TextureFormat::Rgba16Float),
            uniform: rhi.create_buffer(&BufferDesc { label: "post effects", size: std::mem::size_of::<Uniform>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST }),
            shafts: color(rhi, "post light shafts", half),
            ao: OnDemand::new("post ambient occlusion"),
            grid: color(rhi, "post light meter grid", (64, 36)),
            adapted: [color(rhi, "post exposure a", (1, 1)), color(rhi, "post exposure b", (1, 1))],
            display: OnDemand::new("post finished picture"),
            aa: OnDemand::new("post anti-aliased picture"),
            upscaled: OnDemand::new("post upscaled picture"),
            history: [OnDemand::new("post temporal history a"), OnDemand::new("post temporal history b")],
            temporal: Cell::new(History { write: 0, valid: false, view_proj: [[0.0; 4]; 4], size: (0, 0) }),
            size,
            state: Cell::new((0, None, 0)),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Bytes of the targets made so far.
    pub fn bytes(&self) -> u64 {
        let fixed = u64::from(self.size.0.div_ceil(2)) * u64::from(self.size.1.div_ceil(2)) + 64 * 36 + 2;
        fixed * 8 + self.ao.bytes() + self.display.bytes() + self.aa.bytes() + self.upscaled.bytes() + self.history[0].bytes() + self.history[1].bytes()
    }

    fn uniform(&self, asked: &Asked, frame: &FrameInputs<'_, B>, adapt: f32, number: u32) -> Uniform {
        // The sun is where the direction toward it lands on the screen; behind the camera there are no shafts.
        let toward = frame.lighting.direction.map(|v| -v);
        let m = frame.view_proj;
        let clip: [f32; 4] = std::array::from_fn(|row| m[0][row] * toward[0] + m[1][row] * toward[1] + m[2][row] * toward[2]);
        let sun_uv = if clip[3] > 1e-4 { [clip[0] / clip[3] * 0.5 + 0.5, 0.5 - clip[1] / clip[3] * 0.5] } else { [0.0; 2] };
        // An orthographic matrix keeps w at 1: the sun at infinity has no place on its screen.
        let perspective = m[0][3].abs() + m[1][3].abs() + m[2][3].abs() > 1e-6;
        let shaft_strength = asked.shafts.filter(|_| perspective && clip[3] > 1e-4).map_or(0.0, |(s, _)| s);
        // Shafts take the sun's color, not its intensity: exposure already follows the lit scene.
        let sun_color = frame.lighting.color.map(|c| c * 1.3);
        // Before anti-aliasing or upscaling the picture stays display-encoded in a float target.
        let encode_srgb = asked.presenting() || !frame.format.is_srgb();
        Uniform {
            display: [frame.lighting.exposure, if frame.lighting.tonemap { 1.0 } else { 0.0 }, if encode_srgb { 1.0 } else { 0.0 }, frame.bloom_strength],
            sun: [sun_uv[0], sun_uv[1], shaft_strength, self.size.0 as f32 / self.size.1.max(1) as f32],
            sun_color: [sun_color[0], sun_color[1], sun_color[2], asked.shafts.map_or(0.97, |(_, length)| 0.94 + 0.045 * length)],
            exposure: asked.auto.map_or([0.0, 1.0, 1.0, 0.165], |(darkest, brightest, _, target)| [1.0, darkest, brightest, target]),
            frame: [adapt, (number % 1024) as f32, asked.count as f32, 0.0],
            list: asked.list,
            fog: asked.fog.map_or([0.0; 4], |(f, _)| f),
            fog_sky: asked.fog.map_or([0.0; 4], |(_, s)| s),
            eye: [frame.eye[0], frame.eye[1], frame.eye[2], 0.0],
            sun_dir: [toward[0], toward[1], toward[2], 0.0],
            sun_tint: [frame.lighting.color[0], frame.lighting.color[1], frame.lighting.color[2], 0.0],
            inv_view_proj: inverse(m),
            view_proj: m,
            ao: asked.ssao.map_or([0.0; 4], |(strength, radius)| [strength, radius, 1.0, 0.0]),
            present: [if frame.format.is_srgb() { 1.0 } else { 0.0 }, asked.upscale.unwrap_or(0.0), asked.fxaa.unwrap_or(0.0), 0.0],
            sizes: [self.size.0 as f32, self.size.1 as f32, frame.output_size.0 as f32, frame.output_size.1 as f32],
            prev_view_proj: self.temporal.get().view_proj,
            taa: [asked.taa.unwrap_or(1.0), if self.history_ready(asked, frame) { 1.0 } else { 0.0 }, frame.jitter[0], frame.jitter[1]],
        }
    }

    /// Whether the history holds a previous frame of the size this frame resolves at.
    fn history_ready(&self, asked: &Asked, frame: &FrameInputs<'_, B>) -> bool {
        let history = self.temporal.get();
        history.valid && history.size == asked.history_size(self.size, frame.output_size)
    }

    /// Records the effect passes and the final pass into `target`.
    pub fn encode(&self, rhi: &B, encoder: &mut B::Encoder, target: &B::TextureView, effects: &[ScreenEffect], frame: &FrameInputs<'_, B>) {
        let (write, last, number) = self.state.get();
        let now = Instant::now();
        let asked = Asked::read(effects);
        let adapt = match (asked.auto, last) {
            (Some((_, _, speed, _)), Some(last)) => 1.0 - (-speed * now.duration_since(last).as_secs_f32().min(1.0)).exp(),
            _ => 1.0,
        };
        let uniform = self.uniform(&asked, frame, adapt, number);
        rhi.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let pass = |encoder: &mut B::Encoder, label, pipeline, bind: &B::BindGroup, view| fullscreen(rhi, encoder, label, pipeline, bind, view);
        let from_depth = |pipeline| rhi.create_bind_group(pipeline, 0, &[Binding::Uniform { binding: 3, buffer: &self.uniform }, Binding::Texture { binding: 6, view: frame.depth }]);
        if uniform.sun[2] > 0.0 {
            pass(encoder, "post light shafts", &self.shafts_pipeline, &from_depth(&self.shafts_pipeline), &self.shafts.view);
        }
        let ao = asked.ssao.map(|_| self.ao.view(rhi, (self.size.0.div_ceil(2), self.size.1.div_ceil(2))));
        if let Some(ao) = ao.as_deref() {
            pass(encoder, "post ambient occlusion", &self.ao_pipeline, &from_depth(&self.ao_pipeline), ao);
        }
        let current = &self.adapted[write].view;
        if asked.auto.is_some() {
            let meter = rhi.create_bind_group(&self.meter_pipeline, 0, &[Binding::Texture { binding: 0, view: frame.scene }, Binding::Sampler { binding: 2, sampler: frame.sampler }]);
            pass(encoder, "post light meter", &self.meter_pipeline, &meter, &self.grid.view);
            let adapt = rhi.create_bind_group(&self.adapt_pipeline, 0, &[
                Binding::Uniform { binding: 3, buffer: &self.uniform },
                Binding::Texture { binding: 5, view: &self.grid.view },
                Binding::Texture { binding: 7, view: &self.adapted[1 - write].view },
            ]);
            pass(encoder, "post eye adaptation", &self.adapt_pipeline, &adapt, current);
        }
        let display = asked.presenting().then(|| self.display.view(rhi, self.size));
        let final_pipeline = if display.is_some() { &self.final_float_pipeline } else { &self.final_pipeline };
        let final_bind = rhi.create_bind_group(final_pipeline, 0, &[
            Binding::Texture { binding: 0, view: frame.scene },
            Binding::Texture { binding: 1, view: frame.bloom },
            Binding::Sampler { binding: 2, sampler: frame.sampler },
            Binding::Uniform { binding: 3, buffer: &self.uniform },
            Binding::Texture { binding: 4, view: &self.shafts.view },
            Binding::Texture { binding: 6, view: frame.depth },
            Binding::Texture { binding: 7, view: current },
            // Without SSAO the shader does not read binding 8; the meter grid stands in.
            Binding::Texture { binding: 8, view: ao.as_deref().unwrap_or(&self.grid.view) },
        ]);
        pass(encoder, "post final with effects", final_pipeline, &final_bind, display.as_deref().unwrap_or(target));
        if let Some(display) = display.as_deref() {
            self.present(rhi, encoder, target, display, &asked, frame);
        }
        let next = if asked.auto.is_some() { 1 - write } else { write };
        self.state.set((next, asked.auto.map(|_| now), number.wrapping_add(1)));
        let write = self.temporal.get().write;
        self.temporal.set(match asked.taa {
            Some(_) => History { write: 1 - write, valid: true, view_proj: frame.view_proj, size: asked.history_size(self.size, frame.output_size) },
            None => History { write, valid: false, view_proj: frame.view_proj, size: (0, 0) },
        });
    }

    /// Anti-aliases and upscales the finished `display` picture into `target`.
    fn present(&self, rhi: &B, encoder: &mut B::Encoder, target: &B::TextureView, display: &B::TextureView, asked: &Asked, frame: &FrameInputs<'_, B>) {
        let pass = |encoder: &mut B::Encoder, label, pipeline, bind: &B::BindGroup, view| fullscreen(rhi, encoder, label, pipeline, bind, view);
        let read = |pipeline, source| {
            rhi.create_bind_group(pipeline, 0, &[
                Binding::Sampler { binding: 2, sampler: frame.sampler },
                Binding::Uniform { binding: 3, buffer: &self.uniform },
                Binding::Texture { binding: 9, view: source },
            ])
        };
        let load = |pipeline, source| rhi.create_bind_group(pipeline, 0, &[Binding::Uniform { binding: 3, buffer: &self.uniform }, Binding::Texture { binding: 9, view: source }]);
        let resolved = asked.taa.map(|_| self.resolve(rhi, encoder, display, asked, frame));
        let source = resolved.as_deref().unwrap_or(display);
        if asked.upscale.is_some() && resolved.is_none() {
            let aa = asked.fxaa.map(|_| self.aa.view(rhi, self.size));
            if let Some(aa) = aa.as_deref() {
                pass(encoder, "post fxaa", &self.fxaa_float_pipeline, &read(&self.fxaa_float_pipeline, source), aa);
            }
            let upscaled = self.upscaled.view(rhi, frame.output_size);
            pass(encoder, "post edge-adaptive upscale", &self.easu_pipeline, &load(&self.easu_pipeline, aa.as_deref().unwrap_or(source)), &upscaled);
            pass(encoder, "post contrast-adaptive sharpening", &self.rcas_pipeline, &load(&self.rcas_pipeline, &upscaled), target);
        } else if asked.fxaa.is_some() {
            pass(encoder, "post fxaa", &self.fxaa_pipeline, &read(&self.fxaa_pipeline, source), target);
        } else {
            // Sharpening is zero without an upscale: this pass only stores the picture in the target.
            pass(encoder, "post contrast-adaptive sharpening", &self.rcas_pipeline, &load(&self.rcas_pipeline, source), target);
        }
    }

    /// Blends `display` into the temporal history and returns the history written.
    fn resolve(&self, rhi: &B, encoder: &mut B::Encoder, display: &B::TextureView, asked: &Asked, frame: &FrameInputs<'_, B>) -> Ref<'_, B::TextureView> {
        let write = self.temporal.get().write;
        let size = asked.history_size(self.size, frame.output_size);
        let out = self.history[write].view(rhi, size);
        let previous = self.history[1 - write].view(rhi, size);
        let bind = rhi.create_bind_group(&self.taa_pipeline, 0, &[
            Binding::Sampler { binding: 2, sampler: frame.sampler },
            Binding::Uniform { binding: 3, buffer: &self.uniform },
            Binding::Texture { binding: 6, view: frame.depth },
            Binding::Texture { binding: 9, view: display },
            Binding::Texture { binding: 10, view: &previous },
        ]);
        fullscreen(rhi, encoder, "post temporal anti-aliasing", &self.taa_pipeline, &bind, &out);
        out
    }
}
