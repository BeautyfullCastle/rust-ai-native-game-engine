//! Post effects beyond bloom: light shafts, auto exposure and the effects on
//! the finished picture (color grade, vignette, film grain). They run inside
//! the HDR post chain of [`crate::PostProcessor`]; exposure and tone mapping
//! still happen once, in the final pass.
use crate::Lighting;
use orr_rhi::{Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUsage};
use std::cell::Cell;
use std::time::Instant;

/// Slots in [`crate::PostProcessSettings::effects`].
pub const MAX_SCREEN_EFFECTS: usize = 8;

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
}

impl ScreenEffect {
    fn numbers(self) -> Vec<(f32, f32, f32)> {
        match self {
            Self::LightShafts { strength, length } => vec![(strength, 0.0, 4.0), (length, 0.1, 1.0)],
            Self::AutoExposure { darkest, brightest, speed, target } => vec![(darkest, 0.1, 16.0), (brightest, 0.1, 16.0), (speed, 0.1, 20.0), (target, 0.01, 1.0)],
            Self::ColorGrade { saturation, contrast, warmth, tint } => vec![(saturation, 0.0, 2.0), (contrast, 0.0, 1.0), (warmth, -1.0, 1.0), (tint, -1.0, 1.0)],
            Self::Vignette { strength, radius, softness } => vec![(strength, 0.0, 1.0), (radius, 0.1, 1.5), (softness, 0.01, 1.0)],
            Self::FilmGrain { strength } => vec![(strength, 0.0, 1.0)],
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
        }
    }

    /// Whether every number is finite and in its range, and auto exposure's darkest is not above its brightest.
    pub fn is_valid(self) -> bool {
        let in_range = self.numbers().into_iter().all(|(v, low, high)| v.is_finite() && (low..=high).contains(&v));
        in_range && !matches!(self, Self::AutoExposure { darkest, brightest, .. } if darkest > brightest)
    }
}

/// Bytes of the effect targets for a scene of `size`: half-resolution
/// shafts, the 64 x 36 meter grid and two 1 x 1 exposure values.
pub(crate) fn effect_bytes(size: (u32, u32)) -> Option<u64> {
    let half = u64::from(size.0.div_ceil(2)).checked_mul(u64::from(size.1.div_ceil(2)))?;
    half.checked_add(64 * 36 + 2)?.checked_mul(8)
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
}

struct Color<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
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
    Color { _texture: texture, view }
}

/// GPU objects of the effects, made when a frame first asks for an effect.
pub(crate) struct EffectChain<B: Rhi> {
    shafts_pipeline: B::Pipeline,
    meter_pipeline: B::Pipeline,
    adapt_pipeline: B::Pipeline,
    final_pipeline: B::Pipeline,
    uniform: B::Buffer,
    shafts: Color<B>,
    grid: Color<B>,
    adapted: [Color<B>; 2],
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
    pub lighting: &'a Lighting,
    pub format: TextureFormat,
}

impl<B: Rhi> EffectChain<B> {
    pub fn new(rhi: &B, size: (u32, u32), format: TextureFormat) -> Self {
        let shader = rhi.create_shader("post effects", include_str!("shader_post_effects.wgsl"));
        let pipeline = |label, entry, format| rhi.create_pipeline(&PipelineDesc::color(label, &shader, ("vs_fullscreen", entry), &[], format, Blend::Opaque));
        Self {
            shafts_pipeline: pipeline("post light shafts", "fs_shafts", TextureFormat::Rgba16Float),
            meter_pipeline: pipeline("post light meter", "fs_meter", TextureFormat::Rgba16Float),
            adapt_pipeline: pipeline("post eye adaptation", "fs_adapt", TextureFormat::Rgba16Float),
            final_pipeline: pipeline("post final with effects", "fs_final", format),
            uniform: rhi.create_buffer(&BufferDesc { label: "post effects", size: std::mem::size_of::<Uniform>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST }),
            shafts: color(rhi, "post light shafts", (size.0.div_ceil(2), size.1.div_ceil(2))),
            grid: color(rhi, "post light meter grid", (64, 36)),
            adapted: [color(rhi, "post exposure a", (1, 1)), color(rhi, "post exposure b", (1, 1))],
            size,
            state: Cell::new((0, None, 0)),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Records the effect passes and the final pass into `target`.
    pub fn encode(&self, rhi: &B, encoder: &mut B::Encoder, target: &B::TextureView, effects: &[ScreenEffect], frame: &FrameInputs<'_, B>) {
        let (write, last, number) = self.state.get();
        let now = Instant::now();
        let shafts = effects.iter().find_map(|e| match e {
            ScreenEffect::LightShafts { strength, length } => Some((*strength, *length)),
            _ => None,
        });
        let auto = effects.iter().find_map(|e| match e {
            ScreenEffect::AutoExposure { darkest, brightest, speed, target } => Some((*darkest, *brightest, *speed, *target)),
            _ => None,
        });
        // The sun is where the direction toward it lands on the screen; behind the camera there are no shafts.
        let toward = frame.lighting.direction.map(|v| -v);
        let m = frame.view_proj;
        let clip: [f32; 4] = std::array::from_fn(|row| m[0][row] * toward[0] + m[1][row] * toward[1] + m[2][row] * toward[2]);
        let sun_uv = if clip[3] > 1e-4 { [clip[0] / clip[3] * 0.5 + 0.5, 0.5 - clip[1] / clip[3] * 0.5] } else { [0.0; 2] };
        let shaft_strength = shafts.filter(|_| clip[3] > 1e-4).map_or(0.0, |(s, _)| s);
        let sun_color = frame.lighting.color.map(|c| c * frame.lighting.intensity);
        let adapt = match (auto, last) {
            (Some((_, _, speed, _)), Some(last)) => 1.0 - (-speed * now.duration_since(last).as_secs_f32().min(1.0)).exp(),
            _ => 1.0,
        };
        let mut list = [[0.0; 4]; 16];
        let mut count = 0;
        for effect in effects {
            let entry = match *effect {
                ScreenEffect::ColorGrade { saturation, contrast, warmth, tint } => Some(([4.0, saturation, contrast, warmth], tint)),
                ScreenEffect::Vignette { strength, radius, softness } => Some(([5.0, strength, radius, softness], 0.0)),
                ScreenEffect::FilmGrain { strength } => Some(([6.0, strength, 0.0, 0.0], 0.0)),
                _ => None,
            };
            if let Some((a, b)) = entry {
                list[count * 2] = a;
                list[count * 2 + 1] = [b, 0.0, 0.0, 0.0];
                count += 1;
            }
        }
        let uniform = Uniform {
            display: [frame.lighting.exposure, if frame.lighting.tonemap { 1.0 } else { 0.0 }, if frame.format.is_srgb() { 0.0 } else { 1.0 }, frame.bloom_strength],
            sun: [sun_uv[0], sun_uv[1], shaft_strength, self.size.0 as f32 / self.size.1.max(1) as f32],
            sun_color: [sun_color[0], sun_color[1], sun_color[2], shafts.map_or(0.97, |(_, length)| 0.94 + 0.045 * length)],
            exposure: auto.map_or([0.0, 1.0, 1.0, 0.165], |(darkest, brightest, _, target)| [1.0, darkest, brightest, target]),
            frame: [adapt, (number % 1024) as f32, count as f32, 0.0],
            list,
        };
        rhi.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let pass = |encoder: &mut B::Encoder, label, pipeline, bind: &B::BindGroup, view| {
            rhi.encode_pass(
                encoder,
                label,
                Some(&ColorAttachment { view, clear: None, resolve: None }),
                None,
                &[Command::SetPipeline(pipeline), Command::SetBindGroup(0, bind), Command::Draw { vertices: 0..3, instances: 0..1 }],
            )
        };
        if shaft_strength > 0.0 {
            let bind = rhi.create_bind_group(&self.shafts_pipeline, 0, &[Binding::Uniform { binding: 3, buffer: &self.uniform }, Binding::Texture { binding: 6, view: frame.depth }]);
            pass(encoder, "post light shafts", &self.shafts_pipeline, &bind, &self.shafts.view);
        }
        let current = &self.adapted[write].view;
        if auto.is_some() {
            let meter = rhi.create_bind_group(&self.meter_pipeline, 0, &[Binding::Texture { binding: 0, view: frame.scene }, Binding::Sampler { binding: 2, sampler: frame.sampler }]);
            pass(encoder, "post light meter", &self.meter_pipeline, &meter, &self.grid.view);
            let adapt = rhi.create_bind_group(&self.adapt_pipeline, 0, &[
                Binding::Uniform { binding: 3, buffer: &self.uniform },
                Binding::Texture { binding: 5, view: &self.grid.view },
                Binding::Texture { binding: 7, view: &self.adapted[1 - write].view },
            ]);
            pass(encoder, "post eye adaptation", &self.adapt_pipeline, &adapt, current);
        }
        let final_bind = rhi.create_bind_group(&self.final_pipeline, 0, &[
            Binding::Texture { binding: 0, view: frame.scene },
            Binding::Texture { binding: 1, view: frame.bloom },
            Binding::Sampler { binding: 2, sampler: frame.sampler },
            Binding::Uniform { binding: 3, buffer: &self.uniform },
            Binding::Texture { binding: 4, view: &self.shafts.view },
            Binding::Texture { binding: 7, view: current },
        ]);
        pass(encoder, "post final with effects", &self.final_pipeline, &final_bind, target);
        let next = if auto.is_some() { 1 - write } else { write };
        self.state.set((next, auto.map(|_| now), number.wrapping_add(1)));
    }
}
